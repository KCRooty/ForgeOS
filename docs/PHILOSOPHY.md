# Filosofía de Forge OS

Forge OS no fusiona código de Linux, OpenBSD y Windows NT — eso no tiene sentido
(licencias distintas, ABIs distintas, décadas de deuda distinta). Fusiona
**decisiones de diseño**, cada una robada del sitio que la hizo mejor que
nadie, e implementada desde cero en Rust.

## Los seis principios

### 1. Seguro por defecto, no por parche — *(OpenBSD + FreeBSD)*
Deny-first en todo. Un proceso declara una única vez, al arrancar, qué
categorías de syscall necesita (`pledge()`-style). El kernel congela esa
máscara para toda su vida: pedir algo fuera de lista es `EPERM` permanente,
sin excepciones, sin "ampliar permisos más tarde". Añadir seguridad después
de escribir la feature es lo que produce CVEs; declararla como precondición
la hace estructuralmente imposible.

Complementado con dos ideas más allá de lo que ya teníamos:
- **Capsicum** (FreeBSD) — capabilities por *file descriptor individual*,
  más finas que las categorías de `caps.rs`. Un proceso en "modo
  capability" no puede abrir rutas absolutas nuevas, solo operar sobre
  los fds que ya tiene. Objetivo futuro que afina el modelo, no lo
  sustituye.
- **privsep** (OpenBSD, usado en OpenSSH y su propio `httpd`) — patrón
  arquitectónico para cualquier demonio del sistema: partir el servicio
  en un proceso sin privilegios (trabajo real) + un supervisor mínimo con
  privilegios, comunicados por un canal estrecho y auditable. Forma
  estándar de construir servicios en Forge OS a partir de ahora.

### 2. Rápido porque es simple — *(Linux)*
Kernel monolítico. Syscalls directas vía `syscall`/`sysret`, sin capas de
paso de mensajes entre servidores como haría un microkernel puro. Se
sacrifica el aislamiento teórico de un microkernel a cambio de latencia real
más baja — es la misma apuesta que ganó Linux frente a Hurd/Mach.

### 3. Estructurado en capas, no en espagueti — *(Windows NT)*
```
┌─────────────────────────────────────┐
│  Kernel   — scheduler, IRQ, locks    │  mínimo, no crece
├─────────────────────────────────────┤
│  Executive — Object Manager,         │  la mayoría de la lógica
│              I/O Manager, Process Mgr│  vive aquí
├─────────────────────────────────────┤
│  HAL      — Intel/AMD/NVIDIA/ARM,    │  específico de vendor,
│             red, storage             │  detrás de traits
└─────────────────────────────────────┘
```
Cada capa solo conoce la de abajo. Meter un vendor de GPU nuevo no toca el
Kernel ni el Executive — solo añade una implementación de trait en la HAL.

### 4. Todo es un objeto navegable — *(NT Object Manager + Plan 9)*
Ficheros, procesos, sockets, memoria compartida, primitivas de
sincronización — todos son el mismo tipo de handle con refcounting
(`Arc<KernelObject>`, prácticamente gratis en Rust). Un solo `close(handle)`
cierra cualquier cosa. De Plan 9 heredamos que ese conjunto de objetos es
además un único árbol navegable, no tablas separadas por subsistema como
hace Linux (fds, pids, shmids cada uno por su lado).

### 5. Memoria segura por construcción, no por auditoría — *(Rust)*
OpenBSD logra su nivel de seguridad de memoria a base de disciplina y
auditoría humana constante y brutal. Nosotros perseguimos el mismo
resultado porque el compilador rechaza la clase de bug entera en tiempo de
compilación. Mismo objetivo, coste operativo radicalmente distinto.

### 6. Cero deuda heredada
ABI propia desde el día uno. Ninguna intención de compatibilidad binaria
con Windows, Linux o nada externo — si algún día se quiere correr `.exe` o
ELFs de Linux, eso es una capa de traducción *encima* del kernel (estilo
Wine), nunca dentro del núcleo. Identidad propia en todo: nombre propio
para el compositor, el shell, el gestor de paquetes. No clonar, extender.

## Qué NO somos

- No somos ReactOS (no perseguimos compatibilidad binaria con Windows)
- No somos un microkernel puro (elegimos rendimiento sobre aislamiento
  teórico máximo)
- No somos un clon estético de nada — paleta, nombres y decisiones de UX
  son propias de Static Forge
