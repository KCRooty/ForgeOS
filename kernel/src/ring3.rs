//! Transición a ring 3 — M4f (BORRADOR SIN VERIFICAR).
//!
//! Salta de ring 0 a ring 3 de verdad, vía `iretq` con un frame de
//! interrupción construido a mano en la pila. Es un viaje solo de ida:
//! sin preemption real conectada todavía (M4b sigue pendiente en esa
//! parte — el timer suena pero no cede el turno a otra tarea), no hay
//! forma de que el kernel recupere el control después de saltar, salvo
//! que el propio código en ring 3 provoque una excepción o interrupción
//! que sí volvamos a atender (el timer seguirá interrumpiendo y
//! devolviendo el control exactamente ahí, gracias a `RSP0`).
//!
//! Por eso esto NO se ejecuta automáticamente en el arranque — solo vía
//! el comando `ring3test` de la consola de depuración, a demanda, para
//! no dejar el sistema colgado sin que quede claro por qué.

use crate::gdt;

/// Salta a `entry` en ring 3, con `user_stack_top` como pila de
/// usuario. No vuelve nunca — `RSP0` ya está configurado en `gdt.rs`
/// para que interrupciones futuras (el timer, excepciones) tengan
/// dónde aterrizar sin usar una pila basura.
pub unsafe fn enter_ring3(entry: u64, user_stack_top: u64) -> ! {
    core::arch::asm!(
        "push {ss}",       // SS de usuario
        "push {stack}",    // RSP de usuario
        "push {flags}",    // RFLAGS — bit 9 (IF) puesto: interrupciones activas en ring 3
        "push {cs}",       // CS de usuario (con RPL=3 ya incluido en el selector)
        "push {entry}",    // RIP de usuario
        "iretq",
        ss = in(reg) gdt::USER_DATA_SELECTOR as u64,
        stack = in(reg) user_stack_top,
        flags = in(reg) 0x202u64, // IF=1, bit reservado 1 siempre a 1
        cs = in(reg) gdt::USER_CODE_SELECTOR as u64,
        entry = in(reg) entry,
        options(noreturn)
    );
}

/// Igual que `enter_ring3`, pero fija `RAX` justo antes del `iretq`.
/// Necesario para el hijo de `fork()`: no llega a ring 3 vía
/// `sysretq` normal (nunca ejecutó `syscall` él mismo — se fabrica
/// desde cero, ver `syscall.rs::sys_fork`), así que no hay ningún
/// `call`/`ret` que deje el valor de retorno en `rax` de forma
/// natural. Aquí lo ponemos a mano: `rax=0` es la convención de
/// `fork()` para identificar al hijo (el padre recibe el PID real por
/// el camino normal, vía el valor de retorno de `syscall_dispatch`).
///
/// LIMITACIÓN — esto NO es un fork() de propósito general: solo
/// `rip`/`rsp`/`rflags`/`rax` se fabrican a mano; el resto de
/// registros de propósito general (rbx, rcx, rdx, rsi, rdi, rbp,
/// r8-r15) quedan con lo que haya en la CPU en ese momento (código del
/// propio kernel), NO con una copia de lo que tenía el padre en el
/// instante de `fork()`. `elf::TEST_ELF_FORK` funciona porque su
/// primera instrucción tras `syscall` (`mov ebx, eax`) solo depende de
/// `rax`, que sí es correcto — un binario real que dependiera de otro
/// registro justo al volver de `fork()` se rompería. Copiar el resto
/// de registros del padre (guardados en el `SyscallFrame`, que ya casi
/// los tiene todos salvo los callee-saved) es la mejora obvia si esto
/// deja de ser suficiente.
pub unsafe fn enter_ring3_with_rax(entry: u64, user_stack_top: u64, rax_value: u64) -> ! {
    core::arch::asm!(
        "push {ss}",
        "push {stack}",
        "push {flags}",
        "push {cs}",
        "push {entry}",
        "mov rax, {rax_value}",
        "iretq",
        ss = in(reg) gdt::USER_DATA_SELECTOR as u64,
        stack = in(reg) user_stack_top,
        flags = in(reg) 0x202u64,
        cs = in(reg) gdt::USER_CODE_SELECTOR as u64,
        entry = in(reg) entry,
        rax_value = in(reg) rax_value,
        options(noreturn)
    );
}
