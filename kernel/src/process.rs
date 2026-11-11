//! Tabla de procesos — PCB real (BORRADOR SIN VERIFICAR).
//!
//! Une lo que hasta ahora estaba disperso (un PML4 por `mmu.rs`, una
//! `Task` de scheduler por `task.rs`, un slot de `Capabilities` global
//! en `caps.rs`) bajo una identidad de proceso real: PID, padre,
//! espacio de direcciones, estado. Sigue el mismo patrón de "un único
//! slot global" que ya usa `caps::CURRENT` — válido mientras solo haya
//! una tarea corriendo de verdad a la vez (cooperativo, sin SMP), igual
//! que el resto de M4.

use alloc::string::String;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicU64, Ordering};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ProcessState {
    Running,
    /// Terminado, esperando a que su padre lo recoja con `wait()`
    /// (`SYS_WAIT`, `syscall.rs::sys_wait`) — `reap_zombie()` lo saca
    /// de la tabla y libera de verdad su espacio de direcciones
    /// (`mmu::free_address_space`). Si nadie lo recoge, se queda
    /// zombie para siempre (sin `Ember`/PID 1 todavía, nadie hace de
    /// padre huérfano-adoptante — ver TODO.md).
    Zombie(i32),
}

/// Una entrada de la tabla de descriptores de fichero de un proceso —
/// M8, primer paso hacia POSIX real para mlibc. El VFS (`vfs.rs`) no
/// tiene "handles" de verdad, solo `read()`/`write()` de buffer
/// completo — así que un fd real se implementa cargando/acumulando el
/// contenido en memoria aquí, no delegando en el VFS por cada
/// `SYS_READ`/`SYS_WRITE`.
#[derive(Clone)]
pub enum FdEntry {
    /// Abierto para lectura: contenido íntegro cargado del VFS al
    /// abrir, con cursor de lectura — `SYS_READ` sirve trozos de aquí,
    /// nunca vuelve a tocar el VFS.
    ReadFile { data: Vec<u8>, cursor: usize },
    /// Abierto para escritura: buffer en memoria que `SYS_WRITE` va
    /// llenando; se vuelca de verdad al VFS (`vfs::write`) en
    /// `SYS_CLOSE` — no hay escritura incremental real al VFS todavía,
    /// pero es suficiente para que un primer `fopen()`/`fwrite()` de
    /// libc funcione de principio a fin.
    WriteFile { name: String, data: Vec<u8> },
}

/// Dirección virtual base del heap de cada proceso — 704 GiB, índice
/// P4 = 1 (privado, igual razonamiento que el código/pila en
/// `elf.rs`/`process.rs::spawn_elf`), clara de la pila (0x90...), el
/// código (0xA0...) y la pila temporal de `execve` (0xC0...).
pub const HEAP_BASE: u64 = 0x0000_00B0_0000_0000;

/// Dirección virtual base del arena de `mmap()` de cada proceso — 832
/// GiB, misma familia que `HEAP_BASE`, clara también de esta (0xB0...)
/// y de la pila temporal de `execve` (0xC0...).
pub const MMAP_BASE: u64 = 0x0000_00D0_0000_0000;

