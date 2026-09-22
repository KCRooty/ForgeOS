//! Enumeración PCI (BORRADOR SIN VERIFICAR).
//!
//! Mecanismo de configuración PCI clásico #1 (I/O ports 0xCF8/0xCFC) —
//! funciona igual en QEMU y en hardware real. MMCONFIG (PCIe extendido)
//! es una mejora posterior opcional, no un requisito para arrancar.
//!
//! Sin esto, ningún driver real (disco, red, audio, GPU — sección 4 de
//! TODO.md) puede ni encontrar su propio dispositivo. Es el próximo
//! cuello de botella del tablero, según lo dejamos anotado.

use crate::serial_println;

const CONFIG_ADDRESS: u16 = 0xCF8;
const CONFIG_DATA: u16 = 0xCFC;

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

fn config_address(bus: u8, device: u8, function: u8, offset: u8) -> u32 {
    (1u32 << 31)
        | ((bus as u32) << 16)
        | ((device as u32) << 11)
        | ((function as u32) << 8)
        | ((offset as u32) & 0xFC)
}

/// Lee un dword de 32 bits del espacio de configuración de un
/// dispositivo PCI. `offset` debe ir alineado a 4 bytes.
pub fn read_config(bus: u8, device: u8, function: u8, offset: u8) -> u32 {
    unsafe {
        outl(CONFIG_ADDRESS, config_address(bus, device, function, offset));
        inl(CONFIG_DATA)
    }
}

#[derive(Clone, Copy)]
pub struct PciDevice {
    pub bus: u8,
    pub device: u8,
    pub function: u8,
    pub vendor_id: u16,
    pub device_id: u16,
    pub class: u8,
    pub subclass: u8,
    pub prog_if: u8,
}

fn probe_function(bus: u8, device: u8, function: u8) -> Option<PciDevice> {
    let word0 = read_config(bus, device, function, 0x00);
    let vendor_id = (word0 & 0xFFFF) as u16;
    if vendor_id == 0xFFFF {
        return None; // sin dispositivo en este slot/función
    }
    let device_id = (word0 >> 16) as u16;

    let word2 = read_config(bus, device, function, 0x08);
    let class = (word2 >> 24) as u8;
    let subclass = (word2 >> 16) as u8;
    let prog_if = (word2 >> 8) as u8;

    Some(PciDevice {
        bus,
        device,
        function,
        vendor_id,
        device_id,
        class,
        subclass,
        prog_if,
    })
}

fn header_type(bus: u8, device: u8, function: u8) -> u8 {
    let word3 = read_config(bus, device, function, 0x0C);
    ((word3 >> 16) & 0xFF) as u8
}

/// Recorre los 256 buses × 32 dispositivos × 8 funciones posibles.
/// Fuerza bruta, sin seguir puentes PCI-a-PCI todavía — suficiente para
/// encontrar lo que QEMU expone por defecto en el bus 0.
pub fn enumerate(mut on_device: impl FnMut(PciDevice)) {
    for bus in 0u16..=255 {
        let bus = bus as u8;
        for device in 0..32u8 {
            if let Some(dev) = probe_function(bus, device, 0) {
                on_device(dev);

                // header type con el bit 7 puesto = multi-función,
                // hace falta mirar las funciones 1-7 también
                if header_type(bus, device, 0) & 0x80 != 0 {
                    for function in 1..8u8 {
                        if let Some(dev) = probe_function(bus, device, function) {
                            on_device(dev);
                        }
                    }
                }
            }
        }
    }
}

/// Recorre el bus e imprime lo encontrado por serie.
pub fn scan_and_print() {
    let mut count = 0u32;
    enumerate(|dev| {
        count += 1;
        serial_println!(
            "[pci] {:02x}:{:02x}.{} vendor=0x{:04x} device=0x{:04x} class={:02x}.{:02x} prog_if=0x{:02x}",
            dev.bus,
            dev.device,
            dev.function,
            dev.vendor_id,
            dev.device_id,
            dev.class,
            dev.subclass,
            dev.prog_if
        );
    });
    serial_println!("[pci] {} dispositivo(s) encontrados", count);
}
