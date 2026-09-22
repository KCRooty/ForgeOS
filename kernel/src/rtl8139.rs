//! Driver RTL8139 (Ethernet) — detección + lectura de MAC (BORRADOR SIN
//! VERIFICAR).
//!
//! Alcance de esta pasada: encontrar el chip por PCI, despertarlo,
//! hacerle un soft reset, y leer su dirección MAC de fábrica. TX/RX
//! real necesita montar el buffer circular de recepción y los 4
//! descriptores de transmisión — memoria DMA que además debería salir
//! de nuestro allocador físico (`pmm.rs`), no de un buffer estático.
//! Pasada dedicada aparte, no mezclada con esta.

use crate::pci::{self, PciDevice};
use crate::serial_println;

const RTL8139_VENDOR: u16 = 0x10EC;
const RTL8139_DEVICE: u16 = 0x8139;

const REG_MAC0: u16 = 0x00; // 6 bytes, dirección MAC de fábrica
const REG_CONFIG1: u16 = 0x52; // power management
const REG_CMD: u16 = 0x37; // command register
const CMD_RESET: u8 = 0x10;

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
