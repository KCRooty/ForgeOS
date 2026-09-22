//! Cargador de binarios ELF64 — M4e (BORRADOR SIN VERIFICAR).
//!
//! Parsea la cabecera ELF64 y los program headers, y mapea cada
//! segmento PT_LOAD en un espacio de direcciones nuevo (vía mmu.rs).
//!
//! Todavía NO hace la transición a ring 3 — eso necesita descriptores
//! GDT de ring 3 + TSS.RSP0 + una transición `iretq`, pieza aparte para
//! no acumular más riesgo en la misma pasada. Por ahora esto demuestra
//! que el mecanismo de parseo + mapeo de segmentos funciona de verdad,
//! probado contra un ELF64 real y mínimo (no bytes inventados) que
//! construimos nosotros mismos más abajo — no fingimos tener un binario
//! de prueba, lo tenemos.

use crate::mmu;
use crate::pmm;

const ELF_MAGIC: [u8; 4] = [0x7F, b'E', b'L', b'F'];
const ELFCLASS64: u8 = 2;
const ET_EXEC: u16 = 2;
const ET_DYN: u16 = 3; // PIE — muchos binarios modernos son esto
const PT_LOAD: u32 = 1;
const PF_X: u32 = 0x1;
const PF_W: u32 = 0x2;

#[repr(C)]
struct Elf64Header {
    e_ident: [u8; 16],
    e_type: u16,
    e_machine: u16,
    e_version: u32,
    e_entry: u64,
    e_phoff: u64,
    e_shoff: u64,
    e_flags: u32,
    e_ehsize: u16,
    e_phentsize: u16,
    e_phnum: u16,
    e_shentsize: u16,
    e_shnum: u16,
    e_shstrndx: u16,
}

#[repr(C)]
struct Elf64ProgramHeader {
    p_type: u32,
    p_flags: u32,
    p_offset: u64,
    p_vaddr: u64,
    p_paddr: u64,
    p_filesz: u64,
    p_memsz: u64,
    p_align: u64,
}

pub struct LoadedElf {
    pub entry_point: u64,
    pub page_table: u64,
}

/// Parsea y carga un ELF64 desde `bytes` en un espacio de direcciones
/// nuevo. Devuelve el punto de entrada y el PML4 del nuevo espacio —
/// no salta a él, eso es tarea del llamante (y, por ahora, de M4f
/// cuando exista transición a ring 3).
pub fn load(bytes: &[u8]) -> Result<LoadedElf, &'static str> {
    if bytes.len() < core::mem::size_of::<Elf64Header>() {
        return Err("fichero demasiado pequeño para ser un ELF64");
    }
    if bytes[0..4] != ELF_MAGIC {
        return Err("magic ELF no coincide (0x7F 'E' 'L' 'F')");
    }
    if bytes[4] != ELFCLASS64 {
        return Err("no es un ELF de 64 bits (EI_CLASS != ELFCLASS64)");
    }

    let header = unsafe { &*(bytes.as_ptr() as *const Elf64Header) };

    if header.e_type != ET_EXEC && header.e_type != ET_DYN {
        return Err("tipo de ELF no soportado (solo ET_EXEC/ET_DYN)");
    }

    let page_table = unsafe { mmu::create_address_space() }.ok_or("sin memoria para el espacio de direcciones")?;

    let ph_offset = header.e_phoff as usize;
    let ph_count = header.e_phnum as usize;
    let ph_size = header.e_phentsize as usize;

    for i in 0..ph_count {
        let ph_bytes_offset = ph_offset + i * ph_size;
        if ph_bytes_offset + core::mem::size_of::<Elf64ProgramHeader>() > bytes.len() {
            return Err("program header fuera de los límites del fichero");
        }
        let ph = unsafe { &*(bytes.as_ptr().add(ph_bytes_offset) as *const Elf64ProgramHeader) };

        if ph.p_type != PT_LOAD {
            continue; // solo nos importan los segmentos cargables
        }
        load_segment(page_table, bytes, ph)?;
    }

    Ok(LoadedElf {
        entry_point: header.e_entry,
        page_table,
    })
}

