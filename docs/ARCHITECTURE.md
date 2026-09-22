# Arquitectura de Forge OS

Ver [PHILOSOPHY.md](PHILOSOPHY.md) para el razonamiento detrás de cada
decisión. Este documento es el mapa técnico.

## Las tres capas

**Kernel** (`kernel/src/`, módulos base) — scheduler, interrupciones, locks,
GDT/IDT. Se mantiene deliberadamente pequeño; casi nada nuevo entra aquí
después de M4.

**Executive** — Object Manager (handles + refcounting unificado), I/O
Manager (VFS + dispatch de syscalls con enforcement de capabilities),
Process Manager (tabla de procesos, cada uno con su `Capabilities`
congelada). Es donde vive la mayoría de la lógica nueva a partir de M2.

**HAL** — todo lo específico de hardware (GPU por vendor, red, storage)
detrás de traits. Nada de la Executive conoce el vendor concreto.

## Modelo de seguridad: capability gating

`kernel/src/caps.rs` — cada proceso arranca con una máscara de bits
(`Capabilities`). `pledge()` solo puede *reducir* la máscara, nunca
ampliarla; intentarlo es un error. El dispatcher de syscalls (M2+) llama
`enforce(cap)` antes de ejecutar cualquier operación — denegado significa
matar el proceso, no devolver un error silencioso y seguir.

Categorías actuales (`CapMask`, `u32`, una por bit):

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

## Roadmap de milestones

- **M0** ✅ — boot Multiboot2 → Long Mode → Rust, log por serie
- **M1** (en curso) — capability gating (`caps.rs`) ✅, GDT/TSS propias
  (`gdt.rs`) ✅ *borrador sin verificar*, IDT + handlers de excepción
  (`idt.rs`) ✅ *borrador sin verificar* — `#0` divide error, `#3`
  breakpoint, `#6` invalid opcode, `#8` double fault en pila IST separada,
  `#13` GPF, `#14` page fault (con lectura de CR2)
- **M1c** — PIC 8259 remapeado (mover IRQs de hardware fuera del rango
  0-31 que usan las excepciones de CPU)
- **M2** — allocador físico de páginas, heap del kernel, primer dispatcher
  de syscalls real (aquí `caps::enforce` se vuelve operativo)
- **M3** — framebuffer (tag de vídeo Multiboot2) + texto en pantalla
- **M4** — scheduler + tabla de procesos con `Capabilities` por proceso;
  Object Manager
- **M5** — VFS + initramfs
- **M6+** — HAL de gráficos (fase 0-6 ya detallada en la conversación:
  framebuffer software → virtio-gpu → Intel real en el Chuwi → AMD → NVIDIA
  vía firmware GSP → puerto ARM como proyecto hermano)

## Convención de syscalls

Heredada de Linux x86_64: instrucción `syscall`, `RAX` = número, argumentos
en `RDI, RSI, RDX, R10, R8, R9` (System V con R10 sustituyendo a RCX),
retorno en `RAX` (negativo = error). Numeración propia, no calcada de
Linux — solo la convención de registro.
