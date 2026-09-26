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
use crate::{ahci, apic, caps, elf, ext2, framebuffer, mmu, net, partinfo, pci, pmm, preempt, process, ring3, rtl8139, scheduler, vfs};
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
                "comandos: help, meminfo, caps, bp, panic, fb, pci, ahci, net, ping, disktest, partinfo, ls, cat, write, ext2ls, ext2cat, ring3test, synccalltest, synccalldeny, forktest, waittest, exec, ember, preempttest, segvtest, killtest, ps\r\n"
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
        "ember" => run_ember(port),
        "preempttest" => run_preempt_test(port),
        "segvtest" => run_segv_test(port),
        "killtest" => run_kill_test(port),
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
        "partinfo" => run_partinfo(port),
        "ext2ls" => {
            let path = parts.next().unwrap_or("/");
            run_ext2ls(port, path);
        }
        "ext2cat" => match parts.next() {
            Some(path) => run_ext2cat(port, path),
            None => {
                let _ = write!(port, "uso: ext2cat <ruta>\r\n");
            }
        },
        "pci" => pci::scan_and_print(),
        "ahci" => ahci::probe_and_print(),
        "net" => rtl8139::probe_and_print(),
        "ping" => run_ping(port),
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

/// ARP + ICMP echo de extremo a extremo contra la puerta de enlace de
/// QEMU (`-net user`/slirp): 10.0.2.15 como IP propia, 10.0.2.2 como
/// gateway — son los valores fijos que slirp asigna por defecto, sin
/// DHCP real todavía en esta versión. Bucles de sondeo acotados (ver
/// comentario en cada `while`): sin esto, una tarjeta sin respuesta
/// (BAR mal leído, cable "desconectado" en la config de QEMU) colgaría
/// la consola entera para siempre.
fn run_ping(port: &mut SerialPort) {
    let _ = write!(port, "inicializando RTL8139 para ping...\r\n");
    let mut nic = match rtl8139::init_full() {
        Some(n) => n,
        None => {
            let _ = write!(port, "no se pudo inicializar la tarjeta de red\r\n");
            return;
        }
    };

    let src_ip: net::Ipv4 = [10, 0, 2, 15];
    let gateway_ip: net::Ipv4 = [10, 0, 2, 2];

    let _ = write!(
        port,
        "ARP request para {}.{}.{}.{}...\r\n",
        gateway_ip[0], gateway_ip[1], gateway_ip[2], gateway_ip[3]
    );
    let arp_frame = net::build_arp_request(nic.mac, src_ip, gateway_ip);
    if let Err(e) = rtl8139::send(&mut nic, &arp_frame) {
        let _ = write!(port, "fallo al enviar ARP request: {}\r\n", e);
        return;
    }

    // Sondeo acotado: el chip no tiene forma de avisarnos por
    // interrupción todavía (sin preemption real, M4b lo deja
    // pendiente), así que comprobamos `poll_rx` en bucle. 5 millones de
    // vueltas es un margen generoso frente al par de microsegundos que
    // tarda slirp en contestar — mismo orden de magnitud que los demás
    // timeouts de este driver (`send`, reset del chip).
    let mut gateway_mac = None;
    let mut attempts = 0u32;
    while attempts < 5_000_000 {
        if let Some(packet) = rtl8139::poll_rx(&mut nic) {
            if let Some(reply) = net::parse_arp_reply(&packet) {
                if reply.sender_ip == gateway_ip {
                    gateway_mac = Some(reply.sender_mac);
                    break;
                }
            }
        }
        attempts += 1;
    }

    let gateway_mac = match gateway_mac {
        Some(mac) => mac,
        None => {
            let _ = write!(port, "timeout esperando ARP reply\r\n");
            return;
        }
    };
    net::arp_insert(gateway_ip, gateway_mac);
    let _ = write!(
        port,
        "ARP reply: {}.{}.{}.{} esta en {:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}\r\n",
        gateway_ip[0],
        gateway_ip[1],
        gateway_ip[2],
        gateway_ip[3],
        gateway_mac[0],
        gateway_mac[1],
        gateway_mac[2],
        gateway_mac[3],
        gateway_mac[4],
        gateway_mac[5]
    );

    let ident = 0x1234u16;
    let seq = 1u16;
    let _ = write!(
        port,
        "enviando ICMP echo request (id={}, seq={})...\r\n",
        ident, seq
    );
    let ping_frame = net::build_ping(nic.mac, src_ip, gateway_mac, gateway_ip, ident, seq);
    if let Err(e) = rtl8139::send(&mut nic, &ping_frame) {
        let _ = write!(port, "fallo al enviar ping: {}\r\n", e);
        return;
    }

    let mut got_reply = false;
    let mut attempts = 0u32;
    while attempts < 5_000_000 {
        if let Some(packet) = rtl8139::poll_rx(&mut nic) {
            if let Some(reply) = net::parse_icmp_echo_reply(&packet) {
                if reply.id == ident && reply.seq == seq {
                    let _ = write!(
                        port,
                        "pong de {}.{}.{}.{} (id={}, seq={}) — round-trip OK\r\n",
                        reply.src_ip[0], reply.src_ip[1], reply.src_ip[2], reply.src_ip[3], reply.id, reply.seq
                    );
                    got_reply = true;
                    break;
                }
            }
        }
        attempts += 1;
    }

    if !got_reply {
        let _ = write!(port, "timeout esperando ICMP echo reply\r\n");
    }
}

