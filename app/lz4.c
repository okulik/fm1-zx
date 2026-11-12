/* SPDX-FileCopyrightText: 2026 FM-1 ZX contributors */
/* SPDX-License-Identifier: GPL-3.0-or-later */

#include "lz4.h"
#include "zx.h"
#include <string.h>

/* Reads the 255-run extension of a length; fails if it runs off the input or past `limit`
 * (so the sum cannot overflow: limit is at most cap, and each step adds at most 255). */
static int read_len(const uint8_t *src, uint32_t n, uint32_t *ip, uint32_t *len, uint32_t limit)
{
    uint8_t s;
    do {
        if (*ip >= n) {
            return ZX_ERR_FORMAT;
        }
        s = src[(*ip)++];
        *len += s;
        if (*len > limit) {
            return ZX_ERR_FORMAT;
        }
    } while (s == 255u);
    return ZX_OK;
}

int lz4_block_decode(const uint8_t *src, uint32_t n, uint8_t *dst, uint32_t cap, uint32_t *out_len)
{
    uint32_t ip = 0, op = 0;

    if (n == 0u) {
        return ZX_ERR_FORMAT;
    }
    while (ip < n) {
        uint32_t token = src[ip++];
        uint32_t lit = token >> 4;
        uint32_t mlen, off, i;

        if (lit == 15u && read_len(src, n, &ip, &lit, cap) != ZX_OK) {
            return ZX_ERR_FORMAT;
        }
        if (lit > n - ip || lit > cap - op) {
            return ZX_ERR_FORMAT;
        }
        memcpy(dst + op, src + ip, lit);
        ip += lit;
        op += lit;
        if (ip == n) {
            break;   /* the last sequence has literals only */
        }
        if (n - ip < 2u) {
            return ZX_ERR_FORMAT;
        }
        off = (uint32_t)src[ip] | ((uint32_t)src[ip + 1u] << 8);
        ip += 2u;
        if (off == 0u || off > op) {
            return ZX_ERR_FORMAT;
        }
        mlen = token & 15u;
        if (mlen == 15u && read_len(src, n, &ip, &mlen, cap) != ZX_OK) {
            return ZX_ERR_FORMAT;
        }
        mlen += 4u;
        if (mlen > cap - op) {
            return ZX_ERR_FORMAT;
        }
        for (i = 0; i < mlen; i++) {   /* byte by byte: the ranges may overlap */
            dst[op + i] = dst[op + i - off];
        }
        op += mlen;
    }
    *out_len = op;
    return ZX_OK;
}
