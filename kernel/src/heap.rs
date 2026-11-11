//! Heap del kernel — allocator de lista libre con fusión de bloques (M2b).
//!
//! Sustituye al bump allocator de M2 (1 MiB, `dealloc` no-op): con él,
//! cada `Vec`/`String`/`Box` soltado era memoria perdida para siempre, y
//! cargar un binario real desde disco (cientos de KiB, leídos bloque a
//! bloque en `Vec`s temporales) agotaba el heap entero en un solo comando.
//!
//! Diseño — el clásico "first fit" con lista libre ordenada por dirección:
//!
//! - La lista libre vive DENTRO de la propia memoria libre: cada hueco
//!   empieza con un `FreeBlock { size, next }` (16 bytes). Un bloque
//!   ocupado no lleva cabecera ninguna — `GlobalAlloc::dealloc` recibe el
//!   mismo `Layout` que se pidió, así que el tamaño no hace falta guardarlo.
//! - Todo tamaño se redondea a múltiplos de 16 y toda alineación sube a
//!   16 como mínimo. Consecuencia: cualquier resto que deja un corte
//!   (por delante, al alinear; o por detrás, lo que sobra) es 0 o ≥ 16
//!   bytes — siempre cabe un `FreeBlock`, nunca quedan flecos imposibles
//!   de enlazar.
//! - `dealloc` reinserta en orden de dirección y fusiona con el vecino de
//!   delante y con el de detrás si son contiguos. Sin esto la memoria se
//!   recuperaría, pero troceada en huecos cada vez más pequeños.
//!
//! Concurrencia: una sola CPU, pero con preemption real (`preempt.rs`) —
//! el timer puede quitarle el turno a una tarea de kernel en mitad de un
//! `alloc` y dárselo a otra que también pida memoria. Toda operación
//! sobre la lista va con interrupciones deshabilitadas (`IrqOff`,
//! restaura IF a lo que hubiera antes). El día que haya SMP esto pasa a
//! ser un spinlock de verdad — ver "Primitivas de sincronización" en
//! `docs/TODO.md`.
//!
//! `#[alloc_error_handler]` es una feature inestable cuyo nombre/firma ha
//! cambiado entre nightlies a lo largo de los años — si esto no compila
//! tal cual, es el primer sitio a revisar junto con la ABI de idt.rs.

use core::alloc::{GlobalAlloc, Layout};
use core::cell::UnsafeCell;
use core::ptr;

/// 8 MiB fijos en `.bss` (no ocupan sitio en la imagen del kernel en
/// disco). `pmm::init` ya reserva todo hasta `_kernel_end`, así que esta
/// región nunca se entrega como frame libre.
const HEAP_SIZE: usize = 8 * 1024 * 1024;

/// Granularidad: tamaño de `FreeBlock` y alineación mínima de todo bloque.
const UNIT: usize = 16;

#[repr(align(16))]
struct HeapRegion(UnsafeCell<[u8; HEAP_SIZE]>);
unsafe impl Sync for HeapRegion {}

static HEAP: HeapRegion = HeapRegion(UnsafeCell::new([0; HEAP_SIZE]));

/// Cabecera de un hueco libre, escrita en sus primeros 16 bytes.
#[repr(C)]
struct FreeBlock {
    size: usize,
    next: *mut FreeBlock,
}

struct State {
    /// Primer hueco libre (dirección más baja), o null si no queda ninguno.
    head: *mut FreeBlock,
    /// Bytes entregados ahora mismo (ya redondeados a `UNIT`).
    used: usize,
    /// Máximo histórico de `used` — para saber cuánto margen real hay.
    peak: usize,
    initialized: bool,
}

pub struct FreeListAllocator {
    state: UnsafeCell<State>,
}

unsafe impl Sync for FreeListAllocator {}

/// Deshabilita interrupciones mientras viva y restaura IF a su valor
/// anterior al soltarse (no siempre a 1: se puede pedir memoria desde
/// código que ya corría con interrupciones enmascaradas, como una syscall).
struct IrqOff {
    was_enabled: bool,
}

impl IrqOff {
    fn new() -> Self {
        let rflags: u64;
        unsafe {
            core::arch::asm!("pushfq", "pop {}", "cli", out(reg) rflags);
        }
        IrqOff {
            was_enabled: rflags & 0x200 != 0,
        }
    }
}

