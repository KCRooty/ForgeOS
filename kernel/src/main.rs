//! Forge OS — kernel entry point
//! M0: boot.asm nos entrega ya en Long Mode con paginación identity-map
//! del primer GiB. Aquí arranca el kernel real en Rust.

#![no_std]
#![no_main]
#![feature(abi_x86_interrupt)]
#![feature(alloc_error_handler)]

extern crate alloc;

mod caps;
mod console;
mod font;
mod framebuffer;
mod gdt;
mod heap;
mod idt;
mod mb2;
mod pic;
mod pmm;
mod serial;

use alloc::boxed::Box;
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

    // M1b — GDT/TSS propia + IDT con manejadores de excepción.
    gdt::init();
    serial_println!("[gdt] cargada, TSS con pila IST para double fault");
    idt::init();

    pic::init();
    serial_println!("[pic] remapeado a vectores 32-47, todo enmascarado (sin drivers de IRQ aún)");

    // M2 — allocador físico + heap del kernel.
    unsafe { pmm::init(mb2_info_ptr) };
    serial_println!("[pmm] {} frames libres tras reservar kernel + primer MiB", pmm::free_frame_count());

    // Prueba end-to-end del heap: un Box real. Si esto imprime el valor
    // correcto, `#[global_allocator]` funciona de extremo a extremo, no
    // solo que compila.
    let boxed = Box::new(0xC0FFEEu64);
    serial_println!("[heap] Box::new(0xC0FFEE) = 0x{:x} (dirección: {:p})", *boxed, boxed);

    // Prueba end-to-end: disparamos un breakpoint por software (#BP) y
    // comprobamos que el handler se ejecuta y la CPU sigue viva después.
    // Si esto imprime la línea siguiente, el IDT funciona de verdad.
    unsafe { core::arch::asm!("int3") };
    serial_println!("[idt] tras int3: seguimos vivos — el IDT funciona");

    // M1 — demo del modelo de capabilities. Todavía no hay tabla de
    // procesos (eso es M4), así que probamos el mecanismo con un único
    // objeto de prueba: nace sin restricciones, se pledgea a un
    // subconjunto mínimo, y verificamos que el enforcement funciona en
    // ambos sentidos (permite lo pledgeado, deniega lo demás).
    let mut proc_caps = caps::Capabilities::unrestricted();
    proc_caps
        .pledge(caps::CAP_STDIO | caps::CAP_FS_READ)
        .expect("pledge inicial desde unrestricted no debería fallar nunca");
    serial_println!("[caps] pledge: CAP_STDIO | CAP_FS_READ");

    match proc_caps.enforce(caps::CAP_STDIO) {
        Ok(()) => serial_println!("[caps] STDIO -> permitido (correcto)"),
        Err(_) => serial_println!("[caps] STDIO -> denegado (¡ERROR, no debería pasar!)"),
    }

    match proc_caps.enforce(caps::CAP_NET) {
        Ok(()) => serial_println!("[caps] NET -> permitido (¡ERROR, no debería pasar!)"),
        Err(v) => serial_println!(
            "[caps] NET -> denegado (correcto — pedido=0x{:x}, otorgado=0x{:x})",
            v.requested,
            v.granted
        ),
    }

    // Intentar ampliar después de pledgear debe fallar siempre — es la
    // garantía central del modelo (principio 1 de PHILOSOPHY.md).
    match proc_caps.pledge(caps::CAP_STDIO | caps::CAP_FS_READ | caps::CAP_NET) {
        Ok(()) => serial_println!("[caps] ampliar pledge -> permitido (¡ERROR, no debería pasar!)"),
        Err(_) => serial_println!("[caps] ampliar pledge -> denegado (correcto, es irreversible)"),
    }

    // M3 — framebuffer + patrón de prueba visual. "Parche" funcional:
    // demuestra que se escribe de verdad en pantalla, nada de estética.
    unsafe { framebuffer::init(mb2_info_ptr) };
    framebuffer::test_pattern();
    framebuffer::draw_str(20, 20, "FORGE OS BOOT OK", 0x00000000, 4);

    // TODO M2b: heap real con free-list (recuperar memoria de dealloc)
    // TODO M2c: syscalls reales — aquí `caps::enforce` pasa a llamarse
    //           por cada una
    // TODO M3c: ampliar el alfabeto de font.rs más allá de F/O/R/G/E/S/B/T/K
    // TODO M4: scheduler + tabla de procesos; shell real ("Bellows")
    // TODO M6+: Anvil + terminal ("Crucible")

    serial_println!("");
    serial_println!("Boot completo — entrando en la consola de depuración.");
    console::run()
}
