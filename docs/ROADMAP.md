# Roadmap consolidado — subsistemas y presupuesto de recursos

## Presupuesto de memoria — corrección de expectativa

**"3-4GB en idle con Discord/Steam abiertos" no es un objetivo del sistema
operativo — esa cifra la ponen Discord y Steam, no nosotros.** Discord es
Electron (Chromium completo embebido); Steam trae su propio runtime
pesado. Ninguno de los dos corre en Forge OS sin una capa de
compatibilidad binaria con Linux/Windows — proyecto propio, del tamaño
del kernel, **no incluido en este roadmap todavía** (ver "Fuera de
alcance" al final).

**Objetivo real — Forge OS nativo, sin ninguna app de compatibilidad:**

| Escenario | Objetivo |
|---|---|
| Kernel + consola, sin GUI | < 20 MB |
| Kernel + Anvil (escritorio), idle, sin apps abiertas | < 100 MB |
| Anvil + gestor de archivos + terminal + visor multimedia abiertos | < 250 MB |

Referencia: Arch+Hyprland real idlea sobre 400-600 MB (lleva systemd,
D-Bus, PipeWire...). ChromeOS idlea 1-1.5 GB (lleva Chrome completo).
Nosotros no llevamos nada de eso — el objetivo de <100MB en idle es
ambicioso pero realista para un kernel hecho a mano sin herencia de
ningún sistema existente.

## Mapa de features → milestone

| Feature pedida | Dónde vive | Depende de |
|---|---|---|
| **Reloj/horario** | Driver RTC (lectura CMOS) — sencillo, similar a lo que ya vimos en los dos Nyx | Ninguna dependencia nueva — se puede montar pronto (M3c) |
| **Gestor de archivos** | App de Anvil | VFS (M5) + Anvil (M6+) |
| **Visor multimedia** | App de Anvil + decodificadores propios (PNG/BMP/GIF mínimo, como nyxos-dev) | VFS (M5) + Anvil (M6+) |
| **Red — Ethernet** | Driver RTL8139-style (bien documentado, emulable en QEMU) | HAL de red — puede empezar en paralelo a M4 |
| **Red — WiFi** | Driver de vendor real | Más difícil que Ethernet — a menudo necesita firmware binario del fabricante, similar al caso GPU. Fase posterior. |
| **Usuario / admin** | Ya tenemos la pieza central: `caps.rs`. "Admin" no es una bandera mágica, es literalmente `CAP_UNRESTRICTED` vs un proceso de usuario normal pledgeado hacia abajo | M4 (tabla de procesos real) |
| **Comandos (`ls`/`cd`/paquetes)** | "Bellows" (el shell, M4+) + gestor de paquetes propio | M4 |
| **Minimalismo ChromeOS + liviandad Arch/Hyprland** | Filosofía transversal, no un componente — ya está en `PHILOSOPHY.md` (cero servicios de fondo no pedidos) | Aplica a todo lo de arriba |

## Comandos — filosofía de nombres

Los comandos cortos tipo `ls`/`cd`/`cat`/`ps` **no son "de Linux"** en el
sentido de marca — son convención Unix genérica de décadas, no algo que
clonemos, así que se quedan tal cual (mismo criterio que usa nyxos-dev en
sus propios coreutils).

Lo que sí necesita nombre propio:

- **Escalar a admin**: no será `sudo` renombrado por renombrar — se
  diseña como una extensión natural del modelo de capabilities: un
  proceso sin privilegios pide ampliar temporalmente su `Capabilities`,
  requiere autenticación interactiva, y el kernel lo audita. Nombre
  provisional: `elevate <comando>`.
- **Gestor de paquetes**: nombre provisional `forge get <paquete>` /
  `forge rm <paquete>` — corto, memorable, ligado a la marca. **Decidido
  vía el modelo de FreeBSD (ports + pkg):** una colección de recetas que
  compilan desde fuente con nuestro propio `cc` (equivalente a "ports"),
  más binarios precompilados generados a partir de esas mismas recetas
  para instalación rápida (equivalente a "pkg"). No hace falta elegir
  entre "compila siempre" o "solo binarios" — FreeBSD ya demostró que
  las dos cosas conviven desde la misma fuente de verdad. Verificación
  de paquetes vía firma **signify**-style (OpenBSD) — verifica autoría
  real, no solo que el binario no se ha tocado (que es lo único que da
  un hash SHA-256 suelto).

## Fuera de alcance (por ahora)

Ver [COMPATIBILITY.md](COMPATIBILITY.md) para el análisis completo —
capa de compatibilidad Linux (Steam, Discord, Spotify, navegadores
reales) y el estado real del firmware/hardware abierto (coreboot,
Intel ME, AMD PSP). Ver [LANGUAGES.md](LANGUAGES.md) para el mismo
análisis aplicado a lenguajes de programación nativos (Python, C++,
Rust self-hosted, Java, JavaScript, ffmpeg...). Resumen: nada de esto
está a la vuelta de la esquina, y nadie lo tiene resuelto al 100% hoy —
ni Linux, ni FreeBSD, ni nadie.
