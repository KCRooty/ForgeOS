//! IDT + manejadores de excepción — M1b (BORRADOR SIN VERIFICAR EN QEMU).
//!
//! 256 entradas de 16 bytes (formato long mode). Cubrimos las excepciones
//! de CPU que más pronto delatan un bug real: divide error, breakpoint
//! (para probar que el mecanismo funciona sin matar nada), invalid
//! opcode, double fault (en su propia pila IST — ver gdt.rs), GPF y page
//! fault (con lectura de CR2 para saber qué dirección reventó).
//!
//! ADVERTENCIA: el ABI `extern "x86-interrupt"` requiere
//! `#![feature(abi_x86_interrupt)]` en main.rs y su firma exacta
//! (`InterruptStackFrame` por valor vs por referencia) ha variado entre
//! nightlies a lo largo de los años. Es el punto más frágil de este
//! fichero — si no compila tal cual contra tu toolchain, es el primer
//! sitio a revisar.

use crate::gdt;
use crate::serial_println;
use core::mem::size_of;
#[repr(C)]
pub struct InterruptStackFrame {
    pub instruction_pointer: u64,
    pub code_segment: u64,
    pub cpu_flags: u64,
    pub stack_pointer: u64,
    pub stack_segment: u64,
}

#[repr(C, packed)]
#[derive(Clone, Copy)]
struct IdtEntry {
    offset_low: u16,
    selector: u16,
    ist: u8,
    type_attr: u8,
    offset_mid: u16,
    offset_high: u32,
    reserved: u32,
}

impl IdtEntry {
    const fn missing() -> Self {
        IdtEntry {
            offset_low: 0,
            selector: 0,
            ist: 0,
            type_attr: 0,
            offset_mid: 0,
            offset_high: 0,
            reserved: 0,
        }
    }

    fn set_handler(&mut self, handler_addr: u64, ist: u8) {
        self.offset_low = handler_addr as u16;
        self.offset_mid = (handler_addr >> 16) as u16;
        self.offset_high = (handler_addr >> 32) as u32;
        self.selector = gdt::KERNEL_CODE_SELECTOR;
        self.ist = ist;
        self.type_attr = 0x8E; // present, DPL0, interrupt gate de 64 bits
    }
}

#[repr(C, packed)]
struct IdtPointer {
    limit: u16,
    base: u64,
}

static mut IDT: [IdtEntry; 256] = [IdtEntry::missing(); 256];

pub fn init() {
    unsafe {
        IDT[0].set_handler(divide_error as u64, 0);
        IDT[3].set_handler(breakpoint as u64, 0);
        IDT[6].set_handler(invalid_opcode as u64, 0);
        IDT[8].set_handler(double_fault as u64, 1); // ist=1 -> gdt::DOUBLE_FAULT_IST_INDEX
        IDT[13].set_handler(general_protection_fault as u64, 0);
        IDT[14].set_handler(page_fault as u64, 0);
        IDT[crate::apic::TIMER_VECTOR as usize]
            .set_handler(crate::apic::timer_interrupt_handler as u64, 0);
        IDT[(crate::pic::PIC1_OFFSET + crate::keyboard::IRQ) as usize]
            .set_handler(crate::keyboard::keyboard_interrupt_handler as u64, 0);

        let ptr = IdtPointer {
            limit: (size_of::<[IdtEntry; 256]>() - 1) as u16,
            base: core::ptr::addr_of!(IDT) as u64,
        };
        core::arch::asm!("lidt [{}]", in(reg) &ptr, options(readonly, nostack, preserves_flags));
    }
    serial_println!("[idt] cargada: #DE #BP #UD #DF #GP #PF");
}

extern "x86-interrupt" fn divide_error(_frame: InterruptStackFrame) {
    serial_println!("[EXCEPTION] Divide error (#DE)");
    halt();
}

extern "x86-interrupt" fn breakpoint(frame: InterruptStackFrame) {
    let rip = frame.instruction_pointer;
    serial_println!("[EXCEPTION] Breakpoint (#BP) en rip=0x{:x} — continuando", rip);
    // sin halt(): un breakpoint debe poder continuar la ejecución normal
}

extern "x86-interrupt" fn invalid_opcode(frame: InterruptStackFrame) {
    let rip = frame.instruction_pointer;
    serial_println!("[EXCEPTION] Invalid opcode (#UD) en rip=0x{:x}", rip);
    halt();
}

extern "x86-interrupt" fn double_fault(_frame: InterruptStackFrame, error_code: u64) -> ! {
    serial_println!("[FATAL] Double fault (#DF, error_code={}) — pila IST, sin triple fault", error_code);
    halt();
}

extern "x86-interrupt" fn general_protection_fault(frame: InterruptStackFrame, error_code: u64) {
    let rip = frame.instruction_pointer;
    serial_println!("[EXCEPTION] GPF (error_code={}) en rip=0x{:x}", error_code, rip);
    halt();
}

extern "x86-interrupt" fn page_fault(frame: InterruptStackFrame, error_code: u64) {
    let fault_addr: u64;
    unsafe { core::arch::asm!("mov {}, cr2", out(reg) fault_addr) };
    let rip = frame.instruction_pointer;
    serial_println!(
        "[EXCEPTION] Page fault en dirección=0x{:x} (error_code={}, rip=0x{:x})",
        fault_addr,
        error_code,
        rip
    );
    halt();
}

fn halt() -> ! {
    loop {
        unsafe { core::arch::asm!("hlt") };
    }
}
