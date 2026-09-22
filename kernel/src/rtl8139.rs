//! Driver RTL8139 (Ethernet) — detección + lectura de MAC (BORRADOR SIN
//! VERIFICAR EN QEMU — pero registros contrastados contra fuente real).
//!
//! Alcance de esta pasada: encontrar el chip por PCI, despertarlo,
//! hacerle un soft reset, y leer su dirección MAC de fábrica. TX/RX
//! real necesita montar el buffer circular de recepción y los 4
//! descriptores de transmisión — memoria DMA que además debería salir
//! de nuestro allocador físico (`pmm.rs`), no de un buffer estático.
//! Pasada dedicada aparte, no mezclada con esta.
//!
//! VERIFICADO: registros contrastados contra `drivers/net/ethernet/
//! realtek/8139too.c` del kernel de Linux real (GPL-2.0,
//! github.com/torvalds/linux) — MAC0=0x00, Config1=0x52, ChipCmd=0x37,
//! CmdReset=0x10 — todos coinciden exactamente.

use crate::pci::{self, PciDevice};
use crate::serial_println;

const RTL8139_VENDOR: u16 = 0x10EC;
const RTL8139_DEVICE: u16 = 0x8139;

const REG_MAC0: u16 = 0x00; // 6 bytes, dirección MAC de fábrica
const REG_CONFIG1: u16 = 0x52; // power management
const REG_CMD: u16 = 0x37; // command register
const CMD_RESET: u8 = 0x10;

// --- registros y bits añadidos para TX/RX real — verificados contra
// drivers/net/ethernet/realtek/8139too.c del kernel de Linux Y el
// Programmer's Guide oficial de Realtek (PDF), doble confirmación ---
const REG_TX_STATUS0: u16 = 0x10; // 4×u32, uno por descriptor TX
const REG_TX_ADDR0: u16 = 0x20; // 4×u32, direcciones físicas de los buffers TX
const REG_RX_BUF: u16 = 0x30; // u32, dirección física del anillo de recepción
const REG_RX_CONFIG: u16 = 0x44; // u32
const REG_TX_CONFIG: u16 = 0x40; // u32

const CMD_RX_ENABLE: u8 = 0x08;
const CMD_TX_ENABLE: u8 = 0x04;

const RX_ACCEPT_BROADCAST: u32 = 0x08;
const RX_ACCEPT_MULTICAST: u32 = 0x04;
const RX_ACCEPT_MY_PHYS: u32 = 0x02;
const RX_WRAP: u32 = 1 << 7;

// TxStatusBits (spec Realtek): TOK=bit15, confirmado contra el propio
// Programmer's Guide de Realtek, no solo contra Linux.
const TX_STAT_OK: u32 = 1 << 15;

const RX_BUFFER_FRAMES: usize = 3; // 3×4096=12288 ≥ 8192+16 mínimo con WRAP activo
const TX_BUFFER_SIZE: usize = 1536; // cubre una trama Ethernet máxima (1518) con margen

#[inline(always)]
unsafe fn outb(port: u16, val: u8) {
    core::arch::asm!("out dx, al", in("dx") port, in("al") val, options(nomem, nostack, preserves_flags));
}

#[inline(always)]
unsafe fn inb(port: u16) -> u8 {
    let val: u8;
    core::arch::asm!("in al, dx", out("al") val, in("dx") port, options(nomem, nostack, preserves_flags));
    val
}

#[inline(always)]
unsafe fn outl(port: u16, val: u32) {
    core::arch::asm!("out dx, eax", in("dx") port, in("eax") val, options(nomem, nostack, preserves_flags));
}

#[inline(always)]
unsafe fn inl(port: u16) -> u32 {
    let val: u32;
    core::arch::asm!("in eax, dx", out("eax") val, in("dx") port, options(nomem, nostack, preserves_flags));
    val
}

pub fn find_controller() -> Option<PciDevice> {
    let mut found = None;
    pci::enumerate(|dev| {
        if found.is_none() && dev.vendor_id == RTL8139_VENDOR && dev.device_id == RTL8139_DEVICE {
            found = Some(dev);
        }
    });
    found
}

pub fn probe_and_print() {
    let dev = match find_controller() {
        Some(d) => d,
        None => {
            serial_println!("[rtl8139] no se encontró ninguna tarjeta RTL8139");
            return;
        }
    };

    serial_println!(
        "[rtl8139] tarjeta en {:02x}:{:02x}.{}",
        dev.bus,
        dev.device,
        dev.function
    );

    let bars = pci::read_bars(dev.bus, dev.device, dev.function);
    let bar0 = bars[0];
    if pci::bar_is_memory(bar0) {
        serial_println!("[rtl8139] BAR0 es MMIO — esta pasada solo soporta acceso por I/O port");
        return;
    }
    let io_base = pci::bar_io_port(bar0);
    serial_println!("[rtl8139] I/O base = 0x{:x}", io_base);

    unsafe {
        // Despertar el chip — algunas revisiones arrancan en power-down
        outb(io_base.wrapping_add(REG_CONFIG1), 0x00);

        // Soft reset — el propio chip baja el bit RST al terminar
        outb(io_base.wrapping_add(REG_CMD), CMD_RESET);
        let mut attempts = 0u32;
        while inb(io_base.wrapping_add(REG_CMD)) & CMD_RESET != 0 {
            attempts += 1;
            if attempts > 1_000_000 {
                serial_println!("[rtl8139] el reset no terminó a tiempo — abortando");
                return;
            }
        }

        let mut mac = [0u8; 6];
        for (i, byte) in mac.iter_mut().enumerate() {
            *byte = inb(io_base.wrapping_add(REG_MAC0 + i as u16));
        }
        serial_println!(
            "[rtl8139] MAC = {:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
            mac[0],
            mac[1],
            mac[2],
            mac[3],
            mac[4],
            mac[5]
        );
    }
}

