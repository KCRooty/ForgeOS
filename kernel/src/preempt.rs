//! Preemption real vía timer APIC — milestone `partinfo-ext4-preempt`
//! (la otra mitad; ver `partinfo.rs` para el escáner de particiones).
//!
//! `apic.rs` ya arma el timer periódico y cuenta ticks desde M4b, pero
//! el handler original (`extern "x86-interrupt"`) solo hacía EOI —
//! nunca tocaba el scheduler. Esto sustituye ese handler por un
//! trampolín en ensamblador desnudo (mismo patrón que `syscall_entry`
//! en `syscall.rs`: guardar TODO antes de tocar Rust, no confiar en que
//! el ABI normal de Rust proteja nada, porque una interrupción puede
//! caer en CUALQUIER punto del código interrumpido, no solo en el
//! límite de una `call` como el cambio de contexto cooperativo de
//! `task.rs`).
//!
//! ## Por qué reutilizar `task::switch_to` en vez de escribir otro
//! mecanismo de cambio de contexto
//!
//! Una vez que el trampolín ha guardado a mano los 15 registros de
//! propósito general en la propia pila de la tarea interrumpida, llamar
//! a una función Rust normal (`call`, convención SysV) vuelve a ser
//! seguro: todo lo que el código interrumpido necesitaba vivo ya está
//! protegido (o en la pila, por nuestro guardado manual, o en un
//! registro callee-saved, que el propio compilador preserva por
//! convención en cualquier función que llame). Esa función Rust puede
//! entonces llamar a `scheduler::yield_now()` con toda normalidad —
//! que a su vez llama a `task::switch_to`, que hace SU cambio de
//! contexto cooperativo de siempre (callee-saved + rsp). El `ret` de
//! `switch_to` no vuelve aquí necesariamente: vuelve a donde sea que la
//! tarea ENTRANTE cediera el turno la última vez — si esa tarea también
//! fue interrumpida por este mismo timer alguna vez, ese punto es el
//! epílogo de ESTE MISMO trampolín (guardado en su propia pila), y el
//! `ret` desenreda la cadena hasta el `iretq` final de forma simétrica.
//! Si esa tarea cedió el turno cooperativamente (`yield_now()` normal,
//! nunca desde una interrupción), el `ret` cae en código kernel
//! corriente — tan válido como siempre. Ambos caminos de suspensión
//! (cooperativo y preemptivo) son compatibles porque los dos pasan, en
//! algún punto de su cadena de llamadas, por el mismo `switch_to`.
//!
//! ## `sti` explícito antes de `yield_now()`
//!
//! El gate de interrupción de `idt.rs` (`type_attr = 0x8E`) desenmascara
//! IF automáticamente al entrar — necesario para no anidar
//! interrupciones sobre nuestra propia pila a medio construir. Pero si
//! NO lo reactivamos antes de llamar a `yield_now()`, la tarea que
//! entre (si su último punto de suspensión fue un `yield_now()`
//! cooperativo normal, no una interrupción) se reanuda con IF todavía
//! en 0 — interrupciones enmascaradas para siempre hasta la próxima
//! vez que ELLA pase por una interrupción, lo cual mata la propia
//! preemption que se supone que garantiza (y el teclado, y todo lo
//! demás que dependa de IRQs). Mismo bug de fondo, mismo arreglo, que
//! el `sti` ya documentado en `sys_exit()` (`syscall.rs`) — ver TODO.md.
//! La tarea que SÍ fue interrumpida por una IRQ recupera su IF real
//! (estaba en 1, porque corría normal) automáticamente vía `iretq`
//! cuando le vuelva a tocar turno — no hace falta tocar nada para ese
//! caso, `iretq` restaura RFLAGS completo desde el frame que empujó el
//! hardware al entrar.
//!
//! ## Limitación documentada a propósito: sin preemption real en ring 3
//!
//! El trampolín distingue si interrumpió a una tarea de kernel (CPL0,
//! sin cambio de privilegio — el caso de arriba) o a un proceso de
//! usuario (CPL3, con cambio de privilegio). Para CPL3 solo hace EOI y
//! vuelve (`iretq`) SIN tocar el scheduler — el proceso de ring 3 sigue
//! corriendo, el timer es transparente para él. Motivo concreto, no
//! pereza: `gdt.rs` usa un único `TSS.RSP0` global (una sola pila de
//! 16 KiB para TODAS las transiciones ring3→ring0 vía interrupción,
//! nunca actualizada por tarea, a diferencia de la pila de syscalls que
//! sí es per-tarea desde el milestone `wait()`). Si dos procesos de
//! ring 3 distintos quedaran "aparcados" a mitad de esta misma cadena
//! de llamadas (`timer_entry` → `yield_now` → `switch_to`, parados en
//! el `ret` pendiente) compartiendo esa única pila física, el segundo
//! pisaría el estado guardado del primero — el mismo bug de fondo que
//! la pila de syscalls compartida antes de arreglarse. Extender esto a
//! CPL3 de verdad necesita primero un `RSP0` por tarea (mismo patrón
//! que `Task::kernel_stack_top`), pieza aparte. Hasta entonces, Ember
//! sigue sin poder quedarse viva para siempre de forma segura (ver su
//! propia nota en `elf.rs`) — este milestone resuelve la mitad de
//! kernel, no la de ring 3.

use core::arch::naked_asm;
use core::sync::atomic::{AtomicBool, AtomicU64, Ordering};

