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
use core::arch::asm;

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

pub enum TaskState {
    Ready,
    Running,
    Finished,
}

pub struct Task {
    pub id: u64,
    pub context: Context,
    pub state: TaskState,
    // guardamos el Box para que el kernel heap no libere la pila
    // mientras la tarea exista. `None` para la tarea "placeholder" que
    // representa un flujo de ejecución que YA estaba corriendo (el
    // propio boot) — a esa nunca se salta por `entry`, solo se usa como
    // hueco donde guardar sus registros la primera vez que cede el turno.
    _stack: Option<Box<[u8]>>,
}

impl Task {
    /// Tarea nueva de verdad: se le monta una pila propia con la
    /// dirección de `entry` pre-colocada, de forma que el primer
    /// `switch_to` que apunte aquí termine saltando a `entry` vía `ret`.
    pub fn new(id: u64, entry: fn() -> !) -> Task {
        let stack = alloc::vec![0u8; STACK_SIZE].into_boxed_slice();
        let stack_bottom = stack.as_ptr() as u64;
        let stack_end = stack_bottom + STACK_SIZE as u64;

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
            _stack: Some(stack),
        }
    }

    /// Representa un flujo ya en marcha (el boot). No se salta a nada —
    /// `switch_to` rellenará este `context` con los registros reales la
    /// primera vez que este "hilo" ceda el turno.
    pub fn placeholder(id: u64) -> Task {
        Task {
            id,
            context: Context::zero(),
            state: TaskState::Running,
            _stack: None,
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
#[naked]
pub unsafe extern "C" fn switch_to(old: *mut Context, new: *const Context) {
    asm!(
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
        options(noreturn)
    );
}
