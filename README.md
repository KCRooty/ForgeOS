<img src="assets/logo.png" alt="Forge OS" width="96" height="96">

# Forge OS

**Página del proyecto:** [staticforge.me/forge-os](https://staticforge.me/forge-os/)

Kernel monolítico x86_64 en Rust, bare-metal, con filosofía híbrida:
- **Linux**: ABI de syscalls numeradas x86_64, ecosistema de drivers como referencia
- **OpenBSD**: seguridad por defecto — W^X, superficie de ataque mínima, memoria negada por defecto
- **Windows NT**: HAL — capa de abstracción de hardware; el código de vendor (GPU, red, etc)
  vive detrás de traits, nunca hardcodeado en el core del kernel

Bajo el paraguas de Static Forge. **Licencia: GPL-2.0+.**

Ver [docs/PHILOSOPHY.md](docs/PHILOSOPHY.md) (diseño técnico) y
[docs/PRODUCT.md](docs/PRODUCT.md) (a quién sirve y por qué) para el
razonamiento completo detrás de cada decisión.

## Estado

Forge OS arranca, monta un ext2 real y ejecuta programas en C:

```
forge> run /bin/hello
hello, forge! (C + mlibc)
argc=1 argv[0]=/bin/hello
malloc(65536) OK, checksum=8355840 (esperado 8355840)
H(100)=5.187378 (esperado 5.187378)
[run] PID 4 terminó con código 42
```

- **Procesos:** espacios de direcciones propios, `fork`/`execve`/`wait`/
  `exit`, preemption real por timer en kernel y en ring 3, señales con
  su acción por defecto (un fallo en un programa mata al programa, no
  al sistema), Ember como PID 1.
- **Userland:** [mlibc](https://github.com/managarm/mlibc) portada
  (parte ANSI C), binarios ELF estáticos cargados desde disco, FPU/SSE
  por proceso.
- **Almacenamiento y red:** AHCI, ext2 de solo lectura, escáner GPT/MBR,
  RTL8139 con ARP + ICMP (`ping`).
- **Pruebas:** `tools/smoke-test.sh` pasa toda la batería en QEMU y
  comprueba la salida.

Lo siguiente: la parte POSIX de mlibc (`fork`/`exec` desde C) y Bellows,
el shell real. El inventario completo — lo hecho, lo que falta y cada
bug encontrado por el camino — está en [docs/TODO.md](docs/TODO.md).

## Referencias de arquitectura estudiadas
- Asmodeus14/Nyx (Rust, QCLang, motor 3D Gen9.5 hand-rolled)
- nyxos-dev/nyx-os (C+ASM, TLS 1.2 real, DOOM portado, lenguaje N propio)
- Linux (ABI syscalls, modelo de drivers)
- Windows NT (HAL, driver model en capas)
- OpenBSD (pledge/unveil, W^X, seguridad por defecto)

## Build (CachyOS / Arch)
```bash
yay -S rustup nasm qemu-full grub xorriso lld
rustup toolchain install nightly-2026-10-10
rustup component add rust-src --toolchain nightly-2026-10-10
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

## Programas de usuario (C + mlibc)

```bash
yay -S clang lld llvm meson ninja       # clang >= 19
./userland/mlibc-sysdeps/build-mlibc.sh # una vez: clona y compila mlibc
make -C userland                        # compila userland/c/*.c
./tools/make-test-disk.sh               # los copia a /bin de disk.img
```

Desde la consola: `run /bin/hello`, `run /bin/libctest` (autoprueba de
la libc, código de salida = pruebas fallidas), `run /bin/crash div0`.
Un `&` final lanza en segundo plano; `wait` espera a que terminen.

## Pruebas de regresión

```bash
./tools/smoke-test.sh       # una pasada de toda la batería en QEMU
./tools/smoke-test.sh 15    # 15 arranques en frío seguidos
./tools/qemu-console.py "run /bin/hello" ps meminfo   # comandos sueltos
```

Para probar el escáner de particiones (`partinfo.rs`), genera y copia
como `disk.img` cualquiera de los dos discos de:
```bash
./tools/make-partinfo-test-disks.sh   # + parted, dosfstools, ntfs-3g, btrfs-progs
cp partinfo-gpt.img disk.img          # o partinfo-mbr.img
```
Desde la consola: `partinfo`.
