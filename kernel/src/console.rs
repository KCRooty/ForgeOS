//! Consola de depuración interactiva sobre el puerto serie — M2
//! (BORRADOR SIN VERIFICAR).
//!
//! NO es el shell del producto final ("Bellows", M4+, con procesos
//! reales) — es herramienta de bring-up. Aprovecha que el puerto serie
//! ya existe y que `run-qemu.sh` en modo interactivo conecta el teclado
//! real del host a COM1 vía `-serial stdio`. Sin esto, verificar el
//! kernel significa re-flashear y releer `boot.log` cada vez.
//!
//! Ver docs/SHELL.md para la relación con el shell y el terminal reales.

use crate::serial::SerialPort;
use crate::{ahci, caps, framebuffer, pci, pmm, rtl8139};
use alloc::string::String;
use alloc::vec::Vec;
use core::fmt::Write;

const PROMPT: &str = "forge> ";
const BACKSPACE: u8 = 0x08;
const DEL: u8 = 0x7F;
const CR: u8 = b'\r';
const LF: u8 = b'\n';

pub fn run() -> ! {
    let mut port = SerialPort::init();
    let _ = write!(port, "\r\n--- consola de depuracion Forge OS ---\r\n");
    let _ = write!(port, "escribe 'help' para ver los comandos\r\n\r\n");

    loop {
        let _ = write!(port, "{}", PROMPT);
        let line = read_line(&mut port);
        dispatch(&mut port, line.trim());
    }
}

fn read_line(port: &mut SerialPort) -> String {
    let mut buf: Vec<u8> = Vec::new();
    loop {
        // Escuchamos teclado PS/2 y puerto serie a la vez — el primero
        // que tenga un byte listo gana. `hlt` evita busy-spinning
        // mientras esperamos a cualquiera de los dos.
        let byte = loop {
            if let Some(b) = crate::keyboard::try_read_byte() {
                break b;
            }
            if let Some(b) = port.try_read_byte() {
                break b;
            }
            unsafe { core::arch::asm!("hlt") };
        };

        match byte {
            CR | LF => {
                let _ = write!(port, "\r\n");
                break;
            }
            BACKSPACE | DEL => {
                if buf.pop().is_some() {
                    let _ = write!(port, "\x08 \x08"); // retrocede, borra, retrocede
                }
            }
            _ => {
                buf.push(byte);
                port.write_byte(byte); // eco
            }
        }
    }
    String::from_utf8_lossy(&buf).into_owned()
}

fn dispatch(port: &mut SerialPort, line: &str) {
    let mut parts = line.split_whitespace();
    let cmd = match parts.next() {
        Some(c) => c,
        None => return, // línea vacía, solo Enter
    };

    match cmd {
        "help" => {
            let _ = write!(port, "comandos: help, meminfo, caps, bp, panic, fb, pci, ahci, net\r\n");
        }
        "pci" => pci::scan_and_print(),
        "ahci" => ahci::probe_and_print(),
        "net" => rtl8139::probe_and_print(),
        "fb" => {
            if framebuffer::available() {
                framebuffer::test_pattern();
                framebuffer::draw_str(20, 20, "FORGE OS BOOT OK", 0x00000000, 4);
                let _ = write!(port, "patrón + banner redibujados\r\n");
            } else {
                let _ = write!(port, "sin framebuffer disponible\r\n");
            }
        }
        "meminfo" => {
            let free = pmm::free_frame_count();
            let _ = write!(port, "frames libres: {} (~{} KiB)\r\n", free, free * 4);
        }
        "caps" => run_caps_demo(port),
        "bp" => unsafe { core::arch::asm!("int3") },
        "panic" => panic!("panic solicitado desde la consola de depuración"),
        other => {
            let _ = write!(port, "comando desconocido: '{}' (prueba 'help')\r\n", other);
        }
    }
}

fn run_caps_demo(port: &mut SerialPort) {
    let mut c = caps::Capabilities::unrestricted();
    let _ = c.pledge(caps::CAP_STDIO);
    let _ = write!(
        port,
        "pledge STDIO -> enforce STDIO permitido = {}\r\n",
        c.enforce(caps::CAP_STDIO).is_ok()
    );
    let _ = write!(
        port,
        "enforce NET (no pledgeado) permitido = {}\r\n",
        c.enforce(caps::CAP_NET).is_ok()
    );
}
