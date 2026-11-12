/* SPDX-FileCopyrightText: 2026 FM-1 ZX contributors */
/* SPDX-License-Identifier: GPL-3.0-or-later */

/* libc_min.c again, as fmzx_rt_memcpy, fmzx_rt_memset, fmzx_rt_memmove and fmzx_rt_memcmp, for
 * the code in RAMTEXT: XIP's memcpy is out of its branch reach. The device copies of the core
 * and of app/pixels.c call these names (redefine_syms.py renames their references); app.ld puts
 * the .text.fmzx_rt_* sections in .ram_text. */

#define memcpy fmzx_rt_memcpy
#define memmove fmzx_rt_memmove
#define memset fmzx_rt_memset
#define memcmp fmzx_rt_memcmp

#include "libc_min.c"
