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
const PAGE_USER: u64 = 1 << 2;
const PAGE_HUGE: u64 = 1 << 7;
const PAGE_NO_EXECUTE: u64 = 1 << 63;
const ADDR_MASK: u64 = 0x000F_FFFF_FFFF_F000;

const IA32_EFER: u32 = 0xC000_0080;
const EFER_NXE: u64 = 1 << 11;

unsafe fn read_msr(msr: u32) -> u64 {
    let lo: u32;
    let hi: u32;
    core::arch::asm!("rdmsr", in("ecx") msr, out("eax") lo, out("edx") hi, options(nomem, nostack));
    ((hi as u64) << 32) | (lo as u64)
}

unsafe fn write_msr(msr: u32, value: u64) {
    let lo = value as u32;
    let hi = (value >> 32) as u32;
    core::arch::asm!("wrmsr", in("ecx") msr, in("eax") lo, in("edx") hi, options(nomem, nostack));
}

/// Habilita `EFER.NXE`. Imprescindible antes de mapear cualquier página
/// con `PAGE_NO_EXECUTE` (bit 63): sin `NXE=1` ese bit es "reservado" a
/// ojos de la CPU, y cualquier acceso a una entrada que lo tenga puesto
/// hace page fault con el bit RSVD del error code activo (visto en
/// `main.rs`: `map_page(..., executable=false)` fallaba así en la
/// primera prueba real de M4c). Idempotente — se puede llamar más de
/// una vez sin problema.
pub unsafe fn init() {
    let efer = read_msr(IA32_EFER);
    write_msr(IA32_EFER, efer | EFER_NXE);
}

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

/// Convierte una huge page de 2 MiB (entrada P2 con `PAGE_HUGE`) en una
/// tabla P1 normal de 512 entradas de 4 KiB que cubren exactamente el
/// mismo rango físico. No mueve ni copia memoria — la RAM identity-
/// mapeada sigue siendo la misma, solo cambia la granularidad con la
/// que está descrita. Solo válido para huge pages de 2 MiB (nivel P2);
/// nunca se llama para P3 — ver comentario en `get_or_create_next`.
unsafe fn split_huge_page(huge_entry: u64) -> Option<u64> {
    let huge_phys = huge_entry & ADDR_MASK;
    let leaf_flags = (huge_entry & !ADDR_MASK) & !PAGE_HUGE;

    let new_table = pmm::alloc_frame()?;
    for i in 0..512u64 {
        let entry_ptr = (new_table + i * 8) as *mut u64;
        *entry_ptr = (huge_phys + i * 4096) | leaf_flags;
    }
    Some(new_table)
}

/// Lee la entrada `table_phys[index]`; si no está presente, crea una
/// tabla nueva (frame limpio) y la enlaza. Si ya existe pero es una
/// huge page, la parte con `split_huge_page` — pero SOLO si
/// `allow_split` es cierto. En este diseño las únicas huge pages son
/// las 2 MiB de `boot.asm` a nivel P2 (ver `docs/ARCHITECTURE.md`), así
/// que `map_page_in` solo pasa `allow_split=true` en la llamada que
/// camina de P2 a P1 — las llamadas P4→P3 y P3→P2 lo dejan en `false`
/// a propósito: una huge page ahí sería de 1 GiB (nivel P3), que este
/// partidor NO sabe trocear (asume 2 MiB / 512 = 4 KiB), y este sistema
/// nunca las crea, así que fallar explícitamente sigue siendo lo
/// correcto en ese caso.
unsafe fn get_or_create_next(table_phys: u64, index: usize, flags: u64, allow_split: bool) -> Result<u64, &'static str> {
    let entry_ptr = (table_phys + (index as u64) * 8) as *mut u64;
    let entry = *entry_ptr;

    if entry & PAGE_PRESENT != 0 {
        if entry & PAGE_HUGE != 0 {
            if !allow_split {
                return Err("el camino pasa por una huge page existente — partirla no está implementado en esta pasada");
            }
            let new_table = split_huge_page(entry).ok_or("sin memoria física para partir la huge page")?;
            *entry_ptr = new_table | PAGE_PRESENT | flags;
            return Ok(new_table);
        }
        return Ok(entry & ADDR_MASK);
    }

    let new_table = pmm::alloc_frame().ok_or("sin memoria física para tabla intermedia")?;
    core::ptr::write_bytes(new_table as *mut u8, 0, 4096);
    *entry_ptr = new_table | PAGE_PRESENT | flags;
    Ok(new_table)
}

