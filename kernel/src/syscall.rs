//! Mecanismo `syscall`/`sysret` — M4g (BORRADOR SIN VERIFICAR — de todo
//! lo escrito en esta sesión, probablemente lo más delicado, más piezas
//! móviles incluso que AHCI).
//!
//! IMPORTANTE — qué prueba esto y qué NO: `sysretq` siempre devuelve el
//! control a ring 3 (al programa de usuario, justo después de su
//! instrucción `syscall`), nunca "de vuelta al kernel que llamó a
//! `enter_ring3`". Un `exit()` real que recupere el control de, por
//! ejemplo, el comando `ring3test` de la consola, necesita guardar y
//! restaurar contexto como hace `task.rs` — integración con el
//! scheduler, pieza aparte. Lo que SÍ demuestra esta pasada es el viaje
//! completo ring3 → syscall → kernel → sysret → ring3 de nuevo, que es
//! lo que hace la inmensa mayoría de syscalls reales (escribir, leer,
//! pedir memoria...) — no terminan el proceso, vuelven a él.

use crate::caps;
use crate::elf;
use crate::gdt;
use crate::mmu;
use crate::process;
use crate::ring3;
use crate::scheduler;
use crate::serial_println;
use crate::signal;
use crate::vfs;

const IA32_EFER: u32 = 0xC000_0080;
const IA32_STAR: u32 = 0xC000_0081;
const IA32_LSTAR: u32 = 0xC000_0082;
const IA32_FMASK: u32 = 0xC000_0084;

/// Convención SYSRET en modo largo: CS de usuario = este valor + 16,
/// SS de usuario = este valor + 8 (lo exige el hardware, no lo
/// elegimos nosotros). Con nuestro orden de GDT actual
/// (kernel_code=0x08, kernel_data=0x10, user_data=0x18, user_code=0x20)
/// esto ya encaja solo — 0x10+8=0x18=user_data, 0x10+16=0x20=user_code.
const STAR_USER_BASE: u16 = 0x10;

const KERNEL_SYSCALL_STACK_SIZE: usize = 16 * 1024;
// `align(16)` explícito — usado directamente como RSP en `syscall_entry`
// (ver el mismo razonamiento en `task.rs::Task::new` para las pilas de
// kernel por-tarea); un `[u8; N]` estático sin esto solo garantiza
// alineación a 1 byte.
#[repr(align(16))]
struct KernelSyscallStack([u8; KERNEL_SYSCALL_STACK_SIZE]);
static mut KERNEL_SYSCALL_STACK: KernelSyscallStack = KernelSyscallStack([0; KERNEL_SYSCALL_STACK_SIZE]);
static mut SYSCALL_STACK_TOP: u64 = 0;

/// Cambia la pila que usará la PRÓXIMA entrada por `syscall` — llamado
/// por `scheduler::yield_now()` al entrarle el turno a una tarea, con
/// su `Task::kernel_stack_top` (0 = tarea sin pila de syscalls propia,
/// no se toca — un hilo de kernel puro nunca ejecuta `syscall`). Ver
/// el comentario largo en `task.rs::Task::kernel_stack_top` sobre por
/// qué hace falta: sin esto, TODAS las tareas comparten
/// `KERNEL_SYSCALL_STACK`, y una syscall que ceda el turno esperando
/// reanudarse más tarde (`wait()`) se corrompe en cuanto otra tarea
/// hace su propia syscall mientras tanto.
pub fn set_syscall_stack_top(top: u64) {
    unsafe { SYSCALL_STACK_TOP = top };
}
static mut USER_RSP_SCRATCH: u64 = 0;

pub const SYS_PING: u64 = 1;
// Numeración compatible con Linux x86_64 donde aplica — ya documentado
// en ARCHITECTURE.md §Convención de syscalls.
pub const SYS_GETPID: u64 = 39;
pub const SYS_FORK: u64 = 57;
pub const SYS_EXECVE: u64 = 59;
pub const SYS_EXIT: u64 = 60;
pub const SYS_WAIT: u64 = 61; // == wait4 en Linux x86_64; no hay "wait" clásica en esa ABI
pub const SYS_KILL: u64 = 62;

