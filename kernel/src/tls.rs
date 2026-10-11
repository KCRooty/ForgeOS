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
