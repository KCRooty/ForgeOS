//! PIC 8259 (master + slave) — M1c (BORRADOR SIN VERIFICAR EN QEMU).
//!
//! Por defecto el PIC entrega las IRQs de hardware en los vectores 0-15,
//! que colisionan de lleno con las excepciones de CPU (divide error=0,
//! breakpoint=3, etc. — ver idt.rs). Sin remapear, cualquier IRQ real
//! (teclado, timer...) dispararía el handler de excepción equivocado.
//!
//! Remapeamos IRQ0-7 -> vectores 32-39 e IRQ8-15 -> vectores 40-47, y
//! dejamos TODO enmascarado (ninguna IRQ llega todavía) porque aún no
//! tenemos drivers que las atiendan — activar `sti` sin un handler real
//! detrás sería peor que no tener interrupciones en absoluto. Habilitar
//! IRQs concretas es tarea de cada driver cuando exista (M2+).

const PIC1_CMD: u16 = 0x20;
const PIC1_DATA: u16 = 0x21;
const PIC2_CMD: u16 = 0xA0;
const PIC2_DATA: u16 = 0xA1;

const ICW1_INIT: u8 = 0x11; // init + se espera ICW4
const ICW4_8086: u8 = 0x01; // modo 8086, no 8080

pub const PIC1_OFFSET: u8 = 32; // IRQ0-7   -> vectores 32-39
pub const PIC2_OFFSET: u8 = 40; // IRQ8-15  -> vectores 40-47

#[inline(always)]
unsafe fn outb(port: u16, val: u8) {
    core::arch::asm!("out dx, al", in("dx") port, in("al") val, options(nomem, nostack, preserves_flags));
}

#[inline(always)]
unsafe fn inb(port: u16) -> u8 {
    let val: u8;
    core::arch::asm!("in al, dx", out("al") val, in("dx") port, options(nomem, nostack, preserves_flags));
    val
}

/// Pequeña espera de E/S para hardware PIC real lento — escribe a un
/// puerto sin usar (0x80, tradicionalmente "POST code"). En QEMU no hace
/// falta, pero es el patrón estándar de osdev y no cuesta nada mantenerlo
/// por si esto llega a correr en hardware físico antiguo de verdad.
#[inline(always)]
unsafe fn io_wait() {
    outb(0x80, 0);
}

pub fn init() {
    unsafe {
        // Guarda las máscaras actuales (no deberían importar en boot limpio,
        // pero es higiene estándar antes de reprogramar el PIC).
        let mask1 = inb(PIC1_DATA);
        let mask2 = inb(PIC2_DATA);

        // ICW1: inicia la secuencia en ambos PICs
        outb(PIC1_CMD, ICW1_INIT);
        io_wait();
        outb(PIC2_CMD, ICW1_INIT);
        io_wait();

        // ICW2: offset de vector base de cada PIC
        outb(PIC1_DATA, PIC1_OFFSET);
        io_wait();
        outb(PIC2_DATA, PIC2_OFFSET);
        io_wait();

        // ICW3: le dice al master que hay un slave colgado de IRQ2 (bit 2),
        // y al slave cuál es su identidad de cascada (2)
        outb(PIC1_DATA, 0b0000_0100);
        io_wait();
        outb(PIC2_DATA, 0b0000_0010);
        io_wait();

        // ICW4: modo 8086
        outb(PIC1_DATA, ICW4_8086);
        io_wait();
        outb(PIC2_DATA, ICW4_8086);
        io_wait();

        // Restaura máscaras previas... pero en M1c preferimos enmascarar
        // TODO explícitamente: no hay un solo driver de IRQ todavía, así
        // que cualquier IRQ que llegara no tendría quién la atienda.
        let _ = (mask1, mask2); // silenciamos "no usado" a propósito
        outb(PIC1_DATA, 0xFF);
        outb(PIC2_DATA, 0xFF);
    }
}

/// Fin de interrupción — cada handler de IRQ (cuando existan, M2+) debe
/// llamar a esto al terminar, o el PIC deja de entregar más IRQs de ese
/// nivel para siempre. `irq` es el número de IRQ real (0-15), no el
/// vector remapeado.
pub fn end_of_interrupt(irq: u8) {
    unsafe {
        if irq >= 8 {
            outb(PIC2_CMD, 0x20);
        }
        outb(PIC1_CMD, 0x20);
    }
}

/// Habilita una IRQ concreta (0-15) desenmascarándola. Los drivers llaman
/// a esto cuando están listos para recibir su interrupción — nunca se
/// habilita nada de forma global ni implícita.
pub fn unmask(irq: u8) {
    unsafe {
        let (port, bit) = if irq < 8 {
            (PIC1_DATA, irq)
        } else {
            (PIC2_DATA, irq - 8)
        };
        let current = inb(port);
        outb(port, current & !(1 << bit));
    }
}