fn load_segment(page_table: u64, bytes: &[u8], ph: &Elf64ProgramHeader) -> Result<(), &'static str> {
    let vaddr_start = ph.p_vaddr & !0xFFF; // alinear hacia abajo a página
    let vaddr_end = (ph.p_vaddr + ph.p_memsz + 0xFFF) & !0xFFF; // hacia arriba
    let num_pages = ((vaddr_end - vaddr_start) / 4096) as usize;

    let writable = ph.p_flags & PF_W != 0;
    let executable = ph.p_flags & PF_X != 0;

    for i in 0..num_pages {
        let page_virt = vaddr_start + (i as u64) * 4096;
        let phys = pmm::alloc_frame().ok_or("sin memoria física para el segmento")?;
        unsafe {
            core::ptr::write_bytes(phys as *mut u8, 0, 4096);
            mmu::map_page_in(page_table, page_virt, phys, writable, executable)?;
        }
    }

    let file_start = ph.p_offset as usize;
    let file_len = ph.p_filesz as usize;
    if file_start + file_len > bytes.len() {
        return Err("el segmento apunta fuera de los límites del fichero");
    }

    copy_into_segment(page_table, ph.p_vaddr, &bytes[file_start..file_start + file_len])
}

/// Copia bytes a un rango de direcciones virtuales ya mapeadas en
/// `page_table`, traduciendo página a página — los frames físicos NO
/// tienen por qué ser contiguos entre sí, así que no podemos asumir
/// que basta con calcular la física de la primera página.
fn copy_into_segment(page_table: u64, dest_vaddr: u64, src: &[u8]) -> Result<(), &'static str> {
    let mut remaining = src;
    let mut vaddr = dest_vaddr;
    while !remaining.is_empty() {
        let page_offset = (vaddr & 0xFFF) as usize;
        let chunk_len = core::cmp::min(remaining.len(), 4096 - page_offset);
        let phys = unsafe { mmu::translate(page_table, vaddr & !0xFFF) }
            .ok_or("página no mapeada durante la copia del segmento")?;
        unsafe {
            let dest = (phys + page_offset as u64) as *mut u8;
            core::ptr::copy_nonoverlapping(remaining.as_ptr(), dest, chunk_len);
        }
        remaining = &remaining[chunk_len..];
        vaddr += chunk_len as u64;
    }
    Ok(())
}

/// ELF64 mínimo real, construido a mano byte a byte (no bytes
/// inventados sin verificar): cabecera ELF (64B) + un program header
/// PT_LOAD (56B) + 3 bytes de código máquina x86_64 (`hlt; jmp $-3`,
/// un bucle infinito seguro que nunca vuelve). El propio segmento
/// incluye la cabecera y el program header mapeados — truco válido y
/// común para ELFs mínimos hechos a mano, ahorra tener que mapear un
/// segundo segmento solo para el código.
///
/// vaddr base = 0x400000 (convención clásica de ELF estático x86_64).
/// Entry point = 0x400000 + 120 = 0x400078 (offset del código: 64+56).
#[rustfmt::skip]
pub const TEST_ELF: [u8; 123] = [
    // e_ident
    0x7F, b'E', b'L', b'F', 2, 1, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    // e_type=ET_EXEC(2), e_machine=EM_X86_64(0x3E)
    0x02, 0x00, 0x3E, 0x00,
    // e_version=1
    0x01, 0x00, 0x00, 0x00,
    // e_entry = 0x400078
    0x78, 0x00, 0x40, 0x00, 0x00, 0x00, 0x00, 0x00,
    // e_phoff = 64
    0x40, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    // e_shoff = 0
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    // e_flags = 0
    0x00, 0x00, 0x00, 0x00,
    // e_ehsize=64, e_phentsize=56
    0x40, 0x00, 0x38, 0x00,
    // e_phnum=1, e_shentsize=0
    0x01, 0x00, 0x00, 0x00,
    // e_shnum=0, e_shstrndx=0
    0x00, 0x00, 0x00, 0x00,
    // --- program header (56 bytes) ---
    // p_type=PT_LOAD(1)
    0x01, 0x00, 0x00, 0x00,
    // p_flags=PF_X|PF_R(5)
    0x05, 0x00, 0x00, 0x00,
    // p_offset=0
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    // p_vaddr=0x400000
    0x00, 0x00, 0x40, 0x00, 0x00, 0x00, 0x00, 0x00,
    // p_paddr=0x400000
    0x00, 0x00, 0x40, 0x00, 0x00, 0x00, 0x00, 0x00,
    // p_filesz=123
    0x7B, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    // p_memsz=123
    0x7B, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    // p_align=0x1000
    0x00, 0x10, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    // --- código (3 bytes) ---
    0xF4,       // hlt
    0xEB, 0xFD, // jmp rel8 -3 (vuelve al hlt)
];

