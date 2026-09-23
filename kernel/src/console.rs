//! Consola de depuración interactiva sobre el puerto serie — M2
//! (BORRADOR SIN VERIFICAR).
//!
//! NO es el shell del producto final ("Bellows", M4+, con procesos
//! reales) — es herramienta de bring-up. Aprovecha que el puerto serie
//! ya existe y que `run-qemu.sh` en modo interactivo conecta el teclado
//! real del host a COM1 vía `-serial stdio`. Sin esto, verificar el
//! kernel significa re-flashear y releer `boot.log` cada vez.
//!
//! Ver docs/SHELL.md para la relación con el shell y el terminal reales.

use crate::serial::SerialPort;
use crate::{ahci, caps, elf, framebuffer, mmu, pci, pmm, process, ring3, rtl8139, scheduler, vfs};
use alloc::string::String;
use alloc::vec::Vec;
use core::fmt::Write;

const PROMPT: &str = "forge> ";
const BACKSPACE: u8 = 0x08;
const DEL: u8 = 0x7F;
const CR: u8 = b'\r';
const LF: u8 = b'\n';

pub fn run() -> ! {
    let mut port = SerialPort::init();
    let _ = write!(port, "\r\n--- consola de depuracion Forge OS ---\r\n");
    let _ = write!(port, "escribe 'help' para ver los comandos\r\n\r\n");

    loop {
        let _ = write!(port, "{}", PROMPT);
        let line = read_line(&mut port);
        dispatch(&mut port, line.trim());
    }
}

fn read_line(port: &mut SerialPort) -> String {
    let mut buf: Vec<u8> = Vec::new();
    loop {
        // Escuchamos teclado PS/2 y puerto serie a la vez — el primero
        // que tenga un byte listo gana. `hlt` evita busy-spinning
        // mientras esperamos a cualquiera de los dos.
        let byte = loop {
            if let Some(b) = crate::keyboard::try_read_byte() {
                break b;
            }
            if let Some(b) = port.try_read_byte() {
                break b;
            }
            unsafe { core::arch::asm!("hlt") };
        };

        match byte {
            CR | LF => {
                let _ = write!(port, "\r\n");
                break;
            }
            BACKSPACE | DEL => {
                if buf.pop().is_some() {
                    let _ = write!(port, "\x08 \x08"); // retrocede, borra, retrocede
                }
            }
            _ => {
                buf.push(byte);
                port.write_byte(byte); // eco
            }
        }
    }
    String::from_utf8_lossy(&buf).into_owned()
}

fn dispatch(port: &mut SerialPort, line: &str) {
    let mut parts = line.split_whitespace();
    let cmd = match parts.next() {
        Some(c) => c,
        None => return, // línea vacía, solo Enter
    };

    match cmd {
        "help" => {
            let _ = write!(
                port,
                "comandos: help, meminfo, caps, bp, panic, fb, pci, ahci, net, disktest, ls, cat, write, ring3test, synccalltest, synccalldeny, forktest, waittest, exec, ps\r\n"
            );
        }
        "synccalltest" => {
            let _ = write!(port, "pledge: CAP_STDIO concedido — SYS_PING deberia PERMITIRSE\r\n");
            caps::set_current({
                let mut c = caps::Capabilities::unrestricted();
                let _ = c.pledge(caps::CAP_STDIO);
                c
            });
            run_synccall_elf(port);
        }
        "synccalldeny" => {
            let _ = write!(port, "pledge: sin CAP_STDIO — SYS_PING deberia DENEGARSE\r\n");
            caps::set_current({
                let mut c = caps::Capabilities::unrestricted();
                let _ = c.pledge(0); // ninguna capability
                c
            });
            run_synccall_elf(port);
        }
        "ring3test" => {
            let _ = write!(port, "cargando ELF de prueba y saltando a ring 3...\r\n");
            let _ = write!(port, "AVISO: viaje solo de ida — sin preemption real todavia,\r\n");
            let _ = write!(port, "esta consola no respondera despues de esto en esta sesion.\r\n");

            match elf::load(&elf::TEST_ELF_RING3) {
                Ok(loaded) => unsafe {
                    let stack_phys = match pmm::alloc_frame() {
                        Some(f) => f,
                        None => {
                            let _ = write!(port, "sin memoria para la pila de usuario\r\n");
                            return;
                        }
                    };
                    // 704 GiB, índice P4=1 — genuinamente privado (ver
                    // la nota larga en elf.rs sobre por qué una
                    // dirección por debajo de 512 GiB NO lo es, aunque
                    // esté muy por encima de los 4 GiB del identity-map).
                    let user_stack_virt: u64 = 0x0000_00B0_0000_0000;
                    if let Err(e) = mmu::map_page_in(loaded.page_table, user_stack_virt, stack_phys, true, false) {
                        let _ = write!(port, "fallo mapeando la pila de usuario: {}\r\n", e);
                        return;
                    }
                    let user_stack_top = user_stack_virt + 4096;

                    mmu::switch_address_space(loaded.page_table);
                    ring3::enter_ring3(loaded.entry_point, user_stack_top);
                    // nunca se llega aquí — enter_ring3 no vuelve
                },
                Err(e) => {
                    let _ = write!(port, "carga del ELF falló: {}\r\n", e);
                }
            }
        }
        "forktest" => {
            let _ = write!(port, "cargando ELF de fork y arrancandolo como proceso real...\r\n");
            caps::set_current({
                let mut c = caps::Capabilities::unrestricted();
                let _ = c.pledge(caps::CAP_STDIO | caps::CAP_EXEC);
                c
            });
            if let Some(pid) = launch_elf(port, &elf::TEST_ELF_FORK, 0) {
                let _ = write!(port, "PID {} arrancado — cediendo turno para que padre e hijo corran...\r\n", pid);
                // Suficientes yields para que: el padre corra hasta
                // fork()+ping+exit, el hijo (creado por fork() a mitad
                // de camino) corra hasta su propio ping+exit, y volvamos
                // aquí. No hay preemption real todavía (M4b la deja
                // pendiente) — sin ceder explícitamente el turno como
                // aquí, ninguna de las dos tareas nuevas llegaría nunca
                // a ejecutarse.
                for _ in 0..2 {
                    scheduler::yield_now();
                }
                let _ = write!(port, "de vuelta en la consola — revisa 'ps' para ver el resultado.\r\n");
            }
        }
        "waittest" => {
            let _ = write!(port, "cargando ELF de fork+wait y arrancandolo como proceso real...\r\n");
            caps::set_current({
                let mut c = caps::Capabilities::unrestricted();
                let _ = c.pledge(caps::CAP_STDIO | caps::CAP_EXEC);
                c
            });
            if let Some(pid) = launch_elf(port, &elf::TEST_ELF_FORK_WAIT, 0) {
                let _ = write!(port, "PID {} arrancado — cediendo turno...\r\n", pid);
                // El padre bloquea de verdad dentro de wait() (cede el
                // turno en bucle hasta que el hijo sea zombie) — un
                // único yield desde aquí ya dispara toda la cadena:
                // padre entra en wait(), cede al hijo, el hijo corre
                // hasta exit(), vuelve al padre que ahora sí encuentra
                // el zombie, lo recoge (liberando su memoria de verdad)
                // y sale él también.
                for _ in 0..2 {
                    scheduler::yield_now();
                }
                let _ = write!(port, "de vuelta en la consola — 'ps' no deberia listar ya al hijo (memoria liberada).\r\n");
            }
        }
        "exec" => {
            let _ = write!(port, "guardando TEST_ELF_PING_EXIT como 'pingtest' en el VFS...\r\n");
            vfs::write("pingtest", &elf::TEST_ELF_PING_EXIT);
            let _ = write!(port, "arrancando proceso que se reemplaza a si mismo via execve(\"pingtest\")...\r\n");
            caps::set_current({
                let mut c = caps::Capabilities::unrestricted();
                let _ = c.pledge(caps::CAP_STDIO | caps::CAP_EXEC);
                c
            });
            if let Some(pid) = launch_elf(port, &elf::TEST_ELF_EXECVE, 0) {
                let _ = write!(port, "PID {} arrancado — cediendo turno...\r\n", pid);
                for _ in 0..2 {
                    scheduler::yield_now();
                }
                let _ = write!(port, "de vuelta en la consola.\r\n");
            }
        }
        "ps" => {
            let _ = write!(port, "PID  PPID  ESTADO\r\n");
            for p in process::list() {
                let estado: alloc::string::String = match p.state {
                    process::ProcessState::Running => alloc::string::String::from("running"),
                    process::ProcessState::Zombie(code) => alloc::format!("zombie(code={})", code),
                };
                let _ = write!(port, "{:<4} {:<5} {}\r\n", p.pid, p.parent_pid, estado);
            }
        }
        "ls" => {
            for name in vfs::list() {
                let _ = write!(port, "{}\r\n", name);
            }
        }
        "cat" => match parts.next() {
            Some(name) => match vfs::read(name) {
                Some(data) => {
                    for byte in data {
                        port.write_byte(byte);
                    }
                    let _ = write!(port, "\r\n");
                }
                None => {
                    let _ = write!(port, "no existe: {}\r\n", name);
                }
            },
            None => {
                let _ = write!(port, "uso: cat <nombre>\r\n");
            }
        },
        "write" => {
            let name = parts.next();
            let text: alloc::vec::Vec<&str> = parts.collect();
            match name {
                Some(name) if !text.is_empty() => {
                    let content = text.join(" ");
                    vfs::write(name, content.as_bytes());
                    let _ = write!(port, "escrito: {} ({} bytes)\r\n", name, content.len());
                }
                _ => {
                    let _ = write!(port, "uso: write <nombre> <texto...>\r\n");
                }
            }
        }
        "disktest" => {
            match ahci::first_disk() {
                Some(disk) => {
                    let _ = write!(port, "probando escritura+relectura en LBA 2048 (no destructivo)...\r\n");
                    match ahci::write_readback_test(&disk, 2048) {
                        Ok(()) => {
                            let _ = write!(port, "OK — escritura y relectura coinciden, el disco responde bien\r\n");
                        }
                        Err(e) => {
                            let _ = write!(port, "FALLO: {}\r\n", e);
                        }
                    }
                }
                None => {
                    let _ = write!(port, "no se encontró ningún disco SATA listo\r\n");
                }
            }
        }
        "pci" => pci::scan_and_print(),
        "ahci" => ahci::probe_and_print(),
        "net" => rtl8139::probe_and_print(),
        "fb" => {
            if framebuffer::available() {
                framebuffer::test_pattern();
                framebuffer::draw_str(20, 20, "FORGE OS BOOT OK", 0x00000000, 4);
                let _ = write!(port, "patrón + banner redibujados\r\n");
            } else {
                let _ = write!(port, "sin framebuffer disponible\r\n");
            }
        }
        "meminfo" => {
            let free = pmm::free_frame_count();
            let _ = write!(port, "frames libres: {} (~{} KiB)\r\n", free, free * 4);
            let _ = write!(port, "heap usado: {} bytes\r\n", crate::heap::used_bytes());
        }
        "caps" => run_caps_demo(port),
        "bp" => unsafe { core::arch::asm!("int3") },
        "panic" => panic!("panic solicitado desde la consola de depuración"),
        other => {
            let _ = write!(port, "comando desconocido: '{}' (prueba 'help')\r\n", other);
        }
    }
}

fn run_synccall_elf(port: &mut SerialPort) {
    let _ = write!(port, "cargando ELF que ejecuta syscall(SYS_PING) desde ring 3...\r\n");
    let _ = write!(port, "AVISO: sigue siendo viaje de ida al final (jmp $ tras la syscall)\r\n");

    match elf::load(&elf::TEST_ELF_SYSCALL) {
        Ok(loaded) => unsafe {
            let stack_phys = match pmm::alloc_frame() {
                Some(f) => f,
                None => {
                    let _ = write!(port, "sin memoria para la pila de usuario\r\n");
                    return;
                }
            };
            let user_stack_virt: u64 = 0x0000_0080_0000_0000; // 512 GiB, privado del proceso
            if let Err(e) = mmu::map_page_in(loaded.page_table, user_stack_virt, stack_phys, true, false) {
                let _ = write!(port, "fallo mapeando la pila de usuario: {}\r\n", e);
                return;
            }
            let user_stack_top = user_stack_virt + 4096;

            mmu::switch_address_space(loaded.page_table);
            ring3::enter_ring3(loaded.entry_point, user_stack_top);
        },
        Err(e) => {
            let _ = write!(port, "carga del ELF falló: {}\r\n", e);
        }
    }
}

/// Carga `bytes` como ELF, le monta una pila de usuario, y lo arranca
/// como un proceso real de la tabla de `process.rs` — vía
/// `scheduler::spawn_with_space`, no con un `enter_ring3` directo de un
/// solo sentido como `run_synccall_elf`/`ring3test`. Necesario para que
/// haya una tarea a la que el scheduler pueda volver (y ceder turno
/// entre padre/hijo) en vez de quedarse encallado en ring 3 para
/// siempre. `parent_pid` = 0 para "sin padre" (lanzado directamente
/// desde la consola, no por `fork()`).
fn launch_elf(port: &mut SerialPort, bytes: &[u8], parent_pid: u64) -> Option<u64> {
    let loaded = match elf::load(bytes) {
        Ok(l) => l,
        Err(e) => {
            let _ = write!(port, "carga del ELF falló: {}\r\n", e);
            return None;
        }
    };

    unsafe {
        let stack_phys = match pmm::alloc_frame() {
            Some(f) => f,
            None => {
                let _ = write!(port, "sin memoria para la pila de usuario\r\n");
                return None;
            }
        };
        let user_stack_virt: u64 = 0x0000_0090_0000_0000; // 576 GiB, privado del proceso
        if let Err(e) = mmu::map_page_in(loaded.page_table, user_stack_virt, stack_phys, true, false) {
            let _ = write!(port, "fallo mapeando la pila de usuario: {}\r\n", e);
            return None;
        }
        let user_stack_top = user_stack_virt + 4096;

        Some(process::spawn_process(loaded.entry_point, user_stack_top, loaded.page_table, parent_pid))
    }
}

fn run_caps_demo(port: &mut SerialPort) {
    let mut c = caps::Capabilities::unrestricted();
    let _ = c.pledge(caps::CAP_STDIO);
    let _ = write!(
        port,
        "pledge STDIO -> enforce STDIO permitido = {}\r\n",
        c.enforce(caps::CAP_STDIO).is_ok()
    );
    let _ = write!(
        port,
        "enforce NET (no pledgeado) permitido = {}\r\n",
        c.enforce(caps::CAP_NET).is_ok()
    );
}