/// Primera prueba real de Ember (PID 1, modelo rc.d): registra dos
/// "servicios" en el VFS (reutilizando `TEST_ELF_PING_EXIT` — un
/// binario que ya sabemos que arranca y termina limpio, aquí hace de
/// placeholder de un servicio real) y arranca `TEST_ELF_EMBER`, que los
/// lanza en orden, esperando a que cada uno termine del todo antes del
/// siguiente.
fn run_ember(port: &mut SerialPort) {
    let _ = write!(port, "registrando servicios en el VFS: ember-svc0, ember-svc1...\r\n");
    vfs::write("ember-svc0", &elf::TEST_ELF_PING_EXIT);
    vfs::write("ember-svc1", &elf::TEST_ELF_PING_EXIT);

    let _ = write!(port, "arrancando Ember (PID 1) como proceso real...\r\n");
    caps::set_current({
        let mut c = caps::Capabilities::unrestricted();
        let _ = c.pledge(caps::CAP_STDIO | caps::CAP_EXEC);
        c
    });
    if let Some(pid) = launch_elf(port, &elf::TEST_ELF_EMBER, 0) {
        let _ = write!(port, "PID {} (Ember) arrancado — cediendo turno...\r\n", pid);
        // Dos rondas secuenciales de fork+execve+wait encadenadas dentro
        // del mismo proceso — mismo patrón que `waittest` (una ronda,
        // 2 yields bastaron), aquí con margen extra por ser dos rondas.
        for _ in 0..4 {
            scheduler::yield_now();
        }
        let _ = write!(
            port,
            "de vuelta en la consola — revisa 'ps': los dos servicios ya no deberían aparecer \
             (Ember los reapeó con wait(), memoria liberada de verdad); Ember mismo sí queda \
             como zombie(code=0) — nadie lo espera, se lanzó directo desde la consola (PPID 0).\r\n"
        );
    }
}

fn run_partinfo(port: &mut SerialPort) {
    let disk = match ahci::first_disk() {
        Some(d) => d,
        None => {
            let _ = write!(port, "no se encontró ningún disco SATA listo\r\n");
            return;
        }
    };
    match partinfo::scan(&disk) {
        Ok((kind, partitions)) => {
            let kind_str = match kind {
                partinfo::TableKind::Mbr => "MBR",
                partinfo::TableKind::Gpt => "GPT",
            };
            let _ = write!(port, "tabla de particiones: {}\r\n", kind_str);
            let _ = write!(port, "#   LBA inicio   sectores      tipo/nombre          filesystem\r\n");
            for p in partitions {
                let label = match (&p.mbr_type, &p.gpt_name) {
                    (Some(t), _) => alloc::format!("0x{:02x}", t),
                    (None, Some(name)) => name.clone(),
                    _ => alloc::string::String::from("?"),
                };
                let _ = write!(
                    port,
                    "{:<3} {:<13} {:<13} {:<20} {}\r\n",
                    p.index, p.start_lba, p.sector_count, label, p.filesystem
                );
            }
        }
        Err(e) => {
            let _ = write!(port, "fallo al escanear la tabla de particiones: {}\r\n", e);
        }
    }
}

/// Monta el ext2 del primer disco AHCI encontrado — sin tabla de
/// particiones todavía (`partinfo.rs` es aparte), así que asume que el
/// filesystem empieza en el LBA 0 del disco entero.
fn mount_ext2(port: &mut SerialPort) -> Option<ext2::Ext2Fs> {
    let disk = match ahci::first_disk() {
        Some(d) => d,
        None => {
            let _ = write!(port, "no se encontró ningún disco SATA listo\r\n");
            return None;
        }
    };
    match ext2::Ext2Fs::mount(disk) {
        Ok(fs) => Some(fs),
        Err(e) => {
            let _ = write!(port, "fallo al montar ext2: {}\r\n", e);
            None
        }
    }
}

fn run_ext2ls(port: &mut SerialPort, path: &str) {
    let Some(fs) = mount_ext2(port) else { return };
    let inode = match fs.resolve(path) {
        Ok(i) => i,
        Err(e) => {
            let _ = write!(port, "{}: {}\r\n", path, e);
            return;
        }
    };
    if !inode.is_dir() {
        let _ = write!(port, "{}: no es un directorio\r\n", path);
        return;
    }
    match fs.read_dir(&inode) {
        Ok(entries) => {
            for e in entries {
                let kind = if e.is_dir { "/" } else { "" };
                let _ = write!(port, "{}{}\r\n", e.name, kind);
            }
        }
        Err(e) => {
            let _ = write!(port, "fallo al listar: {}\r\n", e);
        }
    }
}