/// El timer de M4b (`apic.rs`) lleva armado y disparando desde el
/// primer arranque — antes de este fichero, su handler solo contaba
/// ticks y mandaba EOI, nunca tocaba el scheduler. Para no cambiar el
/// comportamiento de TODO lo que arrancó después de M4b (VFS, red,
/// syscalls, `console::run()`...) sin que ningún milestone posterior
/// lo tuviera en cuenta, la preemption de verdad se queda detrás de
/// esta puerta, apagada por defecto — `timer_tick_ring0` sigue
/// contando ticks y mandando EOI siempre, pero solo llama a
/// `yield_now()` si esto está a `true`. `preempttest` (comando de
/// consola) es quien la enciende, de forma acotada, para su propia
/// demostración.
static ENABLED: AtomicBool = AtomicBool::new(false);

pub fn set_enabled(enabled: bool) {
    ENABLED.store(enabled, Ordering::Relaxed);
}

/// Trampolín de entrada para `apic::TIMER_VECTOR` — sustituye al
/// `extern "x86-interrupt" fn timer_interrupt_handler` original.
///
/// CS está SIEMPRE en el mismo offset relativo a la pila justo al
/// entrar, tanto si el hardware empujó 3 qwords (RIP/CS/RFLAGS, sin
/// cambio de privilegio) como 5 (+ RSP/SS, con cambio de privilegio) —
/// en ambos casos RIP va primero, CS justo después. Comprobamos su bit
/// RPL para decidir qué camino tomar, sin necesidad de manejar los dos
/// tamaños de frame de forma distinta en el resto del código: nunca
/// tocamos esa parte del frame directamente, solo apilamos encima y
/// desapilamos exactamente lo mismo antes de `iretq` — es el propio
/// `iretq` quien decide cuántos qwords consumir mirando el CS real que
/// encuentre, no nosotros.
#[unsafe(naked)]
pub unsafe extern "C" fn timer_entry() {
    naked_asm!(
        "push rax",
        "mov ax, [rsp + 16]", // CS empujado por el hardware (ver nota de arriba)
        "test al, 3",
        "jnz 2f",
        // --- interrumpió una tarea de kernel (CPL0) — preemption real ---
        "1:",
        "push rcx", "push rdx", "push rbx",
        "push rsi", "push rdi", "push rbp",
        "push r8",  "push r9",  "push r10", "push r11",
        "push r12", "push r13", "push r14", "push r15",
        "call {tick}",
        "pop r15", "pop r14", "pop r13", "pop r12",
        "pop r11", "pop r10", "pop r9",  "pop r8",
        "pop rbp", "pop rdi", "pop rsi",
        "pop rbx", "pop rdx", "pop rcx",
        "pop rax",
        "iretq",
        // --- interrumpió un proceso de ring 3 (CPL3) — solo EOI, ver
        // limitación documentada en la cabecera del fichero ---
        "2:",
        "push rcx", "push rdx", "push rbx",
        "push rsi", "push rdi", "push rbp",
        "push r8",  "push r9",  "push r10", "push r11",
        "push r12", "push r13", "push r14", "push r15",
        "call {eoi}",
        "pop r15", "pop r14", "pop r13", "pop r12",
        "pop r11", "pop r10", "pop r9",  "pop r8",
        "pop rbp", "pop rdi", "pop rsi",
        "pop rbx", "pop rdx", "pop rcx",
        "pop rax",
        "iretq",
        tick = sym timer_tick_ring0,
        eoi = sym timer_tick_ring3_eoi_only,
    );
}

/// Camino CPL0: cuenta el tick y manda EOI siempre; cede el turno de
/// verdad solo si `set_enabled(true)` (ver nota de cabecera sobre
/// `ENABLED`) — el scheduler decide entonces si hay otra tarea `Ready`
/// a la que saltar.
extern "C" fn timer_tick_ring0() {
    crate::apic::note_tick();
    unsafe { crate::apic::send_eoi() };
    if !ENABLED.load(Ordering::Relaxed) {
        return;
    }
    // El `sti` reactiva IF antes de ceder el turno — ver la nota larga
    // de cabecera sobre por qué hace falta (mismo bug de fondo que el
    // de `sys_exit()`). Solo hace falta cuando de verdad vamos a
    // cambiar de tarea: si `ENABLED` está a `false` no llegamos aquí.
    unsafe { core::arch::asm!("sti") };
    crate::scheduler::yield_now();
}

/// Camino CPL3: cuenta el tick y manda EOI — sin tocar el scheduler
/// (ver "Limitación documentada a propósito" en la cabecera).
extern "C" fn timer_tick_ring3_eoi_only() {
    crate::apic::note_tick();
    unsafe { crate::apic::send_eoi() };
}

// --- verificación: dos tareas "avariciosas" que nunca ceden el turno
// por su cuenta (a diferencia de task_a/task_b de M4a, que SÍ llaman a
// yield_now() ellas mismas) — si avanzan igualmente, es la prueba de
// que el timer las está interrumpiendo de verdad, no que se están
// portando bien. Comando `preempttest` de la consola.

static SPIN_A_COUNT: AtomicU64 = AtomicU64::new(0);
static SPIN_B_COUNT: AtomicU64 = AtomicU64::new(0);

fn spin_a() -> ! {
    loop {
        SPIN_A_COUNT.fetch_add(1, Ordering::Relaxed);
    }
}

fn spin_b() -> ! {
    loop {
        SPIN_B_COUNT.fetch_add(1, Ordering::Relaxed);
    }
}

/// Registra las dos tareas avariciosas en el scheduler — no empiezan a
/// correr hasta que les toque turno, igual que cualquier otra tarea.
pub fn spawn_spin_tasks() {
    crate::scheduler::spawn(spin_a);
    crate::scheduler::spawn(spin_b);
}

pub fn spin_counts() -> (u64, u64) {
    (SPIN_A_COUNT.load(Ordering::Relaxed), SPIN_B_COUNT.load(Ordering::Relaxed))
}

