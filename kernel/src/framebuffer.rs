//! Framebuffer lineal — M3, "parche" visual (BORRADOR SIN VERIFICAR).
//! Funcional, no bonito: el objetivo es demostrar que se escribe de
//! verdad en la pantalla, nada más.
//!
//! Requiere el tag de framebuffer de Multiboot2 (pedido explícitamente
//! en boot.asm) y asume 32bpp RGB directo — el caso común de VBE/GOP. Si
//! el firmware entrega otra profundidad, no dibujamos nada y lo decimos
//! por serie en vez de escribir basura en memoria a ciegas.

use crate::serial_println;

#[repr(C)]
struct FbTagHeader {
    tag_type: u32,
    size: u32,
    addr: u64,
    pitch: u32,
    width: u32,
    height: u32,
    bpp: u8,
    fb_type: u8,
    reserved: u8,
}

const TAG_FRAMEBUFFER: u32 = 8;
const TAG_END: u32 = 0;

pub struct Framebuffer {
    addr: u64,
    pitch: u32,
    width: u32,
    height: u32,
}

static mut FB: Option<Framebuffer> = None;

/// # Safety
/// Debe llamarse una sola vez, con el puntero Multiboot2 original.
pub unsafe fn init(mb2_ptr: u64) {
    let total_size = *(mb2_ptr as *const u32);
    let end = mb2_ptr + total_size as u64;
    let mut addr = mb2_ptr + 8;

    while addr < end {
        let tag_type = *(addr as *const u32);
        let tag_size = *((addr + 4) as *const u32);

        if tag_type == TAG_END {
            break;
        }

        if tag_type == TAG_FRAMEBUFFER {
            let tag = &*(addr as *const FbTagHeader);
            if tag.bpp == 32 {
                FB = Some(Framebuffer {
                    addr: tag.addr,
                    pitch: tag.pitch,
                    width: tag.width,
                    height: tag.height,
                });
                serial_println!(
                    "[fb] {}x{} @ 32bpp, pitch={}, addr=0x{:x}",
                    tag.width,
                    tag.height,
                    tag.pitch,
                    tag.addr
                );
            } else {
                serial_println!("[fb] tag encontrado pero bpp={} no soportado (solo 32)", tag.bpp);
            }
            return;
        }

        addr += (tag_size as u64 + 7) & !7;
    }

    serial_println!("[fb] no se encontró tag de framebuffer — sin salida gráfica");
}

pub fn available() -> bool {
    unsafe { FB.is_some() }
}

/// Sin comprobación de límites en el hot path — el llamante (fill_rect)
/// es responsable de no salirse.
unsafe fn put_pixel(fb: &Framebuffer, x: u32, y: u32, color: u32) {
    let offset = (y * fb.pitch) as u64 + (x * 4) as u64;
    let ptr = (fb.addr + offset) as *mut u32;
    ptr.write_volatile(color);
}

pub fn fill_rect(x: u32, y: u32, w: u32, h: u32, color: u32) {
    unsafe {
        let fb = match &FB {
            Some(fb) => fb,
            None => return,
        };
        let x_end = (x + w).min(fb.width);
        let y_end = (y + h).min(fb.height);
        for py in y..y_end {
            for px in x..x_end {
                put_pixel(fb, px, py, color);
            }
        }
    }
}

/// El "parche" en sí: ocho barras verticales de color bien distintas
/// entre sí, tipo carta de ajuste — prueba de que el framebuffer escribe
/// de verdad, sin pretender ser un escritorio.
pub fn test_pattern() {
    let dims = unsafe { FB.as_ref().map(|fb| (fb.width, fb.height)) };
    let (width, height) = match dims {
        Some(d) => d,
        None => {
            serial_println!("[fb] sin framebuffer disponible, no se dibuja el patrón de prueba");
            return;
        }
    };

    const COLORS: [u32; 8] = [
        0x00FFFFFF, // blanco
        0x00FFFF00, // amarillo
        0x0000FFFF, // cian
        0x0000FF00, // verde
        0x00FF00FF, // magenta
        0x00FF0000, // rojo
        0x000000FF, // azul
        0x00000000, // negro
    ];

    let stripe_w = width / COLORS.len() as u32;
    for (i, &color) in COLORS.iter().enumerate() {
        fill_rect(i as u32 * stripe_w, 0, stripe_w, height, color);
    }

    serial_println!("[fb] patrón de prueba dibujado (8 barras de color)");
}

/// Dibuja un carácter en (x,y) usando la fuente 8x8 de `font.rs`,
/// escalado `scale` veces (cada "píxel" de fuente se convierte en un
/// bloque de `scale`x`scale` píxeles reales — así se ve a tamaño legible
/// en una pantalla de 1024x768 sin necesitar una fuente más grande).
pub fn draw_char(x: u32, y: u32, c: char, color: u32, scale: u32) {
    let bitmap = crate::font::glyph(c);
    for (row, bits) in bitmap.iter().enumerate() {
        for col in 0..8u32 {
            if bits & (0x80 >> col) != 0 {
                fill_rect(x + col * scale, y + row as u32 * scale, scale, scale, color);
            }
        }
    }
}

/// Cadena completa, avanzando el cursor un ancho de glifo + un hueco
/// pequeño entre letras.
pub fn draw_str(x: u32, y: u32, s: &str, color: u32, scale: u32) {
    let mut cursor_x = x;
    for c in s.chars() {
        draw_char(cursor_x, y, c, color, scale);
        cursor_x += crate::font::GLYPH_WIDTH * scale + scale;
    }
}
