# crt1.S — punto de entrada de todo binario C de ForgeOS (x86_64).
#
# El kernel (process::write_initial_stack_frame) deja en [rsp] el frame
# SysV: argc, argv[], NULL, envp[], NULL, auxv[]. Aquí solo se pasa ese
# puntero a __mlibc_entry (entry.cpp) junto con la dirección de main().
#
# RSP llega alineado a 16; el `call` empuja la dirección de retorno y
# deja la alineación que la ABI SysV espera dentro de la función llamada.

.section .text
.global _start
.type _start, @function
_start:
	xor %ebp, %ebp              # fin de la cadena de frames (backtraces)
	mov %rsp, %rdi              # arg0: puntero al frame inicial
	lea main(%rip), %rsi        # arg1: main() — RIP-relativo, el binario
	                            # vive en 640 GiB, fuera del alcance de
	                            # un inmediato absoluto de 32 bits
	call __mlibc_entry
	ud2                         # __mlibc_entry nunca vuelve
.size _start, . - _start

.section .note.GNU-stack,"",%progbits
