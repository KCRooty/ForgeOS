//! Tareas cooperativas del kernel — M4a (BORRADOR SIN VERIFICAR — de
//! todo lo escrito hasta ahora, esto es probablemente lo más frágil).
//!
//! Primer paso de M4: las tareas todavía NO tienen su propio espacio de
//! direcciones (eso es M4c, necesita el gestor de memoria virtual que
//! todavía no existe — ver TODO.md sección 1). Esto son "hilos de
//! kernel" cooperativos, todos compartiendo el mapeo flat que ya
//! tenemos. Es el escalón antes de fork/exec real, no fork/exec real.
//!
//! ADVERTENCIA sobre `#[naked]`: la sintaxis de funciones naked en Rust
//! ha cambiado entre nightlies (algunas versiones recientes exigen
//! `naked_asm!` en vez de `asm!` dentro de `#[naked]`, o el atributo
//! `#[unsafe(naked)]`). Si esto no compila tal cual, es el PRIMER sitio
//! a mirar — más incluso que la ABI de idt.rs.

use alloc::boxed::Box;
use core::arch::naked_asm;

pub const STACK_SIZE: usize = 16 * 1024; // 16 KiB por tarea

#[repr(C)]
#[derive(Clone, Copy)]
pub struct Context {
    // registros callee-saved (System V AMD64) + rsp. Es lo mínimo que
    // hace falta preservar en un cambio de contexto cooperativo: como
    // `switch_to` es (para el compilador) una llamada de función normal,
    // los registros caller-saved ya los protege el propio código
    // generado en cada lado antes/después de la llamada.
    rbx: u64,
    rbp: u64,
    r12: u64,
    r13: u64,
    r14: u64,
    r15: u64,
    rsp: u64,
}

impl Context {
    pub const fn zero() -> Self {
        Context {
            rbx: 0,
            rbp: 0,
            r12: 0,
            r13: 0,
            r14: 0,
            r15: 0,
            rsp: 0,
        }
    }
}

#[derive(PartialEq, Eq, Clone, Copy)]
pub enum TaskState {
    Ready,
    Running,
    Finished,
}

pub struct Task {
    pub id: u64,
    pub context: Context,
    pub state: TaskState,
    /// 0 = comparte el espacio de direcciones activo (hilo de kernel
    /// normal, M4a). Distinto de 0 = PML4 propio (proceso real, M4d) —
    /// el scheduler cambia CR3 a esto al entrarle el turno.
    pub page_table: u64,
    /// PID del proceso (`process.rs`) al que pertenece esta tarea — 0
    /// para un hilo de kernel puro sin identidad de proceso. El
    /// scheduler lo restaura en `process::CURRENT_PID` en cada cambio
    /// de tarea (igual que CR3): sin esto, `current_pid()` se quedaba
    /// pegado al último proceso que arrancó (solo se fijaba una vez,
    /// en su trampolín) en vez de reflejar quién corre de verdad en
    /// cada momento — un hijo terminando mientras el padre seguía
    /// pausado dentro de `wait()` dejaba `current_pid()` apuntando al
    /// hijo (ya reapeado) cuando el padre se reanudaba.
    pub pid: u64,
    /// Pila de kernel PROPIA para atender syscalls de esta tarea — 0
    /// para los hilos de kernel puros (boot/task_a/task_b) que nunca
    /// ejecutan `syscall` desde ring 3. El scheduler la activa (via
    /// `syscall::set_syscall_stack_top`) al entrarle el turno a la
    /// tarea, igual que ya hace con `page_table`/CR3. Antes de esto,
    /// TODAS las tareas compartían una única `KERNEL_SYSCALL_STACK`
    /// global — inofensivo mientras una syscall fuera un viaje de ida
    /// (`exit()`, nunca se reanuda), pero corrompía cualquier syscall
    /// que SÍ esperara reanudarse más tarde (`wait()`): si otra tarea
    /// hacía su propia syscall mientras la primera seguía "pausada"
    /// dentro de la suya, la pila compartida quedaba pisada.
    pub kernel_stack_top: u64,
    // guardamos los Box para que el kernel heap no libere las pilas
    // mientras la tarea exista. `None` para la tarea "placeholder" que
    // representa un flujo de ejecución que YA estaba corriendo (el
    // propio boot) — a esa nunca se salta por `entry`, solo se usa como
    // hueco donde guardar sus registros la primera vez que cede el turno.
    _stack: Option<Box<[u8]>>,
    _kernel_stack: Option<Box<[u8]>>,
}

