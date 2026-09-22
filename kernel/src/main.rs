//! Forge OS — kernel entry point
//! M0: boot.asm nos entrega ya en Long Mode con paginación identity-map
//! del primer GiB. Aquí arranca el kernel real en Rust.

#![no_std]
#![no_main]

mod serial;

use core::panic::PanicInfo;

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    serial_println!("[PANIC] {}", info);
    loop {
        unsafe { core::arch::asm!("hlt") };
    }
}

/// Llamado desde boot.asm (`long_mode_start`) con RDI = puntero Multiboot2.
#[no_mangle]
pub extern "C" fn kernel_main_upper(mb2_info_ptr: u64) -> ! {
    serial_println!("Forge OS — M0 boot OK");
    serial_println!("Multiboot2 info @ 0x{:x}", mb2_info_ptr);
    serial_println!("Ring 0, Long Mode, paginación identity-map activa.");

    // TODO M1: GDT/TSS propias, IDT + excepciones, PIC/APIC
    // TODO M2: allocator físico, heap del kernel
    // TODO M3: framebuffer (Multiboot2 tag de vídeo) + texto en pantalla
    // TODO M4: scheduler cooperativo mínimo

    loop {
        unsafe { core::arch::asm!("hlt") };
    }
}
