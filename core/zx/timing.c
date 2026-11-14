/* SPDX-FileCopyrightText: 2026 FM-1 ZX contributors */
/* SPDX-License-Identifier: GPL-3.0-or-later */

/* 48K contention and the CPU callbacks that apply it.
 *
 * Z80 runs a whole instruction and only then adds its length to cpu.cycles, so
 * a callback cannot read the current T-state from the CPU. Z80InsnClock tells
 * each callback where in the instruction its M-cycle starts: clock.cycles is
 * that offset (Extra mode also yields the extra T-states some M-cycles hold).
 * The M-cycle's frame T-state is insn_t0 + clock.cycles + wait, where wait is
 * the delay already inserted earlier in the same instruction. A delay is added
 * to cpu.cycles with z80_wait (Z80_WITH_VOLATILE_CYCLES makes the library
 * re-read cpu.cycles after the instruction function returns).
 *
 * The start of each M-cycle is contended, and so are the internal "extra"
 * T-states that follow a memory or I/O M-cycle (e.g. the 5 after a JR operand,
 * which are contended at the operand's address). Only the extras of an M1/IR
 * cycle are not contended. */

#ifndef Z80_WITH_VOLATILE_CYCLES
#error "timing.c needs Z80_WITH_VOLATILE_CYCLES: contention is added with z80_wait mid-instruction"
#endif

#include "zx.h"
#include "zx_internal.h"

/* Contention over one 8-T-state phase, indexed by (t - 14335) & 7. */
static const uint8_t pattern[8] = {6, 5, 4, 3, 2, 1, 0, 0};

uint32_t zx_contention(uint32_t tstate)
{
	uint32_t t = tstate - ZX_FIRST_CONTENDED;   /* wraps to a huge value below the paper */
	uint32_t phase;

	if (t >= ZX_SCREEN_LINES * ZX_LINE_TSTATES) return 0;
	phase = t % ZX_LINE_TSTATES;   /* constant divisor: the compiler emits a multiply */
	if (phase >= 128u) return 0;
	return pattern[phase & 7u];
}

static inline int is_contended(uint16_t address)
{
	return (address & 0xC000u) == 0x4000u;
}

/* T-state at which the M-cycle now starting begins. */
static inline uint32_t mcycle_t(const zx_machine *m)
{
	return m->insn_t0 + m->clock.cycles + m->wait;
}

static inline void delay(zx_machine *m, uint32_t d)
{
	if (d) {
		m->wait += d;
		z80_wait(&m->cpu, d);
	}
}

/* `extra` internal T-states that follow an M-cycle of `length` T at the same
 * (contended) address (Fuse's addr:1 x n): each is contended, and
 * the next one starts 1 + delay later. `t` is the M-cycle's start. */
static void contend_extra(zx_machine *m, uint32_t t, uint32_t length, uint8_t extra)
{
	uint32_t i;

	t += length;
	for (i = 0; i < extra; i++) {
		uint32_t d = zx_contention(t);

		delay(m, d);
		t += d + 1u;
	}
}

/* Memory M-cycle (operand fetch, read or write): contend, then step the clock. */
static inline void mem_mcycle(zx_machine *m, uint16_t address)
{
	uint8_t extra = z80_insn_clock_extra_m(&m->clock);

	if (is_contended(address)) {
		uint32_t t = mcycle_t(m);
		uint32_t d = zx_contention(t);

		delay(m, d);
		if (extra) contend_extra(m, t + d, 3u, extra);
	}
	z80_insn_clock_extra_add_m(&m->clock, extra);
}

static uint8_t cpu_fetch_opcode(void *context, uint16_t address)
{
	zx_machine *m = context;
	uint8_t opcode = zx_mem_read(m, address);
	uint8_t fetched = opcode;
	uint8_t extra;
	uint8_t start = 0;   /* clock offset of this M1 */

	/* LD-BYTES trap: the CPU runs the hook, which loads the block and returns
	 * RET to execute, so time the instruction as the RET it becomes. */
	if (address == ZX_LD_BYTES && m->tape != NULL && !m->insn_open) {
		fetched = Z80_HOOK;
		opcode = 0xC9u;
	}

	if (m->insn_open) {
		start = m->clock.cycles;
	} else {
		z80_insn_clock_start(&m->clock, 0);
		m->insn_t0 = m->run_base + (uint32_t)m->cpu.cycles;
		m->wait = 0;
		m->insn_pc = address;
	}
	extra = z80_insn_clock_extra_m1(&m->clock, opcode);
	/* The table sets clock.cycles to 4 only for CB/ED-prefixed opcodes; for DD/FD
	 * ones (and DD CB) it leaves 0, dropping the prefix M1s. Keep our own offset. */
	m->clock.cycles = start;
	m->insn_open = m->clock.prefix != Z80_INSN_CLOCK_PREFIX_NONE;

	if (is_contended(address)) delay(m, zx_contention(mcycle_t(m)));
	z80_insn_clock_extra_add_m1(&m->clock, extra);
	return fetched;
}