impl Drop for IrqOff {
    fn drop(&mut self) {
        if self.was_enabled {
            unsafe { core::arch::asm!("sti", options(nomem, nostack)) };
        }
    }
}

const fn round_up(value: usize, to: usize) -> usize {
    (value + to - 1) & !(to - 1)
}

/// Tamaño y alineación reales con los que se atiende un `Layout`.
fn effective(layout: Layout) -> (usize, usize) {
    let size = round_up(layout.size().max(1), UNIT);
    let align = layout.align().max(UNIT);
    (size, align)
}

impl FreeListAllocator {
    const fn new() -> Self {
        FreeListAllocator {
            state: UnsafeCell::new(State {
                head: ptr::null_mut(),
                used: 0,
                peak: 0,
                initialized: false,
            }),
        }
    }

    /// Primera llamada: todo el heap es un único hueco libre.
    unsafe fn ensure_init(state: &mut State) {
        if !state.initialized {
            let first = HEAP.0.get() as *mut FreeBlock;
            (*first).size = HEAP_SIZE;
            (*first).next = ptr::null_mut();
            state.head = first;
            state.initialized = true;
        }
    }
}

unsafe impl GlobalAlloc for FreeListAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let (size, align) = effective(layout);
        let _irq = IrqOff::new();
        let state = &mut *self.state.get();
        Self::ensure_init(state);

        // `link` apunta siempre al puntero que enlaza con `block` (la
        // cabeza de la lista o el `next` del hueco anterior) — así
        // desenlazar o sustituir un hueco es una sola escritura, sin caso
        // especial para el primero.
        let mut link: *mut *mut FreeBlock = &mut state.head;
        while !(*link).is_null() {
            let block = *link;
            let start = block as usize;
            let block_size = (*block).size;
            let aligned = round_up(start, align);
            let front = aligned - start;

            if front <= block_size && block_size - front >= size {
                let back = block_size - front - size;
                let mut after = (*block).next;

                if back > 0 {
                    let rest = (aligned + size) as *mut FreeBlock;
                    (*rest).size = back;
                    (*rest).next = after;
                    after = rest;
                }
                if front > 0 {
                    // El trozo de delante sigue siendo el mismo hueco,
                    // solo que más corto.
                    (*block).size = front;
                    (*block).next = after;
                } else {
                    *link = after;
                }

                state.used += size;
                if state.used > state.peak {
                    state.peak = state.used;
                }
                return aligned as *mut u8;
            }
            link = &mut (*block).next;
        }
        ptr::null_mut() // sin hueco suficiente — `alloc_error` decide
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        let (size, _) = effective(layout);
        let _irq = IrqOff::new();
        let state = &mut *self.state.get();

        // Busca el punto de inserción: `prev` < ptr < `next`.
        let mut prev: *mut FreeBlock = ptr::null_mut();
        let mut next = state.head;
        while !next.is_null() && (next as usize) < ptr as usize {
            prev = next;
            next = (*next).next;
        }

        let block = ptr as *mut FreeBlock;
        (*block).size = size;
        (*block).next = next;

        // Fusión hacia delante: el hueco siguiente empieza justo donde
        // acaba este.
        if !next.is_null() && ptr as usize + size == next as usize {
            (*block).size += (*next).size;
            (*block).next = (*next).next;
        }

        if prev.is_null() {
            state.head = block;
        } else if prev as usize + (*prev).size == ptr as usize {
            // Fusión hacia atrás: el hueco anterior acaba justo aquí.
            (*prev).size += (*block).size;
            (*prev).next = (*block).next;
        } else {
            (*prev).next = block;
        }

        state.used -= size;
    }
}

#[global_allocator]
static ALLOCATOR: FreeListAllocator = FreeListAllocator::new();

/// Foto del estado del heap, para `meminfo` y para las pruebas.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Stats {
    pub total: usize,
    pub used: usize,
    pub peak: usize,
    /// Número de huecos libres — 1 significa "sin fragmentación".
    pub free_blocks: usize,
    /// El hueco libre más grande: la mayor petición que cabría ahora.
    pub largest_free: usize,
}

