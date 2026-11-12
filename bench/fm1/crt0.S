/* SPDX-FileCopyrightText: 2026 FM-1 ZX contributors */
/* SPDX-License-Identifier: GPL-3.0-or-later */

    .section .text.crt0, "ax"
    .global _start
_start:
    goto 1f
    rts
1:
    cli
    sp = 0x01C7A000
    ssp = 0x01C7C000
    call main
    goto .
