# Arquitectura de Forge OS

Ver [PHILOSOPHY.md](PHILOSOPHY.md) para el razonamiento detrás de cada
decisión. Este documento es el mapa técnico.

## Nota de arquitectura: kernel flat, no higher-half (por ahora)

El diseño original planteaba un kernel higher-half (`0xFFFFFFFF80000000`,
patrón `PML4[511]` como en nyxos-dev). Al escribir M2 se detectó que
`boot.asm` (M0) solo monta identity-map del primer GiB — nunca mapea ese
rango higher-half — lo que habría causado un page fault inmediato al
saltar del boot a Rust. Corregido bajando el kernel a direcciones flat
(virtual == físico, `KERNEL_BASE = 0x100000`) en `targets/linker.ld`.
Migrar a higher-half real es tarea futura, deliberadamente pospuesta hasta
tener un ciclo de compilación+QEMU real para validar el mapeo de páginas
adicional sin adivinar aritmética de bits a ciegas.

## Verificación de registros contra fuente real

Desde AHCI/RTL8139: antes de comprometerse a una tabla de offsets de
registros de hardware "sacada de memoria", se busca la fuente real
(típicamente el driver correspondiente en `drivers/` del kernel de
Linux — GPL-2.0, público) y se contrasta. No se copia el código —
Linux usa su propio modelo de driver, no encaja aquí — pero los
offsets de registro son hechos de hardware, no opiniones de diseño, y
verificarlos así convierte "borrador sin verificar, riesgo alto" en
"offsets confirmados, solo la lógica de Rust por verificar". Práctica
a repetir en cada driver nuevo que toque registros de hardware.

## Precedente real: NT separa Executive de "personalidad" de subsistema

Confirmado explorando la estructura real de ReactOS (clean-room de NT):
`ntoskrnl/` tiene subcarpetas `ob/` (Object Manager), `mm/` (memoria),
`ke/` (kernel), `io/` (I/O Manager), `ps/` (procesos), `se/`
(seguridad), `lpc/` (IPC) — exactamente los nombres de componente que ya
planteábamos para nuestro Executive, no una idea abstracta nuestra.

Más importante: `subsystems/csr/` + `subsystems/win/` — el Executive de
NT es **agnóstico de API**. Win32 no vive dentro del kernel, corre como
servidor de subsistema en espacio de usuario encima de un Executive
genérico (históricamente NT también soportó personalidades POSIX y
OS/2 así, en paralelo). Es precedente real de 30+ años del mismo patrón
que planteamos en `COMPATIBILITY.md` para la futura capa de
compatibilidad Linux: un Executive propio, agnóstico, con "Bellows +
Anvil" como nuestra personalidad nativa y un futuro servidor de
subsistema Linux-compatible corriendo al lado, no dentro del kernel.

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
- **M1c** ✅ *borrador sin verificar* — PIC 8259 remapeado (IRQ0-7 →
  vectores 32-39, IRQ8-15 → 40-47), todo enmascarado hasta que existan
  drivers reales que las atiendan
- **M2** ✅ *borrador sin verificar* — parser mínimo de tags Multiboot2
  (`mb2.rs`), allocador físico de páginas por bitmap (`pmm.rs`), heap del
  kernel vía bump allocator (`heap.rs`) con `#[global_allocator]` real y
  prueba end-to-end (`Box::new`)
- **M2b** — heap real con free-list (recuperar memoria liberada, no solo
  avanzar un puntero)
- **M2c** — primer dispatcher de syscalls real (aquí `caps::enforce` se
  vuelve operativo)
- **M3** ✅ *borrador sin verificar* — tag de framebuffer pedido en
  `boot.asm` (1024×768@32bpp), parser + patrón de prueba de 8 barras de
  color (`framebuffer.rs`), comando `fb` en la consola para redibujar a
  demanda
- **M3b** ✅ *borrador sin verificar* — fuente bitmap 8x8 mínima diseñada
  a mano (`font.rs`, alfabeto parcial: F/O/R/G/E/S/B/T/K + espacio),
  `draw_char`/`draw_str` en `framebuffer.rs`, banner "FORGE OS BOOT OK"
  sobre el patrón de barras
- **M4a** ✅ *borrador sin verificar* — tareas cooperativas del kernel
  (`task.rs` + `scheduler.rs`), cambio de contexto real, demo de 2
  tareas intercaladas
- **M4b (primer paso)** ✅ *borrador sin verificar* — Local APIC +
  timer periódico (`apic.rs`), primera activación de interrupciones de
  hardware (`sti`) en todo el kernel. Identity-map de `boot.asm`
  ampliado de 1 a 4 GiB para alcanzar el Local APIC (~0xFEE00000)
- **M4b (continuación)** — conectar el timer con el scheduler para
  preemption real de verdad; necesita guardar el frame de registros
  completo desde un handler asíncrono, no solo callee-saved
- **M4c** — espacios de direcciones por tarea (necesita gestor de
  memoria virtual real); Object Manager; tabla de procesos con
  `Capabilities` por proceso
- **M5** — VFS + initramfs
- **M6+** — HAL de gráficos (fase 0-6 ya detallada en la conversación:
  framebuffer software → virtio-gpu → Intel real → AMD → NVIDIA vía
  firmware GSP → puerto ARM como proyecto hermano)

  > **Nota sobre la fase "Intel real":** QEMU no emula un motor de render
  > GPU a nivel de registros — solo dispositivos virtuales (virtio-gpu,
  > Bochs VBE). Esta fase necesita hardware Intel físico de verdad, y de
  > momento no hay uno asignado (pendiente de si llega un ThinkPad).
  > Hasta entonces, esta fase queda bloqueada; el resto del roadmap
  > (framebuffer, virtio-gpu) no depende de ella.

  > **Nota sobre APIs 3D (OpenGL/OpenGL ES/Vulkan):** Anvil (el
  > compositor 2D) no necesita ninguna de las tres — fill_rect/blit es
  > aceleración 2D pura, ya cubierta por la fase BLT de arriba. Una API
  > 3D solo hace falta para juegos/apps con geometría real, y es una
  > capa aparte, mucho más grande, encima del driver de GPU.
  >
  > **Atajo real (vía BoredOS):** portar **TinyGL** — un subconjunto de
  > OpenGL renderizado *por software*. Da 3D funcional sin necesitar
  > ningún driver de GPU, es lo que permite a BoredOS correr DOOM. Es
  > con diferencia el camino más corto a 3D real y debería intentarse
  > **antes** que cualquier driver de GPU con aceleración.
  >
  > Si algún día se aborda aceleración real: **OpenGL ES primero**
  > (spec más acotada, Mesa como referencia para verificar cada
  > función, mismo método que ya usamos con los registros de
  > AHCI/RTL8139) — Vulkan queda como aspiración mucho más lejana
  > (spec de miles de páginas; Mesa tardó años con equipo grande solo
  > en RADV), con AMDVLK como referencia cuando llegue ese momento.

## Convención de syscalls

Heredada de Linux x86_64: instrucción `syscall`, `RAX` = número, argumentos
en `RDI, RSI, RDX, R10, R8, R9` (System V con R10 sustituyendo a RCX),
retorno en `RAX` (negativo = error). Numeración propia, no calcada de
Linux — solo la convención de registro.
