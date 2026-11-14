/* SPDX-FileCopyrightText: 2026 FM-1 ZX contributors */
/* SPDX-License-Identifier: GPL-3.0-or-later */

/* Method B's table for host tests: the built-in engines plus the test fixtures' engines.
 * Compiled by the host build (zx-sys build.rs) only, never into the firmware.
 *
 * For now it holds no engine (an unused entry, as C has no empty arrays). */

#include "sound.h"

const snd_engine snd_engine_test_table[] = {
	{ "(none)" },   /* unused: snd_engine_test_count is 0 */
};
const uint32_t snd_engine_test_count = 0;
