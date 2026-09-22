; boot.asm — Forge OS boot trampoline
; GRUB (Multiboot2) nos entrega en modo protegido de 32 bits.
; Aquí montamos paginación temporal identity-map de los primeros 2 GiB,
; activamos PAE + long mode, y saltamos a Rust en 64 bits.
;
; Patrón estándar osdev — mismo que usan nyxos-dev/nyx-os y Asmodeus14/Nyx
; en su respectivo stub de arranque.

bits 32

section .multiboot2
align 8
mb2_header_start:
    dd 0xE85250D6                ; magic Multiboot2
    dd 0                         ; arquitectura: i386 protected mode
    dd mb2_header_end - mb2_header_start
    dd -(0xE85250D6 + 0 + (mb2_header_end - mb2_header_start))

    ; tag de framebuffer (type=5): le pedimos a GRUB un framebuffer
    ; lineal de 1024x768 @ 32bpp. Sin esto, el tag de framebuffer podría
    ; no aparecer en la info que nos devuelve al arrancar.
    align 8
    tag_fb_start:
    dw 5                          ; type = framebuffer
    dw 0                          ; flags: 0 = obligatorio
    dd tag_fb_end - tag_fb_start  ; size
    dd 1024                       ; width
    dd 768                        ; height
    dd 32                         ; depth (bpp)
    tag_fb_end:

    ; tag final
    align 8
    dw 0
    dw 0
    dd 8
mb2_header_end:

section .boot.bss
align 4096
p4_table:    resb 4096
p3_table:    resb 4096
; 4 tablas P2 contiguas — cada una cubre 1 GiB (512 huge pages de 2 MiB),
; así que las 4 juntas identity-mapean 0-4 GiB. Necesario para llegar al
; Local APIC (~0xFEE00000) y al I/O APIC (~0xFEC00000), que viven por
; encima del primer GiB que teníamos mapeado hasta ahora.
p2_tables:   resb 4096 * 4
stack_bottom: resb 16384
stack_top:

section .boot.text
global _start
extern kernel_main_upper   ; símbolo Rust: fn kernel_main_upper(mb2_ptr: u64) -> !

_start:
    mov esp, stack_top
    mov edi, ebx            ; puntero a la info Multiboot2 (arg del kernel)

    call check_multiboot
    call check_cpuid
    call check_long_mode

    call setup_page_tables
    call enable_paging

    lgdt [gdt64.pointer]
    jmp gdt64.code:long_mode_start

check_multiboot:
    cmp eax, 0x36d76289
    jne .no_mb
    ret
.no_mb:
    mov al, "0"
    jmp error

check_cpuid:
    pushfd
    pop eax
    mov ecx, eax
    xor eax, 1 << 21
    push eax
    popfd
    pushfd
    pop eax
    push ecx
    popfd
    cmp eax, ecx
    je .no_cpuid
    ret
.no_cpuid:
    mov al, "1"
    jmp error

check_long_mode:
    mov eax, 0x80000000
    cpuid
    cmp eax, 0x80000001
    jb .no_lm
    mov eax, 0x80000001
    cpuid
    test edx, 1 << 29
    jz .no_lm
    ret
.no_lm:
    mov al, "2"
    jmp error

setup_page_tables:
    ; P4[0] -> P3
    mov eax, p3_table
    or eax, 0b11
    mov [p4_table], eax

    ; P3[0..3] -> cada una de las 4 tablas P2 (identity-map 0-4 GiB)
    mov eax, p2_tables
    or eax, 0b11
    mov [p3_table + 0*8], eax

    mov eax, p2_tables
    add eax, 4096
    or eax, 0b11
    mov [p3_table + 1*8], eax

    mov eax, p2_tables
    add eax, 4096*2
    or eax, 0b11
    mov [p3_table + 2*8], eax

    mov eax, p2_tables
    add eax, 4096*3
    or eax, 0b11
    mov [p3_table + 3*8], eax

    ; Rellenamos las 4 tablas P2 seguidas como si fueran un array de 2048
    ; entradas de 2 MiB cada una -> identity-map completo 0-4 GiB.
    mov ecx, 0
.map_p2:
    mov eax, 0x200000
    mul ecx                 ; edx:eax = ecx * 0x200000 (dirección física)
    or eax, 0b10000011      ; present + writable + huge page
    mov ebx, ecx
    shl ebx, 3               ; ebx = ecx*8 (offset en bytes dentro de p2_tables)
    add ebx, p2_tables
    mov [ebx], eax
    inc ecx
    cmp ecx, 2048
    jne .map_p2
    ret

enable_paging:
    mov eax, p4_table
    mov cr3, eax

    mov eax, cr4
    or eax, 1 << 5           ; PAE
    mov cr4, eax

    mov ecx, 0xC0000080      ; EFER MSR
    rdmsr
    or eax, 1 << 8           ; LME
    wrmsr

    mov eax, cr0
    or eax, 1 << 31          ; PG
    mov cr0, eax
    ret

error:
    mov dword [0xb8000], 0x4f524f45  ; "ER" rojo sobre negro en VGA text
    mov dword [0xb8004], 0x4f3a4f52  ; "R:"
    mov byte  [0xb8008], al
    hlt

section .boot.data
align 8
gdt64:
    dq 0
.code: equ $ - gdt64
    dq (1<<43) | (1<<44) | (1<<47) | (1<<53) ; código 64-bit, ring0
.pointer:
    dw $ - gdt64 - 1
    dq gdt64

bits 64
section .text
long_mode_start:
    mov ax, 0
    mov ss, ax
    mov ds, ax
    mov es, ax
    mov fs, ax
    mov gs, ax

    mov rsp, stack_top
    ; edi ya trae el puntero Multiboot2 (calling convention System V: rdi = arg1)
    call kernel_main_upper
.hang:
    hlt
    jmp .hang
