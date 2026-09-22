#!/usr/bin/env bash
# Forge OS — build script (CachyOS / Arch)
# Requiere: rustup (nightly + rust-src + llvm-tools-preview), nasm, lld,
#           grub (grub-mkrescue), xorriso
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT/kernel"

echo "[1/3] Compilando kernel (nightly + build-std)..."
cargo build --release

BIN="$ROOT/target/x86_64-forge/release/forge-kernel"
if [ ! -f "$BIN" ]; then
    echo "ERROR: no se generó el binario en $BIN"
    exit 1
fi

echo "[2/3] Montando estructura de ISO..."
mkdir -p "$ROOT/iso/boot"
cp "$BIN" "$ROOT/iso/boot/forge-kernel.bin"

echo "[3/3] Generando ForgeOS.iso con grub-mkrescue..."
grub-mkrescue -o "$ROOT/ForgeOS.iso" "$ROOT/iso" 2>/dev/null

echo ""
echo "Listo: $ROOT/ForgeOS.iso"
echo "Arranca con: ./tools/run-qemu.sh"
