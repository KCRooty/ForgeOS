# Fuente PSF1 — pendiente de añadir

Este directorio está vacío a propósito — `psf.rs` es solo el parser,
todavía no hay ningún fichero de fuente empotrado (para no romper el
build con un `include_bytes!` apuntando a un fichero que no existe).

## Para activarlo

```bash
# en tu CachyOS
ls /usr/share/kbd/consolefonts/*.psfu.gz | head
gunzip -k /usr/share/kbd/consolefonts/Lat2-Terminus16.psfu.gz
cp /usr/share/kbd/consolefonts/Lat2-Terminus16.psfu kernel/assets/font.psf
```

Luego, en `main.rs`:
```rust
static FONT_DATA: &[u8] = include_bytes!("../assets/font.psf");
// ...
let font = psf::PsfFont::parse(FONT_DATA).expect("fuente PSF inválida");
framebuffer::draw_str_psf(&font, 20, 20, "FORGE OS BOOT OK", 0x00000000, 2);
```

Cuando tengas el fichero en su sitio, dímelo y lo enlazo — así no dejo
un `include_bytes!` que te rompa el primer `cargo build`.
