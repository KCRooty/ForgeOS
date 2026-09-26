//! GDT + TSS — M1b + M4f (BORRADOR SIN VERIFICAR EN QEMU TODAVÍA).
//!
//! En long mode la segmentación real ya no importa (todo es flat), pero
//! seguimos necesitando una GDT válida con selectores de código/datos y
//! un descriptor de TSS. El TSS nos da dos cosas: una pila IST separada
//! para el manejador de double fault, y (desde M4f) `RSP0` — la pila
//! que la CPU carga automáticamente cuando una interrupción interrumpe
//! código en ring 3 y hay que volver a ring 0. Sin RSP0 configurado, la
//! primera interrupción estando en ring 3 usaría una pila basura.
//!
//! Valores de descriptor estándar de osdev — mismo patrón que
//! Asmodeus14/Nyx (`gdt.rs`) y nyxos-dev/nyx-os.

use core::mem::size_of;

pub const KERNEL_CODE_SELECTOR: u16 = 0x08;
pub const KERNEL_DATA_SELECTOR: u16 = 0x10;
pub const USER_DATA_SELECTOR: u16 = 0x18 | 3; // RPL=3
pub const USER_CODE_SELECTOR: u16 = 0x20 | 3; // RPL=3
pub const TSS_SELECTOR: u16 = 0x28;

// present, ring0, código, ejecutable+legible, long mode (bit L)
const KERNEL_CODE: u64 = 0x00AF9A000000FFFF;
// present, ring0, datos, escribible
const KERNEL_DATA: u64 = 0x00CF92000000FFFF;
// mismos descriptores que los de kernel, con DPL=3 en vez de DPL=0
// (bits 45-46 del descriptor, dentro del byte de acceso: 0x92->0xF2,
// 0x9A->0xFA — el resto de bits no cambia)
const USER_DATA: u64 = 0x00CFF2000000FFFF;
const USER_CODE: u64 = 0x00AFFA000000FFFF;

/// Índice en TSS.ist[] (0-based). El IDT referencia esto con ist=1
/// (el campo IST del IDT es 1-based: 0 significa "no cambiar de pila").
pub const DOUBLE_FAULT_IST_INDEX: usize = 0;

const DOUBLE_FAULT_STACK_SIZE: usize = 4096 * 4; // 16 KiB
const RSP0_STACK_SIZE: usize = 4096 * 4; // 16 KiB

#[repr(C, packed)]
pub struct Tss {
    reserved0: u32,
    pub rsp: [u64; 3],
    reserved1: u64,
    pub ist: [u64; 7],
    reserved2: u64,
    reserved3: u16,
    iomap_base: u16,
}

impl Tss {
    const fn new() -> Self {
        Tss {
            reserved0: 0,
            rsp: [0; 3],
            reserved1: 0,
            ist: [0; 7],
            reserved2: 0,
            reserved3: 0,
            iomap_base: size_of::<Tss>() as u16, // sin bitmap de I/O
        }
    }
}

#[repr(C, packed)]
struct GdtPointer {
    limit: u16,
    base: u64,
}

// null, kernel_code, kernel_data, user_data, user_code, tss_low, tss_high
static mut GDT: [u64; 7] = [0; 7];
static mut TSS: Tss = Tss::new();
static mut DOUBLE_FAULT_STACK: [u8; DOUBLE_FAULT_STACK_SIZE] = [0; DOUBLE_FAULT_STACK_SIZE];
static mut RSP0_STACK: [u8; RSP0_STACK_SIZE] = [0; RSP0_STACK_SIZE];

pub fn init() {
    unsafe {
        let df_stack_top = DOUBLE_FAULT_STACK.as_ptr() as u64 + DOUBLE_FAULT_STACK_SIZE as u64;
        TSS.ist[DOUBLE_FAULT_IST_INDEX] = df_stack_top;

        let rsp0_top = RSP0_STACK.as_ptr() as u64 + RSP0_STACK_SIZE as u64;
        TSS.rsp[0] = rsp0_top;

        GDT[0] = 0;
        GDT[1] = KERNEL_CODE;
        GDT[2] = KERNEL_DATA;
        GDT[3] = USER_DATA;
        GDT[4] = USER_CODE;

        let tss_addr = core::ptr::addr_of!(TSS) as u64;
        let tss_limit = (size_of::<Tss>() - 1) as u64;

        // Descriptor de sistema de 16 bytes (TSS disponible de 64 bits, tipo 0x9)
        let low = (tss_limit & 0xFFFF)
            | ((tss_addr & 0xFFFFFF) << 16)
            | (0x89u64 << 40) // present, DPL0, tipo=TSS disponible
            | (((tss_limit >> 16) & 0xF) << 48)
            | (((tss_addr >> 24) & 0xFF) << 56);
        let high = (tss_addr >> 32) & 0xFFFFFFFF;

        GDT[5] = low;
        GDT[6] = high;

        let ptr = GdtPointer {
            limit: (size_of::<[u64; 7]>() - 1) as u16,
            base: core::ptr::addr_of!(GDT) as u64,
        };

        core::arch::asm!("lgdt [{}]", in(reg) &ptr, options(readonly, nostack, preserves_flags));
        reload_segments();

        core::arch::asm!("ltr ax", in("ax") TSS_SELECTOR, options(nostack, preserves_flags));
    }
}

/// Cambia `TSS.RSP0` — la pila que la CPU carga automáticamente cuando
/// una interrupción/excepción interrumpe código en ring 3 y hay que
/// volver a ring 0. Llamado por `scheduler::yield_now()` al entrarle
/// el turno a una tarea, con su `Task::kernel_stack_top` — mismo valor
/// que ya usa `syscall::set_syscall_stack_top`, reutilizado a propósito
/// (un `syscall` y una interrupción nunca están "en vuelo" a la vez
/// para la MISMA tarea: mientras corre ring 3 no hay syscall activa, y
/// mientras se atiende una syscall ya se está en CPL0 — el timer que
/// interrumpa esa ejecución cae por la rama CPL0 de
/// `preempt::timer_entry`, que ni mira RSP0).
///
/// Antes de esto había un único `RSP0_STACK` global para TODAS las
/// transiciones ring3→ring0 — inofensivo mientras `preempt.rs` no
/// tocara el scheduler en ese caso (solo mandaba EOI), pero exactamente
/// el mismo bug de fondo que ya tuvieron la pila de syscalls compartida
/// (milestone `wait()`) y la pila de páginas por descubrir: si dos
/// procesos de ring 3 quedaran "aparcados" a la vez en esa misma pila
/// física (uno preemptado, el otro corriendo después), el segundo
/// pisaría el estado guardado del primero.
pub fn set_rsp0(top: u64) {
    unsafe { TSS.rsp[0] = top };
}

/// CS solo se puede recargar con un far jump / far return; DS/ES/SS/FS/GS
/// con un `mov` normal basta (en long mode determinan poco, pero SS debe
/// ser válido para que `push`/`pop` no falten).
unsafe fn reload_segments() {
    core::arch::asm!(
        "push {code_sel}",
        "lea rax, [2f + rip]",
        "push rax",
        "retfq",
        "2:",
        "mov ax, {data_sel}",
        "mov ds, ax",
        "mov es, ax",
        "mov ss, ax",
        "mov fs, ax",
        "mov gs, ax",
        code_sel = const KERNEL_CODE_SELECTOR,
        data_sel = const KERNEL_DATA_SELECTOR,
        out("rax") _,
    );
}
