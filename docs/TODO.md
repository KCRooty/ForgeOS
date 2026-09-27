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

**Actualización — `preempt.rs`, preemption real vía timer APIC (Claude
Code, verificado en QEMU):** el timer de M4b llevaba disparando desde
el primer arranque, pero su handler original solo contaba ticks y
mandaba EOI — nunca tocaba el scheduler. Ahora `preempt::timer_entry`
(trampolín en ensamblador desnudo, mismo patrón que `syscall_entry` en
`syscall.rs`: guardar los 15 registros de propósito general a mano
antes de tocar Rust, porque una interrupción puede caer en cualquier
punto del código interrumpido) sustituye a ese handler. Cuando
interrumpe una tarea de kernel (CPL0), llama de verdad a
`scheduler::yield_now()` — reutilizando el `task::switch_to`
cooperativo de siempre, no un mecanismo nuevo (ver la nota larga en
`preempt.rs` sobre por qué eso es seguro y simétrico).

**Verificado con el peor caso posible, no el más fácil:** el comando
`preempttest` arranca dos tareas (`spin_a`/`spin_b`) que jamás llaman a
`yield_now()` por su cuenta — un bucle `loop { count += 1 }` puro, sin
ceder el turno nunca. Con la preemption activada, ambas avanzaron más
de 13 millones de iteraciones cada una en 21 ticks, de forma
intercalada — la única explicación posible es que el timer las está
forzando a ceder el turno de verdad. `task_a`/`task_b` (la demo
cooperativa de M4a, que nunca termina) también recibieron turnos
extra durante la ventana de la prueba, señal de que el round-robin
reparte con justicia entre tareas viejas y nuevas por igual.

**Por qué la preemption no está encendida por defecto:** el propio
timer sigue armado y disparando desde M4b durante TODO el arranque
(VFS, red, syscalls, `console::run()`...), pero antes de este
milestone eso era inofensivo (el handler no hacía nada más que
contar). Encenderla de golpe para el resto del arranque habría
cambiado el comportamiento de código ya verificado en milestones
anteriores sin que ninguno de ellos lo hubiera tenido en cuenta —
`task_a`/`task_b`, que nunca terminan, habrían empezado a imprimir sin
parar, y la consola habría competido por CPU con ellas. En vez de eso,
`preempt::ENABLED` (flag global, `false` por defecto) decide si
`timer_tick_ring0` llega a llamar a `yield_now()` — el timer sigue
contando ticks y mandando EOI siempre, pero solo cambia de tarea
cuando algo lo pide explícitamente (`preempttest`, de momento). El
resto del arranque queda exactamente igual que antes, byte a byte en
el log de serie — verificado comparando el arranque normal antes y
después de este cambio.

**Limitación documentada a propósito: sin preemption real en ring 3
todavía.** El trampolín SÍ distingue si interrumpió una tarea de
kernel (CPL0) o un proceso de usuario (CPL3) — mirando el CS que el
hardware dejó en la pila, en un offset fijo sin importar cuántos qwords
empujara. Para CPL3 solo hace EOI y vuelve, sin tocar el scheduler.
Motivo concreto: `gdt.rs` usa un único `TSS.RSP0` global para TODAS las
transiciones ring3→ring0 por interrupción (a diferencia de la pila de
syscalls, que sí es per-tarea desde `wait()`) — si dos procesos de
ring 3 quedaran aparcados a la vez en esa misma pila compartida, el
segundo pisaría el estado del primero (mismo bug de fondo que la pila
de syscalls compartida, ya arreglado ahí). Extender esto a CPL3 de
verdad necesita un `RSP0` por tarea primero, pieza aparte — hasta
entonces, Ember sigue sin poder quedarse viva para siempre de forma
segura (ver su propia nota).

**Actualización — señales: SIGKILL/SIGTERM/SIGSEGV (Claude Code,
verificado en QEMU) — último punto pendiente del TODO original de esta
reconstrucción.** Alcance a propósito acotado: solo la acción por
defecto de cada señal (terminar el proceso) — nadie puede instalar un
manejador propio todavía, eso necesita una máscara de señales
pendientes por proceso + un trampolín de `sigreturn`, pieza bastante
más grande, aparte.

