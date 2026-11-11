//! Puerto serie COM1 (UART 16550) — canal de debug y CI.
//! Mismo patrón que `serial.rs` en Asmodeus14/Nyx y el driver serie
//! de nyxos-dev/nyx-os: es la fuente de verdad para logs headless en QEMU.

use core::cell::UnsafeCell;
use core::fmt;
use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

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

/// IRQ de COM1 en el PIC (vector `pic::PIC1_OFFSET + 4`).
pub const IRQ: u8 = 4;

/// La UART solo se programa una vez. `SerialPort::init()` se llama en
/// cada `serial_println!`, y antes reprogramaba el chip entero en cada
/// llamada — incluido el "vaciar FIFO" del registro FCR, que TIRABA los
/// bytes recibidos y aún no leídos. Cada línea de log del kernel podía
/// comerse lo que se estuviera tecleando en la consola en ese instante.
static HARDWARE_READY: AtomicBool = AtomicBool::new(false);

/// `true` cuando la recepción va por interrupción (`enable_rx_interrupt`):
/// desde entonces solo el manejador lee del chip, y `try_read_byte` saca
/// los bytes de `RX_BUFFER`.
static RX_BY_IRQ: AtomicBool = AtomicBool::new(false);

const RX_BUF_SIZE: usize = 256;

/// Cola circular de recepción: la llena el manejador de la IRQ y la vacía
/// la consola. Un productor y un consumidor en una sola CPU — cada lado
/// solo escribe su propio índice, así que no hace falta más protección.
struct RxBuffer {
    data: UnsafeCell<[u8; RX_BUF_SIZE]>,
    head: AtomicUsize, // siguiente hueco a escribir (manejador)
    tail: AtomicUsize, // siguiente byte a leer (consola)
}
unsafe impl Sync for RxBuffer {}

static RX_BUFFER: RxBuffer = RxBuffer {
    data: UnsafeCell::new([0; RX_BUF_SIZE]),
    head: AtomicUsize::new(0),
    tail: AtomicUsize::new(0),
};

impl SerialPort {
    pub fn init() -> Self {
        if !HARDWARE_READY.swap(true, Ordering::Relaxed) {
            unsafe {
                outb(COM1 + 1, 0x00); // deshabilita interrupciones
                outb(COM1 + 3, 0x80); // habilita DLAB
                outb(COM1 + 0, 0x03); // divisor 3 -> 38400 baud
                outb(COM1 + 1, 0x00);
                outb(COM1 + 3, 0x03); // 8 bits, sin paridad, 1 stop bit
                outb(COM1 + 2, 0xC7); // FIFO, clear, umbral 14 bytes
                outb(COM1 + 4, 0x0B); // DTR + RTS + OUT2 (OUT2 deja pasar la IRQ al PIC)
            }
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

    fn is_data_ready() -> bool {
        unsafe { inb(COM1 + 5) & 0x01 != 0 }
    }

    pub fn try_read_byte(&mut self) -> Option<u8> {
        if !RX_BY_IRQ.load(Ordering::Relaxed) {
            // Arranque temprano, antes de `enable_rx_interrupt`: sondeo directo.
            return if Self::is_data_ready() { Some(unsafe { inb(COM1) }) } else { None };
        }
        let tail = RX_BUFFER.tail.load(Ordering::Relaxed);
        if tail == RX_BUFFER.head.load(Ordering::Relaxed) {
            return None;
        }
        let byte = unsafe { (*RX_BUFFER.data.get())[tail] };
        RX_BUFFER.tail.store((tail + 1) % RX_BUF_SIZE, Ordering::Relaxed);
        Some(byte)
    }
}

/// Pasa la recepción de COM1 a interrupciones (IRQ4). Llamar una vez,
/// con la IDT y el PIC ya montados.
///
/// Leer por sondeo solo funciona si quien sondea corre a menudo: la FIFO
/// del chip guarda 16 bytes, y con otros procesos ocupando la CPU la
/// consola podía pasar ticks enteros sin mirar — lo tecleado (o pegado,
/// o enviado por un script) más allá de esos 16 bytes se perdía. Con la
/// IRQ, cada byte pasa a `RX_BUFFER` en cuanto llega, corra quien corra.
pub fn enable_rx_interrupt() {
    SerialPort::init();
    RX_BY_IRQ.store(true, Ordering::Relaxed);
    unsafe { outb(COM1 + 1, 0x01) }; // IER: interrupción por dato recibido
    crate::pic::unmask(IRQ);
}

pub extern "x86-interrupt" fn serial_interrupt_handler(_frame: crate::idt::InterruptStackFrame) {
    // Vaciar la FIFO entera: la UART solo vuelve a levantar la línea de
    // interrupción cuando llegue un byte nuevo, no por los que queden.
    while SerialPort::is_data_ready() {
        let byte = unsafe { inb(COM1) };
        let head = RX_BUFFER.head.load(Ordering::Relaxed);
        let next = (head + 1) % RX_BUF_SIZE;
        if next != RX_BUFFER.tail.load(Ordering::Relaxed) {
            unsafe { (*RX_BUFFER.data.get())[head] = byte };
            RX_BUFFER.head.store(next, Ordering::Relaxed);
        } // cola llena: el byte se descarta, la IRQ nunca bloquea
    }
    crate::pic::end_of_interrupt(IRQ);
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
