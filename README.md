# Forge OS (nombre provisional)

Kernel monolítico x86_64 en Rust, bare-metal, con filosofía híbrida:
- **Linux**: ABI de syscalls numeradas x86_64, ecosistema de drivers como referencia
- **OpenBSD**: seguridad por defecto — W^X, superficie de ataque mínima, memoria negada por defecto
- **Windows NT**: HAL — capa de abstracción de hardware; el código de vendor (GPU, red, etc)
  vive detrás de traits, nunca hardcodeado en el core del kernel

Bajo el paraguas de Static Forge. **Licencia: GPL-2.0+.**

Ver [docs/PHILOSOPHY.md](docs/PHILOSOPHY.md) (diseño técnico) y
[docs/PRODUCT.md](docs/PRODUCT.md) (a quién sirve y por qué) para el
razonamiento completo detrás de cada decisión.

## Estado: M4d — procesos con espacio de direcciones propio (PML4 real, CR3 cambia de verdad), sin verificar en QEMU (en construcción)

## Referencias de arquitectura estudiadas
- Asmodeus14/Nyx (Rust, QCLang, motor 3D Gen9.5 hand-rolled)
- nyxos-dev/nyx-os (C+ASM, TLS 1.2 real, DOOM portado, lenguaje N propio)
- Linux (ABI syscalls, modelo de drivers)
- Windows NT (HAL, driver model en capas)
- OpenBSD (pledge/unveil, W^X, seguridad por defecto)

## Build (CachyOS / Arch)
```bash
yay -S rustup nasm qemu-full grub xorriso lld
rustup toolchain install nightly
rustup component add rust-src llvm-tools-preview --toolchain nightly
rustup override set nightly
cargo build --target targets/x86_64-forge.json -Zbuild-std=core,alloc,compiler_builtins -Zbuild-std-features=compiler-builtins-mem
```

## Run en QEMU
```bash
./tools/run-qemu.sh
```
