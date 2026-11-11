; crt0test.asm — binario de prueba de `crt0test` (consola de depuración).
;
; Comprueba, sin libc de por medio, las dos cosas que un `_start` real da
; por hechas al arrancar:
;
;   1. El frame inicial SysV que deja el kernel en la pila
;      (process::write_initial_stack_frame): RSP alineado a 16, argc,
;      argv[0] = "crt0test", argv[1] = NULL, envp vacío, y el vector
;      auxiliar AT_PAGESZ=4096 / AT_ENTRY=_start / AT_NULL.
;   2. mmap() anónimo de DOS páginas: escribe un patrón distinto en cada
;      una y los relee (si solo se hubiera mapeado la primera, la
;      escritura en la segunda sería un fallo de página), y munmap().
;
; ELF64 completo escrito a mano (cabecera + un PT_LOAD), ensamblado con
; `nasm -f bin`: no pasa por ningún enlazador. Para regenerar el array
; incrustado en kernel/src/elf.rs (TEST_ELF_CRT0):
;
;   make -C userland build/crt0test_rust_bytes.txt

BITS 64
BASE equ 0x000000A000000000        ; 640 GiB, como todos los TEST_ELF_*
org BASE

SYS_PING   equ 1
SYS_MMAP   equ 9
SYS_MUNMAP equ 11
SYS_GETPID equ 39
SYS_EXIT   equ 60
SYS_WRITE  equ 63                  ; ForgeOS: write es 63, no 1

ehdr:
    db 0x7F, "ELF", 2, 1, 1, 0     ; ELF64, little-endian, versión 1
    times 8 db 0
    dw 2                           ; e_type    = ET_EXEC
    dw 0x3E                        ; e_machine = x86_64
    dd 1                           ; e_version
    dq _start                      ; e_entry
    dq phdr - ehdr                 ; e_phoff
    dq 0                           ; e_shoff
    dd 0                           ; e_flags
    dw 64                          ; e_ehsize
    dw 56                          ; e_phentsize
    dw 1                           ; e_phnum
    dw 0, 0, 0                     ; sin secciones

phdr:
    dd 1                           ; p_type  = PT_LOAD
    dd 7                           ; p_flags = RWX (código y datos juntos)
    dq 0                           ; p_offset
    dq BASE                        ; p_vaddr
    dq BASE                        ; p_paddr
    dq file_end - ehdr             ; p_filesz
    dq file_end - ehdr             ; p_memsz
    dq 0x1000                      ; p_align

; write(1, %1, %2)
%macro PRINT 2
    mov eax, SYS_WRITE
    mov edi, 1
    lea rsi, [rel %1]
    mov edx, %2
    syscall
%endmacro

_start:
    mov rbx, rsp                   ; rbx = frame inicial (lo conserva `syscall`)

    ; --- 1. frame inicial ---
    test rbx, 15                   ; RSP alineado a 16
    jnz .bad_stack
    cmp qword [rbx], 1             ; argc == 1
    jne .bad_stack
    mov rsi, [rbx + 8]             ; argv[0]
    test rsi, rsi
    jz .bad_stack
    lea rdi, [rel expected_name]
    mov ecx, expected_name_len     ; incluye el NUL final
    repe cmpsb                     ; argv[0] == "crt0test"
    jne .bad_stack
    cmp qword [rbx + 16], 0        ; argv[1] == NULL
    jne .bad_stack
    cmp qword [rbx + 24], 0        ; envp[0] == NULL
    jne .bad_stack
    cmp qword [rbx + 32], 6        ; AT_PAGESZ
    jne .bad_stack
    cmp qword [rbx + 40], 4096
    jne .bad_stack
    cmp qword [rbx + 48], 9        ; AT_ENTRY
    jne .bad_stack
    lea rax, [rel _start]
    cmp [rbx + 56], rax
    jne .bad_stack
    cmp qword [rbx + 64], 0        ; AT_NULL
    jne .bad_stack

    PRINT msg_stack_ok, msg_stack_ok_len
    jmp .mmap_test

.bad_stack:
    PRINT msg_stack_bad, msg_stack_bad_len

    ; --- 2. mmap(NULL, 8192, PROT_READ|PROT_WRITE, MAP_PRIVATE|MAP_ANONYMOUS, -1, 0) ---
.mmap_test:
    mov eax, SYS_MMAP
    xor edi, edi
    mov esi, 8192
    mov edx, 3
    mov r10d, 0x22
    mov r8, -1
    xor r9d, r9d
    syscall
    cmp rax, -4096                 ; errno negativo (o -1) = fallo
    jae .bad_mmap
    mov r12, rax                   ; r12 = base del mapeo

    mov rax, 0x1111111111111111
    mov [r12], rax                 ; primera página
    mov rax, 0x2222222222222222
    mov [r12 + 4096], rax          ; segunda página
    mov rax, 0x1111111111111111
    cmp [r12], rax
    jne .bad_mmap
    mov rax, 0x2222222222222222
    cmp [r12 + 4096], rax
    jne .bad_mmap

    PRINT msg_mmap_ok, msg_mmap_ok_len

    mov eax, SYS_MUNMAP
    mov rdi, r12
    mov esi, 8192
    syscall
    mov rdi, rax                   ; ping(resultado de munmap) — debe ser 0
    mov eax, SYS_PING
    syscall
    jmp .finish

.bad_mmap:
    PRINT msg_mmap_bad, msg_mmap_bad_len

.finish:
    mov eax, SYS_GETPID
    syscall
    mov rdi, rax                   ; ping(pid)
    mov eax, SYS_PING
    syscall

    PRINT msg_done, msg_done_len
    mov eax, SYS_EXIT
    xor edi, edi
    syscall

expected_name:      db "crt0test", 0
expected_name_len   equ $ - expected_name

msg_stack_ok:       db "crt0test: pila inicial correcta (argc/argv/envp/auxv)", 10
msg_stack_ok_len    equ $ - msg_stack_ok
msg_stack_bad:      db "crt0test: FALLO - la pila inicial no tiene la forma esperada", 10
msg_stack_bad_len   equ $ - msg_stack_bad
msg_mmap_ok:        db "crt0test: mmap() de dos paginas anonimas funciona de verdad", 10
msg_mmap_ok_len     equ $ - msg_mmap_ok
msg_mmap_bad:       db "crt0test: FALLO - mmap() no devolvio dos paginas utilizables", 10
msg_mmap_bad_len    equ $ - msg_mmap_bad
msg_done:           db "crt0test: completado", 10
msg_done_len        equ $ - msg_done

file_end:
