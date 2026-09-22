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
            serial_println!("[ahci] puerto {}: {} (sig=0x{:x})", port, kind, sig);
        }

        if !found_any {
            serial_println!("[ahci] ningún puerto con dispositivo activo (¿VM sin disco SATA adjunto?)");
        }
    }
}
