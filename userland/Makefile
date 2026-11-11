# Makefile del userland de ForgeOS.
#
#   make            -> build/hello-asm.elf, build/crt0test.elf
#                        (ensamblador puro, sin libc; van incrustados
#                        en el kernel, ver kernel/src/elf.rs)
#                      build/hello, build/libctest, build/crash,
#                      build/fpustress   (C + mlibc estática; van a
#                        /bin del disco, ver tools/make-test-disk.sh)
#   make asm        -> solo el de ensamblador (no necesita mlibc)
#   make clean
#
# Los programas en C necesitan el sysroot de mlibc:
#   ./mlibc-sysdeps/build-mlibc.sh      (una vez; tarda un par de minutos)
#
# Requiere: nasm, clang >= 19, lld. toolchain.sh elige el clang más nuevo
# instalado; se puede forzar con LLVM_SUFFIX (p.ej. `make LLVM_SUFFIX=-20`).

LLVM_SUFFIX ?= $(shell bash toolchain.sh)
CC   := clang$(LLVM_SUFFIX)
LLD  := ld.lld$(LLVM_SUFFIX)
NASM ?= nasm

SYSROOT := sysroot/usr
BUILD   := build

# Mismas banderas con las que se compila mlibc (ver
# mlibc-sysdeps/cross/x86_64-forgeos.txt.in). -fPIE: todo el código es
# RIP-relativo, imprescindible para vivir en 640 GiB.
CFLAGS := -target x86_64-elf -ffreestanding -fno-stack-protector -fPIE \
          -O2 -Wall -Wextra \
          -nostdinc -isystem $(SYSROOT)/include \
          -isystem $(shell $(CC) -print-resource-dir)/include

# Binario estático no-PIE con base en 0xA0_0000_0000 (640 GiB, índice
# P4 = 1: privado de cada proceso — ver kernel/src/elf.rs).
# -u frg_panic/frg_log: mlibc los referencia como símbolos débiles, y un
# enlazado estático no saca de libc.a un miembro solo por una referencia
# débil — sin forzarlo quedarían resueltos a la dirección 0.
LDFLAGS := -static --image-base=0xA000000000 -z max-page-size=0x1000 \
           -u frg_panic -u frg_log --strip-all

C_PROGRAMS := hello libctest crash fpustress

all: asm c
asm: $(BUILD)/hello-asm.elf $(BUILD)/crt0test.elf
c: $(addprefix $(BUILD)/,$(C_PROGRAMS))

$(BUILD):
	mkdir -p $@

# --- ensamblador puro (M9) ---
$(BUILD)/hello-asm.o: hello.asm | $(BUILD)
	$(NASM) -f elf64 $< -o $@

$(BUILD)/hello-asm.elf: $(BUILD)/hello-asm.o linker.ld
	$(LLD) -T linker.ld -static -o $@ $<

# --- C + mlibc (M11) ---
$(SYSROOT)/lib/libc.a:
	@echo "Falta el sysroot de mlibc — ejecuta primero: userland/mlibc-sysdeps/build-mlibc.sh"; false

$(BUILD)/%.o: c/%.c $(SYSROOT)/lib/libc.a | $(BUILD)
	$(CC) $(CFLAGS) -c $< -o $@

$(BUILD)/%: $(BUILD)/%.o $(SYSROOT)/lib/libc.a
	$(LLD) $(LDFLAGS) -o $@ $(SYSROOT)/lib/crt1.o $< $(SYSROOT)/lib/libm.a $(SYSROOT)/lib/libc.a

# --- binarios incrustados en el kernel (kernel/src/elf.rs) ---
# Solo hace falta regenerarlos si cambia su fuente; el resultado se pega
# a mano sobre el array correspondiente.
$(BUILD)/crt0test.elf: crt0test.asm | $(BUILD)
	$(NASM) -f bin $< -o $@

rust_bytes = python3 -c "import sys; d = open(sys.argv[1], 'rb').read(); \
print(f'pub static {sys.argv[2]}: [u8; {len(d)}] = ['); \
[print('    ' + ' '.join(f'0x{b:02X},' for b in d[i:i+16])) for i in range(0, len(d), 16)]; \
print('];')"

# TEST_ELF_HELLO  <- hello.asm
$(BUILD)/hello_rust_bytes.txt: $(BUILD)/hello-asm.elf
	$(rust_bytes) $< TEST_ELF_HELLO > $@

# TEST_ELF_CRT0   <- crt0test.asm
$(BUILD)/crt0test_rust_bytes.txt: $(BUILD)/crt0test.elf
	$(rust_bytes) $< TEST_ELF_CRT0 > $@

clean:
	rm -rf $(BUILD)

.PHONY: all asm c clean
.SECONDARY:
