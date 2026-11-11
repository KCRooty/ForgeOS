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

/// Estado FPU/SSE "recién encendido" en formato `fxsave`: FCW=0x037F
/// (todas las excepciones x87 enmascaradas, precisión extendida),
/// MXCSR=0x1F80 (todas las excepciones SSE enmascaradas, redondeo al más
/// cercano), pila x87 vacía y XMM0-15 a cero — lo que la ABI SysV
/// garantiza a un proceso al arrancar.
static CLEAN_FXSTATE: FxState = {
    let mut bytes = [0u8; 512];
    bytes[0] = 0x7F; // FCW, byte bajo
    bytes[1] = 0x03; // FCW, byte alto
    bytes[24] = 0x80; // MXCSR, byte bajo
    bytes[25] = 0x1F; // MXCSR, byte alto
    FxState(bytes)
};

/// Habilita SSE para ring 3 — llamar una vez, pronto en el arranque.
///
/// Sin esto, la primera instrucción SSE de un proceso (y cualquier
/// binario C de x86_64 las usa: la ABI pasa `double` en XMM, y clang
/// vectoriza `memcpy`/`memset`) provoca #UD:
/// - CR0.EM=0 (no emular la FPU), CR0.MP=1, CR0.TS=0;
/// - CR4.OSFXSR=1 (el OS declara que salva el estado con fxsave/fxrstor,
///   cosa que `scheduler::yield_now` ya hace desde M8e),
///   CR4.OSXMMEXCPT=1 (excepciones SSE por #XM en vez de #UD).
///
/// El propio kernel sigue compilado en soft-float (`targets/x86_64-forge.json`)
/// y no toca ni un registro XMM: por eso basta con salvar/restaurar el
/// estado al cambiar de tarea, sin hacerlo en cada entrada al kernel.
pub fn enable_sse() {
    unsafe {
        let mut cr0: u64;
        core::arch::asm!("mov {}, cr0", out(reg) cr0, options(nomem, nostack));
        cr0 &= !((1 << 2) | (1 << 3)); // EM=0, TS=0
        cr0 |= 1 << 1; // MP=1
        core::arch::asm!("mov cr0, {}", in(reg) cr0, options(nomem, nostack));

        let mut cr4: u64;
        core::arch::asm!("mov {}, cr4", out(reg) cr4, options(nomem, nostack));
        cr4 |= (1 << 9) | (1 << 10); // OSFXSR | OSXMMEXCPT
        core::arch::asm!("mov cr4, {}", in(reg) cr4, options(nomem, nostack));

        fpu_init();
    }
}

/// Deja la FPU/SSE en el estado limpio de `CLEAN_FXSTATE` — para una
/// tarea que entra por primera vez. Un `finit` a secas no bastaba: no
/// toca MXCSR ni los XMM, así que el proceso nuevo heredaba el modo de
/// redondeo y los registros vectoriales del último que corrió.
pub unsafe fn fpu_init() {
    fxrstor(&CLEAN_FXSTATE);
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
