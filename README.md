# Forge OS

Kernel monolítico x86_64 en Rust, bare-metal, con filosofía híbrida:
- **Linux**: ABI de syscalls numeradas x86_64, ecosistema de drivers como referencia
- **OpenBSD**: seguridad por defecto — W^X, superficie de ataque mínima, memoria negada por defecto
- **Windows NT**: HAL — capa de abstracción de hardware; el código de vendor (GPU, red, etc)
  vive detrás de traits, nunca hardcodeado en el core del kernel

Bajo el paraguas de Static Forge. **Licencia: GPL-2.0+.**

Ver [docs/PHILOSOPHY.md](docs/PHILOSOPHY.md) (diseño técnico) y
[docs/PRODUCT.md](docs/PRODUCT.md) (a quién sirve y por qué) para el
razonamiento completo detrás de cada decisión.

## Estado: red (`ping`), Ember (PID 1 real, arranca solo en el boot y no termina nunca), ext2 de solo lectura, escáner GPT/MBR (`partinfo`), preemption real en kernel y en ring 3 (`preempttest`/`preempttest3`), señales SIGKILL/SIGTERM/SIGSEGV (`segvtest`, `killtest`) y `read`/`write`/`open`/`close`/`brk`/`mmap`/`munmap` reales + pila inicial compatible con `crt0` — primer paso POSIX completo hacia mlibc (`posixtest`, `crt0test`) — todo verificado en QEMU con más de 35 arranques repetidos desde cero, tras arreglar DOS bugs reales de concurrencia que solo salían a la luz repitiendo el arranque: `execve()` no sincronizaba la tabla de páginas nueva con la copia que usa el scheduler (corrupción intermitente), y un `sti` que faltaba en el cambio de contexto dejaba el sistema entero bloqueado para siempre en cuanto un hilo de kernel que nunca cede el turno recibía su primer turno (ver `docs/TODO.md`). Con esto, el TODO original de la reconstrucción está completo; el trabajo actual apunta a escribir el sysdeps de mlibc contra esta ABI + Bellows (shell real).

## Referencias de arquitectura estudiadas
- Asmodeus14/Nyx (Rust, QCLang, motor 3D Gen9.5 hand-rolled)
- nyxos-dev/nyx-os (C+ASM, TLS 1.2 real, DOOM portado, lenguaje N propio)
- Linux (ABI syscalls, modelo de drivers)
- Windows NT (HAL, driver model en capas)
- OpenBSD (pledge/unveil, W^X, seguridad por defecto)

## Build (CachyOS / Arch)
```bash
yay -S rustup nasm qemu-full grub xorriso lld
rustup toolchain install nightly-2026-07-01
rustup component add rust-src --toolchain nightly-2026-07-01
./tools/build.sh
```

`kernel/rust-toolchain.toml` fija el nightly exacto (el esquema del
target-spec JSON cambia entre nightlies, así que no vale cualquiera) —
`tools/build.sh` ya hace `cd kernel` antes de invocar `cargo build`,
que es donde rustup detecta y aplica ese pin automáticamente.

## Run en QEMU
```bash
./tools/run-qemu.sh              # gráfico, interactivo
./tools/run-qemu.sh --headless   # sin display, log a boot.log
```

Ambos modos arrancan con `-nic user,model=rtl8139` — necesario para que
QEMU exponga de verdad una tarjeta RTL8139 por PCI (el modelo por
defecto sin esta flag es un e1000, que el driver de `rtl8139.rs` ni
detecta). Desde la consola de depuración, `ping` hace un ARP + ICMP
echo completo contra el gateway de slirp (10.0.2.2).

Si existe `disk.img` en la raíz del repo, ambos modos lo adjuntan
también por AHCI automáticamente (con `-boot order=d`, para que SeaBIOS
no intente arrancar de ese disco en vez del CD-ROM si tiene una firma
MBR válida) — genéralo con:
```bash
./tools/make-test-disk.sh   # requiere e2fsprogs (mkfs.ext2, debugfs)
```
Crea un ext2 real de 16 MiB con un par de ficheros de prueba. Desde la
consola: `ext2ls /`, `ext2cat /hello.txt`.

Para probar el escáner de particiones (`partinfo.rs`), genera y copia
como `disk.img` cualquiera de los dos discos de:
```bash
./tools/make-partinfo-test-disks.sh   # + parted, dosfstools, ntfs-3g, btrfs-progs
cp partinfo-gpt.img disk.img          # o partinfo-mbr.img
```
Desde la consola: `partinfo`.