impl Task {
    /// Tarea nueva de verdad: se le monta una pila propia con la
    /// dirección de `entry` pre-colocada, de forma que el primer
    /// `switch_to` que apunte aquí termine saltando a `entry` vía `ret`.
    pub fn new(id: u64, entry: fn() -> !) -> Task {
        let stack = alloc::vec![0u8; STACK_SIZE].into_boxed_slice();
        let stack_bottom = stack.as_ptr() as u64;
        let stack_end = stack_bottom + STACK_SIZE as u64;

        // `mov rsp, [stack_top]` en `syscall_entry` usa esto TAL CUAL
        // como RSP inicial de la syscall — a diferencia de la pila de
        // usuario de arriba, esto no pasa por ningún `ret` que ya deje
        // la alineación correcta por su cuenta, así que hace falta
        // forzarla a mano: `Vec<u8>` pide align=1 al allocator (bump,
        // `heap.rs`), así que el puntero devuelto puede caer en
        // cualquier byte — sin este `& !0xF`, un RSP no alineado a 16
        // rompe la convención SysV justo antes del `call {handler}`
        // (que espera RSP%16==8 en la entrada de la función llamada),
        // corrompiendo quien sabe qué con las instrucciones SSE
        // alineadas que el código generado pueda usar de por medio.
        let kernel_stack = alloc::vec![0u8; STACK_SIZE].into_boxed_slice();
        let kernel_stack_top = (kernel_stack.as_ptr() as u64 + STACK_SIZE as u64) & !0xF;

        // Alineación: tras el `ret` de switch_to, que consume 8 bytes de
        // la pila (la dirección de `entry` que dejamos aquí), RSP debe
        // quedar en la convención SysV de "justo dentro de una función
        // llamada por `call`" — RSP % 16 == 8. Eso exige que la
        // dirección donde ESCRIBIMOS `entry` (y por tanto el `context.rsp`
        // que restauramos) sea ella misma ≡ 0 (mod 16) ANTES del `ret`.
        // `& !0xF` alinea hacia abajo a 16; restamos 16 antes de alinear
        // para dejar margen dentro del buffer y no escribir un byte más
        // allá del final.
        let write_addr = (stack_end - 16) & !0xF;

        unsafe {
            (write_addr as *mut u64).write(entry as usize as u64);
        }

        let mut context = Context::zero();
        context.rsp = write_addr;

        Task {
            id,
            context,
            state: TaskState::Ready,
            page_table: 0,
            pid: 0,
            kernel_stack_top,
            _stack: Some(stack),
            _kernel_stack: Some(kernel_stack),
        }
    }

    /// Representa un flujo ya en marcha (el boot). No se salta a nada —
    /// `switch_to` rellenará este `context` con los registros reales la
    /// primera vez que este "hilo" ceda el turno. `kernel_stack_top=0`
    /// — el boot nunca ejecuta `syscall` desde ring 3 (es kernel puro),
    /// así que no necesita una pila de syscalls propia.
    pub fn placeholder(id: u64) -> Task {
        Task {
            id,
            context: Context::zero(),
            state: TaskState::Running,
            page_table: 0,
            pid: 0,
            kernel_stack_top: 0,
            _stack: None,
            _kernel_stack: None,
        }
    }
}

/// Guarda el contexto actual en `old`, carga el contexto de `new`, y
/// salta a él. La primera vez que se salta a una tarea nueva, ese salto
/// es un `ret` hacia la `entry` que `Task::new` dejó preparada en su
/// pila; las veces siguientes es un `ret` real hacia quien llamó a
/// `switch_to` la última vez que esa tarea cedió el turno — técnica
/// estándar de corutinas/hilos cooperativos (misma idea que usan
/// ucontext o boost::context).
#[unsafe(naked)]
pub unsafe extern "C" fn switch_to(old: *mut Context, new: *const Context) {
    naked_asm!(
        "mov [rdi + 0],  rbx",
        "mov [rdi + 8],  rbp",
        "mov [rdi + 16], r12",
        "mov [rdi + 24], r13",
        "mov [rdi + 32], r14",
        "mov [rdi + 40], r15",
        "mov [rdi + 48], rsp",
        "mov rbx, [rsi + 0]",
        "mov rbp, [rsi + 8]",
        "mov r12, [rsi + 16]",
        "mov r13, [rsi + 24]",
        "mov r14, [rsi + 32]",
        "mov r15, [rsi + 40]",
        "mov rsp, [rsi + 48]",
        "ret",
    );
}
