//! Puerto serie COM1 (UART 16550) — canal de debug y CI.
//! Mismo patrón que `serial.rs` en Asmodeus14/Nyx y el driver serie
//! de nyxos-dev/nyx-os: es la fuente de verdad para logs headless en QEMU.

use core::fmt;

const COM1: u16 = 0x3F8;

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

pub struct SerialPort;

impl SerialPort {
    pub fn init() -> Self {
        unsafe {
            outb(COM1 + 1, 0x00); // deshabilita interrupciones
            outb(COM1 + 3, 0x80); // habilita DLAB
            outb(COM1 + 0, 0x03); // divisor 3 -> 38400 baud
            outb(COM1 + 1, 0x00);
            outb(COM1 + 3, 0x03); // 8 bits, sin paridad, 1 stop bit
            outb(COM1 + 2, 0xC7); // FIFO, clear, umbral 14 bytes
            outb(COM1 + 4, 0x0B); // IRQs habilitadas, RTS/DSR set
        }
        SerialPort
    }

    fn is_transmit_empty() -> bool {
        unsafe { inb(COM1 + 5) & 0x20 != 0 }
    }

    pub fn write_byte(&mut self, byte: u8) {
        while !Self::is_transmit_empty() {}
        unsafe { outb(COM1, byte) };
    }
}

impl fmt::Write for SerialPort {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        for byte in s.bytes() {
            self.write_byte(byte);
        }
        Ok(())
    }
}

#[macro_export]
macro_rules! serial_println {
    ($($arg:tt)*) => {{
        use core::fmt::Write;
        let mut port = $crate::serial::SerialPort::init();
        let _ = writeln!(port, $($arg)*);
    }};
}
