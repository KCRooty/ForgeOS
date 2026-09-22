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
use crate::gdt;
use crate::serial_println;

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
static mut KERNEL_SYSCALL_STACK: [u8; KERNEL_SYSCALL_STACK_SIZE] = [0; KERNEL_SYSCALL_STACK_SIZE];
static mut SYSCALL_STACK_TOP: u64 = 0;
static mut USER_RSP_SCRATCH: u64 = 0;

pub const SYS_PING: u64 = 1;

/// Argumentos de una syscall, en el orden `syscall/sysret` de Linux
/// x86_64 (RAX=número, RDI/RSI/RDX/R10/R8/R9=args) — ya documentado en
/// `ARCHITECTURE.md` §Convención de syscalls, esto es la primera vez
/// que se implementa de verdad.
#[repr(C)]
pub struct SyscallFrame {
    pub num: u64,
    pub arg0: u64,
    pub arg1: u64,
    pub arg2: u64,
    pub arg3: u64,
    pub arg4: u64,
    pub arg5: u64,
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
        SYSCALL_STACK_TOP = KERNEL_SYSCALL_STACK.as_ptr() as u64 + KERNEL_SYSCALL_STACK_SIZE as u64;

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
        other => {
            serial_println!("[syscall] número desconocido: {}", other);
            u64::MAX
        }
    }
}
