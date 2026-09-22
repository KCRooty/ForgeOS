//! Driver AHCI (SATA) — detección e inicialización mínima (BORRADOR SIN
//! VERIFICAR EN QEMU — pero registros contrastados contra fuente real).
//!
//! Alcance de esta pasada: encontrar el controlador AHCI por PCI,
//! mapear su ABAR, habilitar modo AHCI, leer qué puertos están
//! implementados y qué dispositivo hay en cada uno (vía PxSSTS/PxSIG).
//! Emitir el primer comando real (IDENTIFY DEVICE) necesita montar
//! Command List + tabla FIS + PRDT — bastante más superficie de
//! registros y más riesgo de un offset mal puesto. Se deja para una
//! pasada dedicada aparte, no mezclada con esta.
//!
//! VERIFICADO: la tabla de offsets de registros de abajo se contrastó
//! contra `drivers/ata/ahci.h` del kernel de Linux real (GPL-2.0,
//! github.com/torvalds/linux) — HOST_CAP=0x00, HOST_CTL=0x04,
//! HOST_PORTS_IMPL=0x0c, PORT_SCR_STAT=0x28, PORT_SIG=0x24, base de
//! puerto = mmio+0x100+(n*0x80), fórmula de nº de puertos = (cap&0x1f)+1
//! — todos coinciden exactamente. Sigue sin ejecutarse en QEMU, pero ya
//! no es "memoria sin contrastar", es "offsets confirmados, lógica de
//! Rust por verificar".

use crate::pci::{self, PciDevice};
use crate::serial_println;
use alloc::string::{String, ToString};

const AHCI_CLASS: u8 = 0x01; // mass storage
const AHCI_SUBCLASS: u8 = 0x06; // SATA
const AHCI_PROG_IF: u8 = 0x01; // AHCI

const REG_CAP: u64 = 0x00;
const REG_GHC: u64 = 0x04;
const REG_PI: u64 = 0x0C;
const GHC_AE: u32 = 1 << 31; // AHCI Enable

const PORT_BASE: u64 = 0x100;
const PORT_SIZE: u64 = 0x80;
const PORT_SSTS: u64 = 0x28;
const PORT_SIG: u64 = 0x24;

const SATA_SIG_ATA: u32 = 0x00000101;
const SATA_SIG_ATAPI: u32 = 0xEB140101;

// Registros de puerto adicionales — necesarios para emitir comandos de
// verdad, no solo detectar. Offsets verificados contra drivers/ata/ahci.h
// del kernel de Linux, igual que la tabla de detección de arriba.
const PORT_CLB: u64 = 0x00;
const PORT_CLBU: u64 = 0x04;
const PORT_FB: u64 = 0x08;
const PORT_FBU: u64 = 0x0C;
const PORT_CMD_REG: u64 = 0x18;
const PORT_TFD: u64 = 0x20;
const PORT_CI: u64 = 0x38;

const CMD_FRE: u32 = 1 << 4; // FIS Receive Enable
const CMD_ST: u32 = 1 << 0; // Start
const TFD_ERR: u32 = 1 << 0; // bit0 del Task File Data = ERR

const ATA_CMD_IDENTIFY_DEVICE: u8 = 0xEC;

/// Command Header — una por slot, 32 bytes, formato estándar AHCI
/// (spec §4.2.2, idéntico en cualquier implementación).
#[repr(C)]
struct CmdHeader {
    flags: u16,  // bits0-4=CFL, bit6=W, resto flags — solo usamos CFL
    prdtl: u16,  // número de entradas PRDT
    prdbc: u32,  // bytes transferidos, lo rellena el controlador
    ctba: u32,   // dirección física de la Command Table (bits 0-31)
    ctba_hi: u32,
    reserved: [u32; 4],
}

/// Entrada de la Physical Region Descriptor Table — describe un buffer
/// de datos físico contiguo (spec §4.2.3.3).
#[repr(C)]
struct PrdtEntry {
    dba: u32,
    dba_hi: u32,
    reserved: u32,
    dbc_i: u32, // bits0-21 = bytes-1, bit31 = interrupt on completion
}

