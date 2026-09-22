---
title: FORGE OS — Documento Maestro v1.0
project: Forge OS / Static Forge
date: Septiembre 2026
author: Core (Static Forge)
---

# FORGE OS — Documento Maestro

> Sistema operativo completo (kernel + userland) escrito desde cero en Rust  
> Proyecto de **Static Forge** — por Core  
> Licencia: GPL-2.0+

---

## ÍNDICE

1. [Identidad del proyecto](#identidad)
2. [Filosofía de diseño](#filosofia)
3. [Arquitectura técnica](#arquitectura)
4. [Estado actual del kernel](#estado-kernel)
5. [Inventario de ficheros](#inventario)
6. [Historial de milestones](#milestones)
7. [Userland planificado](#userland)
8. [Escritorio — Anvil](#anvil)
9. [Shell y terminal](#shell)
10. [Modelo de seguridad](#seguridad)
11. [Stack de red](#red)
12. [Filesystem](#filesystem)
13. [Compatibilidad de apps](#compatibilidad)
14. [Lenguajes nativos](#lenguajes)
15. [Roadmap y memoria de uso](#roadmap)
16. [TODO maestro](#todo)
17. [Referentes externos](#referentes)
18. [Entorno de desarrollo](#entorno)
19. [Errores conocidos y lecciones](#errores)

---

## 1. IDENTIDAD DEL PROYECTO {#identidad}

| Campo | Valor |
|---|---|
| **Nombre** | Forge OS |
| **Organización** | Static Forge |
| **Autor** | Core (Alfons Herrera) |
| **Lenguaje** | Rust nightly (x86_64-unknown-none, no_std) |
| **Licencia** | GPL-2.0+ |
| **Target** | x86_64, bare metal, sin stdlib |
| **Boot** | Multiboot2 → GRUB (actual) / Limine (decisión futura) |
| **Tipo** | Sistema operativo completo: kernel + userland |
| **Repo local** | `/home/alfy/proyectos/forge-os/` (CachyOS) |
| **PC de desarrollo** | AMD Ryzen 5 5600 + RTX 3060 Ti / Windows 11 |

**Forge OS es un SO COMPLETO — no solo un kernel.** El kernel (milestones M0-M4+) es el prerrequisito, pero el destino final es kernel + userland completo con shell, terminal, compositor, gestor de paquetes y aplicaciones propias.

---

## 2. FILOSOFÍA DE DISEÑO {#filosofia}

Forge OS no fusiona código de Linux, OpenBSD y Windows NT — fusiona **decisiones de diseño**, implementadas desde cero en Rust.

### Los seis principios

**1. Seguro por defecto, no por parche** *(OpenBSD + FreeBSD)*  
Deny-first. Un proceso declara al arrancar qué syscalls necesita (`pledge()`-style). El kernel congela esa máscara para siempre: pedir algo fuera de lista es `EPERM` permanente, sin excepciones. Complementado con Capsicum (capabilities por fd individual) y privsep (patrón para demonios: proceso sin privilegios + supervisor mínimo).

**2. Rápido porque es simple** *(Linux)*  
Kernel monolítico. Syscalls directas vía `syscall`/`sysret`, sin paso de mensajes entre servidores. La misma apuesta que ganó Linux frente a Hurd/Mach.

**3. Estructurado en capas, no en espagueti** *(Windows NT)*
```
┌─────────────────────────────────────┐
│  Kernel   — scheduler, IRQ, locks   │  mínimo, no crece
├─────────────────────────────────────┤
│  Executive — Object Manager,        │  la mayoría de la lógica
│              I/O Manager, Process   │  vive aquí
├─────────────────────────────────────┤
│  HAL      — Intel/AMD/NVIDIA/ARM,   │  específico de vendor,
│             red, storage            │  detrás de traits
└─────────────────────────────────────┘
```
Precedente real validado: ReactOS (`ntoskrnl/` tiene `ob/`, `mm/`, `ke/`, `io/`, `ps/`, `se/`, `lpc/` — exactamente nuestro Executive). NT también tiene `subsystems/csr/` + `subsystems/win/`: el Executive es **agnóstico de API**. Win32 no vive dentro del kernel, corre como servidor de subsistema. Es el mismo patrón que usaremos para nuestra capa de compatibilidad Linux futura.

**4. Todo es un objeto navegable** *(NT Object Manager + Plan 9)*  
Ficheros, procesos, sockets, memoria compartida, primitivas de sincronización — todos son el mismo tipo de handle con refcounting (`Arc<KernelObject>`). De Plan 9 heredamos que ese conjunto es un único árbol navegable.

**5. Memoria segura por construcción, no por auditoría** *(Rust)*  
OpenBSD lo consigue con disciplina y auditoría humana brutal. Nosotros perseguimos el mismo resultado porque el compilador rechaza la clase de bug entera en tiempo de compilación.

**6. Cero deuda heredada**  
ABI propia desde el día uno. Sin compatibilidad binaria con nada externo. Si algún día se quiere correr ELFs de Linux, es una capa de traducción *encima* del kernel, nunca dentro.

### Qué NO somos
- No somos ReactOS (sin compatibilidad binaria con Windows)
- No somos un microkernel puro (elegimos rendimiento sobre aislamiento teórico)
- No somos un clon estético de nada

---

## 3. ARQUITECTURA TÉCNICA {#arquitectura}

### Estructura de directorios
```
forge-os/
├── kernel/
│   ├── src/           ← todo el código Rust
│   ├── boot/
│   │   └── boot.asm   ← Multiboot2, Long Mode, identity-map 0-4GiB
│   ├── assets/        ← EXT2_TEST_IMAGE.md, README.md
│   ├── .cargo/
│   │   └── config.toml
│   ├── Cargo.toml
│   └── build.rs
├── targets/
│   ├── x86_64-forge.json   ← target personalizado no_std
│   └── linker.ld           ← KERNEL_BASE = 0x100000 (flat, no higher-half)
├── docs/
│   ├── ARCHITECTURE.md
│   ├── PHILOSOPHY.md
│   ├── PRODUCT.md
│   ├── ROADMAP.md
│   ├── TODO.md
│   ├── DESKTOP.md
│   ├── SHELL.md
│   ├── LANGUAGES.md
│   └── COMPATIBILITY.md
├── iso/boot/grub/grub.cfg
└── tools/
    ├── run-qemu.sh
    └── build.sh
```

### Nota de arquitectura: kernel flat, no higher-half (por ahora)
El diseño original planteaba kernel higher-half (`0xFFFFFFFF80000000`, patrón `PML4[511]`). Al escribir M2 se detectó que `boot.asm` solo monta identity-map del primer GiB. Corregido bajando el kernel a direcciones flat (virtual == físico, `KERNEL_BASE = 0x100000`). **Migrar a higher-half real es tarea futura**, pospuesta hasta tener QEMU real para validar.

### Convención de syscalls
Heredada de Linux x86_64: instrucción `syscall`, `RAX` = número, argumentos en `RDI, RSI, RDX, R10, R8, R9`, retorno en `RAX` (negativo = error). Numeración compatible con Linux donde aplica:
- `SYS_GETPID = 39`
- `SYS_FORK = 57`
- `SYS_EXECVE = 59`
- `SYS_EXIT = 60`
- `SYS_PING = 999` (debug)

### Boot manager — decisión: Limine
- Soporta Multiboot2 nativamente — compatible con `boot.asm` M0 desde el día uno
- Soporta BIOS y UEFI
- Moderno, sin el bloat de GRUB
- Protocolo de arranque propio como alternativa a Multiboot2
- Referente: documentación oficial de CachyOS sobre gestores de arranque

---

## 4. ESTADO ACTUAL DEL KERNEL {#estado-kernel}

**Snapshot más reciente:** `forge-os-fork-execve.zip` — **5158 líneas**  
**Total de milestones entregados:** 44 zips de desarrollo

### Módulos implementados (borradores sin verificar en QEMU)

| Fichero | Descripción | Estado |
|---|---|---|
| `boot.asm` | Multiboot2, Long Mode, identity-map 0-4 GiB, salto a Rust | ✅ borrador |
| `main.rs` | kmain, arranque de todos los subsistemas (376 L) | ✅ borrador |
| `serial.rs` | Puerto serie COM1, log de kernel | ✅ borrador |
| `gdt.rs` | GDT, TSS, selectores ring 0 y ring 3 (DPL=3), TSS.RSP0 | ✅ borrador |
| `idt.rs` | IDT 256 entradas, handlers #0/#3/#6/#8/#13/#14, timer 0x40, teclado IRQ1=33 | ✅ borrador |
| `pic.rs` | PIC 8259 remapeado IRQ0-7→32-39, IRQ8-15→40-47 | ✅ borrador |
| `pmm.rs` | Bitmap allocator (MAX_FRAMES=1M, 4 GiB), `alloc_frame()`, `alloc_contiguous(count)` | ✅ borrador |
| `heap.rs` | Bump allocator 1 MiB, `#[global_allocator]` | ✅ borrador |
| `mb2.rs` | Parser de tags Multiboot2 | ✅ borrador |
| `framebuffer.rs` | 1024×768@32bpp, fuente 8×8, `draw_char_psf()`, test_pattern() | ✅ borrador |
| `font.rs` | Glifos 8×8 a mano | ✅ borrador |
| `psf.rs` | Parser PSF1 para fuente real desde assets | ✅ borrador |
| `caps.rs` | `Capabilities(u32)`, `pledge()`, `enforce()`, 9 caps + CAP_UNRESTRICTED | ✅ borrador |
| `apic.rs` | Local APIC, timer periódico, vector 0x40, `tick_count()` (AtomicU64) | ✅ borrador |
| `preempt.rs` | `preempt_handler` naked, `PREEMPT_ENABLED` AtomicBool, `enable()`/`disable()` | ✅ borrador |
| `task.rs` | `Context` (callee-saved + RSP), `Task`, `switch_to` naked, `Task::new()` | ✅ borrador |
| `scheduler.rs` | Round-robin, `init()`, `spawn()`, `spawn_with_space()`, `yield_now()` | ✅ borrador |
| `mmu.rs` | `map_page()`, `map_page_in()`, `create_address_space()`, `clone_address_space()`, `switch_address_space()`, `translate()`, `unmap_page()` | ✅ borrador |
| `pci.rs` | Enumeración 0xCF8/0xCFC, lectura de BARs | ✅ borrador |
| `keyboard.rs` | PS/2 IRQ1, scancodes Set1 US, buffer circular 64B | ✅ borrador |
| `ahci.rs` | `init_port()`, `read_sectors()`, `write_sectors()`, `first_disk()` (518 L) | ✅ borrador |
| `rtl8139.rs` | TX real con `init_full()`, `send()` polling TxStatOK (264 L) | ✅ borrador |
| `elf.rs` | Parser cabecera + program headers PT_LOAD, TEST_ELF, TEST_ELF_RING3, TEST_ELF_SYSCALL (277 L) | ✅ borrador |
| `ring3.rs` | `enter_ring3()` via iretq, `enter_ring3_with_rax()` (para fork hijo) | ✅ borrador |
| `syscall.rs` | MSRs (EFER.SCE, STAR, LSTAR, FMASK), naked entry, SyscallFrame extendido con user_rflags/user_rip, SYS_FORK/EXECVE/EXIT/GETPID (349 L) | ✅ borrador |
| `vfs.rs` | BTreeMap plano, `write()`, `read()`, `delete()`, `list()` | ✅ borrador |
| `ext2.rs` | Superbloque, grupos, inodos, rutas, árbol de extents ext4, feature flags (364 L) | ✅ borrador |
| `partinfo.rs` | GPT + MBR fallback, ext2/3/4, btrfs, NTFS, FAT32, `FsType::installable()` (220 L) | ✅ borrador |
| `pipe.rs` | Buffer circular FIFO 4096B/pipe, `create()`, `write()`, `read()`, `pending()`, `close()` | ✅ borrador |
| `process.rs` | PCB, `ProcessState`, tabla global de procesos, `alloc_pid()`, `current_pid()`, `register()`, `mark_zombie()` | ✅ borrador |
| `console.rs` | REPL de depuración sobre serie, 20+ comandos (301 L) | ✅ borrador |

### Comandos de consola disponibles
```
help, meminfo, caps, bp, panic, fb, pci, ahci, net, disktest,
ls, cat, write, ring3test, synccalltest, synccalldeny,
mount, e2ls, e2cat, ps, forktest, exec
```

---

## 5. INVENTARIO DE FICHEROS {#inventario}

### kernel/src/ — 30 módulos, 5158 líneas totales

```
ahci.rs          518 L  — driver AHCI (SATA), TX+RX, verificado vs Linux
main.rs          376 L  — punto de entrada del kernel, arranque de subsistemas
ext2.rs          364 L  — filesystem ext2/ext4 read-only, árbol de extents
syscall.rs       349 L  — handler naked syscall/sysret, dispatcher, fork/execve/exit
console.rs       301 L  — REPL de depuración sobre COM1
elf.rs           277 L  — loader ELF64, TEST_ELF*, mapeo en espacio propio
rtl8139.rs       264 L  — driver RTL8139, TX real verificado vs Realtek PG
mmu.rs           262 L  — gestión de memoria virtual 4 niveles, clone_address_space
partinfo.rs      220 L  — escáner GPT/MBR, detección de FS
framebuffer.rs   189 L  — 1024×768@32bpp, draw_char, test_pattern
pci.rs           177 L  — enumeración PCI, lectura de BARs
task.rs          151 L  — Task, Context, switch_to naked
idt.rs           138 L  — IDT 256 entradas, 6 handlers de excepción
gdt.rs           136 L  — GDT, TSS, selectores ring 0/3
keyboard.rs      120 L  — PS/2, scancodes Set1 US, buffer circular
pmm.rs           117 L  — allocador físico bitmap
pic.rs           113 L  — PIC 8259 remapeado
preempt.rs       112 L  — preemption via APIC timer
caps.rs          110 L  — sistema de capabilities OpenBSD-style
apic.rs          108 L  — Local APIC, timer periódico
scheduler.rs     105 L  — round-robin, spawn, yield
mb2.rs            97 L  — parser Multiboot2
heap.rs           90 L  — bump allocator, global_allocator
ring3.rs          88 L  — iretq a ring 3, enter_ring3_with_rax
vfs.rs            85 L  — VFS tmpfs plano
pipe.rs           82 L  — pipes IPC, FIFO circular
process.rs        80 L  — PCB, tabla de procesos, PIDs
serial.rs         60 L  — COM1, serial_println!
psf.rs            55 L  — parser PSF1
font.rs           50 L  — glifos 8×8 a mano
```

---

## 6. HISTORIAL DE MILESTONES {#milestones}

44 snapshots de desarrollo generados. En orden cronológico:

| Zip | Contenido añadido |
|---|---|
| `forge-os-m0-scaffold` | Boot Multiboot2, Long Mode, salto a Rust, serial |
| `forge-os-m1-caps` | Sistema de capabilities (caps.rs) |
| `forge-os-m1-product` | Docs PRODUCT.md, PHILOSOPHY.md |
| `forge-os-m1b-idt` | GDT/TSS + IDT + 6 handlers de excepción |
| `forge-os-m1c-pic` | PIC 8259 remapeado |
| `forge-os-m2-heap` | Parser Multiboot2 (mb2.rs), allocador físico bitmap (pmm.rs), heap bump (heap.rs) |
| `forge-os-m3-fb` | Framebuffer 1024×768, test_pattern 8 colores |
| `forge-os-m3b-text` | Fuente bitmap 8×8 a mano, draw_char, banner "FORGE OS BOOT OK" |
| `forge-os-console` | REPL de depuración sobre COM1 |
| `forge-os-anvil-design` | Docs DESKTOP.md — diseño de Anvil |
| `forge-os-roadmap` | ROADMAP.md, SHELL.md |
| `forge-os-todo` | TODO.md maestro |
| `forge-os-bsd-refs` | Referencias BSD integradas en PHILOSOPHY.md |
| `forge-os-reactos-lesson` | Análisis de ReactOS, documentado en ARCHITECTURE.md |
| `forge-os-compat` | COMPATIBILITY.md — estrategia de compatibilidad |
| `forge-os-languages` | LANGUAGES.md — ranking de lenguajes nativos |
| `forge-os-init` | Primera versión del sistema de inicialización |
| `forge-os-pci` | Enumeración PCI (pci.rs) |
| `forge-os-pci-bars` | Lectura de BARs PCI |
| `forge-os-keyboard` | Driver teclado PS/2, scancodes Set1 |
| `forge-os-m4a-tasks` | Tareas cooperativas, cambio de contexto real (task.rs + scheduler.rs) |
| `forge-os-m4b-apic` | Local APIC, timer periódico, primera activación de sti |
| `forge-os-m4c-mmu` | Gestión de memoria virtual (mmu.rs): map/unmap/create_address_space |
| `forge-os-m4d-procs` | Espacios de direcciones por proceso, spawn_with_space, switch CR3 |
| `forge-os-drivers` | Integración de todos los drivers |
| `forge-os-psf` | Parser PSF1 para fuente real (psf.rs) |
| `forge-os-ahci-read` | Driver AHCI lectura (ahci.rs) |
| `forge-os-ahci-write` | AHCI escritura + FLUSH CACHE |
| `forge-os-ahci-real` | Verificación AHCI contra código Linux real |
| `forge-os-verified-drivers` | AHCI + RTL8139 verificados contra fuentes reales |
| `forge-os-rtl8139-tx` | RTL8139 TX real (registros verificados contra Realtek PG oficial) |
| `forge-os-chromium-arch` | Análisis arquitectura Chromium para referencia |
| `forge-os-native-strategy` | Estrategia de porting nativo vs capa de compatibilidad |
| `forge-os-3dapi` | Análisis APIs 3D (TinyGL como atajo) |
| `forge-os-fileformats` | Análisis formatos de fichero (glTF, OOXML, etc.) |
| `forge-os-ytdlp-goal` | yt-dlp como meta de integración |
| `forge-os-ring3` | Transición a ring 3 via iretq (ring3.rs) |
| `forge-os-syscalls` | Handler naked syscall/sysret, SyscallFrame, SYS_PING |
| `forge-os-caps-connected` | caps::enforce conectado al dispatcher de syscalls real |
| `forge-os-ipc` | Pipes IPC FIFO (pipe.rs) |
| `forge-os-elf-vfs` | ELF64 loader (elf.rs), VFS tmpfs (vfs.rs) |
| `forge-os-ext2` | Filesystem ext2/ext4 read-only (ext2.rs) |
| `forge-os-partinfo-ext4-preempt` | Escáner particiones GPT/MBR (partinfo.rs), preemption APIC (preempt.rs) |
| `forge-os-fork-execve` | PCB (process.rs), clone_address_space, enter_ring3_with_rax, SYS_FORK/EXECVE/EXIT/GETPID |

*(la v1.2 del documento maestro añade dos milestones posteriores a este: `forge-os-m5-netrx` — RX completo del RTL8139 verificado en QEMU — y `forge-os-m6-ping` — stack ARP/ICMP con `net.rs` y ping real a 10.0.2.2 — ver nota en `TODO.md`)*

---

## 7. USERLAND PLANIFICADO {#userland}

Forge OS es kernel + userland. Los componentes de userland ya decididos:

| Componente | Descripción | Bloqueado por |
|---|---|---|
| **Ember** | Init / PID 1, modelo rc.d de FreeBSD | fork()/execve() completos |
| **Bellows** | Shell real (pipelines, redirección, job control) | M4 completo |
| **Crucible** | Terminal emulator, secuencias ANSI | Anvil (M6+) |
| **Anvil** | Compositor/WM unificado (ver sección 8) | HAL gráfico (M6+) |
| **mlibc** | Libc para OSes nuevos (en vez de escribir la nuestra) | VFS + procesos |
| **forge get / forge rm** | Gestor de paquetes, modelo ports+pkg FreeBSD | Bellows + mlibc |
| **Strata** | Gestor de disco/particiones tipo GParted | AHCI + VFS + Anvil |

### Metas de integración intermedias

**DOOM** (meta cercana)  
Precedente: BoredOS lo consiguió con doomgeneric + TinyGL.  
Necesita: M4c → M5 → mlibc → framebuffer (ya) + teclado (ya). Sin red, GPU, TLS.

**yt-dlp** (meta mayor — señal de que el sistema base está sólido)  
Necesita: M4 completo → M5 → stack de red con TLS → Python portado → ffmpeg opcional.

---

## 8. ESCRITORIO — ANVIL {#anvil}

### El principio central
Hyprland real usa 5 procesos separados (compositor, Waybar, rofi, mako, hyprpaper) hablando por IPC. **Anvil rechaza esto: un solo binario, un solo proceso, un solo event loop.**

Precedente validado: Hemera (nyxos-dev) funciona así. Anvil parte de ese patrón y le añade tiling de Hyprland.

### Capas dentro del mismo proceso
```
┌────────────────────────────────────────────────┐
│  Anvil (un solo proceso, un event loop)        │
│  1. Wallpaper       — fondo, siempre debajo    │
│  2. Tiling core     — BSP/dwindle, workspaces  │
│  3. Ventanas cliente — decoraciones, foco      │
│  4. Taskbar/panel   — sin IPC, mismo estado    │
│  5. Launcher overlay — capa modal              │
│  6. Notificaciones  — capa modal, no bloqueante│
└────────────────────────────────────────────────┘
```

### Comportamiento heredado de Hyprland
- Tiling dinámico **BSP/dwindle** por defecto
- **Workspaces** múltiples con layout independiente
- **Keybind-first** — Super+tecla como método principal
- Gaps configurables

### Layout de barras (diseño primera pasada)
- **Barra superior** (macOS/KDE): reloj, fecha, launcher, tray
- **Barra inferior** (Windows): taskbar + apps ancladas + menú de inicio

### Configuración
Un único fichero de texto plano — nada de Registry binario. Cubre WM + barra + wallpaper + keybinds + colores. Formato: TOML o formato propio tipo Hyprland (pendiente de decidir).

### Decisiones cerradas
- Layout: **BSP/dwindle**
- Nombre: **Anvil**
- Barras: dos (superior + inferior)

### Bloqueado por M6+ (HAL gráfico)

---

## 9. SHELL Y TERMINAL {#shell}

### Las tres piezas

**1. Consola de depuración** (buildable ya, M2+)  
REPL mínimo sobre COM1. Herramienta de bring-up, no el shell final. Vive en `console.rs`. `run-qemu.sh` con `-serial stdio` conecta el teclado real del host a COM1.

**2. Bellows** (shell real, bloqueado por M4)  
Pipelines, redirección `&&`/`||`/`;`, quoting, sustitución de comandos, job control, glob. Corre como proceso real en ring 3.  
El nombre viene de "fuelle de fragua" — el fuelle respira comandos hacia el sistema.

**3. Crucible** (terminal, bloqueado por M6+/Anvil)  
App gráfica que aloja Bellows: texto con scroll, cursor, colores ANSI, reenvío de teclas. Ventana dentro de Anvil.

---

## 10. MODELO DE SEGURIDAD {#seguridad}

### Sistema de capabilities (`caps.rs`)

Cada proceso arranca con una máscara de bits (`Capabilities(u32)`). `pledge()` solo puede *reducir* la máscara, nunca ampliarla. El dispatcher de syscalls llama `enforce(cap)` antes de cualquier operación.

| Cap | Cubre |
|---|---|
| `CAP_STDIO` | consola / puerto serie |
| `CAP_FS_READ` / `CAP_FS_WRITE` | VFS |
| `CAP_EXEC` | fork/exec de otros procesos |
| `CAP_NET` | sockets |
| `CAP_MEM_MAP` | mmap/sbrk más allá del heap inicial |
| `CAP_TIME` | relojes |
| `CAP_IPC` | mensajería entre procesos |
| `CAP_GFX` | framebuffer / GPU |
| `CAP_UNRESTRICTED` | admin total |

### Comando de administrador: `elevate`
No es `sudo` renombrado. Basado en capabilities: un proceso sin privilegios pide ampliar temporalmente su `Capabilities`, requiere autenticación interactiva, el kernel lo audita. **Todavía no implementado** (en TODO.md como ❌).

### privsep (patrón para demonios)
Todo servicio del sistema: proceso sin privilegios (trabajo real) + supervisor mínimo con privilegios, comunicados por canal estrecho y auditable. Forma estándar de construir servicios en Forge OS.

---

## 11. STACK DE RED {#red}

### Estado actual
- **RTL8139** — TX real implementado y verificado contra Realtek Programmer's Guide oficial
- **RX** — buffer preparado, sin bucle de extracción todavía (wraparound CAPR pendiente)

### Stack planificado (por orden de desbloqueo)
```
RTL8139 RX → ARP → IPv4 → ICMP (ping) → UDP → TCP
→ DHCP (cliente) → DNS → BSD sockets → WiFi (firmware vendor, más difícil)
```

### Referente
NyxOS tiene el stack TCP/IP completo con DNS, HTTP, TLS y el browser Selene que carga páginas reales. Es la referencia de hasta dónde hay que llegar.

---

## 12. FILESYSTEM {#filesystem}

### VFS tmpfs (implementado)
`vfs.rs` — namespace plano en memoria. `write()`, `read()`, `delete()`, `list()`. Sin persistencia ni jerarquía de directorios todavía.

### ext2/ext4 read-only (implementado)
`ext2.rs` — superbloque, grupos, inodos, resolución de rutas, árbol de extents ext4 (EXT4_EXTENT_MAGIC=0xF30A), descriptores de grupo 64-bit, feature flags.  
Comandos: `mount` / `e2ls` / `e2cat`  
Imágenes de prueba: ver `kernel/assets/EXT2_TEST_IMAGE.md`  
**Limitación actual**: solo bloques directos (12 punteros). Sin bloques indirectos → ficheros > 48 KiB no soportados.

### Escáner de particiones (implementado)
`partinfo.rs` — GPT (firma "EFI PART") + MBR fallback. Detecta: ext2/3/4 (magic 0xEF53), btrfs ("_BHRfS_M" @ LBA128), NTFS ("NTFS    " @ sector 0), FAT32. `FsType::installable()` → solo ext2/ext4.

### Pendiente
- Bloques indirectos ext2 (simple/doble/triple)
- Escritura ext2
- initramfs/tarfs
- /proc y /dev sintéticos
- Tabla de fds por proceso

---

## 13. COMPATIBILIDAD DE APPS {#compatibilidad}

### Estrategia preferida: port nativo
Precedente: BoredOS portó DOOM, TinyGL, TCC, Lua, kilo con "cambios menores" al no tener POSIX completo. La clave: **mlibc** (libc para OSes nuevos, con capa de abstracción) convierte el porting de meses a horas.

### Qué SÍ se puede portar nativamente
Software open source, autocontenido: DOOM, compiladores pequeños (TCC), intérpretes (Lua, Python), editores, herramientas CLI, VLC (candidato).

### Qué NO se puede portar
- **Discord, Spotify, Steam**: binarios cerrados o Electron sin fuente
- **Navegadores reales (Chromium/Firefox)**: millones de líneas, asumen infraestructura completa

### La otra vía: capa de compatibilidad Linux (futuro lejano)
Un "Linuxulator" propio, igual que FreeBSD. Prerrequisitos: muchas más syscalls Linux + enlazador dinámico + libc compatible + drivers GPU con Vulkan. Es tan grande como todo lo que llevamos construido — se aborda después de M6.

### Arquitectura Chromium — dos lecciones aplicables
1. **Separación por procesos** (browser vs renderer): encaja con `caps.rs` — renderer con Capabilities reducidas al mínimo
2. **`content/` como frontera de embedder**: misma idea de capas que Kernel→Executive→HAL

---

## 14. LENGUAJES NATIVOS {#lenguajes}

Dos cosas distintas: compilación cruzada (ya funciona) vs ejecución nativa in-OS (trabajo real).

| Lenguaje | Vía | Dificultad |
|---|---|---|
| **C** | TinyCC portado in-OS (precedente nyxos-dev) | Media |
| **Python** | CPython sin extensiones C | Media |
| **JavaScript** | QuickJS (no V8) | Media |
| **ffmpeg** | Build mínima, solo software | Media-alta |
| **C++** | GCC/Clang completo necesario | Alta |
| **HTML (renderizado)** | Motor de navegador real | Muy alta |
| **Rust (self-hosted)** | rustc depende de LLVM | Muy alta |
| **Java (JVM)** | GC, JIT, biblioteca enorme | Muy alta |

**Orden recomendado:** C → Python → QuickJS → ffmpeg mínimo

---

## 15. ROADMAP Y MEMORIA DE USO {#roadmap}

### Presupuesto de memoria (objetivos)

| Escenario | Objetivo |
|---|---|
| Kernel + consola, sin GUI | < 20 MB |
| Kernel + Anvil idle | < 100 MB |
| Anvil + gestor archivos + terminal + visor | < 250 MB |

*Referencia: Arch+Hyprland idlea 400-600 MB. ChromeOS 1-1.5 GB.*

### Mapa de features → milestone

| Feature | Dónde vive | Depende de |
|---|---|---|
| Reloj/horario | Driver RTC (CMOS) | Nada nuevo — pronto (M3c) |
| Gestor de archivos | App Anvil | VFS (M5) + Anvil (M6+) |
| Visor multimedia | App Anvil + decodificadores | VFS (M5) + Anvil (M6+) |
| Red Ethernet | Driver RTL8139 completo | HAL de red |
| Red WiFi | Driver de vendor + firmware | Fase posterior |
| Usuario/admin | caps.rs + PCB | M4 (tabla de procesos) |
| Comandos ls/cd/ps | Bellows + forge get | M4 |

### Metas de integración

**DOOM** → M4c + M5 + mlibc + framebuffer (ya) + teclado (ya)  
**yt-dlp** → M4 completo + M5 + red+TLS + Python + ffmpeg opcional

---

## 16. TODO MAESTRO {#todo}

Ver `TODO.md` — es el documento vivo, se mantiene aparte de este megadoc.

---

## 17. REFERENTES EXTERNOS {#referentes}

### NyxOS (@nyx_os_ / kazah-png)
- **Stack:** C + x86_64 NASM, v6.4.180
- **Lo que tienen:** TCP/IP completo + browser Selene (DNS+HTTP+TLS), compositor Hemera, 57 syscalls POSIX-like, ext2 R/W, TinyCC self-hosting, lenguaje propio "N", 32 ventanas, 4 workspaces, SMP, DOOM, Tetris, audio
- **Repo:** github.com/nyxos-dev/nyx-os (28 ⭐, 1235 commits)
- **Discord:** dsc.gg/nyxos
- **Lo que tenemos nosotros que ellos no tienen:** Rust (memoria segura), capabilities OpenBSD-style, AHCI propio, escáner de particiones, arquitectura más formal con docs

### CachyOS
- **Por qué importa:** referente de diseño para Static Linux y para las decisiones de Forge OS
- **Gestores de arranque ofrecidos:** systemd-boot (simple/MSI), rEFInd (multiboot), GRUB (LUKS), **Limine** (Multiboot2 nativo, Btrfs snapshots) → **Limine es nuestra elección**
- **Filesystems ofrecidos:** ext4, btrfs+Snapper, XFS, f2fs, ZFS → misma lista que Static Linux

### BoredOS
- Port nativo de DOOM con doomgeneric + TinyGL
- Usa mlibc como libc
- Precedente de que portar software con "cambios menores" es realista

### ReactOS
- Clean-room de NT
- Confirmó que la estructura Kernel→Executive→HAL es real y funciona
- `subsystems/csr/` + `subsystems/win/` → Executive agnóstico de API

---

## 18. ENTORNO DE DESARROLLO {#entorno}

| Campo | Valor |
|---|---|
| **PC de desarrollo** | AMD Ryzen 5 5600 + RTX 3060 Ti / Windows 11 |
| **IDE** | VS Code + Claude Code CLI |
| **Toolchain** | Rust nightly, target `x86_64-forge` custom |
| **Emulador** | QEMU (`run-qemu.sh`) |
| **Build** | `build.sh` → crea ISO con GRUB |
| **Ruta Windows** | `C:\Users\alfy1\proyectos\forge-os` |
| **Ruta CachyOS** | `/home/alfy/proyectos/forge-os` |

### Comando QEMU de referencia
```bash
qemu-system-x86_64 \
  -kernel kernel/forge-kernel.bin \
  -m 512M \
  -serial stdio \
  -no-reboot \
  -hda ext2-test.img \
  -nic user,model=rtl8139
```

### Verificación de drivers — práctica establecida
Antes de comprometerse a una tabla de offsets de registros, se busca la fuente real:
- AHCI: verificado contra `drivers/ata/ahci.h` del kernel Linux (GPL-2.0)
- RTL8139: verificado contra `8139too.c` Linux + Programmer's Guide oficial Realtek
- Los offsets son hechos de hardware, no opiniones de diseño

---

## 19. ERRORES CONOCIDOS Y LECCIONES {#errores}

### Arquitectura
- **Kernel flat, no higher-half:** `boot.asm` solo monta identity-map del primer GiB. El kernel flat es la solución correcta hasta tener QEMU real para validar el mapeo higher-half.
- **identity-map ampliado a 4 GiB:** necesario para llegar al Local APIC (~0xFEE00000).
- **Huge pages no partibles:** `mmu::map_page` falla explícitamente si el camino cruza una huge page, en vez de corromperla.

### Proceso / syscalls
- **SyscallFrame y el orden de push en el naked handler:** push rcx (user RIP) primero, luego r11 (RFLAGS), luego los args — este orden define la layout de `SyscallFrame`. Si se cambia el asm, hay que actualizar la struct.
- **USER_RSP_SCRATCH es un global:** solo soporta un syscall/fork en vuelo a la vez (single-core, sin preemption entre procesos). Anotado en comentarios.
- **FORK_CHILD_* son globals:** misma limitación — un fork() en vuelo a la vez. SMP lo rompería sin spinlocks.
- **fork() diverge del sysretq normal:** el hijo entra directamente con enter_ring3_with_rax desde una tarea kernel, el sysretq del naked handler nunca se ejecuta para él.
- **execve() diverge:** llama enter_ring3 directamente desde syscall_dispatch. La pila del syscall queda "abandonada" — se reutilizará en el siguiente syscall desde SYSCALL_STACK_TOP.

### Drivers
- **RTL8139 TX bit de status:** TxStatOK es bit 15 del registro TxStatus, confirmado contra el Programmer's Guide oficial. Muchos tutoriales lo tienen mal.
- **alloc_contiguous para RTL8139:** el anillo RX de RTL8139 necesita memoria física contigua (64KB+16 bytes). Añadido `pmm::alloc_contiguous(count)` para ello.
- **AHCI PRDT limitado:** máximo 8 sectores (4 KiB) por llamada con un solo descriptor PRDT. Transferencias grandes necesitan múltiples entradas.

### QEMU
- **run-qemu.sh modo interactivo:** `-serial stdio` conecta COM1 al teclado del host. Necesario para usar la consola de depuración.
- **Verificar en QEMU antes de seguir:** todo lo construido son borradores sin verificar. El próximo paso pendiente es compilar en CachyOS y arrancar en QEMU.

---

*Documento generado: Septiembre 2026 — Static Forge*  
*Última revisión: fork()/execve() implementados, PCB real, clone_address_space, 5158 líneas*