- `signal.rs` (nuevo): `deliver_default(pid, sig)` — marca el proceso
  zombie con código `128 + señal` (convención de shell, distinguible de
  un `exit(N)` normal con solo mirar `ps`) y lo saca del round-robin
  del scheduler (`scheduler::mark_finished_by_pid`, nuevo — a
  diferencia de `mark_current_finished()`, no hace falta que sea la
  tarea en ejecución ahora mismo).
- `SYS_KILL` (=62, numeración Linux) — `kill(pid, señal)` desde
  cualquier proceso con `CAP_PROC_CTL` (capability nueva en `caps.rs`,
  más restrictiva que `CAP_EXEC`). Comando `killtest`: un padre mata a
  su hijo con `SIGTERM` ANTES de que el hijo llegue a ejecutar una sola
  instrucción, y luego `wait()` lo recoge exactamente igual que si
  hubiera hecho `exit()` él solo — mismo camino en `process.rs`,
  confirmado.
- `idt.rs::page_fault` — antes hacía `halt()` incondicional ante
  CUALQUIER fallo de página; ahora distingue si vino de ring 3 (mirando
  el CS que dejó el hardware, mismo truco que `preempt::timer_entry`) y
  en ese caso mata solo al proceso (`SIGSEGV`) en vez de colgar el
  kernel entero. Comando `segvtest`: un proceso escribe a la dirección
  0 a propósito — la consola sigue viva después, `ps` lo muestra
  `zombie(code=139)`.

