; hello.asm — primer programa de usuario para ForgeOS
;
; Demuestra la cadena completa M8c-M8f:
;   - Lee argc/argv del frame SysV dejado por el kernel (M8f)
;   - Inicializa TLS vía SYS_ARCH_PRCTL/ARCH_SET_FS (M8c)
;   - SYS_WRITE = 63  (ForgeOS — diverge de Linux donde es 1)
;   - SYS_EXIT  = 60
;
; Compilar (Windows/Linux, requiere NASM + ld.lld):
;   nasm -f elf64 hello.asm -o hello.o
;   ld.lld -T linker.ld -static -o hello.elf hello.o
;
; El ELF resultante se puede embeber como TEST_ELF_HELLO en elf.rs
; y ejecutar con el comando 'hellotest' de la consola del kernel.

bits 64
default rel

section .data
    msg     db "hello, forge!", 10   ; \n
    msg_len equ $ - msg

    ; TCB mínimo para TLS: solo el puntero a sí mismo en [tcb+0]
    ; mlibc real usaría una estructura más compleja aquí.
    tcb     dq 0                      ; TCB[0] = self-pointer (llenado en _start)

section .text
global _start

_start:
    ; rsp+0 = argc, rsp+8 = argv[0] ptr (kernel M8f)
    ; No necesitamos argc/argv para este hello-world, pero
    ; el frame ya está bien formado — lo dejamos en paz.

    ; ── TLS: apuntar FS.base a nuestro TCB mínimo ──────────────────
    lea   rax, [tcb]
    mov   [tcb], rax          ; tcb[0] = &tcb (self-pointer POSIX-style)
    mov   rdi, 0x1002         ; ARCH_SET_FS
    mov   rsi, rax            ; puntero al TCB
    mov   rax, 158            ; SYS_ARCH_PRCTL
    syscall

    ; ── SYS_WRITE(1, msg, msg_len) ──────────────────────────────────
    mov   rdi, 1              ; fd = stdout
    lea   rsi, [msg]
    mov   rdx, msg_len
    mov   rax, 63             ; SYS_WRITE (ForgeOS = 63, no 1)
    syscall

    ; ── SYS_EXIT(0) ─────────────────────────────────────────────────
    xor   rdi, rdi            ; status = 0
    mov   rax, 60             ; SYS_EXIT
    syscall
