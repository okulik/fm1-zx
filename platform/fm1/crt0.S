/* SPDX-FileCopyrightText: 2026 FM-1 ZX contributors */
/* SPDX-License-Identifier: GPL-3.0-or-later */

/* The boot loader enters the app image at its first byte (0x02000120, XIP) in user mode.
 * Interrupts off, the user and supervisor stacks set (tops from app.ld), then C:
 * fmzx_cstart (main.c) never returns. */

    .section .entry, "ax"
    .global _start
_start:
    goto 1f
    rts
1:
    cli
    sp = _user_stack_top
    ssp = _sup_stack_top
    call fmzx_cstart
    goto .
