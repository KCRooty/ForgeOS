//! Terminal de control — la entrada estándar de los procesos (M12).
//!
//! Hay una única terminal: el teclado PS/2 y el puerto serie, los mismos
//! dos orígenes que lee la consola de depuración. `read(0, ...)` desde un
//! proceso pasa por aquí, en modo "cocinado" como una tty de Unix:
//!
//! - el kernel hace el eco y la edición de línea (retroceso);
//! - el proceso no recibe nada hasta que se pulsa Intro, y entonces
//!   recibe la línea entera, con su `\n`;
//! - Ctrl-D al principio de una línea es fin de fichero (`read` devuelve
//!   0); Ctrl-C mata al proceso que está leyendo (SIGINT).
//!
//! Limitación a propósito: no hay "proceso en primer plano". Quien llama
//! a `read(0)` se lleva lo tecleado; la consola no lee mientras espera a
//! un `run` en primer plano, así que no compiten, pero dos procesos
//! leyendo a la vez (uno lanzado con `&`) se repartirían las líneas.

use crate::serial::SerialPort;
use alloc::vec::Vec;

const BACKSPACE: u8 = 0x08;
const DEL: u8 = 0x7F;
const CTRL_C: u8 = 0x03;
const CTRL_D: u8 = 0x04;

/// Línea más larga que se puede editar; lo que pase de ahí se ignora.
const MAX_LINE: usize = 1024;

struct Tty {
    /// Línea a medio teclear: todavía editable, invisible para el proceso.
    editing: Vec<u8>,
    /// Bytes ya confirmados con Intro (o Ctrl-D), pendientes de entregar.
    ready: Vec<u8>,
    /// Ctrl-D con la línea vacía: la próxima lectura devuelve 0.
    eof: bool,
}

struct TtyCell(core::cell::UnsafeCell<Tty>);
// Una sola CPU, y solo se toca desde syscalls (interrupciones enmascaradas).
unsafe impl Sync for TtyCell {}

static TTY: TtyCell = TtyCell(core::cell::UnsafeCell::new(Tty {
    editing: Vec::new(),
    ready: Vec::new(),
    eof: false,
}));

/// Resultado de un intento de lectura que no bloquea.
pub enum Read {
    /// Se copiaron estos bytes al buffer (0 = fin de fichero).
    Data(usize),
    /// Todavía no hay una línea completa: ceder el turno y reintentar.
    WouldBlock,
    /// Se pulsó Ctrl-C.
    Interrupted,
}

/// Procesa lo que haya llegado por teclado/serie y, si ya hay una línea
/// completa, entrega hasta `out.len()` bytes de ella.
pub fn try_read(out: &mut [u8]) -> Read {
    let tty = unsafe { &mut *TTY.0.get() };
    let mut port = SerialPort::init();

    while tty.ready.is_empty() && !tty.eof {
        let byte = match crate::keyboard::try_read_byte().or_else(|| port.try_read_byte()) {
            Some(b) => b,
            None => return Read::WouldBlock,
        };
        match byte {
            b'\r' | b'\n' => {
                port.write_byte(b'\r');
                port.write_byte(b'\n');
                tty.editing.push(b'\n');
                tty.ready.append(&mut tty.editing);
            }
            BACKSPACE | DEL => {
                if tty.editing.pop().is_some() {
                    for &b in b"\x08 \x08" {
                        port.write_byte(b); // retrocede, borra, retrocede
                    }
                }
            }
            CTRL_C => {
                tty.editing.clear();
                for &b in b"^C\r\n" {
                    port.write_byte(b);
                }
                return Read::Interrupted;
            }
            CTRL_D => {
                if tty.editing.is_empty() {
                    tty.eof = true;
                } else {
                    // A mitad de línea: entrega lo tecleado sin `\n`.
                    tty.ready.append(&mut tty.editing);
                }
            }
            _ => {
                if tty.editing.len() < MAX_LINE {
                    tty.editing.push(byte);
                    port.write_byte(byte); // eco
                }
            }
        }
    }

    if tty.ready.is_empty() {
        tty.eof = false; // el fin de fichero se entrega una vez
        return Read::Data(0);
    }
    let n = out.len().min(tty.ready.len());
    out[..n].copy_from_slice(&tty.ready[..n]);
    tty.ready.drain(..n);
    Read::Data(n)
}
