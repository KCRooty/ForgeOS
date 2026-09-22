# TODO maestro — todo lo que falta para un OS completo

Organizado por capa. ✅ = ya construido (aunque sin verificar en QEMU).
Todo lo demás falta. Esto es el inventario completo, no una selección.

---

## 1. Núcleo del kernel (Ring 0, bajo nivel)

- ✅ Boot Multiboot2 → Long Mode
- ✅ GDT/TSS
- ✅ IDT + 6 excepciones de CPU
- ✅ PIC 8259 remapeado
- ✅ Allocador físico de páginas (bitmap)
- ✅ Heap del kernel (bump allocator v1, sin free real)
- ✅ Framebuffer + texto básico
- ❌ **Heap real con free-list** (M2b, ya anotado)
- ✅ **APIC local** *borrador sin verificar* — habilitado, timer
  periódico armado (`apic.rs`), primera activación real de
  interrupciones (`sti`) en todo el kernel, vector 0x40 registrado en
  la IDT. Requirió ampliar el identity-map de `boot.asm` de 1 a 4 GiB
  (el Local APIC vive en ~0xFEE00000, fuera del primer GiB)
- ❌ **I/O APIC** — solo tenemos Local APIC por ahora
- ❌ **Timer del sistema calibrado** — el `initial_count` del timer
  periódico es un valor arbitrario sin calibrar contra PIT/TSC; no
  corresponde a una frecuencia real en Hz todavía
- ❌ **Driver RTC** (reloj de pared — hora/fecha real, ya mencionado)
- ❌ **Gestor de memoria virtual completo** — hoy solo hay identity-map
  fijo montado en boot.asm; falta una API real desde Rust para
  mapear/desmapear páginas arbitrarias, imprescindible para procesos
  con su propio espacio de direcciones
- ❌ **Primitivas de sincronización** — spinlocks, mutex, semáforos;
  sin esto, SMP y cualquier estructura compartida son una bomba de
  relojería
- ❌ **SMP** — arranque de cores adicionales (INIT/SIPI), estructuras
  per-CPU, scheduler independiente por core
- ✅ **PCI enumeration** *borrador sin verificar* — mecanismo clásico
  (puertos 0xCF8/0xCFC), recorrido de 256 buses × 32 dispositivos × 8
  funciones, comando `pci` en la consola de depuración

## 2. Procesos, scheduling y syscalls reales (M4)

- ✅ **M4a: tareas cooperativas del kernel** (`task.rs` + `scheduler.rs`)
  *borrador sin verificar* — cambio de contexto real (`switch_to`, asm
  `#[naked]`), scheduler round-robin, demo de 2 tareas intercaladas.
  **Todavía sin espacio de direcciones propio por tarea** (eso es M4c)
- ❌ **PCB completo** (más allá de la `Task` mínima de M4a — falta
  vincular `Capabilities`, estado de proceso completo, etc.)
- ❌ **Scheduler** — round-robin como mínimo, weighted más adelante
- ❌ **Context switch** — guardar/restaurar registros, FPU/SSE state
- ❌ **fork() / execve() reales** — con carga de ELF64 propio
- ❌ **wait()/exit()** — recolección de procesos zombie
- ❌ **Señales** (signals) — al menos SIGKILL/SIGTERM/SIGSEGV
- ❌ **Dispatcher de syscalls real** — el punto donde `caps::enforce`
  se vuelve operativo por primera vez, no solo demo
- ❌ **IPC** — pipes como mínimo; mailbox/mensajería como en
  Asmodeus14/Nyx es buen precedente
- ❌ **Memoria compartida (SHM)** — necesaria más adelante para el
  compositor gráfico (ventanas cliente)
- ❌ **Ember** (nombre propuesto) — sistema de inicialización, PID 1.
  Primer proceso de userland arrancado por el kernel tras M4c; lanza el
  resto de servicios/demonios en orden de dependencias. Modelo
  **rc.d de FreeBSD** (scripts secuenciales simples, no systemd) — cada
  demonio construido con el patrón **privsep** ya documentado en
  `PHILOSOPHY.md` (proceso sin privilegios + supervisor mínimo). Sin
  esto, el kernel arranca pero nada de userland (red, login, Anvil)
  tiene forma de encenderse solo.

## 3. Filesystem (M5)

- ❌ **VFS** — capa de abstracción, trait `FileSystem`, tabla de montaje
- ❌ **Initramfs/tarfs** — para arrancar userland antes de tener disco
  real montado (patrón usado por los dos Nyx)