**Bug real encontrado (y arreglado) durante la verificación, no antes
de ella:** la primera versión de `deliver_default` llamaba a
`mmu::free_address_space` incondicionalmente. Para un `SIGSEGV`
auto-infligido (el caso normal: el proceso que revienta ES el que está
corriendo), eso libera el PML4 que **CR3 tiene activo en ese mismo
instante** — exactamente lo que `free_address_space` prohíbe en su
propia documentación ("liberar las tablas bajo tus propios pies sería
fatal"). Sin diagnóstico previo, `segvtest` en sí parecía funcionar
(la consola seguía viva) pero el siguiente `ping` colgaba el kernel de
verdad (page fault en ring 0, `error_code` de escritura) — el PML4
recién liberado se reciclaba para el buffer RX del RTL8139 antes de
que el scheduler llegara a cambiar de espacio de direcciones.
Aislado comparando contra `waittest` (que también libera una dirección
de espacio, vía `reap_zombie`, y no lo dispara — porque ahí quien
libera SIEMPRE es el padre, con su propia CR3 activa, nunca la del
proceso que se está liberando). Arreglado: `deliver_default` solo
libera de inmediato si `pid` NO es el proceso activo ahora mismo; si lo
es, la memoria queda pendiente para quien le haga `wait()` más
adelante — mismo comportamiento que ya tiene cualquier `exit()` normal
sin padre esperando.

También se encontró y arregló, en el camino, el mismo `sti` que ya
hizo falta en `sys_exit()` y en `preempt::timer_tick_ring0()` —
`page_fault` es OTRO gate de interrupción que enmascara IF al entrar;
sin reactivarlo antes de `yield_now()`, la consola se quedaba con
interrupciones enmascaradas para siempre (viva de mentira: ya había
impreso el prompt, pero el `hlt` de `read_line()` no despertaba nunca
más).

**Actualización — preemption real también en ring 3 (Claude Code,
verificado en QEMU):** cierra el hueco que `preempt.rs` había dejado a
propósito abierto. `gdt::set_rsp0` (nuevo) hace `TSS.RSP0` per-tarea —
`scheduler::yield_now()` lo activa con el mismo valor que ya usa
`Task::kernel_stack_top` para la pila de syscalls (seguro compartirlo:
un `syscall` y una interrupción nunca están "en vuelo" a la vez para la
MISMA tarea). Con eso, el motivo original para tratar CPL0 y CPL3 de
forma distinta desaparece — `preempt::timer_entry` pierde la rama por
completo, un único camino sirve para cualquier nivel de privilegio
interrumpido (nunca interpreta el frame que deja el hardware, solo
apila encima y desapila lo mismo antes de `iretq`).

**Verificado con el peor caso posible:** `TEST_ELF_SPIN3` no ejecuta
`syscall` NI UNA VEZ — un bucle `inc qword [pila_de_usuario]` puro en
ring 3. Comando `preempttest3`: con la preemption activada, el proceso
avanzó casi 13 millones de iteraciones en 20 ticks (verificado leyendo
su memoria por fuera, vía `mmu::translate` — el proceso nunca hace una
syscall con la que "contestar"), y la consola (que tampoco cede el
turno por su cuenta mientras espera) siguió recibiendo turnos con
normalidad. `segvtest`/`killtest` (ambos también ring 3) siguen
funcionando exactamente igual bajo el nuevo `RSP0` compartido con la
pila de syscalls.

**Lo que NO cambia todavía, a propósito:** Ember (`elf.rs`) sigue
llamando a `exit()` en vez de quedarse viva para siempre — ya no por un
bloqueo técnico (el mecanismo ya existe y está verificado), sino porque
dejar la preemption real encendida por defecto para el resto de la
sesión de consola es una decisión de política aparte: `task_a`/`task_b`
(M4a, tampoco terminan nunca) empezarían a imprimir sin parar de fondo
en cuanto se tomara. Pieza aparte, con esa consecuencia que resolver
primero (o aceptar).

**Actualización — Ember ya se queda viva para siempre de verdad
(Claude Code, verificado en QEMU):** decisión tomada. `task_a`/`task_b`
(M4a) se arreglaron para terminar de verdad tras sus 3 vueltas
(`scheduler::mark_current_finished()` + un último `yield_now()`, mismo
patrón que `sys_exit()`) en vez de repetir para siempre — antes daba
igual porque la preemption real estaba siempre apagada llegados a ese
punto del arranque; ahora ya no. `TEST_ELF_EMBER` termina su secuencia
con un `jmp $` infinito en vez de `exit()`, y el comando `ember` de la
consola enciende `preempt::set_enabled(true)` **antes** de ceder el
turno por primera vez (no después) y no la vuelve a apagar — Ember
pasa a ser un PID 1 real que no termina nunca, verificado con varios
`ps` seguidos a lo largo de una sesión larga (ext2, partinfo, forktest,
meminfo de por medio) mostrando `1  0  running` de forma consistente,
sin que nada más se rompa.

**Bug real encontrado (y arreglado) durante la propia verificación:**
la primera versión encendía `preempt::set_enabled(true)` DESPUÉS del
bucle de `yield_now()` que deja correr a Ember, no antes. Como Ember ya
no termina, en cuanto le tocaba turno corría sus dos rondas y caía en
su `jmp $` infinito — con la preemption todavía apagada en ese momento,
nada podía volver a interrumpirla, así que el último `yield_now()` del
bucle nunca regresaba: la consola entera se quedaba colgada para
siempre, silenciosamente (sin panic, sin error, solo dejaba de
responder). Arreglado invirtiendo el orden: encender la preemption
ANTES del bucle de yields, para que el timer pueda recuperar el
control pase lo que pase, incluso si Ember nunca cede el turno por su
cuenta.

**Interacción documentada, no un bug:** `preempt::ENABLED` es un único
flag global, no un contador de referencias — si después de `ember` se
ejecuta `preempttest`/`preempttest3` (que apagan la preemption al
terminar su propia ventana acotada), también apagan la de Ember, que
se queda entonces parada para siempre en `running` sin avanzar más
(inofensivo: no cuelga nada ni corrompe memoria, simplemente deja de
recibir turno). `preempt::set_enabled(true)` a mano lo restaura.

**Actualización — Ember arranca automáticamente (Claude Code,
verificado en QEMU):** hasta ahora Ember solo arrancaba a demanda desde
el comando `ember` de la consola — un "PID 1" que hacía falta pedir a
mano no es un init real. `main.rs` ahora la arranca ella sola, justo
después de `syscall::init()` (necesita VFS y syscalls ya listos) y
antes del mensaje "Boot completo": registra los dos servicios
placeholder, lanza `TEST_ELF_EMBER`, enciende la preemption real de
forma permanente (mismo orden crítico que ya exigía `run_ember` —
antes de ceder el turno, nunca después) y cede unos yields para dejarla
completar sus dos rondas antes de seguir arrancando. El comando `ember`
de la consola sigue existiendo, ahora para lanzar una segunda instancia
de prueba aparte (con otro PID) sin reiniciar la VM entera.

De paso, `console::launch_elf` (cargar ELF + montar pila de usuario +
`process::spawn_process`) se extrajo a `process::spawn_elf` — antes
solo vivía en `console.rs`, y el arranque automático necesitaba
exactamente lo mismo sin tener un `SerialPort` al que escribir. Un solo
núcleo compartido en vez de dos copias que podrían divergir; verificado
que los comandos que ya usaban `launch_elf` (`forktest`, `waittest`,
`exec`, `segvtest`, `killtest`, `ember`, `preempttest3`) siguen
funcionando exactamente igual.

Verificado con una sesión completa desde cero: el log de arranque
ahora muestra a Ember completando sus dos rondas ANTES de "Boot
completo", y una batería larga después (`forktest`, `ps`, `segvtest`,
`ps`, `killtest`, `ps`, `ext2ls`, `meminfo`) confirma que Ember sigue
`running` de fondo en todo momento sin que nada más se rompa, ahora con
la preemption real activa desde el primer instante en que hay consola.