/// Argumentos de una syscall, en el orden `syscall/sysret` de Linux
/// x86_64 (RAX=número, RDI/RSI/RDX/R10/R8/R9=args) — ya documentado en
/// `ARCHITECTURE.md` §Convención de syscalls, esto es la primera vez
/// que se implementa de verdad.
///
/// `user_rflags`/`user_rip` son los dos últimos valores que empuja
/// `syscall_entry` (`r11`/`rcx`, puestos por la propia instrucción
/// `syscall`) — no forman parte de la convención de argumentos, pero
/// hacen falta para fabricar el punto de retorno del hijo de `fork()`
/// (ver `sys_fork`): el hijo no llega a ring 3 vía `sysretq` normal, así
/// que necesitamos saber a mano dónde reanudarlo.
#[repr(C)]
pub struct SyscallFrame {
    pub num: u64,
    pub arg0: u64,
    pub arg1: u64,
    pub arg2: u64,
    pub arg3: u64,
    pub arg4: u64,
    pub arg5: u64,
    pub user_rflags: u64,
    pub user_rip: u64,
}

/// Handoff para el hijo de `fork()` — el mismo patrón de "slot global
/// único" que `USER_RSP_SCRATCH`: solo soporta un `fork()` en vuelo a
/// la vez (cooperativo, sin SMP, ver `docs/MEGADOC.md` §errores
/// conocidos). El trampolín que arranca la tarea del hijo en el
/// scheduler lee esto inmediatamente, antes de que un `fork()`
/// distinto pueda pisarlo.
static mut FORK_CHILD_PID: u64 = 0;
static mut FORK_CHILD_RIP: u64 = 0;
static mut FORK_CHILD_RSP: u64 = 0;

unsafe fn read_msr(msr: u32) -> u64 {
    let (hi, lo): (u32, u32);
    core::arch::asm!("rdmsr", in("ecx") msr, out("eax") lo, out("edx") hi, options(nomem, nostack));
    ((hi as u64) << 32) | (lo as u64)
}

unsafe fn write_msr(msr: u32, value: u64) {
    let lo = value as u32;
    let hi = (value >> 32) as u32;
    core::arch::asm!("wrmsr", in("ecx") msr, in("eax") lo, in("edx") hi, options(nomem, nostack));
}

pub fn init() {
    unsafe {
        SYSCALL_STACK_TOP = KERNEL_SYSCALL_STACK.0.as_ptr() as u64 + KERNEL_SYSCALL_STACK_SIZE as u64;

        let efer = read_msr(IA32_EFER);
        write_msr(IA32_EFER, efer | 1); // bit0 = SCE, syscall enable

        let star = ((STAR_USER_BASE as u64) << 48) | ((gdt::KERNEL_CODE_SELECTOR as u64) << 32);
        write_msr(IA32_STAR, star);

        write_msr(IA32_LSTAR, syscall_entry as u64);

        // FMASK: bits puestos aquí se BORRAN de RFLAGS al entrar por
        // syscall. Enmascaramos IF (bit9) — durante los primeros
        // instantes del handler, antes de tener una pila de kernel
        // segura montada, no queremos que nos interrumpa nada más.
        write_msr(IA32_FMASK, 0x200);
    }
    serial_println!("[syscall] MSRs configuradas (EFER.SCE, STAR, LSTAR, FMASK)");
}

/// Punto de entrada de `syscall` — ejecuta en ring 0, pero con la pila
/// que tuviera el programa de usuario (¡syscall NO cambia de pila
/// automáticamente, a diferencia de una interrupción con TSS!). Lo
/// primero que hacemos es cambiar a una pila de kernel conocida antes
/// de tocar nada más.
///
/// Limitación conocida de esta pasada: `USER_RSP_SCRATCH` es una única
/// variable global — no es reentrante ni SMP-safe. Válido mientras solo
/// haya una syscall en vuelo a la vez (nuestro caso actual, sin
/// preemption ni multi-core todavía).
#[unsafe(naked)]
pub unsafe extern "C" fn syscall_entry() {
    core::arch::naked_asm!(
        "mov [{user_rsp}], rsp",
        "mov rsp, [{stack_top}]",
        "push rcx", // RIP de retorno a ring 3, puesto por `syscall`
        "push r11", // RFLAGS de usuario, puesto por `syscall`
        "push r9",
        "push r8",
        "push r10",
        "push rdx",
        "push rsi",
        "push rdi",
        "push rax", // número de syscall — al final, para que quede en
                     // la dirección más baja (inicio del SyscallFrame)
        "mov rdi, rsp", // puntero al SyscallFrame, 1er argumento SysV
        "call {handler}", // devuelve el valor de retorno en rax
        "add rsp, 8", // descarta el rax guardado (num) sin tocar el
                        // rax actual, que ya tiene el valor de retorno
        "pop rdi",
        "pop rsi",
        "pop rdx",
        "pop r10",
        "pop r8",
        "pop r9",
        "pop r11",
        "pop rcx",
        "mov rsp, [{user_rsp}]",
        "sysretq",
        user_rsp = sym USER_RSP_SCRATCH,
        stack_top = sym SYSCALL_STACK_TOP,
        handler = sym syscall_dispatch,
    );
}

