# TODO maestro — todo lo que falta para un OS completo

Organizado por capa. ✅ = ya construido (aunque sin verificar en QEMU,
salvo que se indique lo contrario). Todo lo demás falta. Esto es el
inventario completo, no una selección.

**Actualización — primer boot real (Claude Code, sobre este repo):**
el kernel compila y arranca de verdad en QEMU por primera vez. Llega
hasta la consola de depuración (`forge>`) con MMU, espacio de
direcciones por proceso (CR3 real), VFS y pipes confirmados
funcionando, no solo revisados por lectura.

**Actualización — `process.rs`, fork/execve/exit/getpid reales
(Claude Code, verificado en QEMU):** reconstruidos desde cero (el zip
`fork-execve` original no llegó a recuperarse) y verificados de
extremo a extremo con los comandos de consola `forktest`/`exec`/`ps`:
`fork()` clona el espacio de direcciones de verdad (copia profunda,
`mmu::clone_address_space`), el hijo arranca como tarea nueva del
scheduler vía `ring3::enter_ring3_with_rax` (rax=0 para el hijo),
`getpid()`/`ping()`/`exit()` funcionan en ambos, `execve()` reemplaza
el proceso de verdad cargando un binario nuevo desde el VFS. `ps`
muestra la tabla de procesos real con PID/PPID/estado.

En el camino aparecieron y se arreglaron cuatro bugs reales, todos
preexistentes (nunca se había ejecutado nada de esto en QEMU antes de
esta sesión):
- `mmu::map_page` ponía el bit NO_EXECUTE sin que `EFER.NXE` estuviera
  habilitado — page fault por reserved-bit violation (`mmu::init()`).
- Ningún mapeo de `mmu.rs` ponía nunca el bit `PAGE_USER` — cualquier
  acceso desde ring 3 fallaba (faltaba en TODOS los niveles de la
  jerarquía, no solo en la hoja).
- `mmu::translate()` le faltaba el último nivel de indirección: caminaba
  P4→P3→P2 y trataba el puntero a la tabla P1 como si ya fuera la
  página de datos final. Efecto real: `copy_into_segment` escribía el
  contenido de cada ELF cargado ENCIMA de su propia tabla de páginas en
  vez de en la página de código — silencioso hasta que algo intentaba
  ejecutar el ELF cargado.
- `sys_exit()` no restauraba las interrupciones (`sti`) antes de ceder
  el control para siempre — `syscall_entry` las enmascara al entrar
  (FMASK) y normalmente se restauran solo al volver por `sysretq`, pero
  `exit()` diverge a propósito y nunca llega ahí. Sin el `sti`, el
  `hlt` de la consola de depuración se quedaba colgado para siempre en
  cuanto cualquier proceso llamaba a `exit()` — la CPU invitada seguía
  viva (QEMU no crasheaba), simplemente parada sin forma de despertar.
- Los `TEST_ELF*` usaban `vaddr=0x400000` (convención clásica), que
  cae dentro del identity-map real de `boot.asm` (0-4 GiB) — colisión
  con memoria ya mapeada del propio kernel. Movidos a 640 GiB
  (`0x0000_00A0_0000_0000`, índice P4=1) — ver la nota larga en
  `elf.rs` sobre por qué cualquier dirección por debajo de 512 GiB
  (`1<<39`) es, en la práctica, compartida entre todos los procesos.

**Actualización — `wait()` real + liberación de memoria (Claude Code,
verificado en QEMU):** ver sección 2 más abajo. Tres bugs más
encontrados al implementarlo, todos preexistentes:

- **Pila de kernel única compartida entre TODOS los procesos**
  (`KERNEL_SYSCALL_STACK`, un solo global). Inofensivo mientras una
  syscall fuera un viaje de ida (`exit()`, nunca se reanuda), pero
  corrompía cualquier syscall que SÍ esperara reanudarse más tarde: si
  el padre cedía el turno dentro de `wait()` y el hijo hacía sus
  propias syscalls mientras tanto, la pila compartida quedaba pisada
  justo donde el padre tenía su contexto guardado. Arreglado con una
  pila de kernel PROPIA por tarea (`Task::kernel_stack_top`, activada
  por `scheduler::yield_now()` igual que CR3).
