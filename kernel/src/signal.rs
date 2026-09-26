//! Señales — SIGKILL/SIGTERM/SIGSEGV (último punto pendiente del TODO
//! original de esta reconstrucción).
//!
//! Alcance de esta pasada, a propósito acotado: **solo la acción por
//! defecto** de cada señal, que para las tres es la misma —terminar el
//! proceso—. Nadie puede instalar un manejador propio todavía (eso
//! necesita una máscara de señales pendientes por proceso + un
//! trampolín de retorno a userland, con su propia `sigreturn` — pieza
//! bastante más grande, aparte). Lo que SÍ resuelve de verdad esta
//! pasada: **un proceso de ring 3 que revienta con un puntero roto ya
//! no cuelga el kernel entero.**
//!
//! Antes de esto, `idt.rs::page_fault` hacía `halt()` incondicional
//! ante CUALQUIER fallo de página — un bug real de kernel (bueno,
//! haría falta pararse a mirarlo) exactamente igual que un `forktest`
//! con un puntero malo (nada especial, es justo lo que se espera que
//! pase alguna vez). Ahora se distingue por el CS que dejó el hardware
//! en la pila: si el fallo vino de ring 3, es un SIGSEGV normal del
//! proceso — se le mata a él, no al sistema entero; si vino de ring 0,
//! sigue siendo `halt()` (un fallo de página en código de KERNEL es un
//! bug de verdad, no algo que "resolver" matando un proceso).
//!
//! Números de señal iguales a Linux x86_64 — no por compatibilidad
//! binaria (no la tenemos), sino porque no hay motivo para inventar
//! otros números para lo mismo.

use crate::{mmu, process, scheduler, serial_println};

pub const SIGKILL: u32 = 9;
pub const SIGSEGV: u32 = 11;
pub const SIGTERM: u32 = 15;

fn signal_name(sig: u32) -> &'static str {
    match sig {
        SIGKILL => "SIGKILL",
        SIGSEGV => "SIGSEGV",
        SIGTERM => "SIGTERM",
        _ => "señal desconocida",
    }
}

/// Aplica la acción por defecto de `sig` sobre `pid`: termina el
/// proceso y lo saca del round-robin del scheduler para siempre.
///
/// Código de salida: convención de shell (128 + número de señal), así
/// se distingue de un `exit(N)` normal (0-127) con solo mirar `ps`.
///
/// **Liberación de memoria condicional, a propósito:** `pid` puede ser
/// el propio proceso que está corriendo ahora mismo (`SIGSEGV` desde
/// `idt.rs::page_fault`, o `kill(getpid())`) — en ese caso su PML4 es
/// el CR3 ACTIVO en este instante, y `mmu::free_address_space` prohíbe
/// explícitamente liberar el espacio activo ("liberar las tablas bajo
/// tus propios pies sería fatal", ver su doc). Encontrado de la forma
/// difícil: la primera versión de esta función llamaba a
/// `free_address_space` incondicionalmente, y `segvtest` colgaba la
/// red (`ping`) justo después — el PML4 recién liberado (con CR3
/// TODAVÍA apuntando a él) se reciclaba para otra cosa antes de que el
/// scheduler llegara a cambiar de espacio de direcciones. Si `pid` NO
/// es el proceso activo (p. ej. un padre matando a un hijo que nunca
/// llegó a correr, `killtest`), liberar de inmediato es seguro y se
/// hace aquí mismo; si SÍ lo es, la memoria queda pendiente — igual que
/// ya le pasa a cualquier `exit()` normal sin padre que haga `wait()`.
///
/// `false` si `pid` no existe o ya era zombie — no hay nada que matar.
pub fn deliver_default(pid: u64, sig: u32) -> bool {
    let Some(pcb) = process::find(pid) else {
        return false;
    };
    if matches!(pcb.state, process::ProcessState::Zombie(_)) {
        return false;
    }

    if pid != process::current_pid() {
        unsafe { mmu::free_address_space(pcb.page_table) };
    }
    process::mark_zombie(pid, 128 + sig as i32);
    scheduler::mark_finished_by_pid(pid);
    true
}

/// Atajo para el caso de `idt.rs::page_fault`: el proceso que hay que
/// matar es, por definición, el que está corriendo ahora mismo (el que
/// acaba de reventar).
pub fn kill_current(sig: u32) {
    let pid = process::current_pid();
    serial_println!(
        "[signal] PID {} recibe {} — terminando (sin manejador propio instalado)",
        pid,
        signal_name(sig)
    );
    deliver_default(pid, sig);
}
