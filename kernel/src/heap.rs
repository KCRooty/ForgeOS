//! Heap del kernel — bump allocator sobre una región fija — M2 (BORRADOR
//! SIN VERIFICAR).
//!
//! Deliberadamente simple para v1: `alloc` avanza un puntero, `dealloc`
//! es un no-op. Sin multitasking real todavía y sin drivers que reciclen
//! memoria en bucles largos, esto es aceptable como primer paso. Un
//! allocator con free-list que sí recicla memoria (M2b) es justo el tipo
//! de código —punteros enlazados, splitting/merging de bloques— que más
//! quiero escribir CON compilador y depurador delante, no a ciegas.
//!
//! `#[alloc_error_handler]` es una feature inestable cuyo nombre/firma ha
//! cambiado entre nightlies a lo largo de los años — si esto no compila
//! tal cual, es el primer sitio a revisar junto con la ABI de idt.rs.

#![allow(dead_code)]

use core::alloc::{GlobalAlloc, Layout};
use core::cell::UnsafeCell;
use core::ptr;

const HEAP_SIZE: usize = 1024 * 1024; // 1 MiB fijo para empezar

#[repr(align(16))]
struct HeapRegion(UnsafeCell<[u8; HEAP_SIZE]>);
unsafe impl Sync for HeapRegion {}

static HEAP: HeapRegion = HeapRegion(UnsafeCell::new([0; HEAP_SIZE]));

pub struct BumpAllocator {
    next: UnsafeCell<usize>, // offset desde el inicio de HEAP
}

unsafe impl Sync for BumpAllocator {}

impl BumpAllocator {
    const fn new() -> Self {
        BumpAllocator {
            next: UnsafeCell::new(0),
        }
    }
}

unsafe impl GlobalAlloc for BumpAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let heap_start = HEAP.0.get() as usize;
        let next_ptr = self.next.get();
        let current = heap_start + *next_ptr;

        let aligned = (current + layout.align() - 1) & !(layout.align() - 1);
        let new_offset = (aligned - heap_start) + layout.size();

        if new_offset > HEAP_SIZE {
            return ptr::null_mut(); // heap agotado, sin pánico aquí
        }

        *next_ptr = new_offset;
        aligned as *mut u8
    }

    unsafe fn dealloc(&self, _ptr: *mut u8, _layout: Layout) {
        // no-op a propósito — v1, ver comentario de cabecera.
    }
}

#[global_allocator]
static ALLOCATOR: BumpAllocator = BumpAllocator::new();

pub fn used_bytes() -> usize {
    unsafe { *ALLOCATOR.next.get() }
}

#[alloc_error_handler]
fn alloc_error(layout: Layout) -> ! {
    crate::serial_println!(
        "[FATAL] Heap agotado pidiendo {} bytes (align {})",
        layout.size(),
        layout.align()
    );
    loop {
        unsafe { core::arch::asm!("hlt") };
    }
}
