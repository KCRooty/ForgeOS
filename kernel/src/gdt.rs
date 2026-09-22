//! GDT + TSS — M1b (BORRADOR SIN VERIFICAR EN QEMU TODAVÍA).
//!
//! En long mode la segmentación real ya no importa (todo es flat), pero
//! seguimos necesitando una GDT válida con selectores de código/datos y
//! un descriptor de TSS. El TSS es lo que nos da una pila IST separada
//! para el manejador de double fault — así un double fault con la pila
//! del kernel ya reventada no provoca un triple fault silencioso (reset
//! de la máquina sin ni un log).
//!
//! Valores de descriptor estándar de osdev — mismo patrón que
//! Asmodeus14/Nyx (`gdt.rs`) y nyxos-dev/nyx-os.

use core::mem::size_of;

pub const KERNEL_CODE_SELECTOR: u16 = 0x08;
pub const KERNEL_DATA_SELECTOR: u16 = 0x10;
pub const TSS_SELECTOR: u16 = 0x18;

// present, ring0, código, ejecutable+legible, long mode (bit L)
const KERNEL_CODE: u64 = 0x00AF9A000000FFFF;
// present, ring0, datos, escribible
const KERNEL_DATA: u64 = 0x00CF92000000FFFF;

/// Índice en TSS.ist[] (0-based). El IDT referencia esto con ist=1
/// (el campo IST del IDT es 1-based: 0 significa "no cambiar de pila").
pub const DOUBLE_FAULT_IST_INDEX: usize = 0;

const DOUBLE_FAULT_STACK_SIZE: usize = 4096 * 4; // 16 KiB

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

static mut GDT: [u64; 5] = [0; 5]; // null, code, data, tss_low, tss_high
static mut TSS: Tss = Tss::new();
static mut DOUBLE_FAULT_STACK: [u8; DOUBLE_FAULT_STACK_SIZE] = [0; DOUBLE_FAULT_STACK_SIZE];

pub fn init() {
    unsafe {
        let stack_top = DOUBLE_FAULT_STACK.as_ptr() as u64 + DOUBLE_FAULT_STACK_SIZE as u64;
        TSS.ist[DOUBLE_FAULT_IST_INDEX] = stack_top;

        GDT[0] = 0;
        GDT[1] = KERNEL_CODE;
        GDT[2] = KERNEL_DATA;

        let tss_addr = core::ptr::addr_of!(TSS) as u64;
        let tss_limit = (size_of::<Tss>() - 1) as u64;

        // Descriptor de sistema de 16 bytes (TSS disponible de 64 bits, tipo 0x9)
        let low = (tss_limit & 0xFFFF)
            | ((tss_addr & 0xFFFFFF) << 16)
            | (0x89u64 << 40) // present, DPL0, tipo=TSS disponible
            | (((tss_limit >> 16) & 0xF) << 48)
            | (((tss_addr >> 24) & 0xFF) << 56);
        let high = (tss_addr >> 32) & 0xFFFFFFFF;

        GDT[3] = low;
        GDT[4] = high;

        let ptr = GdtPointer {
            limit: (size_of::<[u64; 5]>() - 1) as u16,
            base: core::ptr::addr_of!(GDT) as u64,
        };

        core::arch::asm!("lgdt [{}]", in(reg) &ptr, options(readonly, nostack, preserves_flags));
        reload_segments();

        core::arch::asm!("ltr ax", in("ax") TSS_SELECTOR, options(nostack, preserves_flags));
    }
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
