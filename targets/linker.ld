ENTRY(_start)

/* Multiboot2 exige que el kernel se cargue a partir de 1 MiB. */
KERNEL_BASE = 0x00100000;

/* NOTA (M2): kernel "flat" — virtual == físico — a propósito.
 * boot.asm (M0) solo monta identity-map del primer GiB; un layout
 * higher-half (PML4[511], como se planteó al principio) necesita mapeo
 * de páginas adicional que no tiene sentido escribir a mano sin
 * compilador+QEMU delante para validarlo. Migrar a higher-half real es
 * tarea futura una vez exista ese feedback loop — ver docs/ARCHITECTURE.md.
 */

SECTIONS
{
    . = KERNEL_BASE;

    .boot ALIGN(4K) :
    {
        KEEP(*(.multiboot2))
        *(.boot.text)
        *(.boot.data)
        *(.boot.bss)
    }

    .text ALIGN(4K) : { *(.text .text.*) }
    .rodata ALIGN(4K) : { *(.rodata .rodata.*) }
    .data ALIGN(4K) : { *(.data .data.*) }

    .bss ALIGN(4K) :
    {
        __bss_start = .;
        *(COMMON)
        *(.bss .bss.*)
        __bss_end = .;
    }

    _kernel_end = .;
}
