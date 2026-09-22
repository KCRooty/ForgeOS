# Compatibilidad de aplicaciones y firmware abierto

## La decisión de ingeniería: una capa, no N ports

No se portea cada app suelta. Se construye **una capa de compatibilidad
con la ABI de Linux** — un "Linuxulator" propio. Es la estrategia
verificada (ver más abajo) que usa FreeBSD para conseguir Steam/Proton
parcialmente funcional, y desbloquea de golpe: Steam, la mayoría del
software Electron (Discord, Spotify), y cualquier binario Linux que no
dependa de systemd/D-Bus/cgroups en profundidad.

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
| **Navegadores reales (Chromium/Blink, Zen/Gecko)** | Capa de compatibilidad + son el software con más dependencias que existe (V8, compositing GPU, sandboxing por namespaces del kernel, cientos de códecs) | Muy alta — ni con la capa lista es trivial; es la pieza más dura de todo el roadmap |

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

## Dónde vive esto en el roadmap

**No es un milestone cercano.** La capa de compatibilidad Linux completa
es, como mínimo, tan grande como todo lo que llevamos construido del
kernel hasta ahora — probablemente más. Se aborda una vez el OS nativo
(M0-M6, ver `ARCHITECTURE.md`) esté sólido y validado en hardware real,
no antes. VLC como port nativo directo es la única pieza de esta lista
que podría adelantarse, si acaso, una vez exista VFS + red básica.
