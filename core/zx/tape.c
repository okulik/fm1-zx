/* SPDX-FileCopyrightText: 2026 FM-1 ZX contributors */
/* SPDX-License-Identifier: GPL-3.0-or-later */

/* .tap blocks, the LD-BYTES trap and LOAD "" autostart. */

#include <string.h>

#include "zx_internal.h"

#define SYS_E_LINE 0x5C59u
#define SYS_K_CUR 0x5C5Bu
#define EDITOR_E_LINE 0x5CCCu   /* E_LINE after boot: no microdrive channels */

#define AUTO_HOLD_FRAMES 3u     /* each key is held this long, */
#define AUTO_RELEASE_FRAMES 6u  /* then released this long: the ROM frees a key's slot
                                 * only after 5 key-free frames, so a repeat of the
                                 * same key (the second ") needs a longer gap */
#define AUTO_KEYS 4u
#define AUTO_WAIT 1u            /* auto_step: waiting for the editor */

int zx_tape_insert(zx_machine *m, const uint8_t *tap, uint32_t len)
{
	uint32_t pos = 0;

	while (pos < len) {
		if (len - pos < 2u) return ZX_ERR_FORMAT;
		pos += 2u + ((uint32_t)tap[pos] | ((uint32_t)tap[pos + 1u] << 8));
		if (pos > len) return ZX_ERR_FORMAT;
	}
	m->tape = tap;
	m->tape_len = len;
	m->tape_pos = 0;
	return ZX_OK;
}

/* LD-BYTES on entry: A = flag, IX = destination, DE = length, carry set for
 * LOAD and clear for VERIFY. Takes the next block off the tape (nothing at
 * the end of it). Success needs a matching flag byte, a good XOR checksum and
 * at least DE data bytes; then IX += DE, DE = 0 and carry is set, with A = 0
 * and the other flags as the ROM's final "LD A,H; CP 1" leaves them. Failure
 * clears carry and touches nothing else. Either way interrupts end up enabled,
 * as SA/LD-RET does. B, C, H, L and AF' are not reproduced.
 *
 * A block is a 2-byte length, then flag, data, checksum; insert checked the
 * lengths, so a block is wholly inside the tape. */
static void tape_load(zx_machine *m)
{
	Z80 *c = &m->cpu;
	const uint8_t *blk;
	uint32_t blen, i, n = Z80_DE(*c);
	uint16_t dest = Z80_IX(*c);
	int ok = 0;

	c->iff1 = c->iff2 = 1;
	if (m->tape_len - m->tape_pos < 2u) {
		Z80_F(*c) &= (uint8_t)~Z80_CF;   /* end of tape: nothing consumed */
		return;
	}
	blk = m->tape + m->tape_pos;
	blen = (uint32_t)blk[0] | ((uint32_t)blk[1] << 8);
	m->tape_pos += 2u + blen;
	blk += 2;

	if (blen >= 2u && blk[0] == Z80_A(*c) && blen - 2u >= n) {
		uint8_t x = 0;

		for (i = 0; i < blen; i++) x ^= blk[i];
		ok = x == 0;
	}
	if (ok && !(Z80_F(*c) & Z80_CF)) {   /* VERIFY: compare, don't write */
		for (i = 0; i < n && ok; i++)
			ok = zx_mem_read(m, (uint16_t)(dest + i)) == blk[1u + i];
	} else if (ok) {
		for (i = 0; i < n; i++) zx_mem_write(m, (uint16_t)(dest + i), blk[1u + i]);
	}

	if (!ok) {
		Z80_F(*c) &= (uint8_t)~Z80_CF;
		return;
	}
	Z80_IX(*c) = (uint16_t)(dest + n);
	Z80_DE(*c) = 0;
	Z80_AF(*c) = 0x0093u;   /* A = 0; S, H, N, C */
}

/* The CPU calls this for every 0x64 (Z80_HOOK, which is also LD H,H) it
 * executes, so tell the trap from a real LD H,H by the address: the trap is
 * the only way to reach 0x0556 with that opcode, as the ROM's own byte there
 * is not 0x64. The real LD H,H becomes a NOP, which does the same in 4 T. */
uint8_t zx_tape_hook(void *context, uint16_t address)
{
	if (address != ZX_LD_BYTES) return 0x00u;
	tape_load(context);
	return 0xC9u;   /* RET back to LD-BYTES's caller */
}

void zx_tape_autostart(zx_machine *m)
{
	const uint8_t *tape = m->tape;
	uint32_t len = m->tape_len;

	zx_init(m, m->rom);
	m->tape = tape;
	m->tape_len = len;
	m->auto_step = AUTO_WAIT;
}

/* The ROM's editor is waiting for a key once it has set the edit line up and
 * put the cursor in it: K_CUR = E_LINE, at the address a fresh 48K gives it
 * (the RAM test fills memory with other patterns first). Interrupts are on. */
static int editor_ready(const zx_machine *m)
{
	uint16_t e_line = (uint16_t)(zx_mem_read(m, SYS_E_LINE) | (zx_mem_read(m, SYS_E_LINE + 1u) << 8));
	uint16_t k_cur = (uint16_t)(zx_mem_read(m, SYS_K_CUR) | (zx_mem_read(m, SYS_K_CUR + 1u) << 8));

	return e_line == EDITOR_E_LINE && k_cur == e_line && m->cpu.iff1;
}

/* Keys typed: J (LOAD), SYMBOL SHIFT+P twice (""), ENTER; rows 5, 6, 7. */
static const uint8_t auto_keys[AUTO_KEYS][3] = {
	{ 0x00u, 0x08u, 0x00u },
	{ 0x01u, 0x00u, 0x02u },
	{ 0x01u, 0x00u, 0x02u },
	{ 0x00u, 0x01u, 0x00u },
};

/* auto_step: 1 waits for the editor; then 2 + 2k presses key k and 3 + 2k
 * releases it; 0 is idle. */
void zx_tape_frame(zx_machine *m)
{
	uint8_t keys[8];

	if (m->auto_step == 0) return;
	if (m->auto_step == AUTO_WAIT) {
		if (!editor_ready(m)) return;
		m->auto_step = 2;
		m->auto_timer = AUTO_HOLD_FRAMES;
	}
	memset(keys, 0, sizeof keys);
	if (!(m->auto_step & 1u)) memcpy(&keys[5], auto_keys[(m->auto_step - 2u) >> 1], 3);
	zx_set_keys(m, keys);
	if (--m->auto_timer == 0) {
		m->auto_step = m->auto_step >= 1u + 2u * AUTO_KEYS ? 0 : (uint8_t)(m->auto_step + 1u);
		m->auto_timer = (m->auto_step & 1u) ? AUTO_RELEASE_FRAMES : AUTO_HOLD_FRAMES;
	}
}
