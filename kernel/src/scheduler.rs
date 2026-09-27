//! Scheduler cooperativo mínimo — M4a (BORRADOR SIN VERIFICAR).
//!
//! Round-robin, sin preferencia, sin preemption todavía — eso necesita
//! timer + IRQ real (APIC timer o PIT), que no existen aún (ver
//! TODO.md sección 1). Cada tarea corre hasta que llama a
//! `yield_now()` por su cuenta.

use crate::task::{Context, Task, TaskState};
use alloc::vec::Vec;
use core::cell::UnsafeCell;

/// Deshabilita IF al crearse, la restaura a lo que fuera ANTES (no
/// siempre a 1) al soltarse — RAII para que `yield_now()` pueda usar
/// `return` anticipado sin duplicar la restauración en cada punto de
/// salida. Bug real encontrado verificando el arranque automático de
/// Ember con preemption real (milestone "Ember arranca automáticamente
/// + preemption real permanente"): `yield_now()`/`switch_to` nunca
/// deshabilitaban IF durante su sección crítica — mientras la
/// preemption estuvo apagada por defecto (todo el resto de la sesión
/// hasta este punto) eso daba igual, porque nada disparaba el timer en
/// medio de un `yield_now()` llamado con IF=1. Pero `ember` deja la
/// preemption encendida PARA SIEMPRE desde el primer instante en que
/// arranca, y el propio arranque automático de `main.rs` cede el turno
/// varias veces con IF=1 (código de kernel normal, no dentro de una
/// syscall enmascarada) — si el timer dispara justo ahí, su propio
/// `timer_tick` vuelve a llamar a `yield_now()` DE FORMA REENTRANTE
/// sobre el mismo `SCHEDULER` global que la llamada externa todavía
/// está mutando (`sched.current`, CR3, `TSS.RSP0`, `CURRENT_PID`, y
/// sobre todo los registros a medio guardar/restaurar de
/// `task::switch_to`, que tampoco es atómico). Confirmado con arranques
/// repetidos en QEMU: en boots "malos" (~1 de cada 2-3), Ember (PID 1)
/// terminaba ejecutando literalmente el código de OTRO proceso (fork
/// espurio, page fault espurio) porque el cambio de contexto se
/// solapaba con una reentrada del timer a medio camino. Con IF
/// deshabilitado durante toda la decisión de scheduling + el cambio de
/// contexto, y restaurado solo una vez que `switch_to` ya volvió (esta
/// misma tarea, ya a salvo), el timer no puede volver a entrar hasta
/// que el cambio de contexto haya terminado de verdad.
struct IrqGuard {
    was_enabled: bool,
}

impl IrqGuard {
    fn disable() -> Self {
        let flags: u64;
        unsafe {
            core::arch::asm!("pushfq", "pop {0}", "cli", out(reg) flags);
        }
        IrqGuard { was_enabled: flags & (1 << 9) != 0 }
    }
}

impl Drop for IrqGuard {
    fn drop(&mut self) {
        if self.was_enabled {
            unsafe { core::arch::asm!("sti") };
        }
    }
}

pub struct Scheduler {
    tasks: Vec<Task>,
    current: usize,
}

struct SchedulerCell(UnsafeCell<Option<Scheduler>>);
unsafe impl Sync for SchedulerCell {}

static SCHEDULER: SchedulerCell = SchedulerCell(UnsafeCell::new(None));

/// El flujo que llama a `init()` se convierte implícitamente en la
/// "tarea 0" — normalmente el propio `kernel_main_upper` tras el boot.
pub fn init() {
    unsafe {
        let boot_task = Task::placeholder(0);
        let mut tasks = Vec::new();
        tasks.push(boot_task);
        *SCHEDULER.0.get() = Some(Scheduler { tasks, current: 0 });
    }
}

