#!/usr/bin/env python3
"""Forge OS — arranca QEMU sin pantalla y teclea comandos en la consola
de depuración por el puerto serie, esperando al prompt `forge>` antes de
cada uno. Imprime el log de serie completo.

    tools/qemu-console.py "run /bin/hello" ps meminfo

Con --expect TEXTO (repetible) comprueba además que cada TEXTO aparece
en la salida, en ese orden; sale con código 1 si falta alguno, si un
comando no devuelve el prompt a tiempo, o si el kernel entra en pánico.
Pensado como base de pruebas de regresión automáticas (docs/TODO.md §10).

Requiere ForgeOS.iso (tools/build.sh); adjunta disk.img si existe, igual
que tools/run-qemu.sh.
"""
import argparse
import os
import selectors
import subprocess
import sys
import time

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
PROMPT = b"forge> "
FATAL = (b"[PANIC]", b"[FATAL]")


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("commands", nargs="*", help="comandos de la consola, en orden")
    ap.add_argument("--expect", action="append", default=[], metavar="TEXTO",
                    help="texto que debe aparecer en la salida (en orden)")
    ap.add_argument("--timeout", type=float, default=30.0,
                    help="segundos máximos por comando, y para el arranque (def. 30)")
    ap.add_argument("--quiet", action="store_true", help="no imprimir el log salvo si algo falla")
    args = ap.parse_args()

    iso = os.path.join(ROOT, "ForgeOS.iso")
    disk = os.path.join(ROOT, "disk.img")
    if not os.path.isfile(iso):
        sys.exit(f"No existe {iso} — ejecuta primero tools/build.sh")

    cmd = ["qemu-system-x86_64", "-cdrom", iso, "-m", "256M", "-no-reboot",
           "-boot", "order=d", "-nic", "user,model=rtl8139",
           "-serial", "stdio", "-display", "none", "-monitor", "none"]
    if os.path.isfile(disk):
        # snapshot=on: las pruebas nunca modifican disk.img de verdad.
        cmd += ["-device", "ahci,id=ahci0",
                "-drive", f"if=none,id=disk0,format=raw,file={disk},snapshot=on",
                "-device", "ide-hd,drive=disk0,bus=ahci0.0"]

    qemu = subprocess.Popen(cmd, stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
    os.set_blocking(qemu.stdout.fileno(), False)
    sel = selectors.DefaultSelector()
    sel.register(qemu.stdout, selectors.EVENT_READ)

    log = bytearray()
    problem = None

    def wait_for_prompt(start):
        """Lee hasta ver un prompt nuevo a partir de `start`. Devuelve el motivo del fallo o None."""
        deadline = time.monotonic() + args.timeout
        while True:
            if any(marker in log[start:] for marker in FATAL):
                return "el kernel entró en pánico"
            if PROMPT in log[start:]:
                return None
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                return f"sin prompt tras {args.timeout:.0f} s"
            if not sel.select(timeout=remaining):
                continue
            chunk = qemu.stdout.read()
            if chunk == b"":
                return "QEMU terminó inesperadamente"
            if chunk:
                log.extend(chunk)

    try:
        problem = wait_for_prompt(0)
        if problem:
            problem = f"arranque: {problem}"
        for command in args.commands:
            if problem:
                break
            start = len(log)
            qemu.stdin.write(command.encode() + b"\r")
            qemu.stdin.flush()
            problem = wait_for_prompt(start)
            if problem:
                problem = f"comando '{command}': {problem}"
    finally:
        qemu.kill()
        qemu.wait()

    text = log.decode("utf-8", errors="replace").replace("\r\n", "\n").replace("\r", "\n")

    missing = []
    pos = 0
    for expected in args.expect:
        found = text.find(expected, pos)
        if found < 0:
            missing.append(expected)
        else:
            pos = found + len(expected)

    failed = bool(problem or missing)
    if failed or not args.quiet:
        sys.stdout.write(text)
        if not text.endswith("\n"):
            sys.stdout.write("\n")
    if problem:
        print(f"\nFALLO — {problem}", file=sys.stderr)
    for expected in missing:
        print(f"FALLO — no aparece (o aparece fuera de orden): {expected!r}", file=sys.stderr)
    sys.exit(1 if failed else 0)


if __name__ == "__main__":
    main()
