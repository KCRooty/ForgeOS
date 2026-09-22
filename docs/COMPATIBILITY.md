# Compatibilidad de aplicaciones y firmware abierto

## Estrategia preferida: **port nativo**, no capa de compatibilidad

Precedente directo y reciente: **BoredOS** (github.com/BoredOS/BoredOS,
GPL-3.0, x86-64 UNIX-like en C). Su propio README lo describe así: no es
completamente POSIX-compatible, así que el software necesita algo de
trabajo de porting, pero *para la mayoría de programas los cambios son
menores*. Con eso han portado DOOM, TinyGL, TCC, Lua, kilo, kirc y tvi.

**Esta es la vía preferida para Forge OS.** Es la que da software
corriendo de verdad *nativamente*, sin emulación ni capas de traducción
— que es el punto de tener un OS propio.

### Las dos piezas que lo hacen viable

1. **mlibc** — libc diseñada explícitamente para OSes nuevos, con capa
   de abstracción por sistema (la usa BoredOS). Portarla, en vez de
   escribir nuestra propia libc desde cero, es lo que convierte "portar
   software" de meses de trabajo por programa a "cambios menores".
   Cambia por completo el cálculo de la sección 6 de `TODO.md`.
2. **TinyGL** — subconjunto de OpenGL renderizado **por software**.
   Permite 3D real sin driver de GPU, desbloqueando la parte gráfica
   mucho antes que el roadmap de HAL de `ARCHITECTURE.md`.

### Qué SÍ se puede portar nativamente

Software open source, autocontenido y portable: DOOM y otros juegos con
fuente liberada, compiladores pequeños (TCC), intérpretes (Lua,
probablemente Python), editores, clientes de red simples, herramientas
CLI. Es una lista larga y genuinamente útil.

### Qué NO se puede portar, por mucho que se quiera

- **Discord, Spotify, Steam (cliente), Claude Code**: binarios cerrados
  o dependientes de Electron/Node completo. No hay fuente que portar.
- **Navegadores reales (Chromium/Firefox)**: la fuente existe, pero son
  el software más complejo del mundo — millones de líneas asumiendo
  infraestructura completa de sistema operativo. Ni BoredOS lo intenta.
- **YouTube**: consecuencia directa de lo anterior.

Para estos, la única vía sería una capa de compatibilidad Linux (ver
abajo) — proyecto del tamaño del kernel entero, y con resultados
parciales incluso en FreeBSD, que lleva 30 años en ello.

## La otra vía: una capa, no N ports

No se portea cada app suelta. Se construye **una capa de compatibilidad
con la ABI de Linux** — un "Linuxulator" propio. Es la estrategia
verificada (ver más abajo) que usa FreeBSD para conseguir Steam/Proton
parcialmente funcional, y desbloquea de golpe: Steam, la mayoría del
software Electron (Discord, Spotify), y cualquier binario Linux que no
dependa de systemd/D-Bus/cgroups en profundidad.

**Precedente adicional, de 30+ años:** NT hace exactamente esto —
Executive agnóstico de API + Win32 como servidor de subsistema en
espacio de usuario (`subsystems/csr` + `subsystems/win` en el código
real de ReactOS, clean-room de NT). No es una idea nuestra rara, es un
patrón arquitectónico probado repetidamente. Ver `ARCHITECTURE.md`.

### Prerrequisitos reales (por orden de bloqueo)

1. **Superficie de syscalls Linux mucho más completa** — hoy tenemos ~9
   syscalls con numeración compatible (mmap, arch_prctl, getrandom...),
   documentados en `NYX-Evolution.txt` de referencia como "prewired para
   std" en el proyecto que estudiamos. Necesitamos decenas más: futex,
   epoll, clock_gettime, sched_*, prctl completo, /proc y /sys sintéticos
   (muchísimo software los lee sin preguntarse si existen de verdad).
2. **Enlazador dinámico + libc compatible** — hoy solo cargamos nuestros
   propios binarios estáticos. Ejecutar un binario Linux real (glibc o
   musl) sin recompilar necesita un `ld.so` compatible y las bibliotecas
   compartidas que ese binario espera encontrar.
3. **Drivers GPU reales con Vulkan** — sin esto no hay Steam, no hay OBS
   con aceleración, no hay nada 3D. Ya está marcado como la pieza más
   dura del roadmap gráfico (`ARCHITECTURE.md`). Ni FreeBSD, con Linux
   real vía Linuxulator y drivers Mesa portados, tiene Vulkan de NVIDIA
   funcionando — solo AMD e Intel.

### Ranking de dificultad real, app por app