fn run_ext2cat(port: &mut SerialPort, path: &str) {
    let Some(fs) = mount_ext2(port) else { return };
    let inode = match fs.resolve(path) {
        Ok(i) => i,
        Err(e) => {
            let _ = write!(port, "{}: {}\r\n", path, e);
            return;
        }
    };
    if inode.is_dir() {
        let _ = write!(port, "{}: es un directorio\r\n", path);
        return;
    }
    match fs.read_inode_data(&inode) {
        Ok(data) => {
            for byte in data {
                port.write_byte(byte);
            }
            let _ = write!(port, "\r\n");
        }
        Err(e) => {
            let _ = write!(port, "fallo al leer: {}\r\n", e);
        }
    }
}

/// Prueba real de preemption: dos tareas de kernel (`preempt::spin_a`/
/// `spin_b`) que jamás llaman a `yield_now()` por su cuenta — si
/// avanzan de todos modos mientras el timer las interrumpe, es la
/// prueba de que el scheduler las está forzando a ceder el turno, no
/// de que se portan bien. Acotado a propósito (`preempt::set_enabled`
/// solo dura lo que tarda este bucle) — ver la nota larga en
/// `preempt.rs` sobre por qué la preemption real NO está encendida por
/// defecto para el resto del arranque/consola.
fn run_preempt_test(port: &mut SerialPort) {
    let _ = write!(port, "arrancando dos tareas que nunca ceden el turno por su cuenta...\r\n");
    preempt::spawn_spin_tasks();
    let (a0, b0) = preempt::spin_counts();
    let ticks0 = apic::tick_count();

    let _ = write!(port, "activando preemption real (timer -> scheduler) durante 20 ticks...\r\n");
    preempt::set_enabled(true);
    while apic::tick_count() < ticks0 + 20 {
        unsafe { core::arch::asm!("hlt") };
    }
    preempt::set_enabled(false);

    let (a1, b1) = preempt::spin_counts();
    let _ = write!(
        port,
        "preemption desactivada de nuevo — {} ticks reales\r\n",
        apic::tick_count() - ticks0
    );
    let _ = write!(port, "tarea A: {} -> {} (avanzo {})\r\n", a0, a1, a1 - a0);
    let _ = write!(port, "tarea B: {} -> {} (avanzo {})\r\n", b0, b1, b1 - b0);
    if a1 > a0 && b1 > b0 {
        let _ = write!(
            port,
            "preemption real confirmada: ambas avanzaron sin llamar a yield_now() nunca\r\n"
        );
    } else {
        let _ = write!(port, "AVISO: alguna de las dos no avanzo — revisar\r\n");
    }
}

/// Dispara un fallo de página real en ring 3 a propósito
/// (`TEST_ELF_SEGV`) — antes de `signal.rs`, esto colgaba el kernel
/// entero (`halt()` incondicional en `idt.rs::page_fault`). Ahora solo
/// debería matar al proceso: la consola tiene que seguir respondiendo
/// después, y `ps` debería mostrarlo `zombie(code=139)` (128+SIGSEGV).
fn run_segv_test(port: &mut SerialPort) {
    let _ = write!(port, "arrancando un proceso que escribe a la dirección 0 a propósito...\r\n");
    caps::set_current(caps::Capabilities::unrestricted());
    if let Some(pid) = launch_elf(port, &elf::TEST_ELF_SEGV, 0) {
        let _ = write!(port, "PID {} arrancado — cediendo turno...\r\n", pid);
        for _ in 0..2 {
            scheduler::yield_now();
        }
        let _ = write!(
            port,
            "de vuelta en la consola — si ves esto, el kernel sobrevivió al fallo de página \
             (revisa 'ps': debería quedar zombie(code=139), 128+SIGSEGV).\r\n"
        );
    }
}

/// `fork()` + `kill(hijo, SIGTERM)` desde el padre, antes de que el
/// hijo llegue a correr — ver la nota larga en `elf.rs::TEST_ELF_KILL`.
fn run_kill_test(port: &mut SerialPort) {
    let _ = write!(port, "cargando ELF de fork+kill y arrancandolo como proceso real...\r\n");
    caps::set_current({
        let mut c = caps::Capabilities::unrestricted();
        let _ = c.pledge(caps::CAP_STDIO | caps::CAP_EXEC | caps::CAP_PROC_CTL);
        c
    });
    if let Some(pid) = launch_elf(port, &elf::TEST_ELF_KILL, 0) {
        let _ = write!(port, "PID {} arrancado — cediendo turno...\r\n", pid);
        for _ in 0..2 {
            scheduler::yield_now();
        }
        let _ = write!(
            port,
            "de vuelta en la consola — 'ps' no deberia listar ya al hijo (kill()+wait() lo reapeo).\r\n"
        );
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
