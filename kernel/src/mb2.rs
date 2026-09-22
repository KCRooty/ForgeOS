//! Parser mínimo de tags Multiboot2 — M2 (BORRADOR SIN VERIFICAR).
//!
//! Solo extraemos el tag de mapa de memoria (type=6) — es lo único que
//! necesita el allocador físico. El tag de framebuffer se añade en M3.

#[repr(C)]
struct Mb2Header {
    total_size: u32,
    reserved: u32,
}

#[repr(C)]
struct TagHeader {
    tag_type: u32,
    size: u32,
}

#[repr(C)]
struct MemMapEntry {
    base_addr: u64,
    length: u64,
    entry_type: u32,
    reserved: u32,
}

const TAG_MEMORY_MAP: u32 = 6;
const TAG_END: u32 = 0;

pub const MEM_AVAILABLE: u32 = 1;

/// Recorre el mapa de memoria Multiboot2 y llama a `f(base, length, tipo)`
/// por cada región. `tipo == MEM_AVAILABLE` significa RAM libre de verdad;
/// cualquier otro valor es reservado/ACPI/defectuoso — el llamante decide
/// qué hacer con eso.
///
/// # Safety
/// `mb2_ptr` debe ser el puntero que GRUB entrega en RDI al arrancar
/// (sin modificar), y la memoria que apunta debe seguir siendo válida.
pub unsafe fn for_each_memory_region(mb2_ptr: u64, mut f: impl FnMut(u64, u64, u32)) {
    let header = &*(mb2_ptr as *const Mb2Header);
    let end = mb2_ptr + header.total_size as u64;
    let mut addr = mb2_ptr + 8; // tras el header fijo

    while addr < end {
        let tag = &*(addr as *const TagHeader);
        if tag.tag_type == TAG_END {
            break;
        }

        if tag.tag_type == TAG_MEMORY_MAP {
            let entry_size = *((addr + 8) as *const u32) as u64;
            let entries_start = addr + 16; // header(8) + entry_size(4) + entry_version(4)
            let entries_end = addr + tag.size as u64;
            let mut e = entries_start;
            while e < entries_end {
                let entry = &*(e as *const MemMapEntry);
                f(entry.base_addr, entry.length, entry.entry_type);
                e += entry_size;
            }
        }

        // los tags Multiboot2 van alineados a 8 bytes
        addr += (tag.size as u64 + 7) & !7;
    }
}
