//! VFS mínimo — tmpfs plano en memoria (M5, BORRADOR SIN VERIFICAR).
//!
//! Sin jerarquía de directorios todavía (namespace plano por nombre) —
//! suficiente para demostrar que leer/escribir funciona antes de montar
//! algo más ambicioso (ext2, o formato propio) encima de AHCI, que ya
//! tenemos leyendo y escribiendo sectores reales.

use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec::Vec;
use core::cell::UnsafeCell;

struct TmpFs(UnsafeCell<Option<BTreeMap<String, Vec<u8>>>>);
unsafe impl Sync for TmpFs {}

static FS: TmpFs = TmpFs(UnsafeCell::new(None));

pub fn init() {
    unsafe {
        *FS.0.get() = Some(BTreeMap::new());
    }
}

fn map() -> &'static mut BTreeMap<String, Vec<u8>> {
    unsafe {
        (*FS.0.get())
            .as_mut()
            .expect("vfs::init() no llamado antes de usar el VFS")
    }
}

pub fn write(name: &str, data: &[u8]) {
    map().insert(String::from(name), Vec::from(data));
}

pub fn read(name: &str) -> Option<Vec<u8>> {
    map().get(name).cloned()
}

pub fn delete(name: &str) -> bool {
    map().remove(name).is_some()
}

pub fn list() -> Vec<String> {
    map().keys().cloned().collect()
}