extern "C" fn syscall_dispatch(frame: *mut SyscallFrame) -> u64 {
    let frame = unsafe { &*frame };
    match frame.num {
        SYS_PING => {
            // Primera vez que `caps::enforce` se llama de verdad desde
            // una syscall real, no desde una demo aislada (M1). SYS_PING
            // se trata como una operación de "consola/stdio" — requiere
            // CAP_STDIO igual que cualquier syscall de escritura futura.
            if let Err(v) = caps::enforce_current(caps::CAP_STDIO) {
                serial_println!(
                    "[syscall] SYS_PING DENEGADO — pedido=0x{:x}, otorgado=0x{:x} (falta CAP_STDIO)",
                    v.requested,
                    v.granted
                );
                return u64::MAX; // EPERM, convención simplificada
            }

            serial_println!(
                "[syscall] SYS_PING recibido desde ring 3 — arg0=0x{:x}. Volviendo a ring 3 vía sysret.",
                frame.arg0
            );
            0xC0FFEE
        }
        SYS_GETPID => process::current_pid(),
        SYS_FORK => {
            if let Err(v) = caps::enforce_current(caps::CAP_EXEC) {
                serial_println!(
                    "[syscall] SYS_FORK DENEGADO — pedido=0x{:x}, otorgado=0x{:x} (falta CAP_EXEC)",
                    v.requested,
                    v.granted
                );
                return u64::MAX;
            }
            sys_fork(frame)
        }
        SYS_EXECVE => {
            if let Err(v) = caps::enforce_current(caps::CAP_EXEC) {
                serial_println!(
                    "[syscall] SYS_EXECVE DENEGADO — pedido=0x{:x}, otorgado=0x{:x} (falta CAP_EXEC)",
                    v.requested,
                    v.granted
                );
                return u64::MAX;
            }
            sys_execve(frame)
        }
        SYS_EXIT => sys_exit(frame),
        SYS_WAIT => sys_wait(frame),
        SYS_KILL => sys_kill(frame),
        other => {
            serial_println!("[syscall] número desconocido: {}", other);
            u64::MAX
        }
    }
}

/// `fork()` — clona el proceso actual: nuevo PID, copia profunda del
/// espacio de direcciones (`mmu::clone_address_space`), y una tarea de
/// scheduler nueva para el hijo. El padre recibe el PID del hijo por
/// el camino normal (el valor de retorno de esta función se convierte
/// en `rax` en `syscall_entry`, vía `sysretq`, igual que `SYS_PING`).
/// El hijo NO vuelve por aquí — arranca en `fork_child_trampoline`,
/// ver su comentario.
fn sys_fork(frame: &SyscallFrame) -> u64 {
    let parent_pid = process::current_pid();
    let parent = match process::find(parent_pid) {
        Some(p) => p,
        None => {
            serial_println!(
                "[syscall] fork: PID actual ({}) no está registrado en la tabla de procesos",
                parent_pid
            );
            return u64::MAX;
        }
    };

    let child_p4 = match unsafe { mmu::clone_address_space(parent.page_table) } {
        Some(p4) => p4,
        None => {
            serial_println!("[syscall] fork: sin memoria física para clonar el espacio de direcciones");
            return u64::MAX;
        }
    };

    let child_pid = process::alloc_pid();
    process::register(child_pid, parent_pid, child_p4);

    // Handoff de un solo slot hacia fork_child_trampoline — ver su
    // aviso de "un fork en vuelo a la vez".
    unsafe {
        FORK_CHILD_PID = child_pid;
        FORK_CHILD_RIP = frame.user_rip;
        FORK_CHILD_RSP = USER_RSP_SCRATCH;
    }
    scheduler::spawn_with_space(fork_child_trampoline, child_p4, child_pid);

    serial_println!(
        "[syscall] fork: PID {} -> hijo PID {} (PML4 nuevo @ 0x{:x})",
        parent_pid,
        child_pid,
        child_p4
    );

    child_pid
}

