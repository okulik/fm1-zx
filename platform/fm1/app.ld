/* SPDX-FileCopyrightText: 2026 FM-1 ZX contributors */
/* SPDX-License-Identifier: GPL-3.0-or-later */

/* The firmware image: .entry, .ram_text (load copy), .text/.rodata and .data (load copy),
 * back to back in XIP from 0x02000120, the boot loader's entry point. fmzx_cstart (main.c)
 * copies .ram_text and .data to SRAM and zeroes .bss and .pool; .noinit is left as it was.
 * The game pack is not linked: it sits in the app slot at ALIGN(_app_image_end, 4096).
 *
 * SRAM code (RAMTEXT) and XIP code are out of each other's branch reach (+-4 MiB): calls
 * between them go through function pointers. check_reach.py (run after the link) fails the
 * build on any direct call or jump between the two, and on RAMTEXT code taking an address in
 * XIP (where some calls would still be in reach, the linker does not catch them). */

ENTRY(_start)

MEMORY
{
    XIP     (rx)  : ORIGIN = 0x02000120, LENGTH = 0x8DFBC   /* the app slot */
    RAMTEXT (rwx) : ORIGIN = 0x01C00000, LENGTH = 48K
    /* 0x01C0C000 - 0x01C0E000: reserved, nothing linked */
    RAM     (rw)  : ORIGIN = 0x01C0E000, LENGTH = 64K
    POOL    (rw)  : ORIGIN = 0x01C1E000, LENGTH = 0x01C78000 - 0x01C1E000
    /* the stacks: _user_stack_top, _sup_stack_top below (crt0.S) */
    NOINIT  (rw)  : ORIGIN = 0x01C7C000, LENGTH = 0x3D50
}

_app_slot_end = ORIGIN(XIP) + LENGTH(XIP);
_user_stack_top = 0x01C7A000;    /* user stack 0x01C78000 - 0x01C7A000 */
_sup_stack_top = 0x01C7C000;     /* supervisor stack 0x01C7A100 - 0x01C7C000 */

SECTIONS
{
    .entry : { KEEP(*(.entry)) } > XIP

    /* The Z80 core and the sound library (the device copies, platform/fm1/CMakeLists.txt) and
     * what calls them directly (pix_compose_zx, the game step), the audio interrupt
     * (felucca-bsp's isr_alnk0 wrapper, fm1_alnk0_irq) with the FIFO, the LCD pump, and the mem*
     * copies they call. */
    .ram_text : ALIGN(4)
    {
        _ram_text_start = .;
        *(.ram_text .ram_text.*)
        *libzx_core.a:*(.text .text.*)
        *libzx_sound.a:*(.text .text.*)
        *(.text.pix_compose_zx)
        *(.text.isr_alnk0)
        *(.text.fmzx_rt_*)
        . = ALIGN(4);
        _ram_text_end = .;
    } > RAMTEXT AT > XIP
    _ram_text_load = LOADADDR(.ram_text);

    .text : ALIGN(4)
    {
        *(.text .text.*)
        KEEP(*(.rodata.fmzx_product))
        *(EXCLUDE_FILE(*libzx_core.a:* *libzx_sound.a:* *audio.c.obj) .rodata EXCLUDE_FILE(*libzx_core.a:* *libzx_sound.a:* *audio.c.obj) .rodata.*)
        . = ALIGN(4);
        __init_array_start = .;
        KEEP(*(SORT(.init_array.*)))
        KEEP(*(.init_array))
        __init_array_end = .;
        . = ALIGN(4);
    } > XIP

    /* The core's tables (contention, flags) and the sound library's (the sine) are read from
     * SRAM, and the audio interrupt reads no flash. */
    .data : ALIGN(4)
    {
        _data_start = .;
        *libzx_core.a:*(.rodata .rodata.*)
        *libzx_sound.a:*(.rodata .rodata.*)
        *audio.c.obj(.rodata .rodata.*)
        *(.data .data.*)
        *(.sdata .sdata.*)
        . = ALIGN(4);
        _data_end = .;
    } > RAM AT > XIP
    _data_load = LOADADDR(.data);
    _app_image_end = LOADADDR(.data) + SIZEOF(.data);

    .bss (NOLOAD) : ALIGN(4)
    {
        _bss_start = .;
        *(.bss .bss.*)
        *(.sbss .sbss.*)
        *(COMMON)
        . = ALIGN(4);
        _bss_end = .;
    } > RAM

    /* Big buffers (section(".pool.*")): the ROM copy, the LCD bands, the machine, the game file,
     * the sound mixer, the frame's samples, the audio FIFO and DMA halves. */
    .pool (NOLOAD) : ALIGN(4)
    {
        _pool_start = .;
        *(.pool .pool.*)
        . = ALIGN(4);
        _pool_end = .;
    } > POOL

    /* Kept across resets: the boot options first (emu-gate writes them), then the crash record. */
    .noinit (NOLOAD) : ALIGN(4)
    {
        KEEP(*(.noinit.bootopt))
        *(.noinit .noinit.*)
    } > NOINIT

    /DISCARD/ : { *(.comment) *(.note*) }
}

ASSERT(SIZEOF(.ram_text) <= 48K, "RAMTEXT over 48 KiB")
ASSERT(_ram_text_end <= 0x01C0C000, "code in the reserved 0x01C0C000-0x01C0E000")
ASSERT(ADDR(.data) >= 0x01C0E000, "data in the reserved 0x01C0C000-0x01C0E000")
ASSERT(_app_image_end <= _app_slot_end, "image larger than the app slot")
ASSERT(LOADADDR(.ram_text) == ADDR(.entry) + SIZEOF(.entry), ".ram_text must load right after .entry")
ASSERT(ADDR(.text) == LOADADDR(.ram_text) + SIZEOF(.ram_text), ".text must follow .ram_text's load copy")
ASSERT(LOADADDR(.data) == ADDR(.text) + SIZEOF(.text), ".data must load right after .text")
ASSERT(fmzx_bootopt == ORIGIN(NOINIT), "fmzx_bootopt must be first in NOINIT")
ASSERT(_pool_end <= 0x01C78000, "POOL runs into the user stack")