pub fn stats() -> Stats {
    let _irq = IrqOff::new();
    unsafe {
        let state = &mut *ALLOCATOR.state.get();
        FreeListAllocator::ensure_init(state);
        let mut free_blocks = 0;
        let mut largest_free = 0;
        let mut block = state.head;
        while !block.is_null() {
            free_blocks += 1;
            largest_free = largest_free.max((*block).size);
            block = (*block).next;
        }
        Stats {
            total: HEAP_SIZE,
            used: state.used,
            peak: state.peak,
            free_blocks,
            largest_free,
        }
    }
}

/// Prueba de arranque: reserva, libera en desorden y comprueba que el
/// heap vuelve EXACTAMENTE al estado de antes (mismos bytes usados,
/// mismo número de huecos, mismo hueco máximo). Si la fusión de bloques
/// fallara en cualquiera de sus dos sentidos, quedarían huecos de más.
pub fn self_test() -> Result<(), &'static str> {
    use alloc::vec::Vec;

    let before = stats();

    {
        // Tamaños y alineaciones variados, liberados en un orden distinto
        // al de reserva (pares primero, luego impares) para forzar
        // fusiones hacia delante, hacia atrás y por los dos lados a la vez.
        let layouts = [
            Layout::from_size_align(24, 8).unwrap(),
            Layout::from_size_align(4096, 4096).unwrap(),
            Layout::from_size_align(1, 1).unwrap(),
            Layout::from_size_align(100_000, 16).unwrap(),
            Layout::from_size_align(512, 64).unwrap(),
            Layout::from_size_align(33, 2).unwrap(),
            Layout::from_size_align(8192, 4096).unwrap(),
            Layout::from_size_align(7, 1).unwrap(),
        ];
        let mut ptrs = [ptr::null_mut::<u8>(); 8];
        for (i, layout) in layouts.iter().enumerate() {
            let p = unsafe { ALLOCATOR.alloc(*layout) };
            if p.is_null() {
                return Err("alloc devolvió null con el heap casi vacío");
            }
            if p as usize % layout.align() != 0 {
                return Err("alloc devolvió un puntero mal alineado");
            }
            // Patrón propio por bloque: si dos bloques se solaparan, el
            // segundo pisaría el patrón del primero.
            unsafe { ptr::write_bytes(p, 0xA0 + i as u8, layout.size()) };
            ptrs[i] = p;
        }
        for (i, layout) in layouts.iter().enumerate() {
            let bytes = unsafe { core::slice::from_raw_parts(ptrs[i], layout.size()) };
            if bytes.iter().any(|&b| b != 0xA0 + i as u8) {
                return Err("dos bloques reservados se solapan");
            }
        }
        for i in (0..8).step_by(2).chain((1..8).step_by(2)) {
            unsafe { ALLOCATOR.dealloc(ptrs[i], layouts[i]) };
        }
        if stats() != (Stats { peak: stats().peak, ..before }) {
            return Err("el heap no vuelve a su estado inicial tras liberar (fusión de bloques rota)");
        }
    }

    {
        // Reutilización real: lo que el bump allocator no podía hacer.
        // 64 rondas de 1 MiB son 64 MiB en total sobre un heap de 8.
        for round in 0..64u8 {
            let mut v: Vec<u8> = Vec::with_capacity(1024 * 1024);
            v.resize(1024 * 1024, round);
            if v[0] != round || v[1024 * 1024 - 1] != round {
                return Err("memoria reutilizada no conserva lo escrito");
            }
        }
        if stats() != (Stats { peak: stats().peak, ..before }) {
            return Err("el heap no vuelve a su estado inicial tras 64 rondas de 1 MiB");
        }
    }

    Ok(())
}

#[alloc_error_handler]
fn alloc_error(layout: Layout) -> ! {
    // Sin `stats()` aquí a propósito: formatear ya es lo único que se
    // puede hacer sin heap, y recorrer la lista no añade nada si lo que
    // pasó es que está corrupta.
    crate::serial_println!(
        "[FATAL] Heap agotado pidiendo {} bytes (align {})",
        layout.size(),
        layout.align()
    );
    loop {
        unsafe { core::arch::asm!("hlt") };
    }
}