- **`process::current_pid()` no se restauraba al cambiar de tarea** —
  solo se fijaba una vez, en el trampolín de cada proceso. Un hijo
  terminando mientras el padre seguía pausado dejaba `current_pid()`
  pegado al PID del hijo (ya reapeado) cuando el padre se reanudaba.
  Arreglado asociando el PID a la propia `Task` (`Task::pid`) y
  restaurándolo en `yield_now()`.
- **La pila de kernel por-tarea nueva no estaba alineada a 16 bytes** —
  usada tal cual como RSP en `syscall_entry` (`mov rsp, [stack_top]`),
  pero `Vec<u8>` pide alineación 1 al allocator; sin forzar `& !0xF`,
  el `call {handler}` de después rompía la convención SysV justo en la
  entrada de la función llamada. Se manifestaba como corrupción de
  memoria aleatoria (PIDs y punteros con basura, paniques de "index out
  of range") en cuanto se encadenaban varios procesos en la misma
  sesión (`forktest` seguido de `exec`, por ejemplo) — la pila estática
  compartida original tampoco tenía esta garantía (ahora con
  `#[repr(align(16))]` explícito, por si acaso).

**Actualización — red funcional de extremo a extremo (Claude Code,
verificado en QEMU):** milestones `m5-netrx` y `m6-ping` reconstruidos
desde cero (mismo caso que `fork-execve`: los zips originales no
llegaron a recuperarse) y verificados con el comando de consola `ping`,
que hace un ARP request + ICMP echo request reales contra el gateway de
slirp de QEMU (10.0.2.2) y confirma la respuesta de vuelta, dos veces
seguidas en la misma sesión sin fallos.

- `pci.rs`: `write_config`/`write_config_u16`/`enable_bus_master` — sin
  esto el chip nunca inicia DMA por su cuenta, aunque los descriptores
  TX/RX estén bien configurados (Bus Master Enable, bit 2 del registro
  Command).
- `rtl8139.rs`: `poll_rx()` — extracción real de paquetes del anillo de
  recepción: cabecera de 4 bytes (status + longitud), copia del
  payload descontando la FCS, avance de `rx_cur` alineado a dword con
  wraparound en `RX_LOGICAL_SIZE`, y el ajuste de `CAPR = rx_cur - 16`
  que exige el hardware (mismo offset que usa Linux, no es libre
  elección nuestra).
- `net.rs` (nuevo): `inet_csum` (RFC 1071), tabla ARP mínima (array
  estático de 8 entradas), `build_arp_request`/`parse_arp_reply`,
  `build_ping`/`parse_icmp_echo_reply`.
- Comando `ping` en la consola: ARP request → espera bloqueante
  acotada (sondeo de `poll_rx`, sin preemption real todavía) → ICMP
  echo request → espera acotada de la respuesta → reporta round-trip.

**Descubrimiento en el camino, no un bug de nuestro código:** el modelo
de NIC que QEMU expone por defecto sin flags de red explícitas es un
`e1000`, no un `rtl8139` — en este host, sin `-nic user,model=rtl8139`,
`rtl8139::find_controller()` no encontraba nada (comportamiento
correcto: sencillamente no había ningún RTL8139 en el bus). Corregido
en `tools/run-qemu.sh`, que ahora siempre pasa esa flag explícita en
vez de confiar en el modelo por defecto de la instalación de QEMU de
turno.

**Actualización — Ember, primer boceto de PID 1 (Claude Code, verificado
en QEMU):** modelo rc.d de FreeBSD (secuencial, cada servicio termina
del todo antes del siguiente — no paralelo tipo systemd). `TEST_ELF_EMBER`
(`elf.rs`) arranca dos "servicios" (`ember-svc0`/`ember-svc1`, hoy
placeholders de `TEST_ELF_PING_EXIT` registrados en el VFS por el
comando `ember` de la consola) en orden: `fork()` → hijo `execve()` →
padre `wait()` + `ping()` del PID reapeado, dos veces seguidas dentro
del mismo proceso. Confirmado con `ps`: los dos servicios desaparecen
(memoria liberada de verdad), Ember mismo queda `zombie(code=0)` —
nadie lo espera, se lanzó directo desde la consola con PPID 0, como
`forktest`. Probado además lanzando `ember` dos veces seguidas en la
misma sesión sin fallos (PIDs correctamente reutilizados).

**Limitación documentada a propósito:** un PID 1 real no termina
nunca — se queda vivo para reapear huérfanos y relanzar servicios
caídos. Éste sí llama a `exit()` al final de su secuencia de arranque:
sin preemption real (`preempt.rs`, sigue pendiente), un bucle infinito
en ring 3 monopolizaría la única CPU cooperativa para siempre y la
consola de depuración no volvería a responder. Cuando `preempt.rs`
exista, Ember pasa a quedarse vivo de verdad — pieza aparte. Tampoco
hay parseo de scripts de shell todavía (eso es Bellows) — la "secuencia
de servicios" hoy es una lista fija hecha a mano en el propio binario,
no ficheros `/etc/rc.d/*` leídos en tiempo de arranque.

**Bug real encontrado (no de Ember, preexistente — `m4i-fork-execve`):**
`sys_execve()` mapeaba la pila de usuario del proceso reemplazado en
`0x0000_0070_0000_0000` (448 GiB), **por debajo** del límite de 512 GiB
documentado en `elf.rs` (el punto donde el índice P4 deja de ser
compartido). El comentario original decía "privado del nuevo espacio",
pero no lo era — era P4[0], compartido por TODO el kernel. Invisible
hasta ahora porque ningún binario había llamado a `execve()` dos veces
en el mismo arranque: la primera llamada dejaba la página mapeada para
siempre en el P4[0] compartido (`free_address_space` nunca lo toca, a
propósito — es la tabla del kernel), y la segunda chocaba contra esa
misma dirección ya ocupada (`"ya había una página mapeada en esa
dirección virtual"`). `ember` es el primer caso real que ejercita esto
(dos rondas de `execve()` en el mismo proceso). Movido a
`0x0000_00C0_0000_0000` (768 GiB, índice P4 = 1, genuinamente privado).

**Actualización — ext2 real de solo lectura (Claude Code, verificado en
QEMU contra un ext2 de verdad):** `ext2.rs` (nuevo) monta un ext2
clásico (rev1, sin extents/64bit/metadata_csum — eso es ext4), resuelve
rutas absolutas, lista directorios y lee ficheros completos con
punteros directos **y** el indirecto simple. Cada campo del
superbloque/descriptor de grupo/inodo se lee por offset de byte
explícito (`from_le_bytes`, sin `#[repr(C)]`) para no repetir el susto
de padding del target-spec JSON. Comandos de consola `ext2ls`/`ext2cat`.

**Verificación real, no solo "compila":** `tools/make-test-disk.sh`
(nuevo) genera `disk.img` con `mkfs.ext2`+`debugfs` de verdad (fuera de
Forge OS, en el host) — un ext2 real hecho por herramientas de Linux
estándar, no por nuestro propio código, así que confirma que *leemos*
el formato de verdad, no solo que somos consistentes con nosotros
mismos. Contenido de prueba: `hello.txt` (texto corto), `sub/nested.txt`
(subdirectorio), y `big.bin` (20 KiB con patrón determinista) — a
propósito más grande que los 12 bloques directos con bloque de 1 KiB,
para ejercitar el puntero indirecto simple de verdad. `ext2cat
/big.bin` se comparó byte a byte contra el original: coincide exacto.
`tools/run-qemu.sh` adjunta `disk.img` por AHCI automáticamente si
existe (`-device ahci,id=ahci0` + `-drive`/`-device ide-hd`).

**Bonus no buscado:** con un disco de verdad adjunto por primera vez en
todo el proyecto, tanto `ahci.rs` (detección + IDENTIFY + lectura del
MBR) como el comando `disktest` (escritura+relectura real) quedan
verificados en QEMU por primera vez — antes decían "sin verificar" de
buena fe, nunca habían tenido un disco SATA real delante.

**Limitaciones documentadas a propósito:** sin doble/triple indirecto
(ficheros más allá de ~268 bloques con bloque de 1 KiB dan error
explícito, no truncan en silencio); sin tabla de particiones — asume
que el ext2 empieza en el LBA 0 del disco entero (`partinfo.rs` es
milestone aparte); solo lectura, sin journal (ext3/ext4 real de verdad
es aparte).

**Actualización — `partinfo.rs`, escáner GPT/MBR + detección de
filesystem (Claude Code, verificado en QEMU contra tablas de
particiones y filesystems reales):** lee MBR clásico o GPT (detecta
GPT vía el MBR protector, tipo 0xEE), y para cada partición identifica
el filesystem por firma real en su contenido — no por el byte de tipo
de la tabla, que solo se reporta como dato informativo aparte. Comando
`partinfo`.

**Verificación real, no solo "compila":** `tools/make-partinfo-test-
disks.sh` (nuevo) genera dos discos con `parted`+`losetup`+`mkfs.*`
reales (fuera de Forge OS): `partinfo-mbr.img` (MBR, ext4+fat32) y
`partinfo-gpt.img` (GPT, las 6 combinaciones que sabemos distinguir:
ext2/ext3/ext4/fat32/ntfs/btrfs). Cada offset (superbloque ext2 a
+1024, `"FAT32   "` a +82, `"NTFS    "` a +3, magic de btrfs a
+0x10040, cabecera GPT en LBA 1 con su tabla de entradas) se contrastó
primero leyendo los bytes crudos de las imágenes reales con Python
antes de escribir una sola línea de Rust — los 8 escenarios (2 MBR + 6
GPT) salieron correctos a la primera en QEMU.

**Encontrado por el camino (infraestructura de pruebas, no un bug de
Forge OS):** con un disco que tiene una firma MBR válida (0x55AA) de
verdad, SeaBIOS intenta arrancar desde el disco SATA antes que desde
el CD-ROM y se queda colgado ahí — sin llegar siquiera a GRUB, cero
salida por serie. El `disk.img` de `ext2.rs` nunca lo disparó porque no
tiene tabla de particiones (sector 0 a ceros). Arreglado añadiendo
`-boot order=d` a `tools/run-qemu.sh` — fuerza arrancar del CD-ROM
siempre, sin importar qué disco SATA esté adjunto.

---

## 0. Reconstrucción pendiente (importado desde el histórico de chat)

Este repo se importó a partir de 37 snapshots de desarrollo pasados desde
Claude web (ver `docs/MEGADOC.md` para el documento maestro completo). El
propio megadoc (v1.2) documenta milestones posteriores al último snapshot
que se pudo importar (`rtl8139-tx`) para los que no llegó a recuperarse el
zip correspondiente. `process.rs` y fork/execve/exit/getpid (milestone
`fork-execve`) ya se reconstruyeron y verificaron — ver arriba. Queda:

- ✅ **`partinfo.rs`** — escáner de particiones GPT/MBR, detección de FS
  por firma real (ext2/3/4, btrfs, NTFS, FAT32) — parte `partinfo` del
  milestone `partinfo-ext4-preempt` ya reconstruida y verificada, ver
  arriba (`preempt.rs`, la otra mitad, sigue pendiente, justo abajo)
- ❌ **`preempt.rs`** — preemption real vía timer APIC, separado de M4b
  (milestone `partinfo-ext4-preempt`)
- ✅ **`ext2.rs`** — ext2 clásico real de solo lectura, punteros directos
  + indirecto simple (milestone `ext2`) ya reconstruido y verificado —
  ver arriba. **Sin árbol de extents** (eso es ext4, fuera de alcance
  de esta pasada) ni doble/triple indirecto todavía
- ✅ **RTL8139 RX real** — bucle de extracción de paquetes del anillo,
  wraparound de `CAPR` (milestone `m5-netrx`) ya reconstruido y
  verificado — ver arriba
- ✅ **`pci.rs`: `write_config`/`write_config_u16`** — Bus Master Enable
  (milestone `m5-netrx`) ya reconstruido y verificado — ver arriba
- ✅ **`net.rs`** — tabla ARP, `inet_csum` (RFC 1071), `build_ping`,
  `parse_icmp_echo_reply` — ping ICMP real verificado en QEMU contra
  10.0.2.2 (milestone `m6-ping`) ya reconstruido y verificado — ver
  arriba
- ❌ **AHCI/RTL8139 "verified-drivers"** — pasada de doble verificación
  de ambos drivers contra fuentes oficiales, más allá de lo ya integrado

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
- ✅ **Gestor de memoria virtual (M4c)** *borrador sin verificar* —
  `mmu.rs`: `map_page`/`unmap_page` sobre la jerarquía de páginas
  activa, creación de tablas intermedias sobre la marcha, probado con
  escritura+lectura real fuera del identity-map de 4 GiB. **Limitación:**
  no parte huge pages existentes — falla explícitamente si el camino
  las cruza, en vez de corromperlas
- ❌ **Espacios de direcciones por proceso** — `map_page` ya soporta
  operar sobre un CR3 ajeno al activo; falta la lógica de
  crear/cambiar espacios de direcciones completos
- ❌ **Primitivas de sincronización** — spinlocks, mutex, semáforos;
  sin esto, SMP y cualquier estructura compartida son una bomba de
  relojería
- ❌ **SMP** — arranque de cores adicionales (INIT/SIPI), estructuras
  per-CPU, scheduler independiente por core
- ✅ **PCI enumeration** *borrador sin verificar* — mecanismo clásico
  (puertos 0xCF8/0xCFC), recorrido de 256 buses × 32 dispositivos × 8
  funciones, lectura de BARs (BAR0-BAR5, MMIO vs I/O port), comando
  `pci` en la consola de depuración

## 2. Procesos, scheduling y syscalls reales (M4)

- ✅ **M4a: tareas cooperativas del kernel** (`task.rs` + `scheduler.rs`)
  *borrador sin verificar* — cambio de contexto real (`switch_to`, asm
  `#[naked]`), scheduler round-robin, demo de 2 tareas intercaladas.
  **Todavía sin espacio de direcciones propio por tarea** (eso es M4c)
- ✅ **PCB real (`process.rs`)** *verificado en QEMU* — PID/PPID/estado
  (`Running`/`Zombie(exit_code)`)/PML4 por proceso, tabla global,
  `alloc_pid`/`register`/`mark_zombie`/`set_page_table`. **Pendiente:**
  vincular `Capabilities` por proceso — sigue siendo un único slot
  global (`caps::CURRENT`), no por PID.
- ❌ **Scheduler** — round-robin como mínimo, weighted más adelante
- ❌ **Context switch** — guardar/restaurar registros, FPU/SSE state
- ✅ **Espacios de direcciones por proceso (M4d)** *borrador sin
  verificar* — `mmu::create_address_space` (PML4 propio, comparte
  kernel vía P4[0]), `mmu::map_page_in` (mapeo en un espacio no
  activo), scheduler cambia CR3 en Rust seguro al entrarle el turno a
  una tarea con espacio propio. Probado creando un espacio, mapeando
  algo privado en él, y cambiando CR3 de verdad.
- ✅ **Cargador ELF64 (M4e)** *borrador sin verificar* — parseo de
  cabecera + program headers, mapeo de segmentos PT_LOAD en un espacio
  de direcciones nuevo. **Probado contra un ELF64 real y mínimo
  construido a mano** (`elf::TEST_ELF`, 123 bytes), no bytes inventados
  sin verificar. **Sin transición a ring 3 todavía** — necesita GDT
  ring3 + TSS.RSP0 + `iretq`, pieza aparte
- ✅ **Transición a ring 3 (M4f)** *borrador sin verificar* —
  `ring3.rs`: `iretq` con frame de interrupción construido a mano;
  `gdt.rs` ampliado con selectores de usuario (DPL=3) + `TSS.RSP0`
  (pila de kernel para cuando una interrupción llegue estando en ring
  3). Probado con `TEST_ELF_RING3` — payload sin instrucciones
  privilegiadas (`jmp $`, no `hlt`, que provocaría `#GP` en ring 3).
  Comando `ring3test` en la consola, **viaje solo de ida a propósito**:
  sin preemption conectada (M4b, parte pendiente), no hay forma de
  recuperar el control tras saltar — por diseño no se ejecuta en el
  arranque automático, solo a demanda
- ✅ **Syscalls reales — `syscall`/`sysret` (M4g)** *borrador sin
  verificar — la pieza más delicada de toda la sesión, más piezas
  móviles que AHCI* — `syscall.rs`: MSRs (EFER.SCE, STAR, LSTAR,
  FMASK), entrada naked con cambio a pila de kernel propia (syscall NO
  cambia de pila sola, a diferencia de una interrupción con TSS),
  `SyscallFrame` con la convención ya documentada (RAX=número,
  RDI/RSI/RDX/R10/R8/R9=args). Probado con un tercer ELF
  (`TEST_ELF_SYSCALL`) que ejecuta `syscall` de verdad desde ring 3,
  vía comando `synccalltest`. **Importante:** prueba el viaje completo
  ring3→syscall→kernel→sysret→ring3, que es lo que hace la mayoría de
  syscalls reales — un `exit()` que recupere el control de la consola
  necesita integración con el scheduler (guardar/restaurar contexto
  como `task.rs`), pieza aparte todavía no hecha
- ✅ **`caps::enforce` conectado de verdad (M4h)** *borrador sin
  verificar* — `syscall_dispatch` ahora exige `CAP_STDIO` antes de
  atender `SYS_PING`. Slot global "proceso actual" (`caps::set_current`)
  como paso intermedio honesto — todavía no hay PCB por proceso donde
  guardarlo individualmente (eso es la integración con el scheduler,
  sigue pendiente). Dos comandos de consola para ver los dos casos:
  `synccalltest` (con `CAP_STDIO`, permitido) y `synccalldeny` (sin él,
  denegado con log explícito)
- ✅ **`fork()`/`execve()`/`exit()`/`getpid()` reales (M4i)**
  *verificado en QEMU* — `syscall.rs`: `SYS_FORK`(57)/`SYS_EXECVE`(59)/
  `SYS_EXIT`(60)/`SYS_GETPID`(39), numeración compatible Linux x86_64.
  `fork()` usa `mmu::clone_address_space` (copia profunda de
  `P4[1..=255]`) + `ring3::enter_ring3_with_rax` para arrancar al hijo
  como tarea nueva del scheduler con `rax=0`. `execve()` lee el nombre
  del fichero desde memoria de usuario, lo busca en el VFS, carga el
  ELF y reemplaza el espacio de direcciones — diverge a propósito
  (nunca vuelve a `syscall_dispatch`). `exit()` marca zombie y cede el
  turno para siempre (`scheduler::mark_current_finished` + `sti` +
  `yield_now()` en bucle — el `sti` es imprescindible, ver nota de
  arriba). Comandos de consola `forktest`/`exec`/`ps`. **Limitación
  conocida:** el hijo de `fork()` NO hereda todos los registros del
  padre, solo `rip`/`rsp`/`rflags`/`rax` (ver aviso largo en
  `ring3::enter_ring3_with_rax`) — vale para el payload de prueba, no
  para un `fork()` de propósito general todavía.
- ✅ **`wait()` real (`SYS_WAIT`=61)** *verificado en QEMU* —
  bloqueante de verdad (cede el turno en bucle hasta que el hijo sea
  zombie, mismo patrón cooperativo que `exit()`), recoge el PID y
  código de salida (`arg0` = puntero de usuario opcional a `i32`), y
  **libera de verdad la memoria del hijo**: `mmu::free_address_space`
  (nuevo — reverso exacto de `clone_address_space`, recorre y libera
  P4[1..=255] entero: tablas intermedias + páginas de datos + el
  propio PML4, nunca P4[0]). Comando de consola `waittest`
  (`elf::TEST_ELF_FORK_WAIT`) — confirmado con `ps`: el hijo desaparece
  de la tabla tras `wait()`, ya no queda zombie para siempre.
- ❌ **Señales** (signals) — al menos SIGKILL/SIGTERM/SIGSEGV
- ✅ **Dispatcher de syscalls real** *verificado en QEMU* — ya no es
  demo aislada: `caps::enforce_current` protege `SYS_PING`, `CAP_EXEC`
  protege `SYS_FORK`/`SYS_EXECVE`
- ✅ **IPC — pipes (M4i)** *borrador sin verificar* — `pipe.rs`, buffer
  circular por pipe (FIFO real, probado escribiendo más de lo que se
  lee de golpe y comprobando que el resto queda pendiente). Namespace
  global por ID, sin bloqueo todavía (leer/escribir de un pipe vacío
  no espera — bloquear de verdad necesita integración con el
  scheduler). Mailbox/mensajería como en Asmodeus14/Nyx sigue siendo
  buen precedente para una versión más rica más adelante
- ❌ **Memoria compartida (SHM)** — necesaria más adelante para el
  compositor gráfico (ventanas cliente)
- ✅ **Ember (primer boceto)** *verificado en QEMU* — modelo rc.d de
  FreeBSD (secuencial, no systemd), comando `ember` de la consola —
  ver actualización arriba. **Pendiente para que sea un PID 1 real:**
  arrancado automáticamente por el kernel (hoy solo a demanda desde la
  consola, con PPID 0 igual que un test más), lista de servicios real
  en vez de dos placeholders hardcodeados, permanencia indefinida tras
  el arranque (necesita `preempt.rs` primero — ver limitación
  documentada arriba), y el patrón **privsep** ya documentado en
  `PHILOSOPHY.md` (proceso sin privilegios + supervisor mínimo) para
  cada demonio real que Ember termine lanzando.

## 3. Filesystem (M5)

- ✅ **VFS mínimo — tmpfs plano (M5, primer paso)** *borrador sin
  verificar* — `vfs.rs`, namespace plano en memoria (sin directorios
  todavía), write/read/delete/list, probado con escritura+lectura real
  y comandos `ls`/`cat`/`write` en la consola. **Sin persistencia** —
  para eso está `ext2.rs`, ver abajo
- ❌ **Initramfs/tarfs** — para arrancar userland antes de tener disco
  real montado (patrón usado por los dos Nyx)
- ✅ **Filesystem persistente real — ext2 de solo lectura** *verificado
  en QEMU contra un ext2 real hecho con `mkfs.ext2`/`debugfs`* —
  `ext2.rs`: superbloque, descriptores de grupo, inodos, directorios
  (`ext2_dir_entry_2` con `file_type`), lectura de ficheros con
  punteros directos + indirecto simple. Comandos `ext2ls`/`ext2cat`.
  Decisión ya tomada: ext2 clásico (compatible con herramientas
  externas de Linux para depurar discos desde fuera), no un formato
  propio. **Pendiente:** doble/triple indirecto, ext3/ext4 real
  (journal, extents), escritura, tabla de particiones (`partinfo.rs`).
  **Aspiración a largo plazo inspirada en ZFS** (FreeBSD, si algún día
  se decide un filesystem propio en vez de quedarse en ext2): bloques
  con checksum y snapshots copy-on-write — no en v1.
- ❌ **/proc y /dev sintéticos** — Windows y Linux los dan por hecho
  (info de procesos navegable, nodos de dispositivo) — sin esto no se
  siente "como Windows y Linux" ni de lejos
- ❌ **Tabla de descriptores de fichero por proceso**

## 4. HAL — drivers de hardware

- ✅ **Almacenamiento — AHCI (lectura + escritura reales)** *verificado
  en QEMU con un disco SATA real adjunto por primera vez (`disk.img`,
  ver `ext2.rs` arriba y `tools/make-test-disk.sh`)* — Command List +
  Command Table + FIS H2D; `IDENTIFY DEVICE`, **`READ DMA EXT` y `WRITE
  DMA EXT` (LBA48)** + `FLUSH CACHE EXT`. Comando `disktest` en la
  consola (escritura+relectura no destructiva) confirmado, y `ext2.rs`
  entero corre encima de `read_sectors` de verdad. Registros
  verificados contra Linux. **Limitación actual:** un solo PRDT →
  máximo 8 sectores (4 KiB) por llamada; transferencias grandes
  necesitarán múltiples entradas PRDT
- ❌ **Almacenamiento — NVMe**
- ✅ **Red — RTL8139 (TX + RX real)** *verificado en QEMU — registros
  TxStatus/TxAddr/RxBuf/RxConfig/TxConfig verificados contra Linux Y el
  Programmer's Guide oficial de Realtek (doble fuente)* — `init_full()`:
  reset, 3 frames contiguos para el anillo RX (`pmm::alloc_contiguous`),
  4 buffers TX, RX+TX habilitados en el orden exigido, Bus Master
  Enable activado por PCI. **`send()`** probado con una trama Ethernet
  de broadcast real, esperando `TxStatOK`. **`poll_rx()`** probado de
  extremo a extremo: recibe de verdad la respuesta ARP y el ICMP echo
  reply del comando `ping` (ver actualización arriba)
- ✅ **Red — ARP + ICMP (`net.rs`)** *verificado en QEMU contra el
  gateway de slirp (10.0.2.2)* — `inet_csum` (RFC 1071), tabla ARP
  mínima, `build_arp_request`/`parse_arp_reply`,
  `build_ping`/`parse_icmp_echo_reply`. Comando `ping` en la consola.
  **Sin DHCP** — IP propia (10.0.2.15) fijada a mano, es el valor por
  defecto de slirp
- ❌ **Red — resto del stack**: IP/ICMP genérico (más allá del caso
  echo) → UDP/TCP → DHCP/DNS → sockets BSD
  (`socket`/`connect`/`bind`/...) → WiFi (mucho más difícil, firmware
  de vendor)
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
- ❌ **libc** — **decisión revisada:** en vez de escribir la nuestra
  desde cero, portar **mlibc** (libc pensada para OSes nuevos, con capa
  de abstracción por sistema; es lo que usa BoredOS). Convierte portar
  software de "meses por programa" a "cambios menores". Ver
  `COMPATIBILITY.md`
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
- ✅ **Fuente de consola real** *borrador sin verificar* — parser PSF1
  (`psf.rs`) resuelve "completar el alfabeto bitmap" con una fuente
  real, no más glifos hechos a mano. **Pendiente:** el fichero
  `kernel/assets/font.psf` en sí (instrucciones en
  `kernel/assets/README.md`)
- ❌ **Fuente proporcional real tipo TTF** — PSF1 es monoespaciada de
  consola, suficiente para debug/terminal; TTF de verdad (como hizo
  nyxos-dev con DejaVu Sans) sigue siendo aspiración de Anvil/M6+
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
- ❌ **Cargador glTF/GLB** — el formato 3D realista a priorizar (estándar
  Khronos, el mismo ecosistema que Vulkan/OpenGL ES). FBX, Maya
  (.ma/.mb), 3ds Max, Cinema 4D, Houdini y ZBrush son formatos
  propietarios sin spec pública para reimplementación libre — no se
  persiguen. DOCX/XLSX/PPTX tienen spec abierta (OOXML) pero es un
  proyecto del tamaño de LibreOffice, no algo cercano

## 10. Estabilidad y QA

- ❌ **Batería de tests KAT en CI** — principio ya escrito en
  `PHILOSOPHY.md`, pero no hay ni un solo test automatizado todavía
- ❌ **Tracing/observabilidad en vivo** (equivalente mínimo a
  perf/ftrace) — hueco que no habíamos ni mencionado. Sin esto, un bug
  de *rendimiento* (no de correctitud, que sí cazan los KATs) no tiene
  forma de diagnosticarse salvo prints manuales. No urgente ahora mismo
  (nada que perfilar todavía sin SMP/scheduler real), pero anotado para
  cuando M4 esté más maduro
- ❌ **Pantalla de pánico gráfica** — hoy un panic solo cuelga y loguea
  por serie; Windows tiene BSOD, Linux tiene el panic de consola, los
  dos Nyx tienen pantalla de pánico gráfica — nosotros no tenemos nada
  visual todavía

## 11. Instalador (esto no lo habías pedido, pero hace falta)

- ❌ **Instalador real a disco persistente** — hoy Forge OS solo
  arranca en vivo desde el ISO; no hay forma de instalarlo de forma
  permanente en una máquina. Sin esto no es un "sistema operativo" en
  el sentido que tú quieres, es un live-CD para siempre.
- ❌ **Parser de tabla de particiones** (GPT/MBR) — lectura ya existe
  (`partinfo.rs`, milestone `partinfo-ext4-preempt`, ver sección 0);
  falta la escritura (crear/modificar tablas), que es lo que un
  instalador de verdad necesita
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

## Metas de integración concretas

### DOOM (meta intermedia, más cercana)

Precedente directo: BoredOS lo consiguió con doomgeneric + TinyGL.
Necesita: M4c (procesos) → M5 (VFS) → mlibc portada → framebuffer (ya
lo tenemos) + entrada de teclado (ya lo tenemos). **No necesita red, ni
GPU, ni TLS** — es una meta bastante más cercana que yt-dlp y prueba
que el userland funciona de verdad.

### yt-dlp (meta mayor)

Un objetivo tangible para saber cuándo el sistema esencial está sólido:
que `yt-dlp` corra de verdad en Forge OS. No necesita GPU, motor de
navegador, ni sandboxing complejo — pero sí toca casi toda la pila
esencial de golpe: M4 completo (procesos reales) → M5 (VFS/filesystem) →
stack de red completo con TLS → Python portado (ver `LANGUAGES.md`) →
ffmpeg opcional para calidad completa. El día que esto corra, es la señal
de que el sistema base funciona de punta a punta.
