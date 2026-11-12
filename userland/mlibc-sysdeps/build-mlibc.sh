#!/usr/bin/env bash
# build-mlibc.sh — compila mlibc (estática) con los sysdeps de ForgeOS y
# la instala en userland/sysroot/ (cabeceras + libc.a + crt1.o).
#
# Requiere: git, clang >= 19, lld, llvm (llvm-ar...), meson, ninja.
#   CachyOS/Arch:  yay -S clang lld llvm meson ninja
#   Debian/Ubuntu: apt install clang-20 lld-20 llvm-20 meson ninja-build
# (las cabeceras C++ freestanding de mlibc no compilan con clang 18)
#
# Uso (desde cualquier sitio):  userland/mlibc-sysdeps/build-mlibc.sh
set -euo pipefail

# Commit de managarm/mlibc contra el que están escritos los sysdeps. La
# API interna de sysdeps de mlibc cambia entre versiones — sube este
# valor a mano y recompila cuando quieras actualizar.
MLIBC_REPO="https://github.com/managarm/mlibc"
MLIBC_COMMIT="e1fa835ac77a50d0f5d9d49c1b3003e9d97bde47"

USERLAND="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SYSDEPS_SRC="$USERLAND/mlibc-sysdeps"
MLIBC_DIR="$USERLAND/mlibc"
BUILD_DIR="$MLIBC_DIR/build-forgeos"
SYSROOT="$USERLAND/sysroot"

# Define LLVM_SUFFIX ("" o "-NN") — el clang más nuevo instalado.
source "$USERLAND/toolchain.sh"

echo "==> [1/5] mlibc @ ${MLIBC_COMMIT:0:7}"
if [ ! -d "$MLIBC_DIR/.git" ]; then
    git init -q "$MLIBC_DIR"
    git -C "$MLIBC_DIR" remote add origin "$MLIBC_REPO"
fi
if [ "$(git -C "$MLIBC_DIR" rev-parse HEAD 2>/dev/null || true)" != "$MLIBC_COMMIT" ]; then
    git -C "$MLIBC_DIR" fetch -q --depth=1 origin "$MLIBC_COMMIT"
    git -C "$MLIBC_DIR" checkout -q -f FETCH_HEAD
fi

echo "==> [2/5] Instalando sysdeps de ForgeOS en el árbol de mlibc"
rm -rf "$MLIBC_DIR/sysdeps/forgeos"
cp -r "$SYSDEPS_SRC/forgeos" "$MLIBC_DIR/sysdeps/forgeos"
# abi-bits = enlaces relativos a abis/linux; misma profundidad de
# directorio que en sysdeps/demo, así que se copian tal cual (-P).
cp -rP "$MLIBC_DIR/sysdeps/demo/include/abi-bits" "$MLIBC_DIR/sysdeps/forgeos/include/abi-bits"

if ! grep -q "'forgeos'" "$MLIBC_DIR/meson.build"; then
    python3 - "$MLIBC_DIR/meson.build" <<'PY'
import sys
path = sys.argv[1]
src = open(path).read()
anchor = "elif host_machine.system() == 'demo'"
if anchor not in src:
    sys.exit("meson.build de mlibc ya no tiene la rama 'demo' — revisa MLIBC_COMMIT")
src = src.replace(anchor, "elif host_machine.system() == 'forgeos'\n\tsubdir('sysdeps/forgeos')\n" + anchor, 1)
open(path, "w").write(src)
PY
fi

echo "==> [3/5] Generando cross-file"
RESOURCE_INCLUDE="$("clang$LLVM_SUFFIX" -print-resource-dir)/include"
[ -f "$RESOURCE_INCLUDE/stddef.h" ] || { echo "ERROR: no encuentro las cabeceras de clang en $RESOURCE_INCLUDE"; exit 1; }
CROSS_FILE="$MLIBC_DIR/x86_64-forgeos.cross.txt"
sed -e "s|@CLANG_RESOURCE_INCLUDE@|$RESOURCE_INCLUDE|g" -e "s|@LLVM_SUFFIX@|$LLVM_SUFFIX|g" \
    "$SYSDEPS_SRC/cross/x86_64-forgeos.txt.in" > "$CROSS_FILE"

echo "==> [4/5] meson setup + ninja"
# default_library=static: sin libc.so ni ld.so — el kernel todavía no
# sabe cargar ELFs dinámicos (necesita mmap() de fichero).
# libgcc_dependency=false: no hay compiler-rt/libgcc para x86_64-elf y
# en x86_64 mlibc no necesita ninguna de sus rutinas.
MESON_ARGS=(
    --cross-file "$CROSS_FILE"
    --prefix /usr
    --buildtype release
    -Ddefault_library=static
    -Dlibgcc_dependency=false
    -Dposix_option=enabled
    -Dlinux_option=disabled
    -Dglibc_option=disabled
    -Dbsd_option=disabled
)
if [ -d "$BUILD_DIR/meson-private" ]; then
    meson setup "$BUILD_DIR" "$MLIBC_DIR" "${MESON_ARGS[@]}" --reconfigure > "$BUILD_DIR.setup.log" 2>&1 \
        || { tail -30 "$BUILD_DIR.setup.log"; exit 1; }
else
    meson setup "$BUILD_DIR" "$MLIBC_DIR" "${MESON_ARGS[@]}" > "$BUILD_DIR.setup.log" 2>&1 \
        || { tail -30 "$BUILD_DIR.setup.log"; exit 1; }
fi
ninja -C "$BUILD_DIR"

echo "==> [5/5] Instalando en $SYSROOT"
rm -rf "$SYSROOT"
DESTDIR="$SYSROOT" meson install -C "$BUILD_DIR" --no-rebuild > /dev/null

echo ""
echo "Listo: $SYSROOT/usr/{include,lib}"
echo "Compila los programas de ejemplo con: make -C userland"