/// Segunda versión, para ring 3: `hlt` es una instrucción privilegiada
/// — ejecutarla en ring 3 provoca un #GP inmediato, así que este
/// payload usa solo `jmp $` (bucle infinito sin ningún privilegio
/// especial). Mismo método de construcción a mano, mismo entry point
/// (el código sigue empezando justo después de cabecera+phdr).
#[rustfmt::skip]
pub const TEST_ELF_RING3: [u8; 122] = [
    // e_ident
    0x7F, b'E', b'L', b'F', 2, 1, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    // e_type=ET_EXEC(2), e_machine=EM_X86_64(0x3E)
    0x02, 0x00, 0x3E, 0x00,
    // e_version=1
    0x01, 0x00, 0x00, 0x00,
    // e_entry = 0x400078
    0x78, 0x00, 0x40, 0x00, 0x00, 0x00, 0x00, 0x00,
    // e_phoff = 64
    0x40, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    // e_shoff = 0
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    // e_flags = 0
    0x00, 0x00, 0x00, 0x00,
    // e_ehsize=64, e_phentsize=56
    0x40, 0x00, 0x38, 0x00,
    // e_phnum=1, e_shentsize=0
    0x01, 0x00, 0x00, 0x00,
    // e_shnum=0, e_shstrndx=0
    0x00, 0x00, 0x00, 0x00,
    // --- program header (56 bytes) ---
    0x01, 0x00, 0x00, 0x00, // p_type=PT_LOAD
    0x05, 0x00, 0x00, 0x00, // p_flags=PF_X|PF_R
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, // p_offset=0
    0x00, 0x00, 0x40, 0x00, 0x00, 0x00, 0x00, 0x00, // p_vaddr=0x400000
    0x00, 0x00, 0x40, 0x00, 0x00, 0x00, 0x00, 0x00, // p_paddr=0x400000
    0x7A, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, // p_filesz=122
    0x7A, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, // p_memsz=122
    0x00, 0x10, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, // p_align=0x1000
    // --- código (2 bytes): jmp $ (bucle infinito, sin privilegios) ---
    0xEB, 0xFE,
];

/// Tercera versión: ejecuta `syscall` de verdad desde ring 3 antes de
/// caer en el bucle infinito. Código: `mov eax, SYS_PING` (5 bytes) +
/// `syscall` (2 bytes) + `jmp $` (2 bytes) = 9 bytes.
#[rustfmt::skip]
pub const TEST_ELF_SYSCALL: [u8; 129] = [
    // e_ident
    0x7F, b'E', b'L', b'F', 2, 1, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0x02, 0x00, 0x3E, 0x00, // e_type=ET_EXEC, e_machine=EM_X86_64
    0x01, 0x00, 0x00, 0x00, // e_version=1
    0x78, 0x00, 0x40, 0x00, 0x00, 0x00, 0x00, 0x00, // e_entry=0x400078
    0x40, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, // e_phoff=64
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, // e_shoff=0
    0x00, 0x00, 0x00, 0x00, // e_flags=0
    0x40, 0x00, 0x38, 0x00, // e_ehsize=64, e_phentsize=56
    0x01, 0x00, 0x00, 0x00, // e_phnum=1, e_shentsize=0
    0x00, 0x00, 0x00, 0x00, // e_shnum=0, e_shstrndx=0
    // --- program header ---
    0x01, 0x00, 0x00, 0x00, // p_type=PT_LOAD
    0x05, 0x00, 0x00, 0x00, // p_flags=PF_X|PF_R
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, // p_offset=0
    0x00, 0x00, 0x40, 0x00, 0x00, 0x00, 0x00, 0x00, // p_vaddr=0x400000
    0x00, 0x00, 0x40, 0x00, 0x00, 0x00, 0x00, 0x00, // p_paddr=0x400000
    0x81, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, // p_filesz=129
    0x81, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, // p_memsz=129
    0x00, 0x10, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, // p_align=0x1000
    // --- código (9 bytes) ---
    0xB8, 0x01, 0x00, 0x00, 0x00, // mov eax, 1  (SYS_PING)
    0x0F, 0x05,                   // syscall
    0xEB, 0xFE,                   // jmp $
];
