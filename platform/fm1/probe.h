/* SPDX-FileCopyrightText: 2026 FM-1 ZX contributors */
/* SPDX-License-Identifier: GPL-3.0-or-later */

/* The boot probe (CMake FMZX_BOOT_PROBE, preset fm1-probe): a diagnostic build for a device that
 * starts to a black screen. Before the panel is used, probe_mark blinks the backlight n times after
 * each of the first start-up steps (timed by a CPU loop before TIMER4 is set up, by TIMER4 after).
 * probe_early fills the panel with one colour without DMA right after TIMER4 is set up (magenta),
 * and again after probe_power raises the core voltages as SLOOP does (green).
 * probe_start sets the panel up as SLOOP and m-bootloader do (RGB565), turns it on and draws a white
 * band without DMA and a cyan one with DMA. Then one band per start-up stage (green: done; yellow:
 * done, but the step reported a failure). On a CPU fault it draws a red band and the crash record's
 * words as bits (a white cell 1, a dark blue one 0, the most significant bit on the left), then
 * blinks the backlight fast instead of rebooting. All XIP code, blocking, used before and outside
 * the LCD scheduler. */

#pragma once
#include <stdint.h>

#ifndef FMZX_BOOT_PROBE
#define FMZX_BOOT_PROBE 0
#endif

#define PROBE_GREEN 0x0F0u
#define PROBE_YELLOW 0xFF0u

#if FMZX_BOOT_PROBE
void probe_mark(unsigned n, int timer);
void probe_early(uint32_t rgb444);
void probe_power(void);
void probe_start(int fmt);
void probe_stage(unsigned k, uint32_t rgb444);
void probe_fault(const uint32_t *words, unsigned n);
#else
#define probe_mark(n, t) ((void)0)
#define probe_early(c) ((void)0)
#define probe_power() ((void)0)
#define probe_start(fmt) ((void)0)
#define probe_stage(k, c) ((void)0)
#endif