static uint8_t cpu_mem_read(void *context, uint16_t address)
{
	zx_machine *m = context;

	mem_mcycle(m, address);
	return zx_mem_read(m, address);
}

static void cpu_mem_write(void *context, uint16_t address, uint8_t value)
{
	zx_machine *m = context;

	mem_mcycle(m, address);
	zx_mem_write(m, address, value);
}

/* Interrupt acknowledge: the data bus is 0xFF. Starts a new "instruction"
 * whose M-cycles are the response's stack pushes and vector reads. */
static uint8_t cpu_inta(void *context, uint16_t address)
{
	zx_machine *m = context;
	uint8_t extra;

	(void)address;
	z80_insn_clock_start(&m->clock, 0);
	m->insn_t0 = m->run_base + (uint32_t)m->cpu.cycles;
	m->wait = 0;
	extra = z80_insn_clock_extra_inta(&m->clock, m->cpu.im != 0, 0xFF);
	z80_insn_clock_extra_add_inta(&m->clock, extra);
	m->insn_open = 0;
	return 0xFF;
}

/* Extra T-states a port access adds to its 4-T-state I/O cycle. */
static uint32_t io_delay(uint32_t t, uint16_t port)
{
	uint32_t d = 0;
	uint32_t i;

	if (is_contended(port)) {
		if (!(port & 1u)) {           /* C:1, C:3 */
			d = zx_contention(t);
			d += zx_contention(t + d + 1u);
		} else {                      /* C:1, C:1, C:1, C:1 */
			uint32_t u = t;

			for (i = 0; i < 4u; i++) {
				uint32_t c = zx_contention(u);

				d += c;
				u += c + 1u;
			}
		}
	} else if (!(port & 1u)) {            /* N:1, C:3 */
		d = zx_contention(t + 1u);
	}
	return d;
}

static inline void io_mcycle(zx_machine *m, uint16_t port)
{
	uint32_t t = mcycle_t(m);
	uint32_t d = io_delay(t, port);
	uint8_t extra = z80_insn_clock_extra_m(&m->clock);

	delay(m, d);
	m->io_t = t + d;
	if (extra && is_contended(port)) contend_extra(m, t + d, 4u, extra);
	z80_insn_clock_extra_add_in(&m->clock);
	m->clock.cycles += extra;
}

static uint8_t cpu_in(void *context, uint16_t port)
{
	zx_machine *m = context;

	io_mcycle(m, port);
	return zx_port_in(m, port);
}

static void cpu_out(void *context, uint16_t port, uint8_t value)
{
	zx_machine *m = context;

	io_mcycle(m, port);
	zx_port_out(m, port, value);
}

/* The clock's own register reads (CPIR's compare) must not be timed. */
static uint8_t clock_read(void *context, uint16_t address)
{
	return zx_mem_read(context, address);
}

void zx_timing_init(zx_machine *m)
{
	/* Optional callbacks (halt, nop, nmia, int_fetch, ld_i_a, ld_r_a, reti,
	 * retn, illegal) stay NULL; a NULL nop means HALT_SKIP. */
	m->cpu.context = m;
	m->cpu.fetch_opcode = cpu_fetch_opcode;
	m->cpu.hook = zx_tape_hook;
	m->cpu.fetch = cpu_mem_read;
	m->cpu.read = cpu_mem_read;
	m->cpu.write = cpu_mem_write;
	m->cpu.in = cpu_in;
	m->cpu.out = cpu_out;
	m->cpu.inta = cpu_inta;
	m->cpu.options = Z80_MODEL_ZILOG_NMOS;
	z80_insn_clock_initialize(&m->clock, &m->cpu.af, &m->cpu.bc, &m->cpu.hl, m, clock_read);
}

#ifdef ZX_TESTING
void zx_test_set_tstate(zx_machine *m, uint32_t t)
{
	m->frame_tstate = t;
	m->insn_open = 0;
}

uint8_t zx_test_level(const zx_machine *m)
{
	return m->level;
}

uint32_t zx_test_step(zx_machine *m)
{
	uint32_t ran = 0;

	m->insn_open = 0;
	do {   /* a lone prefix ends a run with the instruction still open */
		m->run_base = m->frame_tstate + ran;
		ran += (uint32_t)z80_run(&m->cpu, 1);
	} while (m->insn_open);
	m->frame_tstate += ran;
	return ran;
}
#endif
