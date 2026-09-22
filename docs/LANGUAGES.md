# Lenguajes de programación — desarrollo y ejecución nativa

## Las dos cosas distintas que "lenguaje nativo" puede significar

1. **Compilación cruzada** — escribes en tu PC (Claude Code, VS Code),
   compilas apuntando al target de Forge OS, el binario corre en Forge
   OS. **Ya funciona hoy** para cualquier lenguaje cuyo compilador
   soporte targets `no_std`/freestanding personalizados (Rust, C, C++,
   Zig...) — es exactamente lo que hacemos con el kernel. Coste: casi
   cero, ya lo tenemos.
2. **Ejecución nativa en vivo** — un intérprete o compilador corriendo
   *dentro* de Forge OS, sin salir del sistema (`python script.py` desde
   la propia consola, `cc programa.c` como hace TinyCC en nyxos-dev).
   **Esto sí es trabajo real**, y el coste varía muchísimo según el
   lenguaje.

Todo lo de abajo es sobre el punto 2 — el 1 ya está resuelto por diseño.

## La base compartida — invertir aquí primero

Igual que con Steam/Discord: casi todo de esta lista se vuelve
alcanzable en cuanto exista **una superficie POSIX-ish decente en
nuestra libc** (ficheros, hilos, memoria dinámica — gran parte se solapa
con M4/M5 ya planificados) más **nuestro propio compilador de C
corriendo dentro del OS** (precedente directo: TinyCC portado en
nyxos-dev, capaz de auto-compilarse). Con esas dos piezas, la mayoría de
software C portable se vuelve candidato realista a port nativo.

## Ranking de dificultad real, lenguaje por lenguaje

| Lenguaje | Vía realista | Dificultad |
|---|---|---|
| **C** | Compilador propio in-OS, precedente directo TinyCC (nyxos-dev) | Media — ya sabemos que se puede, hay un ejemplo funcionando |
| **Python** | Port de CPython sin extensiones en C inicialmente — el intérprete core es sorprendentemente portable, hay precedentes históricos en plataformas muy raras | Media — de lo más alcanzable de la lista, una vez la libc esté decente |
| **JavaScript** | **QuickJS**, no V8 — motor de un solo autor, pensado para ser pequeño y portable, sin las asunciones de plataforma de V8/SpiderMonkey | Media — QuickJS es literalmente la opción de motor JS que existe para este tipo de proyecto |
| **ffmpeg** | Build mínima, solo códecs software, sin aceleración por hardware | Media-alta — el propio proyecto es portable por diseño, pero es grande |
| **C++** | Mucho más difícil que C — TinyCC no cubre C++; hacen falta compiladores del calibre de GCC/Clang, proyectos de esa escala | Alta |
| **HTML** (como renderizado, no como "lenguaje ejecutable") | Ver `COMPATIBILITY.md` — motor de navegador real, la pieza más dura de todo el roadmap | Muy alta |
| **Rust (self-hosted, corriendo dentro de Forge OS)** | rustc depende de LLVM, proceso de bootstrap notoriamente complejo — muy distinto de "compilar Rust PARA Forge OS desde fuera", que ya funciona | Muy alta |
| **Java (JVM)** | El runtime más pesado de la lista — GC, JIT, biblioteca de clases enorme | Muy alta, probablemente la más difícil de toda la lista |

## Recomendación de orden

1. **C in-OS** — primera prioridad, mismo patrón que TinyCC, desbloquea
   además el propio gestor de paquetes (`forge get`/`forge rm`, ya
   anotado en `SHELL.md`) si se decide compilar-desde-fuente
2. **Python** — segunda prioridad, alto impacto (scripting, automatización
   del propio sistema) por esfuerzo relativamente contenido
3. **QuickJS** — tercera, si se quiere algo de JS sin perseguir un
   navegador real
4. **ffmpeg mínimo** — se solapa directamente con la sección de
   multimedia de `TODO.md`
5. **C++, Rust self-hosted, Java** — aspiracionales, sin fecha, se
   revisan si el resto del sistema está sólido y sigue habiendo interés

## Dónde vive esto en el roadmap

Depende de M4 (procesos reales) + M5 (VFS) como mínimo — ningún
intérprete/compilador puede correr sin poder abrir ficheros ni tener
procesos de verdad. No es un milestone inmediato, pero la base que ya
estamos construyendo (M4a en marcha) es literalmente el primer
prerrequisito de todo esto.
