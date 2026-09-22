//! Scheduler cooperativo mínimo — M4a (BORRADOR SIN VERIFICAR).
//!
//! Round-robin, sin preferencia, sin preemption todavía — eso necesita
//! timer + IRQ real (APIC timer o PIT), que no existen aún (ver
//! TODO.md sección 1). Cada tarea corre hasta que llama a
//! `yield_now()` por su cuenta.

use crate::task::{Context, Task};
use alloc::vec::Vec;
use core::cell::UnsafeCell;

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
/// PML4 (creado con `mmu::create_address_space`).
pub fn spawn_with_space(entry: fn() -> !, page_table: u64) -> u64 {
    unsafe {
        let sched = (*SCHEDULER.0.get())
            .as_mut()
            .expect("scheduler::spawn_with_space llamado antes de scheduler::init");
        let id = sched.tasks.len() as u64;
        let mut task = Task::new(id, entry);
        task.page_table = page_table;
        sched.tasks.push(task);
        id
    }
}

/// Añade una tarea nueva a la cola. No empieza a correr hasta que le
/// toque turno por `yield_now()`.
pub fn spawn(entry: fn() -> !) -> u64 {
    spawn_with_space(entry, 0)
}

/// Cede el turno a la siguiente tarea de la cola (round-robin). Si solo
/// hay una tarea (o el scheduler no está inicializado), no hace nada.
pub fn yield_now() {
    unsafe {
        let sched = match (*SCHEDULER.0.get()).as_mut() {
            Some(s) => s,
            None => return,
        };

        if sched.tasks.len() < 2 {
            return;
        }

        let old_idx = sched.current;
        let next_idx = (old_idx + 1) % sched.tasks.len();
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

        let old_ctx: *mut Context = &mut sched.tasks[old_idx].context;
        let new_ctx: *const Context = &sched.tasks[next_idx].context;

        crate::task::switch_to(old_ctx, new_ctx);
    }
}

pub fn task_count() -> usize {
    unsafe { (*SCHEDULER.0.get()).as_ref().map(|s| s.tasks.len()).unwrap_or(0) }
}
