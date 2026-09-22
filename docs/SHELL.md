# Shell, Terminal y consola de depuración

Tres piezas distintas, con dependencias distintas — no confundirlas.

## 1. La consola de depuración (buildable ya, M2)

REPL mínimo sobre el puerto serie. No es el shell del producto final, es
herramienta de bring-up: sin ella, verificar que el kernel funciona
significa re-flashear y leer `boot.log` cada vez que quieres probar algo
distinto. Con ella, se puede interactuar en caliente sobre una sesión de
QEMU ya arrancada.

Vive en `kernel/src/console.rs`. Comandos v1: `help`, `meminfo` (usa
`pmm::free_frame_count()`), `caps` (repite la demo de capability gating a
demanda), `bp` (dispara `int3` para probar el IDT), `panic` (fuerza un
panic controlado para probar el panic handler).

No depende de procesos, GUI, ni PS/2 — lee directamente del registro de
estado de línea de la UART (`COM1+5`), y `run-qemu.sh` en modo interactivo
ya conecta el teclado real del host a ese puerto vía `-serial stdio`.

## 2. El shell real — nombre provisional "Bellows"

El intérprete de comandos completo, equivalente en ambición a `sh.c` de
nyxos-dev: pipelines, redirección, `&&`/`||`/`;`, quoting, sustitución de
comandos, job control, glob de paths. Corre como proceso real en ring 3.

**Bloqueado por M4** — necesita tabla de procesos, `fork`/`execve` reales,
y pipes. No tiene sentido escribir ni una línea de esto antes de que
exista gestión de procesos de verdad.

## 3. El terminal — nombre provisional "Crucible"

App gráfica que aloja al shell: renderiza texto con scroll, cursor,
colores (secuencias de escape ANSI o un subconjunto propio), reenvía
teclas al shell. Vive dentro de Anvil como una ventana normal.

**Bloqueado por M6+** — necesita Anvil (compositor) y, antes que eso, el
framebuffer de M3.

## Por qué el nombre no es "bash"

Coherente con el principio 6 de `PHILOSOPHY.md` — identidad propia, no
clonar. "Bellows" (los fuelles de una fragua) encaja con el tema Static
Forge/Anvil: el fuelle es lo que "respira" comandos hacia el sistema, el
mismo espíritu que un shell respecto al kernel.
