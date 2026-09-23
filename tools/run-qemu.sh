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
    # timeout: la consola de depuración (console.rs) se queda esperando
    # input por serie, que en modo --headless nunca llega — sin timeout,
    # esto colgaría QEMU para siempre. El log hasta el prompt sigue
    # siendo válido aunque el proceso se mate por timeout.
    timeout 20 qemu-system-x86_64 \
        -cdrom "$ISO" \
        -m 256M \
        -no-reboot -no-shutdown \
        -nic user,model=rtl8139 \
        -serial file:"$ROOT/boot.log" \
        -display none
    echo "--- boot.log ---"
    cat "$ROOT/boot.log"
else
    qemu-system-x86_64 \
        -cdrom "$ISO" \
        -m 256M \
        -no-reboot \
        -nic user,model=rtl8139 \
        -serial stdio \
        -vga std
fi
