<!--
SPDX-FileCopyrightText: 2026 FM-1 ZX contributors
SPDX-License-Identifier: GPL-3.0-or-later
-->

# Vendored code in `platform/fm1/bsp/`

## felucca-bsp

- Upstream: https://github.com/hugelton/felucca-bsp
- Commit: `7e09895c3d48988b8e2b0ee1461eb2376224d93f`
- Licence: MPL-2.0 (`bsp/LICENSE`, `bsp/NOTICE`; SPDX header and copyright lines in each file;
  `LICENSES/MPL-2.0.txt`). The files carry no "Incompatible With Secondary Licenses" notice, so
  they combine with our GPL-3.0-or-later code under MPL-2.0 section 3.3.
- Local changes: none. Every file is byte for byte as at the commit above; the firmware hooks
  around it (`platform/fm1/main.c`, `platform/fm1/lcd.c`).

Files, at the same paths as upstream under `bsp/`:

| File | Used for |
| --- | --- |
| `LICENSE`, `NOTICE` | licence text and credits |
| `chip/fm1_cc.h` | `FM1_INLINE` |
| `chip/fm1_cpu1_stop.h` | holding the second core at boot |
| `chip/fm1_flash_access.h` | included by `fm1_cpu1_stop.h` |
| `chip/fm1_gpio.h` | GPIO port registers |
| `chip/fm1_irq.h` | interrupt and exception setup, the crash record, `fm1_fault` hook |
| `chip/fm1_isr.S` | ISR entry wrappers (built from the input and audio tasks on) |
| `chip/fm1_vec.S` | the fatal-vector stubs |
| `chip/fm1_sys.h` | watchdog, reboot, UBOOT entry, mailbox clear |
| `chip/fm1_time.h` | TIMER4 time base and delays |
| `chip/fm1_timer.h` | TIMER5 (input task) |
| `chip/fm1_xip.h` | XIP window addresses |
| `boards/fm1/fm1_adc.h` | SARADC (input task) |
| `boards/fm1/fm1_audio.h` | ALNK0 audio out (audio task) |
| `boards/fm1/fm1_input.h` | key matrix (the UBOOT gesture, input task) |
| `boards/fm1/fm1_lcd_hw.h` | LCD wiring and SPI1 DMA |
| `boards/fm1/fm1_led_anim.h` | included by `fm1_input.h` |

To update: clone upstream, check out the new commit, copy the same files over these, record the
new hash here, and rebuild and rerun the emu-gate tests.
