//! Local APIC + timer periódico — M4b, primer paso (BORRADOR SIN
//! VERIFICAR).
//!
//! Habilita el Local APIC y programa un timer periódico que dispara una
//! interrupción real por hardware. Esto NO conecta todavía esa
//! interrupción con el scheduler para hacer preemption de verdad — eso
//! es el siguiente paso, y es más delicado: un handler asíncrono tiene
//! que guardar TODOS los registros de propósito general (puede
//! interrumpir al proceso en cualquier punto), no solo los
//! callee-saved como el cambio de contexto cooperativo de task.rs. De
//! momento esto solo demuestra que el timer late de verdad.
//!
//! Requiere el identity-map ampliado a 4 GiB de boot.asm — el Local
//! APIC vive en memoria física ~0xFEE00000, muy por encima del primer
//! GiB que teníamos mapeado antes de esta ronda.

use crate::idt::InterruptStackFrame;
use crate::serial_println;
use core::sync::atomic::{AtomicU64, Ordering};

const IA32_APIC_BASE_MSR: u32 = 0x1B;

const REG_EOI: u64 = 0xB0;
const REG_SPURIOUS: u64 = 0xF0;
const REG_LVT_TIMER: u64 = 0x320;
const REG_TIMER_INITIAL_COUNT: u64 = 0x380;
const REG_TIMER_DIVIDE: u64 = 0x3E0;

/// Fuera del rango 0-31 (excepciones de CPU) y 32-47 (PIC legacy, ya
/// remapeado y enmascarado en pic.rs) — sin colisión posible.
pub const TIMER_VECTOR: u8 = 0x40;

static TICKS: AtomicU64 = AtomicU64::new(0);
static mut APIC_BASE: u64 = 0;

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

unsafe fn reg_write(offset: u64, value: u32) {
    ((APIC_BASE + offset) as *mut u32).write_volatile(value);
}

/// # Safety
/// Requiere el identity-map de 4 GiB de boot.asm ya activo (lo está
/// desde el arranque, antes de llegar a Rust) — sin él, acceder a
/// `APIC_BASE` sería un page fault instantáneo.
pub unsafe fn init() {
    let base_msr = read_msr(IA32_APIC_BASE_MSR);
    APIC_BASE = base_msr & 0xFFFFF000; // bits 12-35 = dirección física base

    write_msr(IA32_APIC_BASE_MSR, base_msr | (1 << 11)); // bit 11 = APIC Global Enable

    // Spurious Interrupt Vector Register: bit 8 = APIC software enable.
    // 0xFF es solo un vector alto convencional para "spurious" — no
    // debería dispararse en uso normal, no tiene handler real detrás.
    reg_write(REG_SPURIOUS, 0x1FF);

    serial_println!("[apic] Local APIC habilitado, base física 0x{:x}", APIC_BASE);
}

/// Programa el timer en modo periódico. `initial_count` es un valor de
/// cuenta arbitrario — SIN calibrar contra el PIT/TSC todavía, esto NO
/// corresponde a una frecuencia conocida en Hz, es solo "lo bastante
/// bajo para ver varios ticks en un rato razonable". Calibración real
/// queda para una pasada posterior (ver TODO.md §1, "Timer del sistema").
pub unsafe fn start_periodic_timer(initial_count: u32) {
    reg_write(REG_TIMER_DIVIDE, 0b1011); // divisor = 1 (el más fino disponible)
    reg_write(REG_LVT_TIMER, (TIMER_VECTOR as u32) | (1 << 17)); // bit 17 = modo periódico
    reg_write(REG_TIMER_INITIAL_COUNT, initial_count);
    serial_println!("[apic] timer periódico armado, initial_count={} (sin calibrar)", initial_count);
}

pub fn tick_count() -> u64 {
    TICKS.load(Ordering::Relaxed)
}

/// Handler de la interrupción del timer. De momento SOLO cuenta ticks y
/// manda EOI — todavía no interrumpe una tarea en marcha para hacer
/// preemption real (ver cabecera del fichero).
pub extern "x86-interrupt" fn timer_interrupt_handler(_frame: InterruptStackFrame) {
    TICKS.fetch_add(1, Ordering::Relaxed);
    unsafe { reg_write(REG_EOI, 0) };
}
