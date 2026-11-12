<!--
SPDX-FileCopyrightText: 2026 FM-1 ZX contributors
SPDX-License-Identifier: GPL-3.0-or-later
-->

# Vendored code in `third_party/felucca-loader/`

## Felucca's update loader

- Upstream: https://github.com/hugelton/Felucca
- Commit: `a53ecf428c29c780108fd70a31b71a32a6b5c313` (tag `v1.6.1`, "Felucca 1.6.1")
- Licence: GPL-3.0-only (`LICENSE`, Felucca's own copy of the licence text; SPDX header and
  copyright line in each file: "Copyright (C) 2026 Leo Kuroshita (@kurogedelic), Hügelton
  Instruments"; `LICENSES/GPL-3.0-only.txt`).
- How it is combined: the loader is a separate program. The update package carries it as
  `ota.bin` (LZ4-wrapped by `fmzx-pack fwsc`); the FM-1's boot loader runs it from RAM to write
  an update. It is never linked into the FM-1 ZX image, and nothing of ours is linked into it.
- Local changes: none. Every file is byte for byte as at the commit above, at the same path as
  in Felucca's tree (`firmware/...`).

Files: `firmware/loader/{crt0_ldr.S,ldr_core.c,loader.c,loader.ld}`,
`firmware/src/{usb.c,ota.c,libc.c}` (included by `loader.c`) and
`firmware/hal/{fm1_cc.h,fm1_flash.h,fm1_sys.h,fm1_time.h,fm1_usb.h,fm1_xip.h}`, plus `LICENSE`.

## The build

The CMake target `felucca_loader` (`platform/fm1/CMakeLists.txt`) runs the commands of
Felucca's `tools/build.py` `build_loader()` at that commit, in this directory, with the JieLi
toolchain's tools and none of our compiler flags:

    clang -target pi32v2 -c firmware/loader/crt0_ldr.S -o crt0_ldr.o
    clang -target pi32v2 -Os -ffunction-sections -fno-builtin -Wall -Wno-unused-function \
          -Ifirmware/hal -Ifirmware/src -c firmware/loader/loader.c -o loader.o
    ld --gc-sections -e _start -T firmware/loader/loader.ld crt0_ldr.o loader.o -o loader.elf
    objcopy -O binary -j .text loader.elf loader.bin

Output: `build/fm1/third_party/felucca-loader/loader.bin`. The build fails unless it is

- 8,216 bytes,
- SHA-256 `2cde42b729d787a3e591ed4eaaffbbf33cebee7d58421d33bf6b6d27cc03f1aa`

(`cmake/check_loader.cmake`), which is Felucca 1.6.1's loader. It contains the marker
`FELUCCA-LOADER-1`, which `fmzx-pack verify` looks for. The loader presents itself to update
hosts as `ota-FM-1_900`, Felucca's identity; that string is in the vendored source and stays.

To update: clone upstream, check out the new commit, copy the same files over these, record the
new hash and the new loader size and SHA-256 here and in `platform/fm1/CMakeLists.txt`, rebuild,
and rerun the `fmzx-pack` and emu-gate tests.