/// Mapea una página de 4 KiB: `virt` -> `phys` en el espacio de
/// direcciones activo (CR3). Ambas deben ir alineadas a 4 KiB.
pub unsafe fn map_page(virt: u64, phys: u64, writable: bool, executable: bool) -> Result<(), &'static str> {
    map_page_in(read_cr3(), virt, phys, writable, executable)
}

/// Igual que `map_page`, pero opera sobre un PML4 explícito en vez de
/// leer CR3 — necesario para preparar un espacio de direcciones antes
/// de que esté activo (antes de cambiar CR3 a él).
pub unsafe fn map_page_in(p4: u64, virt: u64, phys: u64, writable: bool, executable: bool) -> Result<(), &'static str> {
    if virt % 4096 != 0 || phys % 4096 != 0 {
        return Err("virt/phys deben estar alineados a 4 KiB");
    }

    // PAGE_USER en las tablas intermedias TAMBIÉN, no solo en la hoja:
    // el bit U/S se exige en TODOS los niveles del camino para que una
    // CPU en ring 3 pueda completar la traducción — si cualquier
    // ancestro es supervisor-only, el acceso de usuario falla ahí,
    // sin importar los permisos de la página final. `map_page_in` solo
    // se usa hoy para memoria de proceso (segmentos ELF, pilas de
    // usuario) — nunca para mapeos kernel-only — así que marcar todo
    // como accesible desde ring 3 es correcto para el uso actual; si
    // algún día hace falta un mapeo privado del kernel por esta vía,
    // esto necesitará un parámetro `user: bool` explícito.
    let inter_flags = PAGE_PRESENT | PAGE_WRITABLE | PAGE_USER;

    let p3 = get_or_create_next(p4, table_index(virt, 3), inter_flags, false)?;
    let p2 = get_or_create_next(p3, table_index(virt, 2), inter_flags, false)?;
    let p1 = get_or_create_next(p2, table_index(virt, 1), inter_flags, true)?;

    let mut leaf_flags = PAGE_PRESENT | PAGE_USER;
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

    // Solo tiene sentido invalidar el TLB si estamos tocando el espacio
    // de direcciones activo — si `p4` es un espacio todavía no cargado
    // en CR3, no hay entrada de TLB que invalidar.
    if p4 == read_cr3() {
        flush_tlb_entry(virt);
    }
    Ok(())
}

/// Crea un espacio de direcciones nuevo (PML4 propio) para un proceso.
/// Comparte automáticamente el mapeo de kernel/identity ya existente
/// (copia la entrada P4[0], que cubre los 0-4 GiB identity-mapeados —
/// kernel, drivers, MMIO) para que las syscalls/interrupciones sigan
/// funcionando sin importar qué proceso esté activo. El resto de
/// entradas quedan libres para mapeos privados del proceso.
pub unsafe fn create_address_space() -> Option<u64> {
    let new_p4 = pmm::alloc_frame()?;
    core::ptr::write_bytes(new_p4 as *mut u8, 0, 4096);

    let current_p4 = read_cr3();
    let shared_entry = *(current_p4 as *const u64);
    *(new_p4 as *mut u64) = shared_entry;

    Some(new_p4)
}

