//! Pila de red mínima: ARP (RFC 826) + IPv4 + ICMP echo (ping).
//!
//! Alcance: justo lo necesario para mandar un ARP request, procesar su
//! reply, y hacer un ping ICMP completo de ida y vuelta sobre el driver
//! RTL8139. Sin DHCP, sin routing real — IP propia y gateway fijados a
//! mano por el llamante (en pruebas: gateway 10.0.2.2, el que da QEMU
//! con `-net user` por defecto).
//!
//! Construcción de tramas a mano, sin structs `#[repr(C)]` — mismo
//! estilo que `elf.rs`: más verboso pero cero dudas sobre padding o
//! endianness implícita del compilador.

use alloc::vec::Vec;

pub type Mac = [u8; 6];
pub type Ipv4 = [u8; 4];

const ETHERTYPE_ARP: u16 = 0x0806;
const ETHERTYPE_IPV4: u16 = 0x0800;

const ARP_HTYPE_ETHERNET: u16 = 1;
const ARP_PTYPE_IPV4: u16 = 0x0800;
const ARP_OP_REQUEST: u16 = 1;
const ARP_OP_REPLY: u16 = 2;

const IP_PROTO_ICMP: u8 = 1;

const ICMP_TYPE_ECHO_REQUEST: u8 = 8;
const ICMP_TYPE_ECHO_REPLY: u8 = 0;

const BROADCAST_MAC: Mac = [0xFF; 6];

/// Internet checksum (RFC 1071) — complemento a uno de la suma en
/// complemento a uno de palabras de 16 bits, con el acarreo plegado de
/// vuelta hasta que cabe en 16 bits. Mismo algoritmo para la cabecera
/// IPv4 y para ICMP, solo cambia sobre qué bytes se calcula.
pub fn inet_csum(data: &[u8]) -> u16 {
    let mut sum: u32 = 0;
    let mut i = 0;
    while i + 1 < data.len() {
        sum += u16::from_be_bytes([data[i], data[i + 1]]) as u32;
        i += 2;
    }
    if i < data.len() {
        // byte impar suelto al final: se cuenta como la mitad alta de
        // una palabra de 16 bits con la mitad baja a cero
        sum += (data[i] as u32) << 8;
    }
    while sum >> 16 != 0 {
        sum = (sum & 0xFFFF) + (sum >> 16);
    }
    !(sum as u16)
}

fn write_eth_header(buf: &mut Vec<u8>, dst: Mac, src: Mac, ethertype: u16) {
    buf.extend_from_slice(&dst);
    buf.extend_from_slice(&src);
    buf.extend_from_slice(&ethertype.to_be_bytes());
}

// --- ARP ---------------------------------------------------------------

/// Tabla ARP mínima: array estático de tamaño fijo, sin heap ni locks —
/// el kernel es cooperativo y de un solo core, así que no hay carrera
/// real entre lecturas/escrituras mientras no se ceda el turno en medio.
const ARP_TABLE_SIZE: usize = 8;
static mut ARP_TABLE: [Option<(Ipv4, Mac)>; ARP_TABLE_SIZE] = [None; ARP_TABLE_SIZE];

pub fn arp_insert(ip: Ipv4, mac: Mac) {
    unsafe {
        for slot in ARP_TABLE.iter_mut() {
            if let Some((existing_ip, _)) = slot {
                if *existing_ip == ip {
                    *slot = Some((ip, mac));
                    return;
                }
            }
        }
        for slot in ARP_TABLE.iter_mut() {
            if slot.is_none() {
                *slot = Some((ip, mac));
                return;
            }
        }
        // tabla llena: sin LRU en esta versión, simplemente no se
        // aprende ninguna entrada nueva más hasta que se libere hueco
    }
}

pub fn arp_lookup(ip: Ipv4) -> Option<Mac> {
    unsafe {
        for slot in ARP_TABLE.iter() {
            if let Some((existing_ip, mac)) = slot {
                if *existing_ip == ip {
                    return Some(*mac);
                }
            }
        }
        None
    }
}

/// Construye un ARP request completo (trama Ethernet incluida) pidiendo
/// la MAC de `target_ip`. `send()` en rtl8139.rs ya rellena hasta el
/// mínimo de 60 bytes de Ethernet, así que los 42 bytes reales de este
/// frame (14 Ethernet + 28 ARP) no hace falta rellenarlos aquí.
pub fn build_arp_request(src_mac: Mac, src_ip: Ipv4, target_ip: Ipv4) -> Vec<u8> {
    let mut frame = Vec::with_capacity(42);
    write_eth_header(&mut frame, BROADCAST_MAC, src_mac, ETHERTYPE_ARP);

    frame.extend_from_slice(&ARP_HTYPE_ETHERNET.to_be_bytes());
    frame.extend_from_slice(&ARP_PTYPE_IPV4.to_be_bytes());
    frame.push(6); // hlen: tamaño de una MAC
    frame.push(4); // plen: tamaño de una IPv4
    frame.extend_from_slice(&ARP_OP_REQUEST.to_be_bytes());
    frame.extend_from_slice(&src_mac); // sender hardware address
    frame.extend_from_slice(&src_ip); // sender protocol address
    frame.extend_from_slice(&[0u8; 6]); // target hardware address: desconocida, es lo que preguntamos
    frame.extend_from_slice(&target_ip); // target protocol address

    frame
}

pub struct ArpReply {
    pub sender_ip: Ipv4,
    pub sender_mac: Mac,
}

