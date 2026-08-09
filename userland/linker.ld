/* Linker script para programas de userspace (Ring 3) de LixOS.
 * Se enlaza en la mitad baja del espacio de direcciones; el kernel vive en -2 GB. */
ENTRY(_start)

SECTIONS
{
    . = 0x400000;

    .text : ALIGN(4K) {
        *(.text .text.*)
    }

    .rodata : ALIGN(4K) {
        *(.rodata .rodata.*)
    }

    .data : ALIGN(4K) {
        *(.data .data.*)
        *(.got .got.*)
    }

    .bss : ALIGN(4K) {
        *(.bss .bss.*)
        *(COMMON)
    }

    /DISCARD/ : {
        *(.eh_frame*)
        *(.comment)
    }
}
