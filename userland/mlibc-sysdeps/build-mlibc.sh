#!/usr/bin/env bash
# build-mlibc.sh — Compila mlibc con sysdeps de ForgeOS desde WSL2/Debian
# Ejecutar desde la raíz del repo ForgeOS:
#   cd /mnt/c/Users/alfy1/proyectos/forge-os-repo
#   bash userland/mlibc-sysdeps/build-mlibc.sh
set -e

REPO_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
MLIBC_DIR="$REPO_ROOT/userland/mlibc"
SYSDEPS_SRC="$REPO_ROOT/userland/mlibc-sysdeps"
BUILD_DIR="$MLIBC_DIR/build-forgeos"

echo "==> Clonando mlibc (managarm/mlibc)..."
if [ ! -d "$MLIBC_DIR" ]; then
    git clone --depth=1 https://github.com/managarm/mlibc "$MLIBC_DIR"
fi

echo "==> Copiando sysdeps de ForgeOS..."
mkdir -p "$MLIBC_DIR/sysdeps/forgeos"
cp "$SYSDEPS_SRC/forgeos/generic.cpp" "$MLIBC_DIR/sysdeps/forgeos/"
cp "$SYSDEPS_SRC/meson.build" "$MLIBC_DIR/sysdeps/forgeos/"

echo "==> Parcheando mlibc/meson.build para reconocer 'forgeos'..."
MLIBC_MESON="$MLIBC_DIR/meson.build"
if ! grep -q "forgeos" "$MLIBC_MESON"; then
    # Busca la línea del último OS conocido (linux/managarm/etc.) e inserta forgeos
    sed -i "s/elif host_machine.system() == 'linux'/elif host_machine.system() == 'forgeos'\n  subdir('sysdeps\/forgeos')\nelif host_machine.system() == 'linux'/" "$MLIBC_MESON"
    echo "    -> parcheado OK"
else
    echo "    -> ya parcheado, omitiendo"
fi

echo "==> Copiando cross-file..."
cp "$SYSDEPS_SRC/cross/x86_64-forgeos.txt" "$MLIBC_DIR/cross/"

echo "==> meson setup..."
mkdir -p "$BUILD_DIR"
cd "$MLIBC_DIR"
meson setup "$BUILD_DIR" \
    --cross-file "cross/x86_64-forgeos.txt" \
    -Dheaders_only=false \
    -Ddisable_crypt_option=true \
    -Ddisable_iconv_option=true \
    --wipe 2>/dev/null || true

meson setup "$BUILD_DIR" \
    --cross-file "cross/x86_64-forgeos.txt" \
    -Dheaders_only=false \
    -Ddisable_crypt_option=true \
    -Ddisable_iconv_option=true

echo "==> ninja..."
ninja -C "$BUILD_DIR"

echo ""
echo "✓ mlibc compilado. Artefactos en: $BUILD_DIR"
echo "  libc.so / libc.a — linkar hello.c con: clang -target x86_64-elf -nostdlib $BUILD_DIR/libc.a hello.c"