#[derive(Clone)]
pub struct Pcb {
    pub pid: u64,
    pub parent_pid: u64,
    /// PML4 físico del espacio de direcciones de este proceso.
    pub page_table: u64,
    pub state: ProcessState,
    /// Índice = fd - 3 (0/1/2 son stdin/stdout/stderr, siempre servidos
    /// por el puerto serie — ver `syscall::sys_read`/`sys_write` — y
    /// nunca ocupan un slot aquí). `None` = fd cerrado, hueco
    /// reutilizable por el siguiente `open()`.
    pub fds: Vec<Option<FdEntry>>,
    /// `brk()` — el límite LÓGICO actual, el que ve el proceso (puede
    /// bajar sin problema, `SYS_BRK` nunca desmapea).
    pub heap_top: u64,
    /// Hasta dónde hay páginas físicas REALMENTE mapeadas — monótono,
    /// solo crece. Necesario porque `heap_top` sí puede bajar: sin este
    /// segundo campo, un `brk()` que baja y luego vuelve a subir DENTRO
    /// de una zona ya mapeada antes intentaría volver a mapear una
    /// página que `mmu::map_page_in` ya tiene como presente — y esa
    /// función falla a propósito en vez de no-opear ("ya había una
    /// página mapeada ahí", pensado para detectar bugs de solapamiento,
    /// no para este caso legítimo). Limitación conocida: como nunca se
    /// desmapea de verdad, un heap que sube y baja mucho desperdicia
    /// memoria física hasta que el proceso entero termina.
    pub heap_mapped_end: u64,
    /// `mmap()` (anónimo únicamente, ver `syscall::sys_mmap`) — arena
    /// que solo crece, nunca reutiliza rango (`munmap()` es un no-op a
    /// propósito, mismo espíritu que `heap_top` nunca desmapeando de
    /// verdad). Cada mapeo nuevo empieza donde acabó el anterior.
    pub mmap_next: u64,
    /// FS.base del proceso — guardado en cada context switch fuera,
    /// restaurado al volver. mlibc lo usa para el TCB (Thread Control
    /// Block): es lo primero que lee en cualquier función de libc, a
    /// través de `%fs:0`. Empieza en 0 (sin TCB) hasta que el proceso
    /// llama a `SYS_ARCH_PRCTL(ARCH_SET_FS, ptr)`.
    pub fs_base: u64,
    /// Puntero de usuario guardado por `SYS_SET_TID_ADDRESS` — mlibc lo
    /// llama al init, espera que devolvamos el TID (= PID en este kernel,
    /// sin hilos). Se guarda por si algún día hace falta FUTEX_WAKE_OP
    /// al hacer exit(); de momento no se usa de verdad.
    pub tid_ptr: u64,
}

static NEXT_PID: AtomicU64 = AtomicU64::new(1);
/// PID del proceso actualmente en ejecución. 0 = ningún proceso de
/// verdad corriendo (código de kernel puro, como el boot o la consola
/// antes del primer `forktest`). Un único slot global, igual que
/// `caps::CURRENT` — no reentrante, válido en cooperativo sin SMP.
static CURRENT_PID: AtomicU64 = AtomicU64::new(0);

struct ProcessTableCell(core::cell::UnsafeCell<Vec<Pcb>>);
unsafe impl Sync for ProcessTableCell {}
static TABLE: ProcessTableCell = ProcessTableCell(core::cell::UnsafeCell::new(Vec::new()));

pub fn alloc_pid() -> u64 {
    NEXT_PID.fetch_add(1, Ordering::Relaxed)
}

pub fn current_pid() -> u64 {
    CURRENT_PID.load(Ordering::Relaxed)
}

pub fn set_current_pid(pid: u64) {
    CURRENT_PID.store(pid, Ordering::Relaxed);
}

/// Da de alta un proceso nuevo en la tabla.
pub fn register(pid: u64, parent_pid: u64, page_table: u64) {
    unsafe {
        (*TABLE.0.get()).push(Pcb {
            pid,
            parent_pid,
            page_table,
            state: ProcessState::Running,
            fds: Vec::new(),
            heap_top: HEAP_BASE,
            heap_mapped_end: HEAP_BASE,
            mmap_next: MMAP_BASE,
            fs_base: 0,
            tid_ptr: 0,
        });
    }
}

pub fn find(pid: u64) -> Option<Pcb> {
    unsafe { (*TABLE.0.get()).iter().find(|p| p.pid == pid).cloned() }
}

/// Acceso mutable de un solo uso al PCB de `pid`, para syscalls que
/// necesitan tocar `fds`/`heap_top` (`SYS_READ`/`SYS_WRITE`/`SYS_OPEN`/
/// `SYS_CLOSE`/`SYS_BRK`) sin repetir el mismo barrido lineal +
/// `iter_mut().find()` que ya hacían `set_page_table`/`mark_zombie` por
/// cada campo nuevo. `None` si el PID no está registrado.
pub fn with_pid_mut<R>(pid: u64, f: impl FnOnce(&mut Pcb) -> R) -> Option<R> {
    unsafe { (*TABLE.0.get()).iter_mut().find(|p| p.pid == pid).map(f) }
}

/// Actualiza el PML4 de un proceso — para `execve()`, que reemplaza
/// el espacio de direcciones entero. `false` si el PID no existía.
pub fn set_page_table(pid: u64, page_table: u64) -> bool {
    unsafe {
        for p in (*TABLE.0.get()).iter_mut() {
            if p.pid == pid {
                p.page_table = page_table;
                return true;
            }
        }
    }
    false
}

