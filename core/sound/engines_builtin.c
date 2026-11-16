/* SPDX-FileCopyrightText: 2026 FM-1 ZX contributors */
/* SPDX-License-Identifier: GPL-3.0-or-later */

/* Method B's built-in table (engines_builtin.h): the entries the mixer starts with, which the
 * firmware copies ahead of a game's learned entries. Method B reaches their readers only through
 * an entry's read pointer, once a frame, so on the device this file runs from flash (XIP), not
 * SRAM: the firmware builds it itself (platform/fm1/CMakeLists.txt), its copy calling the core's
 * zx_peek and zx_get_regs, which are in RAMTEXT, through main.c's fmzx_xip_*. */

#include "engines_builtin.h"

const snd_engine snd_engine_table[] = { SND_ENGINES_BUILTIN };
const uint32_t snd_engine_count = sizeof snd_engine_table / sizeof snd_engine_table[0];
_Static_assert(sizeof snd_engine_table / sizeof snd_engine_table[0] == SND_ENGINE_BUILTIN_MAX,
               "SND_ENGINE_BUILTIN_MAX is the built-in count");
