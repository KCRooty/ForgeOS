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
        // Ningún registro del kernel cruza a ring 3: lo que hubiera en
        // ellos (punteros de kernel, restos de otra tarea) sería una fuga
        // de información hacia el proceso. Los operandos de arriba ya
        // están en la pila, así que se pueden pisar todos.
        "xor eax, eax", "xor ebx, ebx", "xor ecx, ecx", "xor edx, edx",
        "xor esi, esi", "xor edi, edi", "xor ebp, ebp",
        "xor r8d, r8d", "xor r9d, r9d", "xor r10d, r10d", "xor r11d, r11d",
        "xor r12d, r12d", "xor r13d, r13d", "xor r14d, r14d", "xor r15d, r15d",
        "iretq",
        ss = in(reg) gdt::USER_DATA_SELECTOR as u64,
        stack = in(reg) user_stack_top,
        flags = in(reg) 0x202u64, // IF=1, bit reservado 1 siempre a 1
        cs = in(reg) gdt::USER_CODE_SELECTOR as u64,
        entry = in(reg) entry,
        options(noreturn)
    );
}

/// Entra en ring 3 con TODOS los registros de propósito general tomados
/// de `context` — el arranque de un programa (todo a cero) o el hijo de
/// un `fork()`, que debe continuar exactamente donde estaba el padre:
/// un compilador de C guarda variables vivas en `rbx`/`rbp`/`r12`-`r15`
/// a través de la llamada a `fork()`, y cuenta con encontrarlas a la
/// vuelta también en el hijo.
///
/// Los desplazamientos son los de `process::UserContext` (`repr(C)`).
pub unsafe fn enter_ring3_context(context: &crate::process::UserContext) -> ! {
    core::arch::asm!(
        "push {ss}",
        "push qword ptr [rdi + 128]", // rsp
        "push qword ptr [rdi + 136]", // rflags
        "push {cs}",
        "push qword ptr [rdi + 120]", // rip
        "mov rax, [rdi + 0]",
        "mov rbx, [rdi + 8]",
        "mov rcx, [rdi + 16]",
        "mov rdx, [rdi + 24]",
        "mov rsi, [rdi + 32]",
        "mov rbp, [rdi + 48]",
        "mov r8,  [rdi + 56]",
        "mov r9,  [rdi + 64]",
        "mov r10, [rdi + 72]",
        "mov r11, [rdi + 80]",
        "mov r12, [rdi + 88]",
        "mov r13, [rdi + 96]",
        "mov r14, [rdi + 104]",
        "mov r15, [rdi + 112]",
        "mov rdi, [rdi + 40]", // el último: era el puntero al contexto
        "iretq",
        ss = in(reg) gdt::USER_DATA_SELECTOR as u64,
        cs = in(reg) gdt::USER_CODE_SELECTOR as u64,
        in("rdi") context as *const crate::process::UserContext,
        options(noreturn)
    );
}
