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

/// Lee los 6 Base Address Registers (BAR0-BAR5) de una función PCI —
/// offsets 0x10 a 0x24. Cada BAR nos dice dónde vive la memoria MMIO
/// (o el rango de puertos I/O) de ese dispositivo. Sin esto, un driver
/// real no tiene forma de saber a qué dirección escribir.
///
/// Bit 0 del BAR distingue el tipo:
/// - 0 = memory-mapped (BAR con la dirección física en bits 4-31,
///   bits 1-2 indican si es de 32 o 64 bits)
/// - 1 = I/O space (dirección de puerto en bits 2-31)
///
/// Devolvemos el valor crudo — decodificarlo del todo (tamaño del
/// rango, si es 64-bit y ocupa dos BARs consecutivos) es tarea del
/// driver concreto que lo use, cuando exista.
pub fn read_bars(bus: u8, device: u8, function: u8) -> [u32; 6] {
    let mut bars = [0u32; 6];
    for (i, bar) in bars.iter_mut().enumerate() {
        *bar = read_config(bus, device, function, 0x10 + (i as u8) * 4);
    }
    bars
}

/// `true` si el BAR es de memoria (MMIO), `false` si es de I/O ports.
pub fn bar_is_memory(bar: u32) -> bool {
    bar & 0x1 == 0
}

/// Para un BAR de memoria: la dirección física, con los bits de flags
/// ya enmascarados fuera.
pub fn bar_memory_address(bar: u32) -> u64 {
    (bar & 0xFFFFFFF0) as u64
}

/// Para un BAR de I/O: el puerto base, con el bit de flag fuera.
pub fn bar_io_port(bar: u32) -> u16 {
    (bar & 0xFFFFFFFC) as u16
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

/// Recorre el bus e imprime lo encontrado por serie, incluyendo BARs.
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

        for (i, &bar) in read_bars(dev.bus, dev.device, dev.function).iter().enumerate() {
            if bar == 0 {
                continue; // BAR sin usar
            }
            if bar_is_memory(bar) {
                serial_println!("       BAR{} = MMIO @ 0x{:x}", i, bar_memory_address(bar));
            } else {
                serial_println!("       BAR{} = I/O port 0x{:x}", i, bar_io_port(bar));
            }
        }
    });
    serial_println!("[pci] {} dispositivo(s) encontrados", count);
}
