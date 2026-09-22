#!/usr/bin/env bash
# Forge OS — arranque en QEMU
# Sin argumentos: modo gráfico interactivo.
# --headless: boot sin display, log de serie a boot.log (para CI / validación rápida)
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
ISO="$ROOT/ForgeOS.iso"

if [ ! -f "$ISO" ]; then
    echo "No existe $ISO — ejecuta primero ./tools/build.sh"
    exit 1
fi

if [ "${1:-}" == "--headless" ]; then
    qemu-system-x86_64 \
        -cdrom "$ISO" \
        -m 256M \
        -no-reboot -no-shutdown \
        -serial file:"$ROOT/boot.log" \
        -display none
    echo "--- boot.log ---"
    cat "$ROOT/boot.log"
else
    qemu-system-x86_64 \
        -cdrom "$ISO" \
        -m 256M \
        -no-reboot \
        -serial stdio \
        -vga std
fi