**Actualización — bug real de corrupción intermitente en el arranque
de Ember, encontrado y arreglado (Claude Code, verificado en QEMU con
15+ arranques repetidos):** arrancando la ISO desde cero en bucle
(scripted, no a mano) salía mal aproximadamente 1 de cada 2-3 veces —
Ember, tras lanzar y `execve`ar su primer servicio (`ember-svc0`),
parecía ejecutar OTRO `fork()` en vez del `ping()+exit()` real de ese
binario, y la segunda "ronda" acababa en un page fault espurio. Nunca se
había visto porque los arranques manuales anteriores nunca se repitieron
lo bastante como para topar con la ventana de la carrera.

Dos bugs reales, no uno, encontrados por este camino:

1. **`scheduler::yield_now()`/`task::switch_to` no protegían su propia
   sección crítica frente al timer.** Mientras la preemption real
   estuvo apagada por defecto (todo el resto de la sesión hasta este
   punto), daba igual que `yield_now()` no deshabilitara IF — nada
   disparaba el timer en medio de un cambio de contexto. Pero `ember`
   deja la preemption encendida PARA SIEMPRE desde el primer instante en
   que arranca, y el propio arranque automático (`main.rs`) cede el
   turno varias veces con IF=1 (código de kernel normal, no dentro de
   una syscall enmascarada) — si el timer dispara ahí, su propio
   `timer_tick` reentraba en `yield_now()` sobre el mismo `Scheduler`
   global que la llamada externa todavía estaba mutando. Arreglado con
   un guard RAII (`scheduler::IrqGuard`) que deshabilita IF al entrar en
   `yield_now()` y lo restaura (a lo que fuera ANTES, no siempre a 1) al
   salir por cualquier camino, incluidos los `return` tempranos — ver el
   comentario largo en `scheduler.rs`. Necesario, pero (verificado
   después) NO SUFICIENTE por sí solo para eliminar la corrupción — el
   bug real, más profundo, era el siguiente.
2. **El root cause real: `sys_execve` actualizaba la tabla de páginas
   del proceso en `process::Pcb` pero nunca en `scheduler::Task`.** Son
   dos copias PARALELAS del mismo dato (`page_table`) — una la usan
   `wait()`/`kill()`/`reap_zombie` (`process.rs`), la otra es la que
   `scheduler::yield_now()` de verdad lee para decidir a qué CR3 cambiar
   al devolverle el turno a una tarea. `execve()` solo tocaba la
   primera. Mientras un proceso recién `execve`ado nunca cediera el
   turno de ninguna forma antes de llegar a su propio `exit()`, esto no
   se notaba (nunca había que "volver a mirar" ese campo estando a medio
   camino). Pero con preemption real de por medio, el timer puede
   interrumpirlo en CUALQUIER instrucción de su código nuevo — y al
   devolverle el turno más tarde, `yield_now()` restauraba el CR3 VIEJO
   (el PML4 clonado por `fork()` justo antes del `execve`, con una copia
   completa de la memoria del PADRE). El proceso seguía corriendo, pero
   viendo su propia memoria a través del mapeo equivocado — y como
   `ember` usa la misma dirección virtual base para su código que
   cualquier otro binario de prueba (`elf.rs`, 640 GiB, índice P4=1), el
   resultado observable era que el hijo "recién ejecutado" parecía
   ejecutar el propio código de `ember` (otro `fork()`, y a la segunda
   vuelta un page fault) en vez del suyo. Diagnosticado volcando en
   serie los bytes físicos reales en el punto de entrada justo tras el
   `execve` (SIEMPRE correctos, en todos los arranques, buenos y malos
   por igual) y el valor de CR3 antes/después del `mmu::switch_address_space`
   — eso descartó "se cargó mal el ELF" y apuntó directo a "algo
   revierte CR3 más tarde". Arreglado con `scheduler::set_task_page_table`,
   llamado desde `sys_execve` justo al lado de `process::set_page_table`.
3. De paso, la interacción ya documentada de `preempt::ENABLED` (un
   único flag booleano, no contador de referencias — `preempttest`/
   `preempttest3` apagaban también la preemption permanente de `ember`
   al terminar su propia ventana de prueba) se convirtió en un contador
   real (`AtomicU32`, `fetch_add`/`fetch_update` saturando en 0) — cada
   `set_enabled(true)` suma uno, cada `set_enabled(false)` resta uno;
   la preemption solo se apaga de verdad cuando nadie más la pidió.

Verificado exhaustivamente: 15 arranques limpios consecutivos desde
cero (antes del fix, salían mal ~1 de cada 2-3), más una batería de
regresión completa después de cada uno (`forktest`, `waittest`,
`segvtest`, `killtest`, `preempttest`, `preempttest3`, `ext2ls`,
`meminfo`, `ps`) sin ninguna regresión.

**Actualización — segundo bug real, introducido por el propio fix
anterior (Claude Code, verificado en QEMU): `IrqGuard` dejaba
`preempttest` colgado PARA SIEMPRE.** No es el mismo bug que el de
arriba (ese era `execve`/CR3, ya arreglado) — este lo introdujo el
propio `scheduler::IrqGuard` de ese mismo fix, y es más grave: no
corrupción puntual, sino bloqueo TOTAL Y PERMANENTE de todo el sistema
(sin pánico, sin error, simplemente sin más salida por serie nunca
más). Reproducido de inmediato al terminar de verificar el fix
anterior: `preempttest`, ejecutado desde la consola después del
arranque automático de Ember, se quedaba colgado siempre, de forma
100% reproducible (no intermitente como el bug de `execve`).

Causa: el `sti` de restauración de `IrqGuard` vive en su `Drop`, en la
pila de QUIEN LLAMÓ a `yield_now()` — pensado para dispararse cuando
esa misma llamada "vuelve". Pero para una tarea que arranca por
primera vez, `task::switch_to` salta DIRECTAMENTE a su `entry` sin
volver nunca a esa pila — ese `Drop` nunca se ejecuta para ella. Todo
proceso real (vía `enter_ring3`/`enter_ring3_with_rax`) se salva solo
porque su propio `iretq`, unas instrucciones más tarde, restaura
RFLAGS entero de todas formas (IF=1 a mano, sin importar lo que hubiera
antes) — por eso nunca se notó en `ember`, `forktest`, `waittest`,
`segvtest` ni `killtest`. Pero `preempt::spin_a`/`spin_b`
(`preempttest`) son hilos de kernel puro, `fn() -> !`, que nunca hacen
`syscall` ni `iretq` ni vuelven a ceder el turno — la primera vez que
les toca turno, IF se queda enmascarado PARA SIEMPRE. Y con IF=0 el
timer de la APIC no puede volver a disparar NUNCA — nada vuelve a
interrumpir a `spin_a`/`spin_b` jamás, así que nada más en todo el
sistema vuelve a ejecutarse tampoco: el "cuelgue" no es una tarea
atascada, es la CPU entera parada para siempre dentro del bucle
`COUNT.fetch_add(1)` de `spin_a`.

Arreglado añadiendo `sti` al final de `task::switch_to`, justo antes
del `ret` — el ÚNICO punto de tránsito por el que pasa CUALQUIER
cambio de tarea, nueva o reanudada, así que no depende de que la tarea
entrante vuelva a pasar por ningún sitio en concreto. No reabre la
carrera que `IrqGuard` cerraba: para cuando se ejecuta ese `sti`, el
contexto de la tarea entrante ya está cargado del todo (los 6
registros callee-saved + `rsp`), así que un timer que dispare justo
ahí solo significa que a la tarea recién entrada le vuelven a quitar
el turno enseguida — no hay nada a medio guardar que se pueda
corromper.

Verificado: `preempttest` (antes colgado al 100% de las veces tras el
arranque de Ember) ahora completa siempre, con `preempttest3`
inmediatamente después en la misma sesión también limpio. Batería de
regresión completa repetida entera (`forktest`, `waittest`, `segvtest`,
`killtest`, `preempttest`, `preempttest3`, `ext2ls`, `meminfo`, `ps`)
sin ninguna regresión, más 18 arranques adicionales desde cero (10 +
3 en aislado tras descartar contención de recursos por un test en
paralelo, + 5 de la propia batería) sin ningún fallo del bug de
`execve`/CR3 tampoco — los dos arreglos son compatibles entre sí.