/// Marca un proceso como zombie con su código de salida. `false` si el
/// PID no estaba registrado.
pub fn mark_zombie(pid: u64, exit_code: i32) -> bool {
    unsafe {
        for p in (*TABLE.0.get()).iter_mut() {
            if p.pid == pid {
                p.state = ProcessState::Zombie(exit_code);
                return true;
            }
        }
    }
    false
}

/// Lista de todos los procesos — usado por el comando `ps` de la
/// consola de depuración.
pub fn list() -> Vec<Pcb> {
    unsafe { (*TABLE.0.get()).clone() }
}

/// `true` si `parent_pid` tiene al menos un hijo registrado (zombie o
/// no) — para que `wait()` pueda distinguir "no hay nada que esperar"
/// (ECHILD, devolver ya) de "hay hijos pero ninguno ha terminado
/// todavía" (esperar de verdad, cediendo el turno).
pub fn has_children(parent_pid: u64) -> bool {
    unsafe { (*TABLE.0.get()).iter().any(|p| p.parent_pid == parent_pid) }
}

/// Busca el primer hijo zombie de `parent_pid`, lo saca de la tabla, y
/// libera de verdad su espacio de direcciones (PML4 + tablas
/// intermedias + páginas de datos — `mmu::free_address_space`, nunca
/// se llamaba hasta ahora). `None` si no hay ningún hijo zombie
/// todavía (puede que sí haya hijos vivos — ver `has_children`).
pub fn reap_zombie(parent_pid: u64) -> Option<(u64, i32)> {
    unsafe {
        let table = &mut *TABLE.0.get();
        let idx = table
            .iter()
            .position(|p| p.parent_pid == parent_pid && matches!(p.state, ProcessState::Zombie(_)))?;
        let pcb = table.remove(idx);
        let ProcessState::Zombie(code) = pcb.state else {
            unreachable!("filtrado por posición arriba")
        };
        crate::mmu::free_address_space(pcb.page_table);
        Some((pcb.pid, code))
    }
}

/// Recoge un proceso zombie CONCRETO por PID: lo saca de la tabla,
/// libera su espacio de direcciones y devuelve su código de salida.
/// `None` si el PID no existe o sigue vivo. Es el `waitpid(pid)` de
/// quien no es un proceso — la consola de depuración (PID 0), que lanza
/// programas en primer plano con `run` y no puede usar `reap_zombie`
/// (recogería el primer zombie huérfano que hubiera, no el suyo).
pub fn reap_pid(pid: u64) -> Option<i32> {
    unsafe {
        let table = &mut *TABLE.0.get();
        let idx = table.iter().position(|p| p.pid == pid)?;
        let ProcessState::Zombie(code) = table[idx].state else {
            return None;
        };
        let pcb = table.remove(idx);
        crate::mmu::free_address_space(pcb.page_table);
        Some(code)
    }
}

// --- lanzamiento del primer proceso de una demo (no un fork) ---
//
// Mismo patrón de "slot de handoff único" que `syscall::sys_fork` usa
// para el hijo: `scheduler::spawn_with_space` solo acepta un `fn() -> !`
// sin argumentos, así que la única forma de pasarle datos a la tarea
// nueva es dejarlos en globales que ella lea nada más arrancar.

static mut LAUNCH_ENTRY: u64 = 0;
static mut LAUNCH_STACK: u64 = 0;

fn launch_trampoline() -> ! {
    unsafe {
        let entry = LAUNCH_ENTRY;
        let stack = LAUNCH_STACK;
        // `current_pid()` ya lo dejó puesto el scheduler (`yield_now`,
        // con `Task::pid`) antes de saltar aquí — no hace falta
        // fijarlo a mano.
        crate::ring3::enter_ring3(entry, stack);
    }
}

