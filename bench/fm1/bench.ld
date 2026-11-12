/* SPDX-FileCopyrightText: 2026 FM-1 ZX contributors */
/* SPDX-License-Identifier: GPL-3.0-or-later */

ENTRY(_start)

MEMORY
{
    XIP (rx)  : ORIGIN = 0x02000000, LENGTH = 0x80000
    RAM (rwx) : ORIGIN = 0x01C08000, LENGTH = 0x6C000
}

SECTIONS
{
    .text : ALIGN(4)
    {
        KEEP(*(.text.crt0))
        *(.text .text.*)
        *(.rodata .rodata.*)
        . = ALIGN(4);
        _etext = .;
    } > XIP

    .data : ALIGN(4)
    {
        _sdata = .;
        *(.data .data.*)
        *(.sdata .sdata.*)
        . = ALIGN(4);
        _edata = .;
    } > RAM AT > XIP

    _data_load = LOADADDR(.data);

    .bss (NOLOAD) : ALIGN(4)
    {
        _sbss = .;
        *(.bss .bss.*)
        *(.sbss .sbss.*)
        *(COMMON)
        . = ALIGN(4);
        _ebss = .;
    } > RAM

    /DISCARD/ : { *(.comment) *(.note*) }
}

ASSERT(LOADADDR(.data) == _etext, ".data must load right after .text")
