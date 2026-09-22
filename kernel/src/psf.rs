//! Parser de fuentes PSF1 (PC Screen Font) — BORRADOR SIN VERIFICAR,
//! pero formato extremadamente simple y estable, bajo riesgo.
//!
//! Reemplaza el alfabeto parcial hecho a mano de `font.rs`
//! (F/O/R/G/E/S/B/T/K) por una fuente real y completa, sin tener que
//! reconstruir de memoria una tabla ASCII entera glifo a glifo.
//!
//! Formato PSF1: magic de 2 bytes (0x36 0x04) + modo (1 byte) + altura
//! en píxeles (1 byte), luego N glifos de 8 píxeles de ancho ×
//! `charsize` de alto, un bit por píxel, un byte por fila. N es 256 o
//! 512 según el bit 0 del modo.
//!
//! CÓMO OBTENER UNA FUENTE REAL: cualquier sistema Linux (tu CachyOS
//! incluido) trae fuentes PSF1 de fábrica:
//! ```bash
//! ls /usr/share/kbd/consolefonts/*.psfu.gz | head
//! gunzip -k /usr/share/kbd/consolefonts/Lat2-Terminus16.psfu.gz
//! cp /usr/share/kbd/consolefonts/Lat2-Terminus16.psfu kernel/assets/font.psf
//! ```
//! Con el fichero en su sitio, `include_bytes!("../assets/font.psf")` lo
//! empotra en el binario al compilar — sin dependencias externas en
//! tiempo de ejecución, sigue siendo "cero deps".
//!
//! PSF2 (con tabla unicode y tamaño variable) queda como ampliación
//! futura — PSF1 cubre de sobra el uso de consola/depuración.

const PSF1_MAGIC0: u8 = 0x36;
const PSF1_MAGIC1: u8 = 0x04;
const PSF1_MODE512: u8 = 0x01;

pub struct PsfFont<'a> {
    data: &'a [u8],
    pub glyph_height: usize,
    pub num_glyphs: usize,
}

impl<'a> PsfFont<'a> {
    /// Parsea una fuente PSF1 desde bytes crudos. `None` si el magic no
    /// coincide o el fichero está truncado.
    pub fn parse(data: &'a [u8]) -> Option<PsfFont<'a>> {
        if data.len() < 4 || data[0] != PSF1_MAGIC0 || data[1] != PSF1_MAGIC1 {
            return None;
        }
        let mode = data[2];
        let charsize = data[3] as usize;
        let num_glyphs = if mode & PSF1_MODE512 != 0 { 512 } else { 256 };

        let expected_len = 4 + num_glyphs * charsize;
        if data.len() < expected_len {
            return None; // fichero truncado
        }

        Some(PsfFont {
            data,
            glyph_height: charsize,
            num_glyphs,
        })
    }

    /// Filas crudas del glifo para el byte `c` — PSF1 sin tabla unicode
    /// asume orden ASCII/CP437 directo para los primeros 128, que es
    /// exactamente lo que necesitamos para consola.
    pub fn glyph(&self, c: u8) -> Option<&[u8]> {
        let index = c as usize;
        if index >= self.num_glyphs {
            return None;
        }
        let start = 4 + index * self.glyph_height;
        let end = start + self.glyph_height;
        self.data.get(start..end)
    }
}