**Actualización — M8: `read()`/`write()`/`open()`/`close()`/`brk()`
reales, primer paso POSIX hacia mlibc (Claude Code, verificado en
QEMU):** con el TODO original de la reconstrucción completo y los dos
bugs de concurrencia de arriba ya cerrados, el siguiente bloque grande
elegido fue mlibc + Bellows (sección 6). Antes de tocar mlibc en sí,
hacía falta la base que CUALQUIER libc real necesita y que Forge OS no
tenía todavía: descriptores de fichero de verdad (no solo
`vfs::read`/`vfs::write` de buffer completo) y un heap al que
`malloc()` pueda pedir memoria.

Novedades:
- `process::Pcb` gana `fds: Vec<Option<FdEntry>>` (tabla de
  descriptores por proceso — `FdEntry::ReadFile{data,cursor}` para
  lectura, `FdEntry::WriteFile{name,data}` para escritura, ver su
  comentario en `process.rs`) y `heap_top`/`heap_mapped_end` (el
  segundo hace falta aparte porque `heap_top` SÍ puede bajar sin
  desmapear nada, y `mmu::map_page_in` falla a propósito si intentas
  volver a mapear una página ya presente).
- `syscall.rs`: `SYS_READ`(0)/`SYS_OPEN`(2)/`SYS_CLOSE`(3)/`SYS_BRK`(12)
  con la numeración real de Linux x86_64 — pero `SYS_WRITE` NO pudo
  ser el 1 real: **colisión de números encontrada y arreglada antes
  de siquiera llegar a QEMU** (el compilador la cazó sola, con un
  warning nuevo de "unreachable pattern" al comparar contra el
  baseline de 34 warnings de siempre) porque `SYS_PING`, una demo de
  M1 mucho anterior, ya ocupaba ese número — `SYS_WRITE` quedó en 63.
  fd 0/1/2 siempre son stdin/stdout/stderr servidos por el puerto
  serie (stdin todavía sin fuente real, devuelve EOF a propósito,
  documentado como limitación conocida, no bug silencioso); fd≥3 usa
  la tabla nueva. `brk()` solo crece de verdad (mapea páginas físicas
  nuevas); bajar el límite es contabilidad lógica, nunca desmapea.
- Capacidades ya existían para todo esto desde M1 (`CAP_FS_READ`,
  `CAP_FS_WRITE`, `CAP_MEM_MAP`) sin que nada las usara todavía — M8
  es la primera vez que se aplican de verdad.
- `TEST_ELF_POSIX` (comando `posixtest`) es el primer binario de
  prueba de esta sesión que NO se hizo a mano byte a byte: se
  ensambló con `nasm -f bin` de verdad y se envolvió en el mismo
  header ELF64 mínimo de siempre — con más syscalls reales, seguir
  tecleando opcodes a mano ya no es razonable. Prueba la cadena
  completa: `write(1,...)` a stdout real (antes, la única "salida
  visible" era el propio log del kernel, nunca algo escrito por un
  PROCESO) → `open`+`write`+`close`+`open`+`read`+`write(1,...)` — si
  lo que se ve por stdout coincide con lo que se escribió antes, el
  roundtrip por el VFS real funciona de principio a fin → `brk(0)` +
  `brk(+4096)` + escribir y releer un valor de 64 bits en la página
  nueva del heap, con `ping(0xCAFEBABE)` dejando constancia numérica
  en el log de que el mapeo es memoria escribible real, no solo
  contabilidad.

Verificado: `posixtest` funcionó a la primera en QEMU, sin ningún bug
encontrado en tiempo de ejecución (solo la colisión de números, cazada
en tiempo de compilación). 5 arranques limpios en aislado + 2
ejecuciones consecutivas en la misma sesión + batería de regresión
completa (`forktest`, `waittest`, `segvtest`, `killtest`,
`preempttest`, `preempttest3`, `ext2ls`, `meminfo`, `ps`) sin ninguna
regresión. Build limpio, cero warnings nuevos sobre el baseline de 34.

Pendiente para que mlibc en sí sea viable: `crt0` (arranque real de
proceso con `argv`/`envp`, hoy todo arranca en un `entry` fijo sin
convención de pila estándar), `mmap()` de verdad (hoy solo hay `brk()`
lineal), y sobre todo escribir el sysdeps de mlibc contra ESTA ABI
concreta — trabajo de otra sesión, mlibc es un proyecto en sí mismo.

