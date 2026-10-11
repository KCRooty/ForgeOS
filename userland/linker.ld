/* Linker script para binarios de usuario de ForgeOS.
 * Base en 0xA0_0000_0000 (640 GiB) — igual que los ELFs de prueba
 * del kernel (ver elf.rs::TEST_ELF_*). El loader carga el ELF
 * en la dirección p_vaddr tal cual; no hay PIE por ahora. */

ENTRY(_start)

SECTIONS {
    . = 0x000000A000000000;

    .text   : { *(.text .text.*) }
    .rodata : { *(.rodata .rodata.*) }
    .data   : { *(.data .data.*) }
    .bss    : { *(.bss .bss.*) COMMON }

    /DISCARD/ : { *(.note*) *(.comment) *(.eh_frame*) }
}
