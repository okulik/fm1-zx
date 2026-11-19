<!--
SPDX-FileCopyrightText: 2026 FM-1 ZX contributors
SPDX-License-Identifier: GPL-3.0-or-later
-->

# Vendored code in `platform/fm1/bsp/`

## felucca-bsp

- Upstream: https://github.com/hugelton/felucca-bsp
- Commit: `2a5af3dfc8bccb92e37d48a3e7e0bf9a5827e51f`
- Licence: MPL-2.0 (`bsp/LICENSE`, `bsp/NOTICE`; SPDX header and copyright lines in each file;
  `bsp/NOTICE`'s "LICENSES/Apache-2.0.txt" is upstream's path: here the Apache text is
  `docs/third-party/AC79-SDK-Apache-2.0.txt`). The files carry no "Incompatible With Secondary Licenses" notice, so
  they combine with our GPL-3.0-or-later code under MPL-2.0 section 3.3.
- Local changes: none. Every file is byte for byte as at the commit above; the firmware hooks
  around it (`platform/fm1/main.c`, `platform/fm1/lcd.c`, `platform/fm1/audio.c`,
  `platform/fm1/usbota.c`).

Files, at the same paths as upstream under `bsp/`:

| File | Used for |
| --- | --- |
| `LICENSE`, `NOTICE`, `LICENSING.md` | licence text, credits and the licensing notes |
| `chip/fm1_cc.h` | `FM1_INLINE` |
| `chip/fm1_cpu1_stop.h` | holding the second core at boot |
| `chip/fm1_flash.h` | flash driver (update session) |
| `chip/fm1_flash_access.h` | included by `fm1_cpu1_stop.h` and `fm1_flash.h` |
| `chip/fm1_flash_api.h`, `chip/fm1_flash_policy.h`, `chip/fm1_flash_wait.h` | included by `fm1_flash.h` |
| `chip/fm1_guard.h` | flash-write guard; vendored with the set, included by nothing |
| `chip/fm1_gpio.h` | GPIO port registers |
| `chip/fm1_irq.h` | interrupt and exception setup, the crash record, `fm1_fault` hook |
| `chip/fm1_isr.S` | ISR entry wrappers (`isr_timer5`, `isr_alnk0`) |
| `chip/fm1_usb.h` | USB device registers, included by `usb.c` |
| `chip/libc.c` | freestanding libc, included by `loader/loader.c` |
| `chip/ota.c` | the update session over USB |
| `chip/usb.c` | USB device stack |
| `chip/usb_host.c`, `chip/usb_host_desc.h` | USB host side, included by `usb.c` |
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
| `loader/crt0_ldr.S`, `loader/loader.c`, `loader/loader.ld` | the loader's start-up, main and link script |
| `loader/ldr_core.c`, `loader/ldr_screen.c` | the loader's update logic and screen, included by `loader.c` |

To update: clone upstream, check out the new commit, copy the same files over these, record the
new hash here, and rebuild and rerun the emu-gate tests.

## The loader

The CMake target `felucca_loader` (`platform/fm1/CMakeLists.txt`) builds `loader/` with the
commands of felucca-bsp's `tests/check_flash_target.py`, in `platform/fm1/bsp/` and with none of our
compiler flags:

    clang -target pi32v2 -c loader/crt0_ldr.S -o crt0_ldr.o
    clang -target pi32v2 -Os -ffunction-sections -fno-builtin -Wall -Werror -Wno-unused-function \
          -Ichip -Iboards/fm1 -c loader/loader.c -o loader.o
    ld --gc-sections -e _start -T loader/loader.ld crt0_ldr.o loader.o -o loader.elf
    objcopy -O binary -j .text loader.elf loader.bin

Output: `build/fm1/third_party/felucca-loader/loader.bin`. The build fails unless it is

- 15,060 bytes,
- SHA-256 `339e2334e0471dfc3569df5f7b9556363d5f45c0574ff7259921cfa8f9014c16`

(`cmake/check_loader.cmake`). It contains the marker `FELUCCA-LOADER-1`, which `fmzx-pack verify`
looks for. The loader presents itself to update hosts as `ota-FM-1_900`; that string is in the
vendored source and stays.