**Actualización — M8b: `mmap()`/`munmap()` reales + pila inicial de
`crt0` (Claude Code, verificado en QEMU):** segunda mitad del paso
POSIX hacia mlibc, completando lo que M8 dejó pendiente.

- `Pcb.mmap_next` (arena que solo crece, mismo espíritu que
  `heap_mapped_end`) + `SYS_MMAP`(9)/`SYS_MUNMAP`(11) con numeración
  real de Linux x86_64 — sin colisión esta vez. `sys_mmap` solo
  soporta `MAP_ANONYMOUS` (sin fichero detrás — mapear un fichero real
  no está implementado, devuelve error); ignora el `addr` sugerido por
  el llamante (siempre decide su propia dirección) y `prot` (todo
  mapeo anónimo sale RW, nunca ejecutable — no hay ningún caso de uso
  real todavía que necesite distinguir protecciones aquí). `munmap()`
  es un no-op a propósito (devuelve 0 sin desmapear nada), mismo
  espíritu que `brk()` bajando sin desmapear.
- `process::write_initial_stack_frame` — la pila de un proceso recién
  arrancado (`spawn_elf` o `execve()`, nunca `fork()`, que reanuda la
  pila YA EXISTENTE del padre) ahora lleva escrito el frame mínimo que
  un `crt0` real (convención SysV x86_64) espera encontrar: `argc` en
  `[rsp]`, `argv[]` terminado en `NULL`, `envp[]` terminado en `NULL`,
  `auxv[]` terminado en `AT_NULL`. De momento siempre "0 argumentos,
  sin entorno" — ni `spawn_elf` ni `execve()` aceptan todavía pasar
  `argv`/`envp` reales (siguiente paso, cuando algún binario los
  necesite de verdad) — lo que importaba ahora era que la FORMA de la
  pila ya fuera correcta, no rellenarla de contenido real. Nuevo
  `process::map_fresh_user_stack` (mapea la página + escribe el frame)
  es el núcleo compartido entre `spawn_elf` y `sys_execve` — antes cada
  uno mapeaba su pila por su cuenta con código casi idéntico; de paso,
  al tocar `sys_execve`, se limpió un warning preexistente de `unsafe`
  innecesario que llevaba ahí desde M4g.
- `TEST_ELF_CRT0` (comando `crt0test`), igual que `TEST_ELF_POSIX` de
  M8: ensamblado con `nasm -f bin` de verdad, no a mano byte a byte.
  Comprueba que los 5 qwords del frame inicial son EXACTAMENTE cero →
  `mmap(NULL, 8192, ..., MAP_ANONYMOUS, -1, 0)` para dos páginas de
  golpe → escribe un patrón de 64 bits DISTINTO en cada página y las
  relee (si `mmap` solo hubiera mapeado la primera, la escritura en la
  segunda habría hecho page fault) → `munmap()` + `ping()` del
  resultado (debe ser 0).

Verificado: `crt0test` funcionó a la primera en QEMU. 6 arranques
limpios y deterministas en secuencia + batería de regresión completa
(`crt0test`, `posixtest`, `forktest`, `waittest`, `segvtest`,
`killtest`, `preempttest`, `preempttest3`, `ext2ls`, `meminfo`, `ps`)
sin ninguna regresión — `posixtest` sigue funcionando exactamente
igual, confirmando que compartir `map_fresh_user_stack` con `spawn_elf`
no le rompió nada. Build limpio, cero warnings nuevos (de hecho uno
MENOS que el baseline de 34, por la limpieza de `unsafe` mencionada
arriba).

Con `brk()`+`mmap()`+una pila `crt0`-compatible ya reales y
verificados, lo que queda para que mlibc en sí sea viable es
enteramente trabajo de otra sesión: escribir su sysdeps contra esta
ABI concreta (mlibc es un proyecto en sí mismo, no una tarde), y
cuando haga falta pasar `argv`/`envp` de verdad, extender `execve()`
para aceptarlos.

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
  arriba
