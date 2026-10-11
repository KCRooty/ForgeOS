//! TLS — Thread-Local Storage: lectura/escritura del MSR IA32_FS_BASE.
//!
//! El FS.base (MSR 0xC0000100) es el registro que mlibc usa para apuntar
//! al TCB (Thread Control Block) de cada hilo — el primer campo que lee
//! al entrar en cualquier función de libc. Sin save/restore en cada
//! context switch, el TCB de un proceso apunta al espacio de otro:
//! cualquier llamada a libc corrompe silenciosamente datos de otro proceso.
//!
//! El MSR se lee/escribe con `rdmsr`/`wrmsr` (siempre disponibles en ring 0,
//! sin necesitar FSGSBASE en CPUID); para ring 3, `ARCH_SET_FS` vía
//! `SYS_ARCH_PRCTL` es el único camino — los procesos no pueden tocar
//! directamente los MSRs.

const IA32_FS_BASE: u32 = 0xC000_0100;

/// Área de 512 bytes para `fxsave`/`fxrstor` — alineación a 16 bytes
/// requerida por la instrucción. Guarda el estado completo de la FPU x87,
/// MMX y XMM0-XMM15 + MXCSR de una tarea.
#[repr(C, align(16))]
#[derive(Clone, Copy)]
pub struct FxState(pub [u8; 512]);

impl FxState {
    pub const fn new() -> Self {
        FxState([0; 512])
    }
}

/// Guarda el estado FPU/SSE actual en `dst` (debe ser 16-byte aligned).
pub unsafe fn fxsave(dst: *mut FxState) {
    core::arch::asm!("fxsave64 [{}]", in(reg) dst, options(nostack, preserves_flags));
}

/// Restaura el estado FPU/SSE desde `src`.
pub unsafe fn fxrstor(src: *const FxState) {
    core::arch::asm!("fxrstor64 [{}]", in(reg) src, options(nostack, preserves_flags));
}

/// Reinicia la FPU al estado por defecto (`finit`) — para tareas nuevas
/// que nunca han tenido un estado FPU guardado.
pub unsafe fn fpu_init() {
    core::arch::asm!("finit", options(nostack, preserves_flags));
}

/// Lee el FS.base actual del procesador.
pub fn read_fs_base() -> u64 {
    let (hi, lo): (u32, u32);
    unsafe {
        core::arch::asm!(
            "rdmsr",
            in("ecx") IA32_FS_BASE,
            out("eax") lo,
            out("edx") hi,
            options(nomem, nostack),
        );
    }
    ((hi as u64) << 32) | (lo as u64)
}

/// Escribe `val` en FS.base. Llámalo solo desde ring 0 (context switch o
/// handler de SYS_ARCH_PRCTL).
pub fn write_fs_base(val: u64) {
    let lo = val as u32;
    let hi = (val >> 32) as u32;
    unsafe {
        core::arch::asm!(
            "wrmsr",
            in("ecx") IA32_FS_BASE,
            in("eax") lo,
            in("edx") hi,
            options(nomem, nostack),
        );
    }
}
