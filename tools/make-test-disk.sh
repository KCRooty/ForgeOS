#!/usr/bin/env bash
# Forge OS — genera disk.img, un disco raw de prueba con un ext2 real
# (sin tabla de particiones: ext2.rs todavía asume que el filesystem
# empieza en el LBA 0 — partinfo.rs es milestone aparte).
#
# Requiere: e2fsprogs (mkfs.ext2, debugfs). En CachyOS/Arch: `yay -S
# e2fsprogs` (normalmente ya viene con el sistema base).
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
IMG="$ROOT/disk.img"
STAGE="$(mktemp -d)"
trap 'rm -rf "$STAGE"' EXIT

echo "[1/3] Creando imagen de 16 MiB..."
rm -f "$IMG"
truncate -s 16M "$IMG"

# Bloque de 1 KiB, inodos de 128 bytes, y sin ninguna de las features
# "modernas" (extents, 64bit, metadata_csum, flex_bg, uninit_bg...) que
# ext2.rs no entiende todavía — esto es ext2 clásico de propósito,
# árbol de extents es ext4 y queda fuera del alcance de esta pasada.
echo "[2/3] Formateando ext2 (bloque 1 KiB, sin features modernas)..."
mkfs.ext2 -q -F -b 1024 \
    -O ^resize_inode,^dir_index,^extent,^64bit,^metadata_csum,^huge_file,^flex_bg,^uninit_bg,^sparse_super2 \
    -I 128 -L FORGEROOT "$IMG"

echo "[3/3] Poblando ficheros de prueba..."
mkdir -p "$STAGE/sub"
echo -n "Hola desde ext2 real en Forge OS" > "$STAGE/hello.txt"
# 20 KiB — a propósito más grande que 12 bloques directos (12 KiB con
# bloque de 1 KiB), para ejercitar el puntero indirecto simple.
python3 -c "
data = bytes((i % 251) for i in range(20*1024))
open('$STAGE/big.bin', 'wb').write(data)
"
echo -n "fichero dentro de un subdirectorio" > "$STAGE/sub/nested.txt"

debugfs -w -R "mkdir /sub" "$IMG" > /dev/null
debugfs -w -R "write $STAGE/hello.txt hello.txt" "$IMG" > /dev/null
debugfs -w -R "write $STAGE/big.bin big.bin" "$IMG" > /dev/null
debugfs -w -R "write $STAGE/sub/nested.txt /sub/nested.txt" "$IMG" > /dev/null

echo ""
echo "Listo: $IMG"
echo "Arranca con red+disco: ./tools/run-qemu.sh (ya lo adjunta si existe)"