- ❌ **Filesystem persistente real** — ext2 es la opción pragmática
  (compatible con herramientas externas de Linux para depurar discos
  desde fuera), o diseñar uno propio — decisión pendiente. **Aspiración
  a largo plazo inspirada en ZFS** (FreeBSD): bloques con checksum
  (detección de corrupción de datos) y snapshots copy-on-write — no en
  v1, pero como dirección de diseño del filesystem propio si se decide
  no usar ext2 tal cual.
- ❌ **/proc y /dev sintéticos** — Windows y Linux los dan por hecho
  (info de procesos navegable, nodos de dispositivo) — sin esto no se
  siente "como Windows y Linux" ni de lejos
- ❌ **Tabla de descriptores de fichero por proceso**

## 4. HAL — drivers de hardware

- ❌ **Almacenamiento**: AHCI (SATA), luego NVMe
- ❌ **Red**: driver Ethernet (RTL8139, emulable en QEMU) → ARP/IP/ICMP
  → UDP/TCP → DHCP/DNS → sockets BSD (`socket`/`connect`/`bind`/...) →
  WiFi (mucho más difícil, firmware de vendor)
- ❌ **USB**: xHCI → HID (teclado/ratón USB, no solo PS/2) → almacenamiento
  masivo USB
- ✅ **Input PS/2 (teclado)** *borrador sin verificar* — scancodes Set
  1, layout US QWERTY (`keyboard.rs`), IRQ1 vía APIC/PIC, buffer
  circular, integrado en la consola de depuración junto al puerto serie
- ❌ **Input PS/2 (ratón)** — solo teclado por ahora
- ❌ **Audio**: Sound Blaster 16 (emulable en QEMU) → HDA (hardware real
  moderno)
- ❌ **ACPI**: parsing de tablas, apagado/reinicio limpio, gestión
  térmica — sin esto no hay ni siquiera un `shutdown` decente
- ❌ **Gráficos**: todo el roadmap de `ARCHITECTURE.md` (framebuffer →
  virtio-gpu → Intel real → AMD → NVIDIA → ARM)

## 5. Seguridad, usuarios y autenticación

- ❌ **Primitivas criptográficas** — SHA-256 como mínimo (para hashear
  contraseñas, verificar paquetes); AES si se quiere algo cifrado en
  disco más adelante
- ❌ **RNG real** — RDRAND/RDSEED hardware, o CSPRNG si no está disponible
- ❌ **Cuentas de usuario** — almacenamiento de credenciales, login
- ❌ **Pantalla de login** (Anvil)
- ❌ **`elevate`** — escalar a admin vía capabilities ampliadas,
  autenticación interactiva, auditoría de cada escalada
- ❌ **Logging/auditoría** — quién hizo qué y cuándo, básico pero real

## 6. Userland, toolchain y shell

- ❌ **crt0** — arranque de proceso userland, `argv`/`envp`
- ❌ **libc propia** — malloc/free, `str*`/`mem*`, printf/snprintf,
  stdio con buffer, ctype
- ❌ **~50-60 coreutils** — `ls`, `cat`, `cp`, `mv`, `rm`, `ps`, `top`,
  etc. (referencia directa: los coreutils de nyxos-dev)
- ❌ **Bellows** (el shell real, M4+) — pipelines, redirección, job
  control, `&&`/`||`/`;`, quoting, sustitución de comandos
- ❌ **Gestor de paquetes** (`forge get`/`forge rm`) — modelo ports+pkg
  de FreeBSD ya decidido, ver `ROADMAP.md`
- ❌ **`dlopen`/`dlsym`** — enlazado dinámico, si se quiere en algún
  momento

## 7. Escritorio — Anvil (M6+)

- ❌ **Compositor core** — ventanas, z-order, drag/resize, damage tracking
- ❌ **Tiling BSP/dwindle** (ya decidido)
- ❌ **Dos barras** (superior macOS/KDE-style, inferior Windows-style —
  ya decidido)
- ❌ **Wallpaper**
- ❌ **Launcher / menú de inicio**
- ❌ **Notificaciones**
- ❌ **Syscalls de ventana** — diseño propio tipo `SYS_WIN_CREATE`/
  `PRESENT`/`POLL_EVENT` (referencia: nyxos-dev)
