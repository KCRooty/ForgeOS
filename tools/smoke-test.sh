#!/usr/bin/env bash
# Forge OS — batería de regresión en QEMU.
#
# Arranca la ISO, teclea los comandos de prueba de la consola de
# depuración por el puerto serie y comprueba que la salida contiene lo
# esperado. Sale con 0 solo si TODO pasa.
#
#   tools/smoke-test.sh          una pasada
#   tools/smoke-test.sh 15       15 arranques en frío seguidos — las
#                                carreras de arranque/scheduler solo
#                                salen repitiendo (ver docs/TODO.md)
#
# Requiere, ya generados:
#   ForgeOS.iso   tools/build.sh
#   disk.img      userland/mlibc-sysdeps/build-mlibc.sh   (una vez)
#                 make -C userland
#                 tools/make-test-disk.sh
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
ROUNDS="${1:-1}"
LOG="$ROOT/smoke-test.log"

[ -f "$ROOT/disk.img" ] || { echo "Falta disk.img — ver la cabecera de este script"; exit 1; }

COMMANDS=(
    forktest waittest exec segvtest killtest
    preempttest preempttest3
    posixtest crt0test hellotest
    "ext2cat /hello.txt" "ext2cat /sub/nested.txt"
    ping
    "run /bin/hello"
    "run /bin/libctest uno dos"
    "run /bin/fpustress 1 &" "run /bin/fpustress 2 &" "run /bin/fpustress 3"
    wait
    "run /bin/crash null" "run /bin/crash div0" "run /bin/crash ud"
    "run /bin/crash priv" "run /bin/crash stack"
    "run /bin/hello"
    ps
)

# En el orden en que deben aparecer en el log.
EXPECT=(
    "[heap] free-list OK"
    "[ember] PID 1 vivo de fondo"
    # forktest / waittest / exec
    "fork: PID 4 -> hijo PID 5"
    "wait: PID 6 recogió al hijo PID 7 (código 0)"
    "'pingtest' cargado"
    # segvtest / killtest
    "recibe SIGSEGV"
    "recogió al hijo PID 11 (código 143)"
    # preempttest / preempttest3
    "preemption real confirmada"
    "preemption real en ring 3 confirmada"
    # posixtest / crt0test / hellotest
    "contenido real via open+write+close sobre el VFS"
    "arg0=0xcafebabe"
    "crt0test: pila inicial correcta"
    "crt0test: mmap() de dos paginas anonimas funciona de verdad"
    "hello, forge!"
    # ext2 / red
    "Hola desde ext2 real en Forge OS"
    "fichero dentro de un subdirectorio"
    "round-trip OK"
    # C + mlibc desde disco
    "hello, forge! (C + mlibc)"
    "H(100)=5.187378"
    "terminó con código 42"
    "argv (3): [/bin/libctest] [uno] [dos]"
    "libctest: TODO OK"
    # fallos provocados: muere el proceso, no el kernel
    "terminado por la señal 11 (código 139)"
    "terminado por la señal 8 (código 136)"
    "terminado por la señal 4 (código 132)"
    "terminado por la señal 11 (código 139)"
    "terminado por la señal 11 (código 139)"
    # ...y el sistema sigue ejecutando programas después
    "terminó con código 42"
)

ARGS=(--timeout 120)
for e in "${EXPECT[@]}"; do ARGS+=(--expect "$e"); done

for round in $(seq 1 "$ROUNDS"); do
    if ! "$ROOT/tools/qemu-console.py" "${ARGS[@]}" "${COMMANDS[@]}" > "$LOG" 2>&1; then
        echo "FALLO en la pasada $round/$ROUNDS — log completo en $LOG"
        grep -n "FALLO" "$LOG" || true
        exit 1
    fi
    # Comprobaciones que no dependen del orden.
    for needle in "fpustress[1]: OK" "fpustress[2]: OK" "fpustress[3]: OK"; do
        grep -qF "$needle" "$LOG" || { echo "FALLO en la pasada $round/$ROUNDS — no aparece: $needle (log en $LOG)"; exit 1; }
    done
    if grep -qE "  FALLO  |crt0test: FALLO|fpustress\[[0-9]+\]: FALLO" "$LOG"; then
        echo "FALLO en la pasada $round/$ROUNDS — una prueba informó de error (log en $LOG)"
        grep -nE "FALLO" "$LOG"
        exit 1
    fi
    echo "pasada $round/$ROUNDS OK"
done
echo "smoke-test: todo OK ($ROUNDS pasada(s))"
