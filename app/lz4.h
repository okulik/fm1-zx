/* SPDX-FileCopyrightText: 2026 FM-1 ZX contributors */
/* SPDX-License-Identifier: GPL-3.0-or-later */

/* Decoder for one raw LZ4 block (no frame, no size prefix). */
#ifndef FMZX_LZ4_H
#define FMZX_LZ4_H

#include <stdint.h>

#define LZ4_MAX_CAP 0x7FFFFFFFu   /* keeps every length sum below 2^32 */

/* Decodes src[0..n) into dst[0..cap). ZX_OK with *out_len set, or ZX_ERR_FORMAT for a
 * malformed block (or a cap above LZ4_MAX_CAP): it never reads past n, never writes past cap, and does O(n + cap) work. */
int lz4_block_decode(const uint8_t *src, uint32_t n, uint8_t *dst, uint32_t cap, uint32_t *out_len);

#endif
