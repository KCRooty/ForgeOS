//! Gestor de memoria virtual — M4c (BORRADOR SIN VERIFICAR).
//!
//! Permite mapear/desmapear páginas de 4 KiB arbitrarias en la
//! jerarquía de páginas actualmente activa (la que apunta CR3), creando
//! las tablas intermedias (P3/P2/P1) que hagan falta sobre la marcha.
//!
//! Necesario para: MMIO fuera del identity-map estático de 4 GiB que
//! monta boot.asm, y es la primitiva base para cuando cada proceso
//! tenga su propio espacio de direcciones (fork/exec real, más
//! adelante — ahí se llamará con un CR3 distinto por proceso en vez de
//! operar siempre sobre el activo).
//!
//! Limitación deliberada de esta pasada: si el camino hacia una
//! dirección virtual pasa por una huge page de 2 MiB ya existente (las
//! que monta boot.asm para el identity-map), esta versión falla
//! explícitamente en vez de partirla — partir una huge page en curso es
//! una operación más delicada (hay que reconstruir 512 entradas de 4
//! KiB equivalentes) que se deja para una pasada aparte.

use crate::pmm;

const PAGE_PRESENT: u64 = 1 << 0;
const PAGE_WRITABLE: u64 = 1 << 1;
const PAGE_HUGE: u64 = 1 << 7;
const PAGE_NO_EXECUTE: u64 = 1 << 63;
const ADDR_MASK: u64 = 0x000F_FFFF_FFFF_F000;

unsafe fn read_cr3() -> u64 {
    let val: u64;
    core::arch::asm!("mov {}, cr3", out(reg) val, options(nomem, nostack));
    val & ADDR_MASK
}

unsafe fn flush_tlb_entry(virt: u64) {
    core::arch::asm!("invlpg [{}]", in(reg) virt, options(nostack));
}

/// Índice dentro de una tabla para el nivel dado (3=P4, 2=P3, 1=P2, 0=P1).
fn table_index(virt: u64, level: u8) -> usize {
    ((virt >> (12 + 9 * level as u64)) & 0x1FF) as usize
}

/// Lee la entrada `table_phys[index]`; si no está presente, crea una
/// tabla nueva (frame limpio) y la enlaza. Falla si la entrada ya
/// existe pero es una huge page (no podemos tratarla como tabla).
unsafe fn get_or_create_next(table_phys: u64, index: usize, flags: u64) -> Result<u64, &'static str> {
    let entry_ptr = (table_phys + (index as u64) * 8) as *mut u64;
    let entry = *entry_ptr;

    if entry & PAGE_PRESENT != 0 {
        if entry & PAGE_HUGE != 0 {
            return Err("el camino pasa por una huge page existente — partirla no está implementado en esta pasada");
        }
        return Ok(entry & ADDR_MASK);
    }

    let new_table = pmm::alloc_frame().ok_or("sin memoria física para tabla intermedia")?;
    core::ptr::write_bytes(new_table as *mut u8, 0, 4096);
    *entry_ptr = new_table | PAGE_PRESENT | flags;
    Ok(new_table)
}

/// Mapea una página de 4 KiB: `virt` -> `phys`. Ambas deben ir
/// alineadas a 4 KiB. Falla si ya había algo mapeado en esa dirección
/// virtual (evita pisar un mapeo existente por accidente).
pub unsafe fn map_page(virt: u64, phys: u64, writable: bool, executable: bool) -> Result<(), &'static str> {
    if virt % 4096 != 0 || phys % 4096 != 0 {
        return Err("virt/phys deben estar alineados a 4 KiB");
    }

    let p4 = read_cr3();
    let inter_flags = PAGE_PRESENT | PAGE_WRITABLE; // tablas intermedias siempre RW

    let p3 = get_or_create_next(p4, table_index(virt, 3), inter_flags)?;
    let p2 = get_or_create_next(p3, table_index(virt, 2), inter_flags)?;
    let p1 = get_or_create_next(p2, table_index(virt, 1), inter_flags)?;

    let mut leaf_flags = PAGE_PRESENT;
    if writable {
        leaf_flags |= PAGE_WRITABLE;
    }
    if !executable {
        leaf_flags |= PAGE_NO_EXECUTE;
    }

    let p1_entry_ptr = (p1 + (table_index(virt, 0) as u64) * 8) as *mut u64;
    if *p1_entry_ptr & PAGE_PRESENT != 0 {
        return Err("ya había una página mapeada en esa dirección virtual");
    }
    *p1_entry_ptr = (phys & ADDR_MASK) | leaf_flags;

    flush_tlb_entry(virt);
    Ok(())
}

/// Desmapea una página de 4 KiB previamente mapeada con `map_page`.
/// No libera el frame físico — eso es responsabilidad del llamante
/// (vía `pmm::free_frame`) si corresponde.
pub unsafe fn unmap_page(virt: u64) -> Result<(), &'static str> {
    if virt % 4096 != 0 {
        return Err("virt debe estar alineado a 4 KiB");
    }
    let p4 = read_cr3();

    let p3_entry = *((p4 + (table_index(virt, 3) as u64) * 8) as *const u64);
    if p3_entry & PAGE_PRESENT == 0 {
        return Err("no estaba mapeado (falta tabla P3)");
    }
    let p3 = p3_entry & ADDR_MASK;

    let p2_entry = *((p3 + (table_index(virt, 2) as u64) * 8) as *const u64);
    if p2_entry & PAGE_PRESENT == 0 {
        return Err("no estaba mapeado (falta tabla P2)");
    }
    if p2_entry & PAGE_HUGE != 0 {
        return Err("está dentro de una huge page — no se puede desmapear individualmente en esta pasada");
    }
    let p2 = p2_entry & ADDR_MASK;

    let p1_entry_ptr = (p2 + (table_index(virt, 1) as u64) * 8) as *mut u64;
    if *p1_entry_ptr & PAGE_PRESENT == 0 {
        return Err("no estaba mapeado (falta tabla P1)");
    }
    *p1_entry_ptr = 0;

    flush_tlb_entry(virt);
    Ok(())
}