/// Punto de entrada del hijo de `fork()` como tarea nueva del
/// scheduler. No llega aquí por `sysretq` (nunca ejecutó `syscall` él
/// mismo) — por eso hace falta `enter_ring3_with_rax` en vez del
/// camino normal: fabrica desde cero el mismo punto de retorno que
/// tendría el padre (mismo RIP/RSP de usuario, capturados en
/// `sys_fork` desde el `SyscallFrame`/`USER_RSP_SCRATCH` del padre),
/// pero con `rax=0` — la convención de `fork()` para el hijo.
fn fork_child_trampoline() -> ! {
    unsafe {
        // `current_pid()` ya lo dejó puesto el scheduler (`yield_now`,
        // con `Task::pid`) antes de saltar aquí.
        let rip = FORK_CHILD_RIP;
        let rsp = FORK_CHILD_RSP;
        ring3::enter_ring3_with_rax(rip, rsp, 0);
    }
}

/// `exit()` — marca el proceso actual como zombie y cede el turno para
/// siempre. Su padre lo recoge con `wait()` (`sys_wait`, más abajo),
/// que también libera de verdad la memoria del hijo — suficiente para
/// probar que `exit()` termina el
/// proceso de verdad y no vuelve a ejecutarse.
fn sys_exit(frame: &SyscallFrame) -> ! {
    let pid = process::current_pid();
    let code = frame.arg0 as i32;
    process::mark_zombie(pid, code);
    serial_println!("[syscall] exit: PID {} terminó con código {}", pid, code);
    scheduler::mark_current_finished();
    // `exit()` diverge a propósito: nunca llega al `sysretq` de
    // `syscall_entry`, que es donde normalmente se restaura RFLAGS (y
    // con él, IF) desde `r11`. `syscall_entry` enmascara IF vía FMASK
    // nada más entrar — sin este `sti`, las interrupciones quedan
    // desactivadas PARA SIEMPRE en cuanto cualquier proceso llama a
    // `exit()`, y el `hlt` de `console::read_line()` (que depende de
    // una interrupción para despertar) se cuelga para siempre, aunque
    // la CPU no haya crasheado — visto de primera mano: `forktest`
    // dejaba la consola sin responder a ninguna tecla más.
    unsafe { core::arch::asm!("sti") };
    loop {
        scheduler::yield_now();
    }
}

/// `wait()` — espera a que termine un hijo y recoge su código de
/// salida. `arg0` = puntero de usuario a un `i32` donde escribir el
/// código de salida (0/NULL para ignorarlo, como `wait(NULL)` en
/// POSIX). Devuelve el PID del hijo recogido, o `u64::MAX` si el
/// proceso llamante no tiene NINGÚN hijo (vivo o muerto) — equivalente
/// a ECHILD, no tiene sentido esperar.
///
/// Bloqueante de verdad, sin ninguna primitiva de sincronización
/// nueva: si hay hijos pero ninguno es zombie todavía, cede el turno
/// en bucle (`scheduler::yield_now()`) — mismo patrón cooperativo que
/// ya usa `exit()`. El hijo es una tarea más del scheduler, así que el
/// round-robin acaba llegando a él, dejándolo correr hasta su propio
/// `exit()` — la vuelta siguiente de este bucle lo encuentra zombie.
fn sys_wait(frame: &SyscallFrame) -> u64 {
    let caller = process::current_pid();
    if !process::has_children(caller) {
        return u64::MAX;
    }
    loop {
        if let Some((child_pid, exit_code)) = process::reap_zombie(caller) {
            if frame.arg0 != 0 {
                unsafe { *(frame.arg0 as *mut i32) = exit_code };
            }
            serial_println!(
                "[syscall] wait: PID {} recogió al hijo PID {} (código {}) — memoria liberada",
                caller,
                child_pid,
                exit_code
            );
            return child_pid;
        }
        scheduler::yield_now();
    }
}

/// `kill()` — `arg0` = PID objetivo, `arg1` = número de señal. Solo
/// acción por defecto (terminar), sin manejadores propios todavía — ver
/// `signal.rs`. Si el objetivo es UNO MISMO, el espacio de direcciones
/// que `sysretq` necesitaría para volver a ring 3 ya no existe cuando
/// `deliver_default` termina — mismo camino divergente que `sys_exit()`
/// (`sti` + ceder el turno para siempre), nunca volvemos normalmente.
fn sys_kill(frame: &SyscallFrame) -> u64 {
    if let Err(v) = caps::enforce_current(caps::CAP_PROC_CTL) {
        serial_println!(
            "[syscall] SYS_KILL DENEGADO — pedido=0x{:x}, otorgado=0x{:x} (falta CAP_PROC_CTL)",
            v.requested,
            v.granted
        );
        return u64::MAX;
    }

    let target = frame.arg0;
    let sig = frame.arg1 as u32;
    let caller = process::current_pid();

    if !signal::deliver_default(target, sig) {
        serial_println!("[syscall] kill: PID {} -> PID {} falló (no existe o ya era zombie)", caller, target);
        return u64::MAX;
    }
    serial_println!("[syscall] kill: PID {} mandó señal {} a PID {}", caller, sig, target);

    if target == caller {
        unsafe { core::arch::asm!("sti") };
        loop {
            scheduler::yield_now();
        }
    }

    0
}

/// `execve()` — reemplaza el proceso actual por un binario nuevo.
/// `arg0` = puntero (en el espacio de direcciones YA activo del
/// proceso llamante) a una cadena con el nombre del fichero en el VFS.
/// Diverge a propósito en el camino de éxito: llama a `enter_ring3`
/// directamente desde aquí en vez de volver a `syscall_dispatch` — la
/// pila de kernel de esta syscall queda abandonada (se reutiliza en la
/// siguiente syscall, `SYSCALL_STACK_TOP` es fijo), y `sysretq` del
/// handler naked nunca se ejecuta para esta invocación.
fn sys_execve(frame: &SyscallFrame) -> u64 {
    let name_ptr = frame.arg0 as *const u8;
    let mut name_buf = [0u8; 64];
    let mut len = 0usize;
    unsafe {
        while len < name_buf.len() {
            let b = *name_ptr.add(len);
            if b == 0 {
                break;
            }
            name_buf[len] = b;
            len += 1;
        }
    }
    let name = match core::str::from_utf8(&name_buf[..len]) {
        Ok(s) => s,
        Err(_) => {
            serial_println!("[syscall] execve: nombre de fichero no es UTF-8 válido");
            return u64::MAX;
        }
    };

    let bytes = match vfs::read(name) {
        Some(b) => b,
        None => {
            serial_println!("[syscall] execve: '{}' no existe en el VFS", name);
            return u64::MAX;
        }
    };

    let loaded = match elf::load(&bytes) {
        Ok(l) => l,
        Err(e) => {
            serial_println!("[syscall] execve: carga de ELF falló: {}", e);
            return u64::MAX;
        }
    };

    let stack_phys = match unsafe { crate::pmm::alloc_frame() } {
        Some(f) => f,
        None => {
            serial_println!("[syscall] execve: sin memoria física para la pila de usuario");
            return u64::MAX;
        }
    };
    // 768 GiB, índice P4 = 1 — genuinamente privado del nuevo espacio.
    // Antes vivía en 448 GiB (P4 = 0, POR DEBAJO del límite de 512 GiB
    // documentado en elf.rs): parecía funcionar porque hasta ahora
    // ningún binario había llamado a `execve()` dos veces en el mismo
    // arranque. Lo encontró `ember` (m7-ember): la primera llamada dejó
    // la página mapeada para siempre en el P4[0] COMPARTIDO por todo el
    // kernel (`free_address_space` nunca toca P4[0], a propósito), y la
    // segunda llamada chocaba contra esa misma dirección ya ocupada.
    let user_stack_virt: u64 = 0x0000_00C0_0000_0000;
    if let Err(e) = unsafe { mmu::map_page_in(loaded.page_table, user_stack_virt, stack_phys, true, false) } {
        serial_println!("[syscall] execve: fallo mapeando la pila de usuario: {}", e);
        return u64::MAX;
    }
    let user_stack_top = user_stack_virt + 4096;

    let pid = process::current_pid();
    process::set_page_table(pid, loaded.page_table);

    serial_println!(
        "[syscall] execve: PID {} -> '{}' cargado, entry=0x{:x} (espacio de direcciones reemplazado)",
        pid,
        name,
        loaded.entry_point
    );

    unsafe {
        mmu::switch_address_space(loaded.page_table);
        ring3::enter_ring3(loaded.entry_point, user_stack_top);
    }
}
