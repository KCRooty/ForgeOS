# Diseño del escritorio — Anvil

## El principio central

Hyprland real NO es "todo en uno": el compositor es un binario, la barra es
Waybar (otro binario), el launcher es rofi/wofi (otro), las notificaciones
son mako/dunst (otro), el wallpaper es hyprpaper (otro). Cinco procesos
hablando por IPC. Anvil rechaza esto a propósito: **un solo binario,
un solo proceso, un solo event loop.** Nada de protocolos entre
componentes porque no hay componentes separados — son capas de la misma
cosa.

Precedente ya validado en lo que estudiamos: **Hemera** (nyxos-dev) y el
**compositor** de Asmodeus14/Nyx ya funcionan así — el compositor dibuja
taskbar, start menu y wallpaper él mismo. Anvil parte de ese patrón
y le añade el comportamiento de tiling de Hyprland encima.

## Capas dentro del mismo proceso

```
┌────────────────────────────────────────────────┐
│  Anvil (un solo proceso, un event loop)   │
│                                                  │
│  1. Wallpaper       — fondo, siempre debajo      │
│  2. Tiling core      — layout BSP/master-stack,  │
│                        gaps, workspaces           │
│  3. Ventanas cliente — decoraciones, foco, drag   │
│  4. Taskbar/panel    — leída del mismo estado     │
│                        de ventanas, sin IPC        │
│  5. Launcher overlay — capa modal sobre todo       │
│  6. Notificaciones   — capa modal, no-bloqueante   │
│                                                  │
│  Todas leen/escriben la MISMA estructura de      │
│  estado (lista de ventanas, workspace activo).   │
└────────────────────────────────────────────────┘
```

Las apps cliente (terminal, editor, lo que sea) siguen siendo procesos
aparte — eso sí tiene que serlo, son programas de usuario. Lo que NO se
separa es compositor/barra/launcher/notificaciones/wallpaper: eso es una
sola cosa por diseño, no por optimización.

## Comportamiento heredado de Hyprland

- **Tiling dinámico por defecto (BSP/dwindle)** — cada ventana nueva
  parte el espacio de la ventana enfocada a la mitad, recursivamente
  (comportamiento por defecto de Hyprland). Modo flotante disponible por
  ventana (toggle), no forzado.
- **Workspaces** — múltiples escritorios virtuales, cambio instantáneo,
  cada uno con su propio layout de tiling independiente.
- **Gaps configurables** — espacio entre ventanas y con el borde de
  pantalla, ajustable en el config.
- **Keybind-first** — Super+tecla como método principal de gestión de
  ventanas (mover foco, mover ventana, cambiar workspace, resize), ratón
  como complemento, no como requisito.

## Comportamiento heredado de "estilo Windows/macOS/KDE" — diseño mixto

**Primera pasada, se afina más adelante** — la idea general:

- **Barra superior** (estilo macOS/KDE) — reloj, fecha, botón de
  inicio/launcher, tray de apps en segundo plano. Zona de "estado del
  sistema", siempre visible.
- **Barra inferior** (estilo Windows) — taskbar clásica: apps ancladas +
  ventanas abiertas, con su propio menú de inicio sencillo (más orientado
  a lanzar apps rápido que la barra superior). Dibujada por la misma capa
  4, no un segundo proceso.
- **Escritorio con fondo** — capa 1, wallpaper configurable, posible
  soporte de iconos de escritorio más adelante (no crítico para v1).

Dos barras, mismo proceso, mismo estado compartido — nada de duplicar
lógica de "qué ventanas hay abiertas" entre ellas.

## Configuración — un solo fichero

Filosofía Hyprland en esto sí se queda: **texto plano, declarativo,
editable a mano** (nada de Registry binario). Pero a diferencia del
Hyprland real, donde `hyprland.conf` solo configura el WM y necesitas
`waybar/config.jsonc` + `mako/config` + `hyprpaper.conf` aparte, aquí es
**un único fichero** que cubre WM + barra + wallpaper + keybinds + colores.
Formato exacto (TOML/formato propio tipo Hyprland) — pendiente de decidir.

## Encaje con el roadmap del kernel

Esto vive en el M6+ (HAL de gráficos) del `ARCHITECTURE.md` del kernel:
necesita framebuffer (M3) y, idealmente, el trait `GfxAdapter` (fase 1 del
roadmap gráfico) antes de que tenga sentido escribir una sola línea de
Anvil. No es bloqueante para el diseño — sí lo es para el código.

## Decisiones ya cerradas

- Layout de tiling: **BSP/dwindle**
- Nombre: **Anvil**
- Barras: **dos** — superior estilo macOS/KDE (reloj, fecha, inicio, tray),
  inferior estilo Windows (taskbar + launcher sencillo) — diseño de
  primera pasada, se optimiza más adelante

## Decisiones abiertas

- Formato exacto del fichero de config (TOML vs formato propio tipo
  Hyprland)
- Detalle fino del reparto de responsabilidades entre las dos barras
