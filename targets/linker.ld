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

/* PHDRS explícitos: un segmento PT_NOTE para el Xen PVH entry note
 * (QEMU 11+ requiere esto para cargar el kernel directamente con -kernel),
 * y el segmento PT_LOAD habitual para todo el código y datos.
 * La sección .note.Xen pertenece a ambos: QEMU la lee como PT_NOTE,
 * GRUB la carga junto con el resto como parte del PT_LOAD. */
PHDRS {
    pvh  PT_NOTE FLAGS(4);
    load PT_LOAD FLAGS(7);
}

SECTIONS
{
    . = KERNEL_BASE;

    /* Note XEN PVH — debe preceder al código de boot para que su dirección
     * de carga coincida con el inicio del PT_LOAD. */
    .note.Xen ALIGN(4) : { KEEP(*(.note.Xen)) } :pvh :load

    .boot ALIGN(4K) :
    {
        KEEP(*(.multiboot2))
        *(.boot.text)
        *(.boot.data)
        *(.boot.bss)
    } :load

    .text ALIGN(4K) : { *(.text .text.*) } :load
    .rodata ALIGN(4K) : { *(.rodata .rodata.*) } :load
    .data ALIGN(4K) : { *(.data .data.*) } :load

    .bss ALIGN(4K) :
    {
        __bss_start = .;
        *(COMMON)
        *(.bss .bss.*)
        __bss_end = .;
    } :load

    _kernel_end = .;
}