- ✅ **`preempt.rs`** — preemption real vía timer APIC, separado de M4b
  (milestone `partinfo-ext4-preempt`, segunda mitad) ya reconstruido y
  verificado, **incluida preemption real en ring 3** (`TSS.RSP0` por
  tarea, `preempttest3`) — ver arriba
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
  salta directo con `enter_ring3` sin pasar por `scheduler::spawn`, así
  que aunque `preempt.rs` ya soporta preemption real en ring 3
  (`preempttest3`), aquí no hay ninguna tarea registrada a la que
  volver — por diseño no se ejecuta en el
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
- ✅ **Señales — SIGKILL/SIGTERM/SIGSEGV** *verificado en QEMU* — ver
  actualización arriba. Solo acción por defecto (terminar); **sin
  manejadores propios instalables todavía** — necesita máscara de
  señales por proceso + trampolín `sigreturn`, pieza aparte
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
- ✅ **Ember** *verificado en QEMU* — modelo rc.d de FreeBSD
  (secuencial, no systemd). **Ya se queda viva para siempre** (PID 1
  real, `jmp $` infinito + preemption real permanente desde que
  arranca) — ver actualización arriba. **Ya arranca automáticamente
  desde `main.rs`**, antes de "Boot completo" — ya no hace falta
  pedirlo a mano (el comando `ember` de la consola sigue existiendo,
  para relanzar una segunda instancia de prueba aparte, con otro PID).
  `process::spawn_elf` (nuevo) es el núcleo compartido entre ese
  arranque automático y `console::launch_elf` — antes duplicado.
  **Pendiente:** lista de servicios real en vez de dos placeholders
  hardcodeados, y el patrón
  **privsep** ya documentado en `PHILOSOPHY.md` (proceso sin
  privilegios + supervisor mínimo) para cada demonio real que Ember
  termine lanzando.
- ✅ **`read()`/`write()`/`open()`/`close()`/`brk()`/`mmap()`/`munmap()`
  reales (M8 + M8b)** *verificado en QEMU* — ver actualizaciones M8 y
  M8b arriba. `SYS_READ`(0)/`SYS_OPEN`(2)/`SYS_CLOSE`(3)/`SYS_MMAP`(9)/
  `SYS_MUNMAP`(11)/`SYS_BRK`(12) con numeración real de Linux x86_64;
  `SYS_WRITE`=63 (no pudo ser el 1 real, colisión con `SYS_PING` ya
  existente desde M1). `mmap()` solo soporta `MAP_ANONYMOUS`;
  `munmap()` es un no-op a propósito. Primer paso POSIX real hacia
  mlibc (sección 6) — comandos `posixtest`/`crt0test`. **Pendiente:**
  el sysdeps de mlibc contra esta ABI en sí, y `argv`/`envp` reales
  cuando algún binario los necesite (la FORMA de la pila ya es
  correcta, ver `crt0` en sección 6).

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
- ✅ **Tabla de descriptores de fichero por proceso (M8)** *verificado
  en QEMU* — `process::Pcb.fds`, ver actualización M8 arriba y la
  entrada de `SYS_READ`/`SYS_WRITE`/`SYS_OPEN`/`SYS_CLOSE` en la
  sección 2. **Limitación conocida:** cada fd carga/acumula su
  contenido entero en memoria (el VFS no tiene handles reales) — vale
  para ficheros pequeños, no para uno que no entre en RAM.

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

- ✅ **crt0 — pila inicial (M8b)** *verificado en QEMU* — el frame
  `argc`/`argv`/`envp`/`auxv` que un `_start` real leería ya es
  correcto (`process::write_initial_stack_frame`), ver actualización
  M8b arriba. **Pendiente:** que `spawn_elf`/`execve()` acepten
  `argv`/`envp` de VERDAD (hoy siempre "0 argumentos, sin entorno") —
  la forma ya está, falta rellenarla cuando algún binario real los
  necesite.
- ❌ **libc** — **decisión revisada:** en vez de escribir la nuestra
  desde cero, portar **mlibc** (libc pensada para OSes nuevos, con capa
  de abstracción por sistema; es lo que usa BoredOS). Convierte portar
  software de "meses por programa" a "cambios menores". Ver
  `COMPATIBILITY.md`. **Base de syscalls ya lista (M8+M8b, sección 2):**
  `read`/`write`/`open`/`close`/`brk`/`mmap`/`munmap` reales y
  verificados, más una pila inicial compatible con `crt0` — lo único
  que falta es escribir el sysdeps de mlibc contra esta ABI en sí
  (proyecto propio, otra sesión) y, cuando haga falta, `argv`/`envp`
  reales (ver el punto de `crt0` arriba).
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
