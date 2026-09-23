#!/usr/bin/env bash
# Forge OS — arranque en QEMU
# Sin argumentos: modo gráfico interactivo.
# --headless: boot sin display, log de serie a boot.log (para CI / validación rápida)
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
ISO="$ROOT/ForgeOS.iso"
DISK="$ROOT/disk.img"

if [ ! -f "$ISO" ]; then
    echo "No existe $ISO — ejecuta primero ./tools/build.sh"
    exit 1
fi

# disk.img es opcional (comandos ext2ls/ext2cat/disktest sin él
# simplemente reportan "sin disco") — ./tools/make-test-disk.sh lo
# genera con un ext2 real de prueba si hace falta.
DISK_ARGS=()
if [ -f "$DISK" ]; then
    DISK_ARGS=(-device ahci,id=ahci0 -drive "if=none,id=disk0,format=raw,file=$DISK" -device ide-hd,drive=disk0,bus=ahci0.0)
else
    echo "Aviso: no existe $DISK — arrancando sin disco SATA (ext2ls/ext2cat/disktest no tendrán nada que leer)."
    echo "       Genera uno con ./tools/make-test-disk.sh"
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
        -boot order=d \
        -nic user,model=rtl8139 \
        "${DISK_ARGS[@]}" \
        -serial file:"$ROOT/boot.log" \
        -display none
    echo "--- boot.log ---"
    cat "$ROOT/boot.log"
else
    qemu-system-x86_64 \
        -cdrom "$ISO" \
        -m 256M \
        -no-reboot \
        -boot order=d \
        -nic user,model=rtl8139 \
        "${DISK_ARGS[@]}" \
        -serial stdio \
        -vga std
fi
