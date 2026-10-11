# Makefile para userland de ForgeOS
# Requiere: nasm, ld.lld (o rust-lld renombrado)
# En CachyOS: sudo pacman -S nasm lld
# En Windows: nasm desde nasm.us, ld.lld desde el toolchain Rust nightly
#   (copiar ~/.rustup/toolchains/nightly-*/lib/rustlib/*/bin/rust-lld.exe → ld.lld.exe)

LLD ?= ld.lld
NASM ?= nasm

all: hello.elf

hello.o: hello.asm
	$(NASM) -f elf64 $< -o $@

hello.elf: hello.o linker.ld
	$(LLD) -T linker.ld -static -o $@ $<

# Genera el array Rust para embeber en kernel/src/elf.rs
hello_rust_bytes.txt: hello.elf
	python3 -c "
import sys
data = open('hello.elf','rb').read()
n = len(data)
print(f'pub static TEST_ELF_HELLO: [u8; {n}] = [')
for i in range(0, n, 16):
    chunk = data[i:i+16]
    print('    ' + ','.join(f'0x{b:02X}' for b in chunk) + ',')
print('];')
" > $@

clean:
	rm -f hello.o hello.elf hello_rust_bytes.txt

.PHONY: all clean
