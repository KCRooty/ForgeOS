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
use crate::pmm;
use crate::process;
use crate::process::FdEntry;
use crate::ring3;
use crate::scheduler;
use crate::serial::SerialPort;
use crate::serial_println;
use crate::signal;
use crate::tls;
use crate::vfs;
use alloc::string::String;
use alloc::vec::Vec;

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
pub const SYS_READ: u64 = 0;
// `SYS_WRITE` NO es el 1 real de Linux — SYS_PING, de una demo M1
// mucho anterior a M8, se quedó con ese número primero, y renumerarlo
// ahora rompería los bytes ya ensamblados a mano de cada `TEST_ELF_*`
// existente (`mov eax, 1` está incrustado en decenas de sitios). "Donde
// aplica" en el comentario de arriba significa esto: se sigue a Linux
// salvo colisión con algo que ya existía primero.
pub const SYS_WRITE: u64 = 63;
pub const SYS_OPEN: u64 = 2;
pub const SYS_CLOSE: u64 = 3;
pub const SYS_MMAP: u64 = 9;
pub const SYS_MUNMAP: u64 = 11;
pub const SYS_BRK: u64 = 12;
pub const SYS_GETPID: u64 = 39;
pub const SYS_FORK: u64 = 57;
pub const SYS_EXECVE: u64 = 59;
pub const SYS_EXIT: u64 = 60;
pub const SYS_WAIT4: u64 = 61;            // wait4(pid, *status, options, *rusage)
pub const SYS_SCHED_YIELD: u64 = 24;      // M12
pub const SYS_GETPPID: u64 = 110;         // M12
pub const SYS_KILL: u64 = 62;
pub const SYS_LSEEK: u64 = 8;             // M11 — lseek(fd, offset, whence)
pub const SYS_FSTAT: u64 = 5;             // M8d — fstat(fd, *stat)
// Syscalls que mlibc necesita en su secuencia de arranque — M8c.
pub const SYS_RT_SIGPROCMASK: u64 = 14;   // stub — señales no implementadas aún
pub const SYS_ARCH_PRCTL: u64 = 158;      // ARCH_SET_FS/ARCH_GET_FS — TLS para mlibc
pub const SYS_FUTEX: u64 = 202;           // stub — single-threaded, sin bloqueo real
pub const SYS_SET_TID_ADDRESS: u64 = 218; // mlibc init; devuelve PID como TID

/// Flag de `mmap()` que de verdad comprobamos — el resto de bits reales
/// de Linux (`MAP_PRIVATE`, `MAP_FIXED`...) no aplican: solo existe
/// mapeo anónimo (sin fichero detrás), ver `sys_mmap`.
pub const MAP_ANONYMOUS: u64 = 0x20;

/// Subcomandos de `arch_prctl()` que soportamos — el resto devuelve EINVAL.
pub const ARCH_SET_FS: u64 = 0x1002;
pub const ARCH_GET_FS: u64 = 0x1003;

/// Código de error POSIX compacto — todos los stubs lo usan cuando no
/// soportan la operación pedida, para no devolver u64::MAX (que es un
/// número arbitrario en el espacio del usuario) sino el -ENOSYS real de
/// Linux (38 en negativo, como i64, interpretado como u64).
const ENOSYS: u64 = (-38i64) as u64;
const EINVAL: u64 = (-22i64) as u64;
const ENOENT: u64 = (-2i64) as u64;
const ESRCH: u64 = (-3i64) as u64;
const ECHILD: u64 = (-10i64) as u64;
const E2BIG: u64 = (-7i64) as u64;
const ENOEXEC: u64 = (-8i64) as u64;
const ENOMEM: u64 = (-12i64) as u64;
use crate::usercopy::{self, EFAULT};
const EBADF: u64 = (-9i64) as u64;
const ESPIPE: u64 = (-29i64) as u64;

/// Flag de `open()` para pedir escritura — el resto de bits de Linux
/// (`O_CREAT`, `O_TRUNC`...) no aplican todavía: `sys_open` siempre
/// escribe en blanco (ver su comentario), así que no hay nada que
/// decidir a partir de ellos por ahora.
pub const O_WRONLY: u64 = 0x1;

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
    /// Registros que la ABI SysV obliga a conservar a través de una
    /// llamada. Ninguna syscall los necesita como argumento, pero
    /// `fork()` sí: el hijo tiene que arrancar con los mismos valores
    /// que el padre (ver `sys_fork`).
    pub rbx: u64,
    pub rbp: u64,
    pub r12: u64,
    pub r13: u64,
    pub r14: u64,
    pub r15: u64,
    /// RSP de usuario en el momento del `syscall`, guardado en la pila
    /// de kernel de ESTA tarea — ver `syscall_entry`.
    pub user_rsp: u64,
}

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
/// `USER_RSP_SCRATCH` es una única variable global, pero solo hace de
/// puente durante las dos primeras instrucciones (con IF=0 por FMASK,
/// nada puede colarse en medio): el RSP de usuario se copia enseguida a
/// la pila de kernel de la tarea (`push [user_rsp]`), y a la vuelta se
/// recupera de AHÍ (`pop rsp`), no del global.
///
/// Antes se restauraba del global, y eso era un bug real aunque latente:
/// una syscall que cede el turno a medias (`wait()`) dejaba pasar a otro
/// proceso, cuya propia syscall pisaba el global — y al reanudarse, la
/// primera volvía a ring 3 con el RSP DEL OTRO proceso. Los binarios de
/// prueba en ensamblador no lo notaban porque no usan la pila después de
/// `wait()`; cualquier programa en C sí.
#[unsafe(naked)]
pub unsafe extern "C" fn syscall_entry() {
    core::arch::naked_asm!(
        "mov [{user_rsp}], rsp",
        "mov rsp, [{stack_top}]",
        "push qword ptr [{user_rsp}]", // RSP de usuario, a salvo en la pila de esta tarea
        "push r15", // callee-saved: solo los lee `fork()`, ver `SyscallFrame`
        "push r14",
        "push r13",
        "push r12",
        "push rbp",
        "push rbx",
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
        "pop rbx",
        "pop rbp",
        "pop r12",
        "pop r13",
        "pop r14",
        "pop r15",
        "pop rsp", // RSP de usuario de ESTA tarea
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
        SYS_READ => sys_read(frame),
        SYS_WRITE => sys_write(frame),
        SYS_OPEN => sys_open(frame),
        SYS_CLOSE => sys_close(frame),
        SYS_MMAP => sys_mmap(frame),
        SYS_MUNMAP => 0, // no-op a propósito, ver comentario de `Pcb.mmap_next`
        SYS_BRK => sys_brk(frame),
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
        SYS_WAIT4 => sys_wait4(frame),
        SYS_SCHED_YIELD => {
            scheduler::yield_now();
            0
        }
        SYS_GETPPID => process::find(process::current_pid()).map(|p| p.parent_pid).unwrap_or(0),
        SYS_KILL => sys_kill(frame),
        SYS_FSTAT => sys_fstat(frame),
        SYS_LSEEK => sys_lseek(frame),
        SYS_RT_SIGPROCMASK => sys_rt_sigprocmask(frame),
        SYS_ARCH_PRCTL => sys_arch_prctl(frame),
        SYS_FUTEX => sys_futex(frame),
        SYS_SET_TID_ADDRESS => sys_set_tid_address(frame),
        other => {
            serial_println!("[syscall] número desconocido: {}", other);
            u64::MAX
        }
    }
}

/// `fork()` — clona el proceso actual: nuevo PID, copia profunda del
/// espacio de direcciones (`mmu::clone_address_space`), y una tarea de
/// scheduler nueva para el hijo. El padre recibe el PID del hijo por el
/// camino normal (`rax` vía `sysretq`).
///
/// El hijo no pasa por aquí: su tarea arranca en
/// `process::user_task_trampoline`, que entra en ring 3 con una copia de
/// TODOS los registros del padre en el instante de la syscall y `rax` =
/// 0. Hereda además lo que un programa en C da por hecho que sobrevive
/// a un `fork()`: descriptores abiertos, límites del heap y del arena de
/// `mmap()` (sin ellos su primer `malloc` intentaría mapear encima de
/// páginas que ya tiene), el puntero de TLS y el estado FPU/SSE.
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
            return ENOMEM;
        }
    };

    let child_pid = process::alloc_pid();
    process::register(child_pid, parent_pid, child_p4);

    let context = process::UserContext {
        rax: 0, // lo que `fork()` devuelve en el hijo
        rbx: frame.rbx,
        // `syscall` deja en rcx/r11 el RIP y los RFLAGS de retorno; el
        // padre los verá así al volver por `sysretq`, y el hijo igual.
        rcx: frame.user_rip,
        rdx: frame.arg2,
        rsi: frame.arg1,
        rdi: frame.arg0,
        rbp: frame.rbp,
        r8: frame.arg4,
        r9: frame.arg5,
        r10: frame.arg3,
        r11: frame.user_rflags,
        r12: frame.r12,
        r13: frame.r13,
        r14: frame.r14,
        r15: frame.r15,
        rip: frame.user_rip,
        rsp: frame.user_rsp,
        rflags: frame.user_rflags | 0x200, // con interrupciones, siempre
    };
    let fs_base = tls::read_fs_base();
    process::with_pid_mut(child_pid, |child| {
        child.fds = parent.fds.clone();
        child.heap_top = parent.heap_top;
        child.heap_mapped_end = parent.heap_mapped_end;
        child.mmap_next = parent.mmap_next;
        child.fs_base = fs_base;
        child.tid_ptr = parent.tid_ptr;
        child.start = Some(context);
    });

    scheduler::spawn_with_space(process::user_task_trampoline, child_p4, child_pid);
    // El estado FPU/SSE del padre, tal cual está ahora mismo en la CPU
    // (el kernel no lo toca): modo de redondeo, registros XMM vivos...
    let mut fpu = tls::FxState::new();
    unsafe { tls::fxsave(&mut fpu) };
    scheduler::set_task_fpu(child_pid, fpu);

    serial_println!(
        "[syscall] fork: PID {} -> hijo PID {} (PML4 nuevo @ 0x{:x})",
        parent_pid,
        child_pid,
        child_p4
    );

    child_pid
}

/// `exit()` — marca el proceso actual como zombie y cede el turno para
/// siempre. Su padre lo recoge con `wait()` (`sys_wait`, más abajo),
/// que también libera de verdad la memoria del hijo — suficiente para
/// probar que `exit()` termina el
/// proceso de verdad y no vuelve a ejecutarse.
fn sys_exit(frame: &SyscallFrame) -> ! {
    let pid = process::current_pid();
    // Solo los 8 bits bajos, como en POSIX: `exit(-1)` es el código 255.
    let code = (frame.arg0 & 0xFF) as i32;
    process::mark_zombie(pid, process::exit_status(code));
    process::reparent_children(pid, 0);
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

/// `wait4(pid, status, options, rusage)` — con la ABI de Linux, que es
/// la que usa `waitpid()` de mlibc:
///
/// - `pid` > 0 espera a ese hijo concreto; -1 (o 0: no hay grupos de
///   procesos) a cualquiera.
/// - `status`, si no es NULL, recibe el wait status (`WIFEXITED`,
///   `WEXITSTATUS`, `WIFSIGNALED`, `WTERMSIG`...).
/// - `options`: `WNOHANG` (1) devuelve 0 en vez de esperar si ningún
///   hijo ha terminado todavía.
/// - `rusage` se ignora.
///
/// Devuelve el PID recogido, o `-ECHILD` si no hay a quién esperar.
/// Recoger a un hijo libera de verdad su memoria.
///
/// Bloqueante sin ninguna primitiva de sincronización nueva: si hay
/// hijos pero ninguno es zombie todavía, cede el turno en bucle
/// (`scheduler::yield_now()`), mismo patrón cooperativo que `exit()`.
fn sys_wait4(frame: &SyscallFrame) -> u64 {
    const WNOHANG: u64 = 1;

    let caller = process::current_pid();
    let wanted = match frame.arg0 as i64 {
        pid if pid > 0 => Some(pid as u64),
        _ => None,
    };
    let status_ptr = frame.arg1;
    let options = frame.arg2;

    // Antes de esperar nada: un puntero malo es EFAULT ya, no después de
    // haber recogido (y perdido) el estado del hijo.
    if status_ptr != 0 && usercopy::check(status_ptr, 4, true).is_err() {
        return EFAULT;
    }

    loop {
        let has_target = match wanted {
            Some(pid) => process::is_child(caller, pid),
            None => process::has_children(caller),
        };
        if !has_target {
            return ECHILD;
        }
        if let Some((child_pid, status)) = process::reap_zombie(caller, wanted) {
            if status_ptr != 0 && usercopy::write(status_ptr, status).is_err() {
                return EFAULT;
            }
            serial_println!(
                "[syscall] wait: PID {} recogió al hijo PID {} (código {}) — memoria liberada",
                caller,
                child_pid,
                process::shell_code(status)
            );
            return child_pid;
        }
        if options & WNOHANG != 0 {
            return 0;
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
        return ESRCH;
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

/// Longitud máxima de una ruta que llega de un proceso.
const MAX_PATH: usize = 255;

/// Busca el primer slot libre (`None`) en `fds`, o añade uno nuevo si
/// no hay ninguno — el fd resultante es `3 + índice` (0/1/2 son
/// stdin/stdout/stderr, nunca ocupan un slot real, ver `process.rs`).
fn alloc_fd(fds: &mut Vec<Option<FdEntry>>, entry: FdEntry) -> u64 {
    for (i, slot) in fds.iter_mut().enumerate() {
        if slot.is_none() {
            *slot = Some(entry);
            return (i + 3) as u64;
        }
    }
    fds.push(Some(entry));
    (fds.len() - 1 + 3) as u64
}

/// `read(fd, buf, count)`. `fd=0` (stdin) lee una línea de la terminal
/// (`sys_read_stdin`); `fd>=3` sirve del buffer que `open()` ya cargó
/// entero del VFS.
fn sys_read(frame: &SyscallFrame) -> u64 {
    let fd = frame.arg0;
    let buf = frame.arg1;
    let count = frame.arg2 as usize;
    if usercopy::check(buf, count, true).is_err() {
        return EFAULT;
    }

    if fd == 0 {
        if let Err(v) = caps::enforce_current(caps::CAP_STDIO) {
            serial_println!("[syscall] SYS_READ DENEGADO (stdin) — falta CAP_STDIO (pedido=0x{:x})", v.requested);
            return u64::MAX;
        }
        return sys_read_stdin(buf, count);
    }
    if fd == 1 || fd == 2 {
        return u64::MAX; // EBADF: stdout/stderr no son legibles.
    }
    if let Err(v) = caps::enforce_current(caps::CAP_FS_READ) {
        serial_println!("[syscall] SYS_READ DENEGADO — falta CAP_FS_READ (pedido=0x{:x})", v.requested);
        return u64::MAX;
    }

    let pid = process::current_pid();
    let index = (fd as usize).wrapping_sub(3);
    let result = process::with_pid_mut(pid, |pcb| {
        let Some(Some(FdEntry::ReadFile { data, cursor })) = pcb.fds.get_mut(index) else {
            return u64::MAX;
        };
        let remaining = data.len() - *cursor;
        let n = core::cmp::min(count, remaining);
        if usercopy::copy_to_user(buf, &data[*cursor..*cursor + n]).is_err() {
            return EFAULT;
        }
        *cursor += n;
        n as u64
    });
    result.unwrap_or(u64::MAX)
}

/// `read(0, ...)` — una línea de la terminal (`tty.rs`): bloquea, cediendo
/// el turno, hasta que se pulse Intro. Ctrl-D es fin de fichero y Ctrl-C
/// mata al proceso (SIGINT), como en cualquier Unix.
fn sys_read_stdin(buf: u64, count: usize) -> u64 {
    if count == 0 {
        return 0;
    }
    let mut line = alloc::vec![0u8; count.min(4096)];
    loop {
        match crate::tty::try_read(&mut line) {
            crate::tty::Read::Data(n) => {
                return match usercopy::copy_to_user(buf, &line[..n]) {
                    Ok(()) => n as u64,
                    Err(_) => EFAULT,
                };
            }
            crate::tty::Read::WouldBlock => scheduler::yield_now(),
            crate::tty::Read::Interrupted => {
                // Soltar el buffer a mano: de aquí no se vuelve.
                drop(line);
                signal::kill_current(signal::SIGINT);
                unsafe { core::arch::asm!("sti") };
                loop {
                    scheduler::yield_now();
                }
            }
        }
    }
}

/// `write(fd, buf, count)` — `fd=1`/`2` van al puerto serie (stdout y
/// stderr son el mismo canal en este kernel, no hay terminal separada
/// todavía). `fd>=3` acumula en el buffer en memoria de la entrada
/// `WriteFile`, que se vuelca de verdad al VFS en `close()`.
fn sys_write(frame: &SyscallFrame) -> u64 {
    let fd = frame.arg0;
    let count = frame.arg2 as usize;
    let Ok(bytes) = usercopy::slice(frame.arg1, count) else {
        return EFAULT;
    };

    if fd == 1 || fd == 2 {
        if let Err(v) = caps::enforce_current(caps::CAP_STDIO) {
            serial_println!("[syscall] SYS_WRITE DENEGADO (stdio) — falta CAP_STDIO (pedido=0x{:x})", v.requested);
            return u64::MAX;
        }
        let mut port = SerialPort::init();
        for &b in bytes {
            port.write_byte(b);
        }
        return count as u64;
    }
    if fd == 0 {
        return u64::MAX; // EBADF: stdin no es escribible.
    }
    if let Err(v) = caps::enforce_current(caps::CAP_FS_WRITE) {
        serial_println!("[syscall] SYS_WRITE DENEGADO — falta CAP_FS_WRITE (pedido=0x{:x})", v.requested);
        return u64::MAX;
    }

    let pid = process::current_pid();
    let index = (fd as usize).wrapping_sub(3);
    let result = process::with_pid_mut(pid, |pcb| {
        let Some(Some(FdEntry::WriteFile { data, .. })) = pcb.fds.get_mut(index) else {
            return u64::MAX;
        };
        data.extend_from_slice(bytes);
        count as u64
    });
    result.unwrap_or(u64::MAX)
}

/// `lseek(fd, offset, whence)` — devuelve la posición nueva, o un errno
/// negativo.
///
/// - fd 0/1/2 (el puerto serie) y los ficheros abiertos para escritura
///   (un buffer que solo crece hasta `close()`, ver `FdEntry::WriteFile`)
///   no tienen posición: `ESPIPE`. No es un detalle — es justo lo que
///   pregunta la stdio de mlibc la primera vez que usa un `FILE` (un
///   `lseek(fd, 0, SEEK_CUR)`) para decidir si es un fichero o un flujo.
///   Con cualquier otro error da el `FILE` por roto y `printf` no llega a
///   escribir nada.
/// - Ficheros abiertos para lectura: mueve el cursor de verdad
///   (`SEEK_SET`/`SEEK_CUR`/`SEEK_END`), sin dejarlo salir de [0, tamaño].
fn sys_lseek(frame: &SyscallFrame) -> u64 {
    const SEEK_SET: u64 = 0;
    const SEEK_CUR: u64 = 1;
    const SEEK_END: u64 = 2;

    let fd = frame.arg0;
    let offset = frame.arg1 as i64;
    let whence = frame.arg2;

    if fd <= 2 {
        return ESPIPE;
    }
    let pid = process::current_pid();
    let index = (fd as usize).wrapping_sub(3);
    let result = process::with_pid_mut(pid, |pcb| match pcb.fds.get_mut(index) {
        Some(Some(FdEntry::ReadFile { data, cursor })) => {
            let base = match whence {
                SEEK_SET => 0,
                SEEK_CUR => *cursor as i64,
                SEEK_END => data.len() as i64,
                _ => return EINVAL,
            };
            match base.checked_add(offset) {
                Some(pos) if pos >= 0 && pos <= data.len() as i64 => {
                    *cursor = pos as usize;
                    pos as u64
                }
                _ => EINVAL,
            }
        }
        Some(Some(FdEntry::WriteFile { .. })) => ESPIPE,
        _ => EBADF,
    });
    result.unwrap_or(EBADF)
}

/// `open(path, flags)` — solo mira el bit `O_WRONLY`, ver su
/// definición más arriba. Sin él: lectura, el fichero debe existir ya
/// en el VFS. Con él: escritura, siempre empieza en blanco (como si
/// `O_TRUNC` estuviera puesto siempre) — no hay escritura incremental
/// real al VFS todavía (ver `FdEntry::WriteFile`), así que "abrir para
/// escritura sin truncar" no tiene sentido aquí; `O_CREAT` tampoco hace
/// falta comprobarlo, escribir siempre "crea" el fichero de todas
/// formas (`vfs::write` no distingue crear de sobrescribir).
fn sys_open(frame: &SyscallFrame) -> u64 {
    let writing = frame.arg1 & O_WRONLY != 0;
    if let Err(v) = caps::enforce_current(if writing { caps::CAP_FS_WRITE } else { caps::CAP_FS_READ }) {
        serial_println!(
            "[syscall] SYS_OPEN DENEGADO — pedido=0x{:x}, otorgado=0x{:x}",
            v.requested,
            v.granted
        );
        return u64::MAX;
    }

    let Ok(name) = usercopy::read_cstr(frame.arg0, MAX_PATH) else {
        return EFAULT;
    };
    let name = name.as_str();

    let entry = if writing {
        FdEntry::WriteFile { name: String::from(name), data: Vec::new() }
    } else {
        match vfs::read(name) {
            Some(data) => FdEntry::ReadFile { data, cursor: 0 },
            None => {
                serial_println!("[syscall] open: '{}' no existe en el VFS", name);
                return ENOENT;
            }
        }
    };

    let pid = process::current_pid();
    let fd = process::with_pid_mut(pid, |pcb| alloc_fd(&mut pcb.fds, entry)).unwrap_or(u64::MAX);
    serial_println!(
        "[syscall] open: PID {} -> '{}' ({}), fd={}",
        pid,
        name,
        if writing { "escritura" } else { "lectura" },
        fd
    );
    fd
}

/// `close(fd)` — para un `WriteFile`, este es el momento en que el
/// contenido acumulado en memoria se vuelca de verdad al VFS
/// (`vfs::write`); antes de esto, cerrar sin haber llamado a `close()`
/// perdería los datos (igual que un `fclose()` real necesita para que
/// el buffer de libc no se quede a medias).
fn sys_close(frame: &SyscallFrame) -> u64 {
    let fd = frame.arg0;
    if fd < 3 {
        return 0; // cerrar stdin/stdout/stderr es un no-op válido.
    }
    let pid = process::current_pid();
    let index = (fd as usize).wrapping_sub(3);
    let result = process::with_pid_mut(pid, |pcb| {
        let Some(slot) = pcb.fds.get_mut(index) else {
            return u64::MAX;
        };
        match slot.take() {
            Some(FdEntry::WriteFile { name, data }) => {
                serial_println!("[syscall] close: PID {} vuelca {} bytes en '{}'", pid, data.len(), name);
                vfs::write(&name, &data);
                0
            }
            Some(FdEntry::ReadFile { .. }) => 0,
            None => u64::MAX,
        }
    });
    result.unwrap_or(u64::MAX)
}

/// `mmap(addr, length, prot, flags, fd, offset)` — M8b, segunda mitad
/// de "memoria real para malloc()" (la primera fue `brk()`, M8). Solo
/// soporta el caso que un `malloc()` de verdad necesita para pedir un
/// bloque grande directamente: `MAP_ANONYMOUS` (sin fichero detrás) —
/// mapeo de fichero real (`fd` != -1) devuelve error, no está
/// implementado. `addr` (la sugerencia de dónde mapear) se ignora
/// siempre: esta implementación decide su propia dirección
/// (`Pcb.mmap_next`, arena que solo crece — ver su comentario) en vez
/// de intentar honrar un `addr` de usuario, igual de válido según el
/// contrato POSIX (`MAP_FIXED` sería el único caso que de verdad
/// exigiría respetarlo, y no lo soportamos). `prot` también se ignora
/// — todo mapeo anónimo sale legible+escribible, nunca ejecutable; no
/// hay ahora mismo ningún caso de uso real (código cargado dinámico)
/// que necesite distinguir protecciones aquí.
fn sys_mmap(frame: &SyscallFrame) -> u64 {
    if let Err(v) = caps::enforce_current(caps::CAP_MEM_MAP) {
        serial_println!("[syscall] SYS_MMAP DENEGADO — falta CAP_MEM_MAP (pedido=0x{:x})", v.requested);
        return u64::MAX;
    }

    let length = frame.arg1;
    let flags = frame.arg3;
    if length == 0 {
        return u64::MAX;
    }
    if flags & MAP_ANONYMOUS == 0 {
        serial_println!("[syscall] mmap: solo se soporta MAP_ANONYMOUS (sin fichero detrás)");
        return u64::MAX;
    }

    let pid = process::current_pid();
    let Some(pcb) = process::find(pid) else {
        return u64::MAX;
    };

    let num_pages = (length + 0xFFF) / 0x1000;
    let start = pcb.mmap_next;
    let mut page = start;
    for _ in 0..num_pages {
        let Some(phys) = pmm::alloc_frame() else {
            serial_println!("[syscall] mmap: sin memoria física");
            return u64::MAX;
        };
        // Un mapeo anónimo se entrega a cero, siempre — es lo que promete
        // `mmap()` (el allocator de mlibc cuenta con ello) y evita además
        // filtrar a este proceso lo que otro dejó en el frame.
        unsafe { core::ptr::write_bytes(phys as *mut u8, 0, 4096) };
        if let Err(e) = unsafe { mmu::map_page_in(pcb.page_table, page, phys, true, false) } {
            serial_println!("[syscall] mmap: fallo mapeando 0x{:x}: {}", page, e);
            return u64::MAX;
        }
        page += 0x1000;
    }

    process::with_pid_mut(pid, |p| p.mmap_next = page);
    start
}

/// `brk(addr)` — `addr=0` consulta el límite actual sin cambiar nada
/// (convención estándar de `brk()`). Solo crece de verdad mapeando
/// páginas nuevas cuando `addr` supera lo que YA está mapeado
/// (`heap_mapped_end`, ver su comentario en `process.rs` sobre por qué
/// hace falta un segundo campo aparte de `heap_top`); bajar el límite
/// nunca desmapea, es contabilidad lógica únicamente.
fn sys_brk(frame: &SyscallFrame) -> u64 {
    if let Err(v) = caps::enforce_current(caps::CAP_MEM_MAP) {
        serial_println!("[syscall] SYS_BRK DENEGADO — falta CAP_MEM_MAP (pedido=0x{:x})", v.requested);
        return u64::MAX;
    }

    let requested = frame.arg0;
    let pid = process::current_pid();
    let Some(pcb) = process::find(pid) else {
        return u64::MAX;
    };
    if requested == 0 {
        return pcb.heap_top;
    }
    if requested <= pcb.heap_top {
        process::with_pid_mut(pid, |p| p.heap_top = requested);
        return requested;
    }

    let new_mapped_end = (requested + 0xFFF) & !0xFFF;
    let mut page = pcb.heap_mapped_end;
    while page < new_mapped_end {
        let Some(phys) = pmm::alloc_frame() else {
            serial_println!("[syscall] brk: sin memoria física para crecer el heap");
            return pcb.heap_top;
        };
        if let Err(e) = unsafe { mmu::map_page_in(pcb.page_table, page, phys, true, false) } {
            serial_println!("[syscall] brk: fallo mapeando 0x{:x}: {}", page, e);
            return pcb.heap_top;
        }
        page += 4096;
    }

    process::with_pid_mut(pid, |p| {
        p.heap_top = requested;
        p.heap_mapped_end = new_mapped_end;
    });
    requested
}

/// `arch_prctl(code, addr)` — M8c. Solo ARCH_SET_FS y ARCH_GET_FS.
///
/// `ARCH_SET_FS` es la syscall que mlibc llama para fijar su TCB: escribe
/// `arg1` (la dirección del TCB en el espacio de usuario) en el MSR
/// IA32_FS_BASE, y también lo guarda en el PCB para que el context switch
/// lo restaure cuando otra tarea le quite el turno y se lo devuelva.
/// Sin esto, cualquier función de mlibc que acceda a `%fs:0` obtiene
/// la dirección del TCB del último proceso que corrió — corrupción
/// silenciosa garantizada.
fn sys_arch_prctl(frame: &SyscallFrame) -> u64 {
    let code = frame.arg0;
    let addr = frame.arg1;
    let pid = process::current_pid();

    match code {
        ARCH_SET_FS => {
            tls::write_fs_base(addr);
            process::with_pid_mut(pid, |pcb| pcb.fs_base = addr);
            serial_println!("[syscall] arch_prctl: ARCH_SET_FS PID {} -> FS.base=0x{:x}", pid, addr);
            0
        }
        ARCH_GET_FS => {
            let fs = tls::read_fs_base();
            match usercopy::write(addr, fs) {
                Ok(()) => 0,
                Err(_) => EFAULT,
            }
        }
        _ => {
            serial_println!("[syscall] arch_prctl: código desconocido 0x{:x}", code);
            EINVAL
        }
    }
}

/// `set_tid_address(tidptr)` — M8c, stub para mlibc init.
///
/// mlibc llama a esto nada más arrancar para registrar un puntero donde
/// el kernel escribirá el TID al hacer exit() (para FUTEX_WAKE_OP). En
/// este kernel sin hilos reales, simplemente guardamos el puntero en el
/// PCB y devolvemos el PID como TID — es lo que mlibc espera recibir y
/// lo que usará como `__thread_self->tid`.
fn sys_set_tid_address(frame: &SyscallFrame) -> u64 {
    let tidptr = frame.arg0;
    let pid = process::current_pid();
    process::with_pid_mut(pid, |pcb| pcb.tid_ptr = tidptr);
    pid // TID = PID en un kernel sin threading
}

/// `rt_sigprocmask(how, set, oldset, sigsetsize)` — M8c, stub.
///
/// mlibc lo llama al init para bloquear señales durante operaciones
/// críticas. Este kernel tiene señales básicas (signal.rs) pero sin
/// máscara por proceso todavía. Devolver 0 (éxito sin hacer nada) es
/// correcto: si no hay señales pendientes, enmascarar/desenmascarar es
/// un no-op observable y mlibc continúa sin problema.
fn sys_rt_sigprocmask(_frame: &SyscallFrame) -> u64 {
    0 // éxito, sin máscara real todavía
}

/// `futex(uaddr, op, val, ...)` — M8c, stub single-threaded.
///
/// mlibc usa FUTEX para su mutex interno en libc. En un kernel sin hilos
/// reales:
/// - FUTEX_WAIT / FUTEX_WAIT_PRIVATE: si `*uaddr == val` devolver 0
///   (fingir que ya fue despertado inmediatamente) — sin bloquearse de
///   verdad, que en single-threaded sería un deadlock seguro.
/// - FUTEX_WAKE / FUTEX_WAKE_PRIVATE: devolver 0 (0 waiters despertados).
/// - Cualquier otra operación: ENOSYS.
fn sys_futex(frame: &SyscallFrame) -> u64 {
    let op = frame.arg1 & 0x7F; // quitar FUTEX_PRIVATE_FLAG (bit 7)
    match op {
        0 /* FUTEX_WAIT */ => 0,
        1 /* FUTEX_WAKE */ => 0,
        _ => {
            serial_println!("[syscall] futex: op={} no soportado (single-threaded stub)", op);
            ENOSYS
        }
    }
}

/// `struct stat` de Linux x86_64 — layout idéntico al que usa mlibc en sus
/// sysdeps Linux. 144 bytes, `repr(C)` obligatorio para que el proceso de
/// usuario lea los campos en las posiciones que espera.
#[repr(C)]
#[derive(Clone, Copy)]
struct Stat {
    st_dev:        u64,      // 0
    st_ino:        u64,      // 8
    st_nlink:      u64,      // 16
    st_mode:       u32,      // 24 — S_IFREG (0x8000) o S_IFCHR (0x2000) + permisos
    st_uid:        u32,      // 28
    st_gid:        u32,      // 32
    __pad0:        u32,      // 36
    st_rdev:       u64,      // 40
    st_size:       i64,      // 48
    st_blksize:    i64,      // 56
    st_blocks:     i64,      // 64 — bloques de 512 bytes
    st_atime:      u64,      // 72
    st_atime_nsec: u64,      // 80
    st_mtime:      u64,      // 88
    st_mtime_nsec: u64,      // 96
    st_ctime:      u64,      // 104
    st_ctime_nsec: u64,      // 112
    __unused:      [i64; 3], // 120
}

/// `fstat(fd, statbuf)` — M8d. Rellena un `struct stat` en el espacio de
/// usuario con metadatos del fd abierto. No hay inodes reales ni timestamps —
/// lo que mlibc necesita en el arranque es `st_mode` (tipo de fichero) y
/// `st_size` (tamaño para `lseek()` interno).
///
/// fd 0/1/2 → S_IFCHR (terminal/serie), size 0.
/// fd >= 3  → S_IFREG, size = bytes acumulados en el FdEntry.
fn sys_fstat(frame: &SyscallFrame) -> u64 {
    let fd = frame.arg0;
    let stat_ptr = frame.arg1;
    if usercopy::check(stat_ptr, core::mem::size_of::<Stat>(), true).is_err() {
        return EFAULT;
    }

    let (mode, size): (u32, i64) = match fd {
        0 | 1 | 2 => (0x2000 | 0o666, 0), // S_IFCHR | rw-rw-rw-
        _ => {
            let pid = process::current_pid();
            let index = (fd as usize).wrapping_sub(3);
            let result = process::with_pid_mut(pid, |pcb| match pcb.fds.get(index) {
                Some(Some(FdEntry::ReadFile { data, .. })) => Some((0x8000u32 | 0o644, data.len() as i64)),
                Some(Some(FdEntry::WriteFile { data, .. })) => Some((0x8000u32 | 0o644, data.len() as i64)),
                _ => None,
            });
            match result.flatten() {
                Some(v) => v,
                None => return EBADF,
            }
        }
    };

    let stat = Stat {
        st_dev: 1,
        st_ino: fd + 1,
        st_nlink: 1,
        st_mode: mode,
        st_uid: 0,
        st_gid: 0,
        __pad0: 0,
        st_rdev: 0,
        st_size: size,
        st_blksize: 4096,
        st_blocks: (size + 511) / 512,
        st_atime: 0,
        st_atime_nsec: 0,
        st_mtime: 0,
        st_mtime_nsec: 0,
        st_ctime: 0,
        st_ctime_nsec: 0,
        __unused: [0; 3],
    };
    match usercopy::write(stat_ptr, stat) {
        Ok(()) => 0,
        Err(_) => EFAULT,
    }
}

/// Lo que `sys_execve` necesita para saltar al programa nuevo — solo
/// valores `Copy`: la función no vuelve, así que no puede quedarse con
/// nada que haya que liberar.
struct ExecImage {
    entry_point: u64,
    user_stack_top: u64,
    page_table: u64,
}

/// Primera mitad de `execve()`: todo lo que puede fallar. Lee la ruta y
/// `argv` del proceso llamante, localiza el binario, lo carga en un
/// espacio de direcciones nuevo y le monta la pila. Si algo falla
/// devuelve el errno y el proceso llamante sigue intacto.
///
/// El binario se busca primero en el VFS en memoria (donde viven los de
/// prueba del kernel, por nombre) y, si la ruta es absoluta, en el ext2
/// del disco (`/bin/...`).
fn prepare_exec(frame: &SyscallFrame) -> Result<ExecImage, u64> {
    let path = usercopy::read_cstr(frame.arg0, MAX_PATH).map_err(|_| EFAULT)?;

    // `argv`: vector de punteros terminado en NULL. Un `argv` NULL (lo
    // que pasan los binarios de prueba en ensamblador) equivale a
    // `{ path, NULL }`.
    let mut args: Vec<String> = Vec::new();
    let argv_ptr = frame.arg1;
    if argv_ptr != 0 {
        loop {
            let slot = argv_ptr + (args.len() as u64) * 8;
            let arg_ptr: u64 = usercopy::read(slot).map_err(|_| EFAULT)?;
            if arg_ptr == 0 {
                break;
            }
            if args.len() == process::MAX_ARGS {
                return Err(E2BIG);
            }
            args.push(usercopy::read_cstr(arg_ptr, 4096).map_err(|_| EFAULT)?);
        }
    }
    if args.is_empty() {
        args.push(path.clone());
    }
    let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();

    let bytes = match vfs::read(&path) {
        Some(b) => b,
        None if path.starts_with('/') => crate::ext2::read_file(&path).map_err(|e| {
            serial_println!("[syscall] execve: '{}': {}", path, e);
            ENOENT
        })?,
        None => {
            serial_println!("[syscall] execve: '{}' no existe en el VFS", path);
            return Err(ENOENT);
        }
    };

    let loaded = elf::load(&bytes).map_err(|e| {
        serial_println!("[syscall] execve: '{}': carga de ELF falló: {}", path, e);
        ENOEXEC
    })?;

    // 768 GiB, índice P4 = 1 — privado del espacio nuevo (ver `elf.rs`
    // sobre por qué nada de un proceso puede vivir por debajo de 512 GiB).
    let user_stack_virt: u64 = 0x0000_00C0_0000_0000;
    let user_stack_top = unsafe {
        process::map_fresh_user_stack(loaded.page_table, user_stack_virt, loaded.entry_point, &arg_refs)
    }
    .map_err(|e| {
        serial_println!("[syscall] execve: '{}': {}", path, e);
        unsafe { mmu::free_address_space(loaded.page_table) };
        E2BIG
    })?;

    serial_println!(
        "[syscall] execve: PID {} -> '{}' cargado, entry=0x{:x} (espacio de direcciones reemplazado)",
        process::current_pid(),
        path,
        loaded.entry_point
    );

    Ok(ExecImage {
        entry_point: loaded.entry_point,
        user_stack_top,
        page_table: loaded.page_table,
    })
}

/// `execve(path, argv, envp)` — reemplaza el proceso actual por un
/// binario nuevo. `envp` se ignora todavía (el programa nuevo arranca
/// sin variables de entorno). Los descriptores abiertos se conservan.
///
/// Diverge en el camino de éxito: salta a ring 3 directamente desde aquí
/// en vez de volver a `syscall_dispatch` — la pila de kernel de esta
/// syscall queda abandonada (se reutiliza en la siguiente).
fn sys_execve(frame: &SyscallFrame) -> u64 {
    let image = match prepare_exec(frame) {
        Ok(image) => image,
        Err(errno) => return errno,
    };

    let pid = process::current_pid();
    let old_page_table = process::find(pid).map(|p| p.page_table);
    process::set_page_table(pid, image.page_table);
    process::reset_for_exec(pid);
    // También en el scheduler, no solo en el PCB — ver el comentario
    // largo de `scheduler::set_task_page_table` sobre el bug real que
    // esto arregla (CR3 revertido al espacio VIEJO si el timer preempta
    // a este proceso en cualquier punto después de este `execve`).
    scheduler::set_task_page_table(pid, image.page_table);

    unsafe {
        mmu::switch_address_space(image.page_table);
        tls::write_fs_base(0);
        tls::fpu_init(); // el programa nuevo no hereda el estado FPU del anterior
        // El espacio anterior ya no lo usa nadie (CR3 acaba de cambiar):
        // devolver su memoria.
        if let Some(old) = old_page_table {
            if old != image.page_table {
                mmu::free_address_space(old);
            }
        }
        ring3::enter_ring3(image.entry_point, image.user_stack_top);
    }
}
