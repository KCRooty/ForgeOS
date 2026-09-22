//! Allocador físico de páginas — bitmap — M2 (BORRADOR SIN VERIFICAR).
//!
//! Un bit por frame de 4 KiB: 0 = libre, 1 = ocupado. Se alimenta del
//! mapa de memoria Multiboot2 (mb2.rs) y luego vuelve a marcar como
//! ocupado el primer MiB (BIOS/MMIO/estructuras de boot) y la imagen
//! entera del kernel, aunque el mapa los liste como "disponibles" — no
//! podemos entregar como libre la memoria donde vive nuestro propio
//! código.

use crate::mb2;

const FRAME_SIZE: u64 = 4096;
const MAX_FRAMES: usize = 1024 * 1024; // cubre hasta 4 GiB direccionables
const BITMAP_WORDS: usize = MAX_FRAMES / 64;

// Todo ocupado por defecto — init() libera explícitamente lo que el mapa
// de memoria confirma como RAM disponible. Fallar "cerrado" es más seguro
// que fallar "abierto" si el mapa viene incompleto.
static mut BITMAP: [u64; BITMAP_WORDS] = [u64::MAX; BITMAP_WORDS];

extern "C" {
    /// Definido en targets/linker.ld — primera dirección libre tras la
    /// imagen del kernel (kernel "flat": esta dirección ya es física).
    static _kernel_end: u8;
}

fn set_free(frame: usize) {
    unsafe { BITMAP[frame / 64] &= !(1u64 << (frame % 64)) };
}

fn set_used(frame: usize) {
    unsafe { BITMAP[frame / 64] |= 1u64 << (frame % 64) };
}

fn is_used(frame: usize) -> bool {
    unsafe { BITMAP[frame / 64] & (1u64 << (frame % 64)) != 0 }
}

/// # Safety
/// Debe llamarse una sola vez, al principio del boot, con el puntero
/// Multiboot2 original sin modificar.
pub unsafe fn init(mb2_ptr: u64) {
    // 1. Libera lo que el firmware confirma como RAM disponible.
    mb2::for_each_memory_region(mb2_ptr, |base, len, kind| {
        if kind == mb2::MEM_AVAILABLE {
            let start_frame = (base / FRAME_SIZE) as usize;
            let frame_count = (len / FRAME_SIZE) as usize;
            for f in start_frame..(start_frame + frame_count).min(MAX_FRAMES) {
                set_free(f);
            }
        }
    });

    // 2. Reserva de nuevo el primer MiB completo y toda la imagen del
    //    kernel — sin importar lo que diga el mapa de memoria.
    let kernel_end_phys = core::ptr::addr_of!(_kernel_end) as u64;
    let reserved_end_frame = (kernel_end_phys / FRAME_SIZE) as usize + 1;
    for f in 0..reserved_end_frame.min(MAX_FRAMES) {
        set_used(f);
    }
}

/// Busca y reserva el primer frame libre. `None` si no queda memoria —
/// M2 no tiene política de OOM más allá de fallar limpio; el llamante
/// decide qué hacer (panic, o esperar a que exista swap/reclaim en M4+).
pub fn alloc_frame() -> Option<u64> {
    for i in 0..MAX_FRAMES {
        if !is_used(i) {
            set_used(i);
            return Some(i as u64 * FRAME_SIZE);
        }
    }
    None
}

pub fn free_frame(addr: u64) {
    let frame = (addr / FRAME_SIZE) as usize;
    if frame < MAX_FRAMES {
        set_free(frame);
    }
}

/// Cuenta frames libres — O(n), solo pensado para un printout de boot,
/// no para uso en caliente.
pub fn free_frame_count() -> usize {
    (0..MAX_FRAMES).filter(|&i| !is_used(i)).count()
}

/// Busca `count` frames libres SEGUIDOS — algunos dispositivos (el
/// buffer de recepción del RTL8139, por ejemplo) exigen un rango físico
/// contiguo, no basta con "count frames sueltos" repartidos. Lineal,
/// O(n) — aceptable para las pocas veces que hace falta (anillos DMA de
/// red, no una operación de uso frecuente).
pub fn alloc_contiguous(count: usize) -> Option<u64> {
    if count == 0 || count > MAX_FRAMES {
        return None;
    }
    let mut run_start = 0usize;
    let mut run_len = 0usize;
    for i in 0..MAX_FRAMES {
        if !is_used(i) {
            if run_len == 0 {
                run_start = i;
            }
            run_len += 1;
            if run_len == count {
                for f in run_start..run_start + count {
                    set_used(f);
                }
                return Some(run_start as u64 * FRAME_SIZE);
            }
        } else {
            run_len = 0;
        }
    }
    None
}