/// Da de alta y arranca un proceso nuevo desde cero (no un `fork()`)
/// como tarea del scheduler. Pensado para el primer proceso de una
/// demo de consola (`forktest`, `exec`) o para el arranque automático
/// de `Ember` (PID 1) desde `main.rs` — el camino normal de un sistema
/// real, una vez Ember pueda lanzar servicios reales, es que todo lo
/// demás nazca de `fork()`+`execve()` desde él, no de aquí.
pub unsafe fn spawn_process(entry_point: u64, user_stack_top: u64, page_table: u64, parent_pid: u64) -> u64 {
    let pid = alloc_pid();
    register(pid, parent_pid, page_table);
    LAUNCH_ENTRY = entry_point;
    LAUNCH_STACK = user_stack_top;
    crate::scheduler::spawn_with_space(launch_trampoline, page_table, pid);
    pid
}

/// Límite de argumentos de un proceso nuevo. El frame inicial entero
/// (cadenas + tabla de punteros) tiene que caber en la página más alta
/// de la pila, y `write_initial_stack_frame` lo comprueba byte a byte —
/// esto solo acota la tabla para que el cálculo no pueda desbordar.
pub const MAX_ARGS: usize = 32;

/// Escribe el frame inicial SysV x86_64 en la cima de la pila de usuario
/// y devuelve el RSP con el que debe arrancar el proceso. Es lo que lee
/// `_start` (`crt1.S` de mlibc, o `hello.asm` a mano):
///
/// ```text
///   [rsp]                 argc
///   [rsp+8 ...]           argv[0] .. argv[argc-1]   (punteros)
///                         NULL
///                         NULL                      (envp vacío)
///                         AT_PAGESZ (6), 4096
///                         AT_ENTRY  (9), entry
///                         AT_NULL   (0), 0
///                         (8 bytes de relleno si hace falta para que
///                          RSP quede alineado a 16)
///   ...                   las cadenas de argv, terminadas en NUL
///   stack_virt_top  ->    cima de la pila
/// ```
///
/// mlibc usa AT_PAGESZ en su allocator; AT_ENTRY lo consulta el enlazador
/// dinámico. Sin variables de entorno todavía (`envp` siempre vacío).
///
/// `stack_phys` es el frame físico de la página MÁS ALTA de la pila;
/// todo el frame tiene que caber en ella.
unsafe fn write_initial_stack_frame(
    stack_phys: u64,
    stack_virt_top: u64,
    entry: u64,
    argv: &[&str],
) -> Result<u64, &'static str> {
    if argv.len() > MAX_ARGS {
        return Err("demasiados argumentos para el proceso");
    }
    // argc + punteros de argv + NULL + NULL de envp + 3 pares de auxv.
    let table_qwords = 1 + argv.len() + 1 + 1 + 6;
    let strings_len: usize = argv.iter().map(|a| a.len() + 1).sum();
    // Las cadenas van arriba del todo; la tabla, debajo, alineada a 16.
    let frame_len = (strings_len + 15) / 16 * 16 + (table_qwords * 8 + 15) / 16 * 16;
    if frame_len > 4096 {
        return Err("los argumentos no caben en la página inicial de la pila");
    }

    // Dentro de la página: desplazamiento 4096 = `stack_virt_top`.
    let page = stack_phys as *mut u8;
    let virt_of = |offset: usize| stack_virt_top - 4096 + offset as u64;

    let mut arg_ptrs = [0u64; MAX_ARGS];
    let mut cursor = 4096;
    for (i, arg) in argv.iter().enumerate() {
        cursor -= arg.len() + 1;
        unsafe {
            core::ptr::copy_nonoverlapping(arg.as_ptr(), page.add(cursor), arg.len());
            page.add(cursor + arg.len()).write(0);
        }
        arg_ptrs[i] = virt_of(cursor);
    }

    let table_offset = 4096 - frame_len;
    let table = unsafe { page.add(table_offset) } as *mut u64;
    let mut slot = 0;
    let mut push = |value: u64| {
        unsafe { table.add(slot).write(value) };
        slot += 1;
    };
    push(argv.len() as u64); // argc
    for &ptr in &arg_ptrs[..argv.len()] {
        push(ptr);
    }
    push(0); // fin de argv
    push(0); // fin de envp (vacío)
    push(6); // AT_PAGESZ
    push(4096);
    push(9); // AT_ENTRY
    push(entry);
    push(0); // AT_NULL
    push(0);

    Ok(virt_of(table_offset))
}