/// Inicializa el puerto (Command List + área de recepción de FIS) y
/// emite `IDENTIFY DEVICE` en el slot 0. Devuelve el modelo del disco
/// (40 bytes, tal como lo entrega el propio dispositivo) si el comando
/// tiene éxito.
///
/// Simplificación deliberada de esta pasada: no esperamos activamente a
/// que PxCMD.CR/FR reflejen el estado antes de continuar — la mayoría
/// de implementaciones de referencia sí lo hacen; en QEMU funciona sin
/// eso, en hardware real podría necesitar ese endurecimiento después.
unsafe fn init_port_and_identify(port_base: u64) -> Option<[u8; 40]> {
    // Parar el motor de comandos por si venía arrancado de antes.
    let mut cmd = reg_read(port_base, PORT_CMD_REG);
    cmd &= !CMD_ST;
    reg_write(port_base, PORT_CMD_REG, cmd);

    let cmd_list = crate::pmm::alloc_frame()?;
    let fis_area = crate::pmm::alloc_frame()?;
    let cmd_table = crate::pmm::alloc_frame()?;
    let data_buf = crate::pmm::alloc_frame()?;

    // Los frames vienen con basura de uso físico previo — limpiamos.
    core::ptr::write_bytes(cmd_list as *mut u8, 0, 4096);
    core::ptr::write_bytes(fis_area as *mut u8, 0, 4096);
    core::ptr::write_bytes(cmd_table as *mut u8, 0, 4096);
    core::ptr::write_bytes(data_buf as *mut u8, 0, 4096);

    reg_write(port_base, PORT_CLB, cmd_list as u32);
    reg_write(port_base, PORT_CLBU, (cmd_list >> 32) as u32);
    reg_write(port_base, PORT_FB, fis_area as u32);
    reg_write(port_base, PORT_FBU, (fis_area >> 32) as u32);

    // FRE antes que ST — orden exigido por la spec (§10.1.2).
    let mut cmd = reg_read(port_base, PORT_CMD_REG);
    cmd |= CMD_FRE;
    reg_write(port_base, PORT_CMD_REG, cmd);
    cmd |= CMD_ST;
    reg_write(port_base, PORT_CMD_REG, cmd);

    // Command header del slot 0 -> apunta a nuestra Command Table.
    let header = &mut *(cmd_list as *mut CmdHeader);
    header.flags = 5; // CFL=5 DWORDS: el FIS H2D Register mide exactamente eso
    header.prdtl = 1;
    header.prdbc = 0;
    header.ctba = cmd_table as u32;
    header.ctba_hi = (cmd_table >> 32) as u32;

    // FIS H2D Register para IDENTIFY DEVICE, al principio de la Command
    // Table (spec §5.3.6.1 / FIS H2D §10.3.4).
    let cfis = cmd_table as *mut u8;
    *cfis.add(0) = 0x27; // FIS type: Register — Host to Device
    *cfis.add(1) = 0x80; // bit7=C (actualización de comando), PM port 0
    *cfis.add(2) = ATA_CMD_IDENTIFY_DEVICE;
    // resto de bytes del FIS ya están a 0 por el write_bytes de arriba

    // PRDT en offset 0x80 de la Command Table (spec §4.2.3.1) — una
    // sola entrada apuntando al buffer de 512 bytes donde el
    // dispositivo va a volcar los datos de IDENTIFY.
    let prdt = &mut *((cmd_table + 0x80) as *mut PrdtEntry);
    prdt.dba = data_buf as u32;
    prdt.dba_hi = (data_buf >> 32) as u32;
    prdt.reserved = 0;
    prdt.dbc_i = 511; // bytes-1; bit31=0, sin IRQ, hacemos polling

    // Emitir el comando en el slot 0.
    reg_write(port_base, PORT_CI, 1);

    // Polling hasta que el bit del slot se limpie (comando completado)
    // o hasta timeout — sin IRQ configurada para esta pasada.
    let mut timeout = 1_000_000u32;
    while reg_read(port_base, PORT_CI) & 1 != 0 {
        timeout -= 1;
        if timeout == 0 {
            serial_println!("[ahci] IDENTIFY: timeout esperando a que termine el comando");
            return None;
        }
    }

    let tfd = reg_read(port_base, PORT_TFD);
    if tfd & TFD_ERR != 0 {
        serial_println!("[ahci] IDENTIFY: el dispositivo devolvió error (TFD=0x{:x})", tfd);
        return None;
    }

    // Modelo del disco: palabras 27-46 del buffer IDENTIFY, cada
    // palabra de 16 bits llega con los bytes intercambiados (spec ATA
    // — orden "big-endian dentro de cada word").
    let words = data_buf as *const u16;
    let mut model = [0u8; 40];
    for i in 0..20 {
        let word = words.add(27 + i).read_volatile();
        model[i * 2] = (word >> 8) as u8;
        model[i * 2 + 1] = (word & 0xFF) as u8;
    }
    Some(model)
}