/// Clona recursivamente una subtabla (P3/P2/P1) y su contenido.
/// `level` = 2 para P3, 1 para P2, 0 para P1 (hojas: páginas de 4 KiB
/// de datos reales, se copian byte a byte). Una huge page (P2 con
/// PAGE_HUGE) no debería aparecer bajo P4[1..=255] en este diseño —
/// todo lo privado de un proceso se mapea vía `map_page_in`, que nunca
/// crea huge pages — pero por seguridad, si aparece una, se comparte
/// la entrada tal cual en vez de intentar copiar 2 MiB o fallar.
unsafe fn clone_subtree(src_table: u64, level: u8) -> Option<u64> {
    let new_table = pmm::alloc_frame()?;
    core::ptr::write_bytes(new_table as *mut u8, 0, 4096);

    for i in 0..512usize {
        let src_entry = *((src_table + (i as u64) * 8) as *const u64);
        if src_entry & PAGE_PRESENT == 0 {
            continue;
        }
        let flags = src_entry & !ADDR_MASK;
        let dst_entry_ptr = (new_table + (i as u64) * 8) as *mut u64;

        if level == 0 {
            // Hoja: página de datos de 4 KiB — copia real del contenido.
            let new_frame = pmm::alloc_frame()?;
            let src_phys = src_entry & ADDR_MASK;
            core::ptr::copy_nonoverlapping(src_phys as *const u8, new_frame as *mut u8, 4096);
            *dst_entry_ptr = new_frame | flags;
        } else if src_entry & PAGE_HUGE != 0 {
            // Huge page inesperada aquí — se comparte, no se parte.
            *dst_entry_ptr = src_entry;
        } else {
            let child_phys = src_entry & ADDR_MASK;
            let new_child = clone_subtree(child_phys, level - 1)?;
            *dst_entry_ptr = new_child | flags;
        }
    }

    Some(new_table)
}

/// Crea un espacio de direcciones nuevo con una COPIA PROFUNDA de todo
/// lo privado del proceso origen (`P4[1..=255]`) — no solo la
/// estructura de tablas, también el contenido de cada página de datos.
/// Necesario para `fork()`: el hijo debe tener su propia memoria, no
/// compartir las páginas del padre (eso sería `vfork`/COW, fuera de
/// alcance de esta pasada — ver cabecera del módulo). `P4[0]`
/// (kernel/identity-map) se comparte igual que en `create_address_space`,
/// nunca se clona: es la misma memoria física para todos los procesos
/// por diseño.
pub unsafe fn clone_address_space(src_p4: u64) -> Option<u64> {
    let new_p4 = pmm::alloc_frame()?;
    core::ptr::write_bytes(new_p4 as *mut u8, 0, 4096);

    // P4[0]: compartido, igual que create_address_space.
    *(new_p4 as *mut u64) = *(src_p4 as *const u64);

    for i in 1..512usize {
        let src_entry = *((src_p4 + (i as u64) * 8) as *const u64);
        if src_entry & PAGE_PRESENT == 0 {
            continue;
        }
        let flags = src_entry & !ADDR_MASK;
        let child_phys = src_entry & ADDR_MASK;
        let new_child = clone_subtree(child_phys, 2)?;
        *((new_p4 + (i as u64) * 8) as *mut u64) = new_child | flags;
    }

    Some(new_p4)
}

/// Libera recursivamente una subtabla (P3/P2/P1) y las páginas de
/// datos que cuelgan de ella — el reverso exacto de `clone_subtree`,
/// liberando frames (`pmm::free_frame`) en vez de copiándolos. Mismo
/// convenio de `level` (2=P3, 1=P2, 0=P1) y misma salvedad con huge
/// pages: si aparece una bajo `P4[1..=255]` (no debería, ver
/// `clone_subtree`), se deja intacta — es memoria compartida del
/// kernel, nunca del proceso, y jamás se libera desde aquí.
unsafe fn free_subtree(table: u64, level: u8) {
    for i in 0..512usize {
        let entry = *((table + (i as u64) * 8) as *const u64);
        if entry & PAGE_PRESENT == 0 {
            continue;
        }
        let child = entry & ADDR_MASK;
        if level == 0 {
            pmm::free_frame(child); // hoja: página de datos real
        } else if entry & PAGE_HUGE == 0 {
            // La llamada recursiva libera `child` ella misma al final
            // (es su propio parámetro `table`) — no liberarlo aquí
            // también, sería doble-free.
            free_subtree(child, level - 1);
        }
    }
    pmm::free_frame(table);
}

/// Libera TODO lo privado de un espacio de direcciones (`P4[1..=255]`,
/// incluidas las tablas intermedias y las páginas de datos) más el
/// propio PML4 — nunca `P4[0]` (kernel/identity-map, compartido, jamás
/// propiedad de un proceso). Para `wait()`: hasta ahora un proceso
/// terminado se quedaba zombie para siempre y nadie recuperaba su
/// memoria — fuga de facto en sesiones largas (`process::reap_zombie`
/// es el único llamante).
///
/// # Safety
/// `p4` no debe ser el espacio de direcciones ACTIVO (CR3) en este
/// momento — liberar las tablas bajo tus propios pies sería fatal. El
/// llamante (`process::reap_zombie`, invocado por el padre sobre el
/// PML4 de un hijo ya zombie) cumple esto por construcción: un proceso
/// nunca libera su propio espacio.
pub unsafe fn free_address_space(p4: u64) {
    for i in 1..512usize {
        let entry = *((p4 + (i as u64) * 8) as *const u64);
        if entry & PAGE_PRESENT == 0 {
            continue;
        }
        free_subtree(entry & ADDR_MASK, 2);
    }
    pmm::free_frame(p4);
}

/// Cambia el espacio de direcciones activo. `p4` debe ser una
/// dirección física de una tabla PML4 válida (típicamente devuelta por
/// `create_address_space`).
pub unsafe fn switch_address_space(p4: u64) {
    core::arch::asm!("mov cr3, {}", in(reg) p4, options(nostack));
}

pub unsafe fn current_address_space() -> u64 {
    read_cr3()
}

/// Traduce una dirección virtual a su física, en la jerarquía de
/// páginas dada — sin crear nada, solo lectura. `None` si no está
/// mapeada. Necesario para copiar datos a páginas recién mapeadas en un
/// espacio de direcciones que todavía no está activo (no podemos
/// escribir directamente a la dirección virtual porque CR3 no apunta
/// ahí todavía).
pub unsafe fn translate(p4: u64, virt: u64) -> Option<u64> {
    let p3_entry = *((p4 + (table_index(virt, 3) as u64) * 8) as *const u64);
    if p3_entry & PAGE_PRESENT == 0 {
        return None;
    }
    let p3 = p3_entry & ADDR_MASK;

    let p2_entry = *((p3 + (table_index(virt, 2) as u64) * 8) as *const u64);
    if p2_entry & PAGE_PRESENT == 0 {
        return None;
    }
    if p2_entry & PAGE_HUGE != 0 {
        let huge_phys = p2_entry & ADDR_MASK;
        return Some(huge_phys + (virt & 0x1F_FFFF));
    }
    let p2 = p2_entry & ADDR_MASK;

    // `p1_entry` aquí es la entrada de P2 que apunta a la TABLA P1 —
    // todavía no es la hoja. Faltaba este último nivel de indirección
    // (BUG real: `translate()` solo caminaba 3 niveles — P4→P3→P2 — y
    // trataba el puntero a la tabla P1 como si ya fuera la página de
    // datos final. Efecto observado: `copy_into_segment` escribía el
    // contenido de cada ELF cargado ENCIMA de su propia tabla P1 en vez
    // de en la página de código/datos real — silencioso mientras nadie
    // ejecutara el ELF cargado, y un page fault con bit RSVD en cuanto
    // `forktest` intentó saltar de verdad al entry point).
    let p1_entry = *((p2 + (table_index(virt, 1) as u64) * 8) as *const u64);
    if p1_entry & PAGE_PRESENT == 0 {
        return None;
    }
    let p1 = p1_entry & ADDR_MASK;

    let leaf_entry = *((p1 + (table_index(virt, 0) as u64) * 8) as *const u64);
    if leaf_entry & PAGE_PRESENT == 0 {
        return None;
    }
    Some((leaf_entry & ADDR_MASK) + (virt & 0xFFF))
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