/// Páginas de pila de usuario de un proceso recién arrancado: 256 KiB.
/// Una sola página (lo que había hasta M10) bastaba para los binarios de
/// prueba en ensamblador, pero no para C real: solo el `printf` de mlibc
/// ya gasta varios KiB de pila en buffers de formateo. Se mapea entera
/// al arrancar — sin paginación bajo demanda todavía, así que este es
/// también el límite duro: pasarse por debajo es un SIGSEGV.
pub const USER_STACK_PAGES: u64 = 64;

/// Mapea una pila de usuario nueva, con el frame inicial de
/// `write_initial_stack_frame` ya escrito en su página más alta — núcleo
/// compartido entre `spawn_elf` y `syscall::sys_execve` (los dos únicos
/// sitios donde un proceso "arranca de cero" de verdad, a diferencia de
/// `fork()`, que reanuda la pila YA EXISTENTE del padre). `entry` es el
/// punto de entrada del ELF (necesario para AT_ENTRY en el auxv).
///
/// `stack_virt` es la dirección de la página MÁS ALTA de la pila (la
/// cima queda en `stack_virt + 4096`, igual que cuando la pila era de
/// una sola página); el resto de `USER_STACK_PAGES` se mapean por debajo.
/// `argv` son los argumentos del programa, `argv[0]` incluido.
pub unsafe fn map_fresh_user_stack(
    page_table: u64,
    stack_virt: u64,
    entry: u64,
    argv: &[&str],
) -> Result<u64, &'static str> {
    let mut top_page_phys = 0;
    for i in 0..USER_STACK_PAGES {
        let phys = crate::pmm::alloc_frame().ok_or("sin memoria para la pila de usuario")?;
        // `alloc_frame` no limpia: un frame reciclado trae lo que dejó su
        // dueño anterior (posiblemente otro proceso).
        unsafe { core::ptr::write_bytes(phys as *mut u8, 0, 4096) };
        crate::mmu::map_page_in(page_table, stack_virt - i * 4096, phys, true, false)?;
        if i == 0 {
            top_page_phys = phys;
        }
    }
    unsafe { write_initial_stack_frame(top_page_phys, stack_virt + 4096, entry, argv) }
}

/// Carga un ELF64 desde `bytes`, le monta una pila de usuario, y lo
/// arranca como proceso real (`spawn_process`). Núcleo compartido entre
/// `console::launch_elf` (comandos de la consola, con mensajes de error
/// hacia el puerto serie) y el arranque automático de Ember en
/// `main.rs` (sin puerto al que escribir, solo `serial_println!`) —
/// antes duplicado en `console.rs`, movido aquí para que ambos usen
/// exactamente el mismo camino en vez de dos copias que podrían
/// divergir con el tiempo. `argv` incluye `argv[0]` (el nombre del
/// programa).
pub unsafe fn spawn_elf(bytes: &[u8], parent_pid: u64, argv: &[&str]) -> Result<u64, &'static str> {
    let loaded = crate::elf::load(bytes)?;

    let user_stack_virt: u64 = 0x0000_0090_0000_0000; // 576 GiB, privado del proceso
    let user_stack_top = match unsafe { map_fresh_user_stack(loaded.page_table, user_stack_virt, loaded.entry_point, argv) } {
        Ok(top) => top,
        Err(e) => {
            // El proceso no llega a existir: devolver lo ya cargado.
            unsafe { crate::mmu::free_address_space(loaded.page_table) };
            return Err(e);
        }
    };

    Ok(spawn_process(loaded.entry_point, user_stack_top, loaded.page_table, parent_pid))
}

/// Deja el PCB de `pid` como recién creado en todo lo que depende del
/// espacio de direcciones: heap, arena de `mmap()` y TLS. Para
/// `execve()`, que sustituye el espacio entero — sin esto el programa
/// nuevo heredaba `heap_mapped_end` del anterior y su primer `brk()` daba
/// por mapeadas páginas que en el espacio nuevo no existen.
/// Los descriptores de fichero se conservan, como manda POSIX.
pub fn reset_for_exec(pid: u64) {
    with_pid_mut(pid, |pcb| {
        pcb.heap_top = HEAP_BASE;
        pcb.heap_mapped_end = HEAP_BASE;
        pcb.mmap_next = MMAP_BASE;
        pcb.fs_base = 0;
        pcb.tid_ptr = 0;
    });
}