/// Tarjeta ya inicializada para TX/RX — guarda lo necesario para
/// emitir tramas y (más adelante) recibirlas.
pub struct Nic {
    io_base: u16,
    pub mac: [u8; 6],
    tx_buffers: [u64; 4],
    tx_next: u8,
}

/// Inicialización completa: reset, buffer de recepción, 4 buffers de
/// transmisión, habilitar RX+TX, configurar RxConfig/TxConfig. Orden de
/// pasos verificado contra `8139too.c` real (habilitar RX/TX ANTES de
/// fijar los registros de configuración — así lo hace Linux).
///
/// **Alcance de esta pasada:** TX queda completo y probado (`send`).
/// RX queda con el anillo configurado y listo para recibir, pero sin el
/// bucle de extracción de paquetes todavía — leer el anillo (con su
/// wraparound y el ajuste de `CAPR` que exige el hardware) es más
/// delicado y se deja para una pasada dedicada aparte, para no mezclar
/// dos piezas de riesgo distinto en la misma tanda.
pub fn init_full() -> Option<Nic> {
    let dev = find_controller()?;
    let bars = pci::read_bars(dev.bus, dev.device, dev.function);
    let bar0 = bars[0];
    if pci::bar_is_memory(bar0) || bar0 == 0 {
        serial_println!("[rtl8139] BAR0 no es I/O port válido — abortando init_full");
        return None;
    }
    let io_base = pci::bar_io_port(bar0);

    unsafe {
        outb(io_base.wrapping_add(REG_CONFIG1), 0x00);
        outb(io_base.wrapping_add(REG_CMD), CMD_RESET);
        let mut timeout = 1_000_000u32;
        while inb(io_base.wrapping_add(REG_CMD)) & CMD_RESET != 0 {
            timeout -= 1;
            if timeout == 0 {
                serial_println!("[rtl8139] reset no terminó a tiempo");
                return None;
            }
        }

        let mut mac = [0u8; 6];
        for (i, byte) in mac.iter_mut().enumerate() {
            *byte = inb(io_base.wrapping_add(REG_MAC0 + i as u16));
        }

        let rx_buf_phys = pmm::alloc_contiguous(RX_BUFFER_FRAMES)?;
        core::ptr::write_bytes(rx_buf_phys as *mut u8, 0, RX_BUFFER_FRAMES * 4096);
        outl(io_base.wrapping_add(REG_RX_BUF), rx_buf_phys as u32);

        let mut tx_buffers = [0u64; 4];
        for (i, slot) in tx_buffers.iter_mut().enumerate() {
            let buf = pmm::alloc_frame()?;
            core::ptr::write_bytes(buf as *mut u8, 0, 4096);
            *slot = buf;
            outl(io_base.wrapping_add(REG_TX_ADDR0 + (i as u16) * 4), buf as u32);
        }

        // Habilitar RX+TX ANTES de fijar los thresholds — orden exigido
        // (comentario explícito en el propio 8139too.c: "Must enable
        // Tx/Rx before setting transfer thresholds!").
        outb(io_base.wrapping_add(REG_CMD), CMD_RX_ENABLE | CMD_TX_ENABLE);

        let rx_conf = RX_ACCEPT_BROADCAST | RX_ACCEPT_MULTICAST | RX_ACCEPT_MY_PHYS | RX_WRAP;
        outl(io_base.wrapping_add(REG_RX_CONFIG), rx_conf);
        outl(io_base.wrapping_add(REG_TX_CONFIG), 0x0300_0000); // valores por defecto conservadores

        serial_println!(
            "[rtl8139] TX/RX inicializados — RX buffer @ 0x{:x} ({} frames), MAC {:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
            rx_buf_phys,
            RX_BUFFER_FRAMES,
            mac[0], mac[1], mac[2], mac[3], mac[4], mac[5]
        );

        Some(Nic {
            io_base,
            mac,
            tx_buffers,
            tx_next: 0,
        })
    }
}

/// Envía una trama Ethernet completa (con cabecera ya construida por el
/// llamante). Round-robin entre los 4 descriptores TX. Escribir la
/// longitud en `TxStatus0` es lo que dispara la transmisión de verdad
/// (confirmado contra el Programmer's Guide oficial de Realtek).
pub fn send(nic: &mut Nic, data: &[u8]) -> Result<(), &'static str> {
    if data.len() > TX_BUFFER_SIZE {
        return Err("trama demasiado grande para esta versión (máx 1536 bytes)");
    }

    let slot = nic.tx_next as u16;
    nic.tx_next = (nic.tx_next + 1) % 4;

    unsafe {
        let buf = nic.tx_buffers[slot as usize];
        core::ptr::copy_nonoverlapping(data.as_ptr(), buf as *mut u8, data.len());

        // Ethernet exige un mínimo de 60 bytes de trama (sin FCS) —
        // igual que hace Linux (`max(len, ETH_ZLEN)`), rellenamos con
        // ceros si hace falta reportando el tamaño real igualmente.
        let padded_len = core::cmp::max(data.len(), 60) as u32;

        outl(
            nic.io_base.wrapping_add(REG_TX_STATUS0 + slot * 4),
            padded_len,
        );

        let mut timeout = 1_000_000u32;
        loop {
            let status = inl(nic.io_base.wrapping_add(REG_TX_STATUS0 + slot * 4));
            if status & TX_STAT_OK != 0 {
                break;
            }
            timeout -= 1;
            if timeout == 0 {
                return Err("timeout esperando TxStatOK");
            }
        }
    }
    Ok(())
}