/// Añade una tarea nueva a la cola, con espacio de direcciones propio.
/// `page_table` = 0 para un hilo de kernel normal (comparte el espacio
/// activo, como `spawn`); distinto de 0 para un proceso con su propio
/// PML4 (creado con `mmu::create_address_space`). `pid` = 0 para un
/// hilo de kernel sin identidad de proceso; el PID real (`process.rs`)
/// para una tarea que representa un proceso — el scheduler lo restaura
/// en `process::CURRENT_PID` en cada cambio de tarea.
pub fn spawn_with_space(entry: fn() -> !, page_table: u64, pid: u64) -> u64 {
    unsafe {
        let sched = (*SCHEDULER.0.get())
            .as_mut()
            .expect("scheduler::spawn_with_space llamado antes de scheduler::init");
        let id = sched.tasks.len() as u64;
        let mut task = Task::new(id, entry);
        task.page_table = page_table;
        task.pid = pid;
        sched.tasks.push(task);
        id
    }
}

/// Añade una tarea nueva a la cola. No empieza a correr hasta que le
/// toque turno por `yield_now()`.
pub fn spawn(entry: fn() -> !) -> u64 {
    spawn_with_space(entry, 0, 0)
}

/// Marca la tarea actualmente en ejecución como `Finished` — nunca más
/// recibirá turno en `yield_now()`. Para `exit()`: la tarea sigue
/// "existiendo" (su `Context` no se toca, por si algo la referenciara),
/// pero el round-robin la salta de ahora en adelante.
pub fn mark_current_finished() {
    unsafe {
        let sched = match (*SCHEDULER.0.get()).as_mut() {
            Some(s) => s,
            None => return,
        };
        sched.tasks[sched.current].state = TaskState::Finished;
    }
}

/// Actualiza el `page_table` de la tarea cuyo `Task::pid` coincida —
/// para `execve()` (`syscall.rs::sys_execve`), que reemplaza el espacio
/// de direcciones del proceso actual mientras sigue corriendo (no pasa
/// por `spawn_with_space`, que es donde normalmente se fija este campo).
///
/// BUG real encontrado verificando el arranque automático de Ember:
/// `sys_execve` actualizaba `process::Pcb.page_table` (la copia que usan
/// `wait()`/`kill()`/`reap_zombie`) pero NUNCA esta copia paralela que
/// vive en el `Task` del scheduler — la que `yield_now()` de verdad lee
/// para decidir a qué CR3 cambiar al devolverle el turno a una tarea.
/// Mientras el proceso recién `execve`ado no cediera el turno de ninguna
/// forma antes de llegar a su propio `exit()`, daba igual (nunca se
/// necesitaba volver a mirar ese campo estando en medio). Pero con
/// preemption real de por medio, el timer puede interrumpirlo en
/// CUALQUIER punto de su código nuevo — y al devolverle el turno más
/// tarde, `yield_now()` restauraba el CR3 VIEJO (el de antes del
/// `execve`, todavía con una copia completa del código del PADRE por
/// `fork()`), no el nuevo. El proceso seguía corriendo, pero viendo su
/// propia memoria a través del mapeo equivocado — con el padre siendo
/// `ember` (que usa la misma dirección virtual base para su código que
/// cualquier ELF de prueba, ver `elf.rs`), el resultado era que el hijo
/// "recién ejecutado" parecía ejecutar el propio código de `ember`
/// (otro `fork()`, y a la segunda vuelta un page fault) en vez del
/// `ping()+exit()` real de su binario — reproducido en QEMU: ~1 de cada
/// 2-3 arranques, siempre con las mismas direcciones físicas (nada
/// aleatorio de verdad, solo depende de si el timer dispara o no en la
/// ventana de 2-3 instrucciones entre el `execve` y el `exit()`).
pub fn set_task_page_table(pid: u64, page_table: u64) -> bool {
    unsafe {
        let sched = match (*SCHEDULER.0.get()).as_mut() {
            Some(s) => s,
            None => return false,
        };
        for task in sched.tasks.iter_mut() {
            if task.pid == pid {
                task.page_table = page_table;
                return true;
            }
        }
        false
    }
}

/// Marca como `Finished` la tarea cuyo `Task::pid` coincida —a
/// diferencia de `mark_current_finished()`, no hace falta que sea la
/// tarea en ejecución ahora mismo. Para `signal::deliver_default`: matar
/// un proceso desde OTRO (`kill()`) o desde un manejador de excepción
/// (`SIGSEGV` en `idt.rs`) necesita poder sacar del round-robin a una
/// tarea que no es la que está corriendo en este instante. `false` si
/// ningún `Task` tenía ese PID (no debería pasar si `process::find`
/// encontró el PCB — pid=0 nunca se usa para un proceso real, así que
/// no hay ambigüedad con los hilos de kernel puros).
pub fn mark_finished_by_pid(pid: u64) -> bool {
    unsafe {
        let sched = match (*SCHEDULER.0.get()).as_mut() {
            Some(s) => s,
            None => return false,
        };
        for task in sched.tasks.iter_mut() {
            if task.pid == pid {
                task.state = TaskState::Finished;
                return true;
            }
        }
        false
    }
}

/// Cede el turno a la siguiente tarea `Ready`/`Running` de la cola
/// (round-robin, saltando las `Finished`). Si no hay ninguna otra tarea
/// elegible (o el scheduler no está inicializado), no hace nada.
///
/// Sección crítica protegida con `IrqGuard` — ver su comentario largo
/// sobre el bug real que esto arregla (reentrada del timer a medio
/// `switch_to`). `_guard` se suelta (restaura IF a lo que fuera antes)
/// en CUALQUIER punto de salida de la función, incluidos los `return`
/// tempranos de más abajo, por ser una variable local normal con `Drop`
/// — no hace falta duplicar la restauración a mano en cada uno.
pub fn yield_now() {
    let _guard = IrqGuard::disable();
    unsafe {
        let sched = match (*SCHEDULER.0.get()).as_mut() {
            Some(s) => s,
            None => return,
        };

        if sched.tasks.len() < 2 {
            return;
        }

        let old_idx = sched.current;
        let mut next_idx = old_idx;
        let mut found = false;
        for step in 1..=sched.tasks.len() {
            let candidate = (old_idx + step) % sched.tasks.len();
            if sched.tasks[candidate].state != TaskState::Finished {
                next_idx = candidate;
                found = true;
                break;
            }
        }
        if !found || next_idx == old_idx {
            return; // nadie más elegible — quedarse donde estamos
        }
        sched.current = next_idx;

        // Si la tarea que entra tiene su propio espacio de direcciones,
        // cambiamos CR3 ANTES del cambio de registros — en Rust normal,
        // no dentro del `switch_to` en ensamblador, para no meter más
        // riesgo en la parte ya delicada. El espacio de kernel (P4[0])
        // está compartido por diseño, así que el propio `switch_to` que
        // viene justo después sigue siendo código válido y accesible
        // aunque hayamos cambiado de espacio.
        let next_pt = sched.tasks[next_idx].page_table;
        if next_pt != 0 && next_pt != crate::mmu::current_address_space() {
            crate::mmu::switch_address_space(next_pt);
        }

        // Misma idea que el cambio de CR3, pero para la pila que usará
        // la PRÓXIMA syscall de esta tarea — ver el porqué en
        // `task.rs::Task::kernel_stack_top`. `0` = hilo de kernel puro,
        // nunca ejecuta `syscall`, no hay nada que activar.
        let next_kstack = sched.tasks[next_idx].kernel_stack_top;
        if next_kstack != 0 {
            crate::syscall::set_syscall_stack_top(next_kstack);
            // Mismo valor, reutilizado también como TSS.RSP0 — ver la
            // nota larga en `gdt::set_rsp0` sobre por qué es seguro
            // compartirlo con la pila de syscalls de la tarea. Sin
            // esto, `preempt.rs` no podría hacer preemption real en
            // ring 3 (todas las tareas de usuario compartirían una
            // única pila física para las transiciones por interrupción).
            crate::gdt::set_rsp0(next_kstack);
        }

        // Igual que CR3 y la pila de syscalls: `current_pid()` debe
        // reflejar quién corre AHORA, no quién arrancó primero.
        crate::process::set_current_pid(sched.tasks[next_idx].pid);

        let old_ctx: *mut Context = &mut sched.tasks[old_idx].context;
        let new_ctx: *const Context = &sched.tasks[next_idx].context;

        crate::task::switch_to(old_ctx, new_ctx);
    }
}

pub fn task_count() -> usize {
    unsafe { (*SCHEDULER.0.get()).as_ref().map(|s| s.tasks.len()).unwrap_or(0) }
}