| App | Vía | Dificultad |
|---|---|---|
| **VLC** | Posible port nativo directo (núcleo diseñado para portabilidad, muchos ports históricos a OSes raros) | Media — no necesita la capa Linux completa si se portea a nuestra propia API |
| **OBS Studio** | Port nativo más ambicioso (Qt, muchas dependencias, arquitectura de plugins) | Media-alta |
| **Programas open source pequeños/portables** | Port nativo caso a caso | Variable, evaluar cada uno |
| **Steam (cliente)** | Solo vía capa de compatibilidad Linux — Valve no publica el cliente para OSes nuevos, nunca lo hará | Alta — depende de (1)+(2)+(3) arriba |
| **Discord, Spotify** | Electron — misma capa de compatibilidad, mismos prerrequisitos que Steam | Alta |
| **Navegadores reales (Chromium/Blink, Zen/Gecko)** | Capa de compatibilidad + son el software con más dependencias que existe (V8, compositing GPU, sandboxing por namespaces del kernel, cientos de códecs) | Muy alta — ni con la capa lista es trivial; es la pieza más dura de todo el roadmap. **Su arquitectura sí se estudia** (ver nota abajo) aunque no se intente portar |

## Firmware abierto — lo que es físicamente posible hoy

Nadie tiene esto resuelto al 100%, para ningún sistema operativo:

- **coreboot** existe y funciona en placas concretas, pero incluso ahí
  sigue dependiendo de blobs binarios (Intel FSP) en silicio moderno para
  funcionalidad completa.
- **Libreboot** (el fork que exige cero blobs) tiene soporte de hardware
  extremadamente limitado — su hito reciente más "moderno" es un ThinkPad
  ya antiguo para estándares de 2026.
- **Intel ME**: se puede neutralizar parcialmente (`me_cleaner`), pero
  exige reprogramar físicamente el chip flash SPI — riesgo real de dejar
  la placa inservible.
- **AMD PSP**: integrado en el propio die de la CPU, sin herramienta
  equivalente universal — ni los propios desarrolladores de coreboot lo
  desactivan de forma sistemática en Ryzen moderno.
- **Microcódigo de CPU**: cerrado y firmado en ambos fabricantes, sin
  excepción, para cualquier sistema operativo.

Esto no es un problema que Forge OS resuelva mejor que Linux o FreeBSD —
es una limitación física de cómo se fabrica el silicio x86_64 hoy,
completamente independiente del kernel que corra encima. Donde coreboot
tenga soporte de placa (mayormente hardware de referencia, Chromebooks, y
un puñado de placas de escritorio muy concretas), corre **por debajo**
de Forge OS como firmware — no es algo que nosotros incluyamos ni
mantengamos.

## ChromeOS — aclaración de referencia

La referencia a ChromeOS en `PRODUCT.md`/`DESKTOP.md` es puramente
**visual** (minimalismo, "completitud" de la interfaz) — nunca técnica.
Forge OS no lleva Chrome/Chromium en ningún sitio del núcleo; es
precisamente lo contrario de "liviano" y no aporta nada a la base del
sistema. Ver `PHILOSOPHY.md` — cero deuda heredada.

## Nota: la arquitectura de Chromium sí se estudia (sin intentar portarlo)

No se persigue portar Chromium/Blink — pero su arquitectura real (no la
fuente) aporta dos lecciones directamente aplicables si algún día
construimos un navegador propio mínimo (nuestra versión de "Selene", el
de nyxos-dev):

1. **Separación por procesos (browser process vs renderer process)** —
   la innovación central de Chromium (2008): cada pestaña en su propio
   proceso aislado, así un crash o compromiso no se lleva el navegador
   entero. Encaja directo con lo que ya tenemos: `caps.rs` ya es el
   mecanismo — un proceso "browser" con privilegios normales, un
   proceso "renderer" por pestaña con `Capabilities` reducidas al
   mínimo (sin red directa, sin filesystem, todo mediado por IPC).
2. **`content/` como frontera de embedder** — Google separó el motor
   reutilizable (`content/`) de su navegador concreto (`chrome/`),
   comunicados por una API estable, no por que `content` conozca
   detalles de `chrome`. Misma idea de capas que ya aplicamos en
   `ARCHITECTURE.md` (Kernel→Executive→HAL).

Nota al margen: los repos públicos de Discord (`react-native-screens`,
`discord-api-docs`, etc.) son forks de terceros y documentación de
API — ninguno es el cliente real. Confirma que sigue sin haber fuente
que estudiar ni portar para Discord en sí.

## Dónde vive esto en el roadmap

**Nota (revisitada):** se planteó reabrir virtualización tipo KVM y
scheduler de GPU. Siguen fuera de alcance por los mismos motivos de
siempre — KVM necesita VT-x/AMD-V + hipervisor + virtio, del tamaño de
otro subsistema del kernel; un scheduler de GPU necesita primero un
driver de GPU con cómputo real, que no existe. Quedan como aspiración
lejana explícita, no como próximo paso.

**No es un milestone cercano.** La capa de compatibilidad Linux completa
es, como mínimo, tan grande como todo lo que llevamos construido del
kernel hasta ahora — probablemente más. Se aborda una vez el OS nativo
(M0-M6, ver `ARCHITECTURE.md`) esté sólido y validado en hardware real,
no antes. VLC como port nativo directo es la única pieza de esta lista
que podría adelantarse, si acaso, una vez exista VFS + red básica.
