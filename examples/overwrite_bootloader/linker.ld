/* Linker script for NumWorks N0120 Slot B (External QSPI/OctoSPI Flash) */

ENTRY(Reset_Handler)

MEMORY
{
    /* Slot B starts at 0x90400000 in external flash */
    FLASH (rx)  : ORIGIN = 0x90400000, LENGTH = 4M
    /* AXI SRAM on STM32H725 (256 KB available from 0x24000000 to 0x24040000) */
    SRAM (rwx)  : ORIGIN = 0x24000000, LENGTH = 256K
}

_estack = ORIGIN(SRAM) + LENGTH(SRAM) - 8; /* 0x2403FFF8: 8-byte aligned top of SRAM */

SECTIONS
{
    /* 32-byte NumWorks Epsilon slot header */
    .header :
    {
        . = ALIGN(4);
        LONG(0x00000002)                         /* Slot type (2 = Slot B) */
        LONG(_flash_size)                        /* Total image length in bytes */
        LONG(0xDEC00DF0)                         /* Magic 1 */
        BYTE(0x32); BYTE(0x36); BYTE(0x2E); BYTE(0x33); /* "26.3" */
        BYTE(0x2E); BYTE(0x30); BYTE(0x00); BYTE(0x00); /* ".0\0\0" */
        BYTE(0x63); BYTE(0x75); BYTE(0x73); BYTE(0x74); /* "cust" */
        BYTE(0x6F); BYTE(0x6D); BYTE(0x00); BYTE(0x00); /* "om\0\0" */
        LONG(0xDEC00DF0)                         /* Magic 2 */
    } > FLASH

    /* Entry vector for xip-chainloader at 0x90400020 */
    .entry_vector :
    {
        . = ALIGN(4);
        KEEP(*(.entry_vector))
    } > FLASH

    /* Aligned Cortex-M7 Vector table for VTOR */
    .isr_vector : ALIGN(512)
    {
        KEEP(*(.isr_vector))
    } > FLASH

    /* Program code and constants */
    .text :
    {
        . = ALIGN(4);
        *(.text)
        *(.text*)
        *(.rodata)
        *(.rodata*)
        *(.glue_7)
        *(.glue_7t)
        . = ALIGN(4);
    } > FLASH

    /* C++ static constructors */
    .init_array :
    {
        . = ALIGN(4);
        __init_array_start = .;
        KEEP(*(SORT(.init_array.*)))
        KEEP(*(.init_array*))
        __init_array_end = .;
        . = ALIGN(4);
    } > FLASH

    /* Used by startup to copy initialized data to SRAM */
    _sidata = LOADADDR(.data);

    /* Initialized data in SRAM */
    .data :
    {
        . = ALIGN(4);
        _sdata = .;
        *(.data)
        *(.data*)
        . = ALIGN(4);
        _edata = .;
    } > SRAM AT > FLASH

    /* Zero-initialized BSS in SRAM */
    .bss :
    {
        . = ALIGN(4);
        _sbss = .;
        __bss_start__ = _sbss;
        *(.bss)
        *(.bss*)
        *(COMMON)
        . = ALIGN(4);
        _ebss = .;
        __bss_end__ = _ebss;
    } > SRAM

    /* Non-initialized memory for state preserved across warm reset */
    .noinit (NOLOAD) :
    {
        . = ALIGN(4);
        *(.noinit)
        *(.noinit*)
        . = ALIGN(4);
    } > SRAM

    _flash_size = LOADADDR(.data) + SIZEOF(.data) - ORIGIN(FLASH);

    /* Discard unused sections */
    /DISCARD/ :
    {
        *(.ARM.exidx*)
        *(.ARM.extab*)
        *(.comment)
        *(.note*)
    }
}