/// Interpreta un paquete ya recibido (sin la FCS, tal y como lo entrega
/// `rtl8139::poll_rx`) como ARP reply. `None` si no es Ethernet+ARP
/// reply válido — payloads de otro protocolo, o de otro paso del propio
/// ARP (por ejemplo un request ajeno) no cuentan.
pub fn parse_arp_reply(packet: &[u8]) -> Option<ArpReply> {
    if packet.len() < 42 {
        return None;
    }
    let ethertype = u16::from_be_bytes([packet[12], packet[13]]);
    if ethertype != ETHERTYPE_ARP {
        return None;
    }
    let op = u16::from_be_bytes([packet[20], packet[21]]);
    if op != ARP_OP_REPLY {
        return None;
    }

    let mut sender_mac = [0u8; 6];
    sender_mac.copy_from_slice(&packet[22..28]);
    let mut sender_ip = [0u8; 4];
    sender_ip.copy_from_slice(&packet[28..32]);

    Some(ArpReply {
        sender_ip,
        sender_mac,
    })
}

// --- IPv4 + ICMP ---------------------------------------------------------

/// Payload de 32 bytes para el echo request — sin significado especial,
/// solo relleno reconocible en un volcado de paquetes (74 bytes totales
/// de trama: 14 Ethernet + 20 IPv4 + 8 ICMP + 32 payload, el tamaño
/// clásico de un `ping` de Linux por defecto).
const PING_PAYLOAD: [u8; 32] = *b"forgeos-ping-0123456789abcdefghi";

/// Construye un ICMP Echo Request completo (Ethernet + IPv4 + ICMP)
/// listo para pasar a `rtl8139::send`.
pub fn build_ping(
    src_mac: Mac,
    src_ip: Ipv4,
    dst_mac: Mac,
    dst_ip: Ipv4,
    ident: u16,
    seq: u16,
) -> Vec<u8> {
    // --- ICMP primero: su checksum cubre solo el propio ICMP, así que
    // puede calcularse antes de saber nada de la cabecera IPv4 ---
    let mut icmp = Vec::with_capacity(8 + PING_PAYLOAD.len());
    icmp.push(ICMP_TYPE_ECHO_REQUEST);
    icmp.push(0); // code
    icmp.extend_from_slice(&[0u8, 0u8]); // checksum: placeholder, se rellena después
    icmp.extend_from_slice(&ident.to_be_bytes());
    icmp.extend_from_slice(&seq.to_be_bytes());
    icmp.extend_from_slice(&PING_PAYLOAD);

    let icmp_csum = inet_csum(&icmp);
    icmp[2..4].copy_from_slice(&icmp_csum.to_be_bytes());

    // --- IPv4 ---
    let total_length: u16 = 20 + icmp.len() as u16;
    let mut ip = Vec::with_capacity(20);
    ip.push(0x45); // version 4, IHL 5 (20 bytes, sin opciones)
    ip.push(0x00); // DSCP/ECN
    ip.extend_from_slice(&total_length.to_be_bytes());
    ip.extend_from_slice(&0x0001u16.to_be_bytes()); // identification, valor fijo arbitrario
    ip.extend_from_slice(&0x0000u16.to_be_bytes()); // flags/fragment offset: sin fragmentar
    ip.push(64); // TTL
    ip.push(IP_PROTO_ICMP);
    ip.extend_from_slice(&[0u8, 0u8]); // header checksum: placeholder
    ip.extend_from_slice(&src_ip);
    ip.extend_from_slice(&dst_ip);

    let ip_csum = inet_csum(&ip);
    ip[10..12].copy_from_slice(&ip_csum.to_be_bytes());

    // --- Ethernet + ensamblado final ---
    let mut frame = Vec::with_capacity(14 + ip.len() + icmp.len());
    write_eth_header(&mut frame, dst_mac, src_mac, ETHERTYPE_IPV4);
    frame.extend_from_slice(&ip);
    frame.extend_from_slice(&icmp);

    frame
}

pub struct IcmpEchoReply {
    pub src_ip: Ipv4,
    pub id: u16,
    pub seq: u16,
}

/// Interpreta un paquete recibido como ICMP Echo Reply dirigido a
/// nosotros. `None` para cualquier otra cosa: ARP, TCP/UDP, un echo
/// *request* ajeno (por ejemplo si QEMU nos hiciera ping a nosotros),
/// o un paquete demasiado corto para ser válido.
pub fn parse_icmp_echo_reply(packet: &[u8]) -> Option<IcmpEchoReply> {
    if packet.len() < 14 + 20 + 8 {
        return None;
    }
    let ethertype = u16::from_be_bytes([packet[12], packet[13]]);
    if ethertype != ETHERTYPE_IPV4 {
        return None;
    }

    let ip_start = 14;
    let ihl = (packet[ip_start] & 0x0F) as usize * 4;
    if ihl < 20 || packet.len() < ip_start + ihl + 8 {
        return None;
    }
    let protocol = packet[ip_start + 9];
    if protocol != IP_PROTO_ICMP {
        return None;
    }

    let mut src_ip = [0u8; 4];
    src_ip.copy_from_slice(&packet[ip_start + 12..ip_start + 16]);

    let icmp_start = ip_start + ihl;
    let icmp_type = packet[icmp_start];
    if icmp_type != ICMP_TYPE_ECHO_REPLY {
        return None;
    }

    let id = u16::from_be_bytes([packet[icmp_start + 4], packet[icmp_start + 5]]);
    let seq = u16::from_be_bytes([packet[icmp_start + 6], packet[icmp_start + 7]]);

    Some(IcmpEchoReply { src_ip, id, seq })
}
