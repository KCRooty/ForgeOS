//! Teclado PS/2 — scancodes Set 1, layout US QWERTY (BORRADOR SIN
//! VERIFICAR).
//!
//! Layout: solo US por ahora — el layout español queda anotado en
//! TODO.md §12 ("Localización / distribución de teclado") como
//! pendiente. No me fío de reconstruir de memoria una tabla de
//! scancodes de layout español byte-exacta sin poder verla funcionando;
//! la tabla US Set 1 es la más ampliamente documentada y estable que
//! existe en osdev — mucho menor riesgo de errores sutiles que intentar
//! recordar una variante regional.
//!
//! Maneja mayúsculas/minúsculas vía Shift. Los símbolos con Shift
//! (!@#$%...) quedan sin mapear por ahora — ampliar es trivial una vez
//! esto esté validado.

use crate::idt::InterruptStackFrame;
use crate::pic;
use core::cell::UnsafeCell;

const DATA_PORT: u16 = 0x60;
pub const IRQ: u8 = 1;

#[inline(always)]
unsafe fn inb(port: u16) -> u8 {
    let val: u8;
    core::arch::asm!("in al, dx", out("al") val, in("dx") port, options(nomem, nostack, preserves_flags));
    val
}

const BUF_SIZE: usize = 64;

struct KeyBuffer {
    data: UnsafeCell<[u8; BUF_SIZE]>,
    head: UnsafeCell<usize>,
    tail: UnsafeCell<usize>,
}
unsafe impl Sync for KeyBuffer {}

static BUFFER: KeyBuffer = KeyBuffer {
    data: UnsafeCell::new([0; BUF_SIZE]),
    head: UnsafeCell::new(0),
    tail: UnsafeCell::new(0),
};

static mut SHIFT_HELD: bool = false;

const LEFT_SHIFT: u8 = 0x2A;
const RIGHT_SHIFT: u8 = 0x36;

// Scan Code Set 1 -> ASCII (US QWERTY, sin Shift). 0 = sin mapear.
const SCANCODE_TO_ASCII: [u8; 0x3A] = [
    0, 0, b'1', b'2', b'3', b'4', b'5', b'6', b'7', b'8', // 0x00-0x09
    b'9', b'0', b'-', b'=', 0x08, b'\t', // 0x0A-0x0F (0x0E=backspace)
    b'q', b'w', b'e', b'r', b't', b'y', b'u', b'i', b'o', b'p', // 0x10-0x19
    b'[', b']', b'\r', 0, b'a', b's', // 0x1A-0x1F (0x1C=enter, 0x1D=ctrl)
    b'd', b'f', b'g', b'h', b'j', b'k', b'l', b';', b'\'', b'`', // 0x20-0x29
    0, b'\\', b'z', b'x', b'c', b'v', b'b', b'n', b'm', b',', // 0x2A-0x33 (0x2A=lshift)
    b'.', b'/', 0, b'*', 0, b' ', // 0x34-0x39 (0x36=rshift, 0x38=alt, 0x39=space)
];

fn to_upper(c: u8) -> u8 {
    if c.is_ascii_lowercase() {
        c - 32
    } else {
        c
    }
}

fn push(byte: u8) {
    unsafe {
        let head = *BUFFER.head.get();
        let next = (head + 1) % BUF_SIZE;
        if next == *BUFFER.tail.get() {
            return; // buffer lleno — descartamos la tecla, no bloqueamos la IRQ
        }
        (*BUFFER.data.get())[head] = byte;
        *BUFFER.head.get() = next;
    }
}

/// No bloqueante — la consola de depuración lo consulta junto al puerto
/// serie, tratando el primero que tenga dato listo como entrada.
pub fn try_read_byte() -> Option<u8> {
    unsafe {
        let tail = *BUFFER.tail.get();
        if tail == *BUFFER.head.get() {
            return None; // buffer vacío
        }
        let byte = (*BUFFER.data.get())[tail];
        *BUFFER.tail.get() = (tail + 1) % BUF_SIZE;
        Some(byte)
    }
}

pub fn init() {
    pic::unmask(IRQ);
}

/// Handler de IRQ1 — vector remapeado 32+1=33 (ver idt.rs). Traduce el
/// scancode, gestiona el estado de Shift, y si es una tecla imprimible
/// la mete en el buffer circular.
pub extern "x86-interrupt" fn keyboard_interrupt_handler(_frame: InterruptStackFrame) {
    let scancode = unsafe { inb(DATA_PORT) };
    let is_release = scancode & 0x80 != 0;
    let code = scancode & 0x7F;

    if code == LEFT_SHIFT || code == RIGHT_SHIFT {
        unsafe { SHIFT_HELD = !is_release };
    } else if !is_release && (code as usize) < SCANCODE_TO_ASCII.len() {
        let mut ascii = SCANCODE_TO_ASCII[code as usize];
        if ascii != 0 {
            if unsafe { SHIFT_HELD } {
                ascii = to_upper(ascii);
            }
            push(ascii);
        }
    }

    pic::end_of_interrupt(IRQ);
}
