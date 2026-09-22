//! IPC — pipes en memoria (BORRADOR SIN VERIFICAR).
//!
//! Buffer circular por pipe, namespace por ID numérico — mismo patrón
//! que `vfs.rs` (BTreeMap global, simplificación honesta antes de tener
//! descriptores de fichero por proceso de verdad). Suficiente para que
//! dos tareas se pasen datos entre sí sin compartir memoria a pelo.

use alloc::collections::{BTreeMap, VecDeque};
use core::cell::UnsafeCell;

const PIPE_CAPACITY: usize = 4096;

pub type PipeId = u32;

struct PipeRegistry(UnsafeCell<Option<(BTreeMap<PipeId, VecDeque<u8>>, PipeId)>>);
unsafe impl Sync for PipeRegistry {}

static REGISTRY: PipeRegistry = PipeRegistry(UnsafeCell::new(None));

pub fn init() {
    unsafe {
        *REGISTRY.0.get() = Some((BTreeMap::new(), 0));
    }
}

fn state() -> &'static mut (BTreeMap<PipeId, VecDeque<u8>>, PipeId) {
    unsafe {
        (*REGISTRY.0.get())
            .as_mut()
            .expect("pipe::init() no llamado")
    }
}

/// Crea un pipe nuevo, vacío, y devuelve su ID.
pub fn create() -> PipeId {
    let (map, next_id) = state();
    let id = *next_id;
    *next_id += 1;
    map.insert(id, VecDeque::with_capacity(PIPE_CAPACITY));
    id
}

/// Escribe en el pipe. Devuelve cuántos bytes cupieron de verdad — si el
/// pipe está lleno, escribe lo que quepa y para ahí (no bloquea; no
/// tenemos scheduler esperando todavía, bloquear de verdad es tarea de
/// cuando exista integración con procesos reales).
pub fn write(id: PipeId, data: &[u8]) -> Result<usize, &'static str> {
    let (map, _) = state();
    let buf = map.get_mut(&id).ok_or("pipe no existe")?;
    let mut written = 0;
    for &byte in data {
        if buf.len() >= PIPE_CAPACITY {
            break;
        }
        buf.push_back(byte);
        written += 1;
    }
    Ok(written)
}

/// Lee hasta `max_len` bytes del pipe (los más antiguos primero, FIFO).
/// Devuelve un buffer vacío si no hay nada — no bloqueante, mismo
/// motivo que `write`.
pub fn read(id: PipeId, max_len: usize) -> Result<alloc::vec::Vec<u8>, &'static str> {
    let (map, _) = state();
    let buf = map.get_mut(&id).ok_or("pipe no existe")?;
    let n = core::cmp::min(max_len, buf.len());
    Ok(buf.drain(..n).collect())
}

pub fn close(id: PipeId) -> bool {
    state().0.remove(&id).is_some()
}

pub fn pending(id: PipeId) -> Option<usize> {
    state().0.get(&id).map(|b| b.len())
}
