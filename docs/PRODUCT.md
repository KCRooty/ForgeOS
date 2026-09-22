# Filosofía de producto — Forge OS

Ver [PHILOSOPHY.md](PHILOSOPHY.md) para el diseño técnico del kernel. Este
documento es el otro lado: a quién sirve, por qué alguien lo instalaría, y
cómo "sirve para todo" sin convertirse en una promesa vacía.

## El problema de "multiusos"

Gaming quiere latencia mínima. Un servidor quiere throughput de red y
estabilidad bajo carga sostenida. Seguridad quiere superficie de ataque
mínima. Ninguna distro seria —ni Fedora, ni CachyOS, ni Arch— optimiza las
tres cosas a la vez con una sola configuración, porque **no se puede**: son
objetivos que compiten entre sí a nivel de scheduler, servicios activos por
defecto, y qué se prioriza cuando hay contención de recursos.

La solución de Forge OS no es fingir que no hay tensión. Es separar **núcleo
compartido** (siempre igual, siempre seguro y rápido por diseño) de
**perfiles de instalación** (lo que cambia según para qué lo vayas a usar).

## Núcleo — siempre presente, sin importar el perfil

| Pilar | Qué significa en la práctica |
|---|---|
| **Open Source** | GPL-2.0+. Código público desde el día 1. Cualquier derivado tiene que seguir siendo abierto — sin excepciones, sin forks cerrados. |
| **Liviano** | Cero servicios de fondo no pedidos. Cero telemetría. Arranque mínimo por defecto. |
| **Privado** | Nada sale a internet sin que el usuario lo pida explícitamente. Sin "phone home", sin analytics ocultos. |
| **Estable** | Batería de KATs en CI (heredado de la disciplina de testing que vimos en los dos Nyx) — nada rompe sin que un test lo grite antes de merge. |
| **Rápido** | Kernel monolítico + capability gating barato ya lo garantiza en el diseño, no como optimización posterior. |
| **Personalizable** | WM/shell/tema intercambiables. Configuración en texto plano, nunca un Registry binario ilegible. |
| **Seguro por diseño** | El OS en sí es difícil de comprometer — hardening y sandboxing por proceso vía `caps.rs` activos siempre, no solo en un perfil "de seguridad". Esto NO es un perfil aparte tipo Kali/Parrot: Forge OS no trae herramientas de pentesting preinstaladas, es la base la que es dura de romper. |

## Perfiles — lo que cambia según el uso

Un mismo núcleo, distinta configuración de arranque/paquetes en cada caso —
como Fedora Workstation/Server/Silverblue desde una sola base:

| Perfil | Qué se ajusta |
|---|---|
| **Gaming** | Scheduler de baja latencia, drivers GPU al día (fase HAL ya planeada), servicios de fondo reducidos al mínimo durante sesión activa |
| **Server** | Sin GUI por defecto, red hardening extra, prioridad a throughput/uptime sobre latencia interactiva |
| **Ofimática** | Apps de escritorio + estabilidad ante todo, GUI completa, nada exótico |
| **Hardened** (el "cyberseguridad" original, redefinido) | Capability gating más estricto por defecto (incluyendo Capsicum-style, por fd — ver `PHILOSOPHY.md`), aislamiento de procesos tipo *jails* de FreeBSD para apps no confiables, logging de auditoría activo, superficie de ataque reducida a propósito — sigue siendo uso normal, solo que paranoico |

El "trabajar de todo" no es una promesa de que un solo build hace
perfectamente las cuatro cosas a la vez — es que el mismo núcleo seguro y
rápido es la base de las cuatro, y eliges perfil al instalar según lo que
vayas a hacer con esa máquina en concreto.

## Qué NO prometemos

- No prometemos rendimiento de gaming punta con el perfil Server activo
- No prometemos que el perfil Hardened sea cómodo para gaming
- No incluimos herramientas de pentesting por defecto en ningún perfil —
  eso, si se quiere algún día, sería un perfil aparte explícito, no algo
  mezclado en "seguridad"
