# toolchain.sh — elige el toolchain LLVM para el userland de ForgeOS.
# Pensado para `source` desde bash (build-mlibc.sh) o para ejecutarse
# suelto, en cuyo caso imprime el sufijo elegido (lo usa el Makefile).
#
# Deja en LLVM_SUFFIX "" (binarios sin sufijo: clang, ld.lld... — lo
# normal en Arch/CachyOS) o "-NN" (clang-20, ld.lld-20... — Debian/
# Ubuntu). Se puede forzar desde fuera: LLVM_SUFFIX=-19 make ...
#
# mlibc necesita clang >= 19.

FORGE_MIN_CLANG=19

_forge_clang_major() {
    "$1" -dumpversion 2>/dev/null | cut -d. -f1
}

if [ -z "${LLVM_SUFFIX+x}" ]; then
    LLVM_SUFFIX=""
    _best=0
    for _cand in "" -25 -24 -23 -22 -21 -20 -19; do
        command -v "clang$_cand" > /dev/null 2>&1 || continue
        _ver="$(_forge_clang_major "clang$_cand")"
        if [ "${_ver:-0}" -gt "$_best" ]; then
            _best="$_ver"
            LLVM_SUFFIX="$_cand"
        fi
    done
    unset _cand _ver _best
fi

if [ "$(_forge_clang_major "clang$LLVM_SUFFIX")" -lt "$FORGE_MIN_CLANG" ] 2>/dev/null \
    || ! command -v "clang$LLVM_SUFFIX" > /dev/null 2>&1; then
    echo "ERROR: hace falta clang >= $FORGE_MIN_CLANG (encontrado: clang$LLVM_SUFFIX $(_forge_clang_major "clang$LLVM_SUFFIX"))" >&2
    return 1 2>/dev/null || exit 1
fi

# Ejecutado suelto (no con `source`): imprime el sufijo.
if [ "${BASH_SOURCE[0]:-$0}" = "$0" ]; then
    echo "$LLVM_SUFFIX"
fi