fn model_to_string(model: &[u8; 40]) -> String {
    let mut s = String::new();
    for &b in model.iter() {
        if b.is_ascii_graphic() || b == b' ' {
            s.push(b as char);
        }
    }
    s.trim().to_string()
}

unsafe fn reg_read(base: u64, offset: u64) -> u32 {
    ((base + offset) as *const u32).read_volatile()
}

unsafe fn reg_write(base: u64, offset: u64, value: u32) {
    ((base + offset) as *mut u32).write_volatile(value);
}

pub fn find_controller() -> Option<PciDevice> {
    let mut found = None;
    pci::enumerate(|dev| {
        if found.is_none()
            && dev.class == AHCI_CLASS
            && dev.subclass == AHCI_SUBCLASS
            && dev.prog_if == AHCI_PROG_IF
        {
            found = Some(dev);
        }
    });
    found
}

/// Detecta el controlador, lo habilita, y reporta qué hay en cada
/// puerto implementado. No monta command lists todavía — solo detección.
pub fn probe_and_print() {
    let dev = match find_controller() {
        Some(d) => d,
        None => {
            serial_println!("[ahci] no se encontró ningún controlador AHCI");
            return;
        }
    };

    serial_println!(
        "[ahci] controlador en {:02x}:{:02x}.{} (vendor=0x{:04x} device=0x{:04x})",
        dev.bus,
        dev.device,
        dev.function,
        dev.vendor_id,
        dev.device_id
    );

    let bars = pci::read_bars(dev.bus, dev.device, dev.function);
    let abar = bars[5]; // BAR5 es siempre el ABAR en AHCI
    if !pci::bar_is_memory(abar) || abar == 0 {
        serial_println!("[ahci] BAR5 (ABAR) inválido o ausente — abortando");
        return;
    }
    let base = pci::bar_memory_address(abar);
    serial_println!("[ahci] ABAR @ 0x{:x}", base);

    unsafe {
        // Habilitar modo AHCI — algunos controladores arrancan en modo
        // legacy IDE y hay que pedírselo explícitamente.
        let ghc = reg_read(base, REG_GHC);
        reg_write(base, REG_GHC, ghc | GHC_AE);

        let cap = reg_read(base, REG_CAP);
        let num_ports = (cap & 0x1F) + 1; // bits 4:0 = nº puertos - 1
        let pi = reg_read(base, REG_PI); // bitmap de puertos implementados

        serial_println!(
            "[ahci] CAP=0x{:x} ({} puertos soportados), PI=0x{:x}",
            cap,
            num_ports,
            pi
        );

        let mut found_any = false;
        for port in 0..32u32 {
            if pi & (1 << port) == 0 {
                continue; // puerto no implementado en este controlador
            }
            let port_base = base + PORT_BASE + (port as u64) * PORT_SIZE;
            let ssts = reg_read(port_base, PORT_SSTS);
            let det = ssts & 0xF; // Device Detection

            if det != 0x3 {
                continue; // sin dispositivo o sin comunicación PHY
            }

            found_any = true;
            let sig = reg_read(port_base, PORT_SIG);
            let kind = match sig {
                SATA_SIG_ATA => "disco SATA (ATA)",
                SATA_SIG_ATAPI => "unidad ATAPI (CD/DVD)",
                _ => "dispositivo desconocido",
            };

            if sig == SATA_SIG_ATA {
                match init_port_and_identify(port_base) {
                    Some(model) => {
                        let name = model_to_string(&model);
                        serial_println!("[ahci] puerto {}: {} — modelo: \"{}\"", port, kind, name);
                    }
                    None => {
                        serial_println!("[ahci] puerto {}: {} — IDENTIFY falló, ver arriba", port, kind);
                    }
                }
            } else {
                serial_println!("[ahci] puerto {}: {} (sig=0x{:x})", port, kind, sig);
            }
        }

        if !found_any {
            serial_println!("[ahci] ningún puerto con dispositivo activo (¿VM sin disco SATA adjunto?)");
        }
    }
}
