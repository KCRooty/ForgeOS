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
mod apic;
mod ahci;
mod elf;
mod font;
mod framebuffer;
mod gdt;
mod heap;
mod idt;
mod keyboard;
mod mb2;
mod mmu;
mod pci;
mod pic;
mod pipe;
mod pmm;
mod process;
mod psf;
mod ring3;
mod rtl8139;
mod syscall;
mod vfs;
mod scheduler;
mod serial;
mod task;

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

    keyboard::init(); // desenmascara IRQ1 — no llegará nada hasta el `sti` de más abajo

    // M2 — allocador físico + heap del kernel.
    unsafe { pmm::init(mb2_info_ptr) };
    serial_println!("[pmm] {} frames libres tras reservar kernel + primer MiB", pmm::free_frame_count());

    // EFER.NXE antes de que nada use mmu::map_page con executable=false
    // — el bit NO_EXECUTE de una PTE es "reservado" sin esto, y usarlo
    // hace page fault (visto en la primera prueba real de M4c).
    unsafe { mmu::init() };
    serial_println!("[mmu] EFER.NXE habilitado");

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

    // M4a — primer cambio de contexto real. Dos tareas cooperativas se
    // turnan con la propia función de arranque (tarea 0 implícita).
    scheduler::init();
    scheduler::spawn(task_a);
    scheduler::spawn(task_b);
    serial_println!(
        "[sched] {} tareas registradas — cediendo turno 6 veces",
        scheduler::task_count()
    );
    for _ in 0..6 {
        scheduler::yield_now();
    }
    serial_println!("[sched] de vuelta en boot — si viste A/B intercalados arriba, el cambio de contexto funciona");

    // M4b (primer paso) — Local APIC + timer periódico. Momento delicado:
    // es la PRIMERA vez en todo el kernel que activamos interrupciones de
    // hardware de verdad (`sti`). Si algo en IDT/PIC/APIC está mal, este
    // es el punto donde se notaría — un GPF en bucle, o el timer que
    // nunca llega.
    unsafe {
        apic::init();
        apic::start_periodic_timer(10_000_000); // valor arbitrario, sin calibrar
        serial_println!("[apic] activando interrupciones (sti) por primera vez en el kernel");
        core::arch::asm!("sti");

        while apic::tick_count() < 5 {
            core::arch::asm!("hlt"); // espera eficiente, se despierta con cada interrupción
        }
    }
    serial_println!("[apic] {} ticks recibidos — el timer de hardware funciona de verdad", apic::tick_count());

    // Enumeración PCI — próximo cuello de botella del tablero: sin esto
    // ningún driver real (disco, red, audio, GPU) puede encontrar su
    // propio dispositivo.
    pci::scan_and_print();

    // Primeros drivers reales — nivel "detecta y reporta", no
    // funcionalidad completa todavía (ver cabecera de cada fichero).
    ahci::probe_and_print();
    rtl8139::probe_and_print();

    // M4c — gestor de memoria virtual. Prueba real: mapear una
    // dirección virtual bien fuera del identity-map de 4 GiB de
    // boot.asm, escribir, leer, y comprobar que coincide.
    unsafe {
        match pmm::alloc_frame() {
            Some(test_phys) => {
                let test_virt: u64 = 0x0000_0050_0000_0000; // 320 GiB — fuera del identity-map
                match mmu::map_page(test_virt, test_phys, true, false) {
                    Ok(()) => {
                        let ptr = test_virt as *mut u64;
                        ptr.write_volatile(0xDEAD_BEEF_CAFEu64);
                        let readback = ptr.read_volatile();
                        if readback == 0xDEAD_BEEF_CAFE {
                            serial_println!(
                                "[mmu] map_page funciona: escrito y releído 0x{:x} en virt=0x{:x}",
                                readback,
                                test_virt
                            );
                        } else {
                            serial_println!("[mmu] map_page: valor releído NO coincide — algo va mal");
                        }
                        let _ = mmu::unmap_page(test_virt);
                    }
                    Err(e) => serial_println!("[mmu] map_page falló: {}", e),
                }
            }
            None => serial_println!("[mmu] sin memoria física para la prueba"),
        }
    }

    // M4d — uniendo M4a+M4b+M4c: un espacio de direcciones REAL, no
    // solo hilos de kernel compartiendo el mapa global. Comparte el
    // kernel (P4[0]) a propósito — así las syscalls/interrupciones
    // siguen funcionando sin importar qué proceso esté activo.
    unsafe {
        if let Some(new_space) = mmu::create_address_space() {
            serial_println!("[proc] espacio de direcciones nuevo: PML4 @ 0x{:x}", new_space);

            match pmm::alloc_frame() {
                Some(priv_phys) => {
                    let priv_virt: u64 = 0x0000_0060_0000_0000; // 384 GiB — privado de este espacio
                    match mmu::map_page_in(new_space, priv_virt, priv_phys, true, false) {
                        Ok(()) => serial_println!("[proc] mapeo privado OK (invisible para otros espacios)"),
                        Err(e) => serial_println!("[proc] mapeo privado falló: {}", e),
                    }
                }
                None => serial_println!("[proc] sin memoria física para el mapeo privado"),
            }

            let before = mmu::current_address_space();
            mmu::switch_address_space(new_space);
            serial_println!(
                "[proc] CR3 cambiado de 0x{:x} a 0x{:x} — si ves esta línea, el kernel sigue vivo bajo un PML4 distinto",
                before,
                new_space
            );
        } else {
            serial_println!("[proc] sin memoria física para el nuevo espacio de direcciones");
        }
    }

    // M4e — cargador ELF64, probado contra un binario real y mínimo
    // construido a mano (no bytes inventados) — ver elf.rs::TEST_ELF.
    // Todavía no saltamos a él (eso necesita transición a ring 3,
    // pieza aparte); esto demuestra que parseo + mapeo de segmentos
    // funciona de verdad.
    match elf::load(&elf::TEST_ELF) {
        Ok(loaded) => {
            serial_println!(
                "[elf] cargado OK — entry=0x{:x} (esperado 0xa000000078), PML4=0x{:x}",
                loaded.entry_point,
                loaded.page_table
            );
            if loaded.entry_point == 0xa000000078 {
                serial_println!("[elf] entry point coincide con lo calculado a mano — parseo correcto");
            }
        }
        Err(e) => serial_println!("[elf] carga falló: {}", e),
    }

    // M5 — VFS mínimo (tmpfs plano). Prueba real: escribir, leer, y
    // comprobar que el contenido coincide.
    vfs::init();
    vfs::write("hola.txt", b"Hola desde el VFS de Forge OS");
    match vfs::read("hola.txt") {
        Some(data) => {
            if data == b"Hola desde el VFS de Forge OS" {
                serial_println!("[vfs] escritura + lectura coinciden — VFS funciona");
            } else {
                serial_println!("[vfs] el contenido leído NO coincide con lo escrito");
            }
        }
        None => serial_println!("[vfs] no se encontró el fichero recién escrito"),
    }
    serial_println!("[vfs] ficheros: {:?}", vfs::list());

    // IPC — pipes. Prueba real: escribimos más de lo que leemos de
    // golpe, comprobamos que el resto sigue esperando en el pipe (FIFO
    // de verdad, no solo un buffer que se vacía entero).
    pipe::init();
    let test_pipe = pipe::create();
    match pipe::write(test_pipe, b"Hola") {
        Ok(n) => serial_println!("[pipe] escritos {} bytes", n),
        Err(e) => serial_println!("[pipe] fallo al escribir: {}", e),
    }
    match pipe::read(test_pipe, 2) {
        Ok(data) => serial_println!("[pipe] leídos 2 bytes: {:?} (esperado 'Ho')", data),
        Err(e) => serial_println!("[pipe] fallo al leer: {}", e),
    }
    if let Some(pending) = pipe::pending(test_pipe) {
        serial_println!(
            "[pipe] quedan {} bytes pendientes (esperado 2, 'la') — FIFO funciona",
            pending
        );
    }

    // RTL8139 TX real — trama Ethernet de broadcast construida a mano:
    // dest=ff:ff:ff:ff:ff:ff, src=nuestra MAC, ethertype=0x88B5
    // (reservado para experimentación IEEE, no colisiona con nada real).
    if let Some(mut nic) = rtl8139::init_full() {
        let mut frame = [0u8; 60];
        frame[0..6].copy_from_slice(&[0xFF; 6]); // destino: broadcast
        frame[6..12].copy_from_slice(&nic.mac); // origen: nuestra MAC
        frame[12] = 0x88;
        frame[13] = 0xB5; // ethertype experimental

        match rtl8139::send(&mut nic, &frame) {
            Ok(()) => serial_println!("[rtl8139] trama de prueba enviada y confirmada (TxStatOK)"),
            Err(e) => serial_println!("[rtl8139] fallo al enviar: {}", e),
        }
    }

    // TODO M2b: heap real con free-list (recuperar memoria de dealloc)
    // TODO M2c: syscalls reales — aquí `caps::enforce` pasa a llamarse
    //           por cada una
    // TODO M3c: ampliar el alfabeto de font.rs más allá de F/O/R/G/E/S/B/T/K
    // TODO M4b (continuación): conectar el timer con el scheduler para
    //           preemption real — necesita guardar TODOS los registros
    //           de propósito general desde el handler asíncrono, no solo
    //           los callee-saved de task.rs
    // TODO M4c: espacios de direcciones por tarea (necesita gestor de
    //           memoria virtual real, ver TODO.md §1) — de ahí a fork/exec
    // TODO M6+: Anvil + terminal ("Crucible")

    // M4g — mecanismo syscall/sysret listo (MSRs configuradas). El
    // comando `synccalltest` de la consola lo ejercita de verdad.
    syscall::init();

    serial_println!("");
    serial_println!("Boot completo — entrando en la consola de depuración.");
    console::run()
}

fn task_a() -> ! {
    loop {
        serial_println!("[task A] hola desde la tarea A");
        scheduler::yield_now();
    }
}

fn task_b() -> ! {
    loop {
        serial_println!("[task B] hola desde la tarea B");
        scheduler::yield_now();
    }
}
