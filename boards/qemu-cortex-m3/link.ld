/* Where the parts of a program go in the memory of the LM3S6965, the
 * chip of the board QEMU calls `lm3s6965evb`: 256 KB of flash from
 * address 0, and 64 KB of RAM from 0x20000000.
 *
 * The names that start with an underscore are read by startup.c (what
 * to copy and what to clear before `main`) and by system.c (where the
 * C library's own memory may grow). */

MEMORY {
    FLASH (rx)  : ORIGIN = 0x00000000, LENGTH = 256K
    RAM   (rwx) : ORIGIN = 0x20000000, LENGTH = 64K
}

/* The stack starts at the end of the RAM and grows down. */
_stack_top = ORIGIN(RAM) + LENGTH(RAM);

SECTIONS {
    /* The code and the constants stay in flash. The table of vectors
     * must be the first thing there: the processor reads its first
     * stack pointer and the address of its first instruction from it. */
    .text : {
        KEEP(*(.vectors))
        *(.text*)
        *(.rodata*)
        . = ALIGN(4);
    } > FLASH

    .ARM.exidx : {
        *(.ARM.exidx*)
    } > FLASH

    /* Variables that start with a value: the values are kept in flash,
     * and startup.c copies them into RAM. */
    _data_in_flash = LOADADDR(.data);
    .data : {
        _data_start = .;
        *(.data*)
        . = ALIGN(4);
        _data_end = .;
    } > RAM AT > FLASH

    /* Variables that start at zero: startup.c clears them. */
    .bss : {
        _bss_start = .;
        *(.bss*)
        *(COMMON)
        . = ALIGN(4);
        _bss_end = .;
    } > RAM

    /* What is left of the RAM, between the variables and the stack. */
    _heap_start = .;
}
