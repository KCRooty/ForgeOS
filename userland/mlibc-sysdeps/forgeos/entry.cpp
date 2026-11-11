// entry.cpp — puente entre crt1.S y main() para ForgeOS.
//
// __dlapi_enter() es el arranque del enlazador de mlibc: en un binario
// estático localiza PT_TLS a través de __ehdr_start, reserva el TCB y
// llama a Sysdeps<TcbSet> (SYS_ARCH_PRCTL / ARCH_SET_FS en ForgeOS).

#include <stdint.h>
#include <stdlib.h>
#include <mlibc/elf/startup.h>

extern "C" void __dlapi_enter(uintptr_t *);

extern char **environ;

extern "C" void __mlibc_entry(uintptr_t *entry_stack, int (*main_fn)(int argc, char *argv[], char *env[])) {
	__dlapi_enter(entry_stack);

	auto result = main_fn(mlibc::entry_stack.argc, mlibc::entry_stack.argv, environ);
	exit(result);
}
