//! Acceso validado a memoria de usuario desde una syscall (M12).
//!
//! Hasta M11 toda syscall se fiaba del puntero que le pasaban: lo
//! desreferenciaba tal cual, en ring 0. Un puntero malo —por error o a
//! propósito— era un fallo de página del KERNEL, es decir, el sistema
//! entero parado; y uno que apuntara al propio kernel le dejaba a
//! cualquier proceso leer o escribir memoria de ring 0 (`write(1,
//! direccion_del_kernel, n)` la volcaba por el puerto serie).
//!
//! Todo lo que cruza la frontera pasa ahora por aquí. Un rango es válido
//! solo si, de principio a fin:
//!
//! 1. cae en la mitad de usuario Y fuera de `P4[0]` — las direcciones
//!    por debajo de 512 GiB son el identity-map del kernel, compartido
//!    por todos los procesos (ver `elf.rs`); ningún proceso tiene nada
//!    suyo ahí;
//! 2. cada página está mapeada en el espacio del proceso que llama, con
//!    el bit de usuario, y con el de escritura si se va a escribir.
//!
//! Si no, la syscall devuelve `EFAULT` y el kernel no toca nada.
//!
//! Se comprueba primero y se copia después, sin más: con una sola CPU y
//! las interrupciones enmascaradas durante la syscall, nadie puede
//! desmapear la página entre un paso y el otro. Con SMP o hilos esto
//! necesitará una copia que sobreviva al fallo (tabla de excepciones).

use crate::{mmu, process};
use alloc::string::String;
use alloc::vec::Vec;

/// `-EFAULT`, listo para devolver como resultado de una syscall.
pub const EFAULT: u64 = (-14i64) as u64;

/// Primera dirección que puede pertenecer a un proceso (512 GiB).
const USER_START: u64 = 1 << 39;
/// Fin de la mitad baja canónica del espacio de direcciones.
const USER_END: u64 = 1 << 47;

/// Error de acceso a memoria de usuario — siempre se traduce a `EFAULT`.
#[derive(Debug, Clone, Copy)]
pub struct Fault;

fn current_page_table() -> Result<u64, Fault> {
    process::find(process::current_pid()).map(|p| p.page_table).ok_or(Fault)
}

/// Comprueba que `[addr, addr + len)` es memoria del proceso actual,
/// legible desde ring 3 (y escribible, si `write`).
pub fn check(addr: u64, len: usize, write: bool) -> Result<(), Fault> {
    if len == 0 {
        return Ok(());
    }
    let end = addr.checked_add(len as u64).ok_or(Fault)?;
    if addr < USER_START || end > USER_END {
        return Err(Fault);
    }
    let page_table = current_page_table()?;
    let mut page = addr & !0xFFF;
    while page < end {
        if !unsafe { mmu::user_page_ok(page_table, page, write) } {
            return Err(Fault);
        }
        page += 4096;
    }
    Ok(())
}

/// Copia `src` a la dirección de usuario `dst`.
pub fn copy_to_user(dst: u64, src: &[u8]) -> Result<(), Fault> {
    check(dst, src.len(), true)?;
    unsafe { core::ptr::copy_nonoverlapping(src.as_ptr(), dst as *mut u8, src.len()) };
    Ok(())
}

/// Lee un valor `Copy` (un entero, un puntero) de memoria de usuario.
pub fn read<T: Copy>(src: u64) -> Result<T, Fault> {
    check(src, core::mem::size_of::<T>(), false)?;
    Ok(unsafe { core::ptr::read_unaligned(src as *const T) })
}

/// Escribe un valor `Copy` en memoria de usuario.
pub fn write<T: Copy>(dst: u64, value: T) -> Result<(), Fault> {
    check(dst, core::mem::size_of::<T>(), true)?;
    unsafe { core::ptr::write_unaligned(dst as *mut T, value) };
    Ok(())
}

/// Vista de solo lectura de un buffer de usuario ya validado — para las
/// syscalls que consumen los bytes en el acto (`write()`), sin copia
/// intermedia. Válida mientras dure la syscall.
pub fn slice<'a>(addr: u64, len: usize) -> Result<&'a [u8], Fault> {
    check(addr, len, false)?;
    if len == 0 {
        return Ok(&[]);
    }
    Ok(unsafe { core::slice::from_raw_parts(addr as *const u8, len) })
}

/// Lee una cadena C (terminada en NUL) de como mucho `max` bytes.
/// Valida página a página según avanza: la cadena puede acabar justo
/// antes de una página sin mapear sin que eso sea un error.
pub fn read_cstr(addr: u64, max: usize) -> Result<String, Fault> {
    let page_table = current_page_table()?;
    let mut bytes = Vec::new();
    let mut cursor = addr;
    let mut checked_page = u64::MAX;
    loop {
        if cursor < USER_START || cursor >= USER_END {
            return Err(Fault);
        }
        let page = cursor & !0xFFF;
        if page != checked_page {
            if !unsafe { mmu::user_page_ok(page_table, page, false) } {
                return Err(Fault);
            }
            checked_page = page;
        }
        let byte = unsafe { *(cursor as *const u8) };
        if byte == 0 {
            break;
        }
        if bytes.len() >= max {
            return Err(Fault); // sin NUL dentro del límite
        }
        bytes.push(byte);
        cursor += 1;
    }
    String::from_utf8(bytes).map_err(|_| Fault)
}