- ❌ **Cursor de ratón** — renderizado, no solo posición
- ❌ **Fuente proporcional real** — el 8x8 de bloque actual es solo
  para debug; hace falta algo tipo TTF más adelante (como hizo
  nyxos-dev con DejaVu Sans) o al menos completar el alfabeto bitmap
- ❌ **Portapapeles** (clipboard)

## 8. Apps (todas viven dentro de Anvil, M6+)

- ❌ **Crucible** (terminal)
- ❌ **Gestor de archivos**
- ❌ **Editor de texto**
- ❌ **Visor de imágenes** (necesita decodificadores PNG/BMP/GIF mínimo)
- ❌ **Reproductor multimedia básico** — antes de pensar en VLC
- ❌ **Ajustes del sistema**
- ❌ **Monitor del sistema** (CPU/memoria/procesos — tipo Task Manager)
- ❌ **Gestor de red** (UI sobre el stack de red del punto 4)
- ❌ **Calculadora** (trivial pero esperado)

## 9. Multimedia (transversal)

- ❌ **Decodificadores de imagen propios** — PNG/BMP/GIF mínimo, JPEG
  más adelante (referencia: nyxos-dev tiene los cuatro desde cero)
- ❌ **Pipeline de audio** — reproducción básica sobre el driver de
  audio del punto 4
- ❌ **Decodificación de vídeo** — mucho más difícil, fuera de alcance
  cercano (ver `COMPATIBILITY.md` para VLC como candidato de port nativo)

## 10. Estabilidad y QA

- ❌ **Batería de tests KAT en CI** — principio ya escrito en
  `PHILOSOPHY.md`, pero no hay ni un solo test automatizado todavía
- ❌ **Pantalla de pánico gráfica** — hoy un panic solo cuelga y loguea
  por serie; Windows tiene BSOD, Linux tiene el panic de consola, los
  dos Nyx tienen pantalla de pánico gráfica — nosotros no tenemos nada
  visual todavía

## 11. Instalador (esto no lo habías pedido, pero hace falta)

- ❌ **Instalador real a disco persistente** — hoy Forge OS solo
  arranca en vivo desde el ISO; no hay forma de instalarlo de forma
  permanente en una máquina. Sin esto no es un "sistema operativo" en
  el sentido que tú quieres, es un live-CD para siempre.
- ❌ **Parser de tabla de particiones** (GPT/MBR)
- ❌ **Instalación del bootloader** en el disco de destino
- ❌ **Strata** — gestor de disco/particiones tipo GParted (nombre
  propuesto). Cadena de dependencias real, de abajo a arriba: driver de
  almacenamiento (AHCI/NVMe, sección 4) → parser GPT/MBR → formateo de
  filesystems (mínimo el nuestro, idealmente también FAT32/ext4/NTFS de
  solo-lectura para interoperar con otros OS al lado) → UI (Anvil,
  M6+). Versión mínima en modo texto sobre la consola de depuración es
  factible bastante antes que la versión gráfica.

## 12. Cosas que no has pedido pero "como Windows y Linux" las exige

- ❌ **Localización / distribución de teclado** — español, dado que
  trabajas en español/catalán (nyxos-dev incluso lo tiene documentado
  en su `nyxfetch` — "Keymap: Spanish (ES)")
- ❌ **Zonas horarias**
- ❌ **Mecanismo de actualización del propio sistema operativo**
- ❌ **Snapshots/backups del sistema** (tipo Time Machine o restore
  points de Windows) — nice-to-have, no bloqueante
- ❌ **Arranque seguro / cadena de confianza** — encaja con el perfil
  "Hardened" ya definido en `PRODUCT.md`

---

## Por dónde seguir

Casi todo lo de las secciones 2-9 depende, directa o indirectamente, de
**M4 (procesos y scheduler)** — es el bloqueante más grande de todo el
tablero. Sin procesos reales no hay shell real, no hay apps, no hay
gestor de paquetes, no hay nada que "corra" de verdad más allá de lo que
ya hace el propio kernel.

## Meta de integración concreta: yt-dlp

Un objetivo tangible para saber cuándo el sistema esencial está sólido:
que `yt-dlp` corra de verdad en Forge OS. No necesita GPU, motor de
navegador, ni sandboxing complejo — pero sí toca casi toda la pila
esencial de golpe: M4 completo (procesos reales) → M5 (VFS/filesystem) →
stack de red completo con TLS → Python portado (ver `LANGUAGES.md`) →
ffmpeg opcional para calidad completa. El día que esto corra, es la señal
de que el sistema base funciona de punta a punta.
