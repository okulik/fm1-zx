<!-- SPDX-FileCopyrightText: 2026 FM-1 ZX contributors -->
<!-- SPDX-License-Identifier: GPL-3.0-or-later -->

# Building

## Host (tests)

The C core is built and tested from Rust. From the repository root:

    cd host && cargo test --workspace

This needs a C compiler, libclang (for bindgen) and Rust 1.88 or newer. The
Docker image below carries all of them.

## Firmware (pi32v2, in Docker)

Docker must be running. `tools/fm1-docker` builds the pinned image
(`fmzx-build:local`, `linux/amd64`) on first use and runs the given command in
it, with the repository mounted at the same path. Inside the image it just runs
the command.

    tools/fm1-docker cmake --preset fm1
    tools/fm1-docker cmake --build --preset fm1

Outputs are under `build/fm1`:

- `core/libzx_core.a`: the machine core and both vendored Z80 libraries.
- `bench/fm1/zx_bench.{elf,bin,dis}`: the bare-metal Z80 benchmark image, run
  later in the fm1-emu emulator. The result block is at `0x01C80000`.

Size of `zx_core` (Release, pi32v2): `.text` 13,478 bytes, plus 13,696 bytes of
read-only data (contention and flag tables). The benchmark image is 43,564 bytes.

The first image build can take several minutes (the toolchain and Rust are
downloaded; on Apple silicon the image runs under amd64 emulation).

## Toolchain pin

The JieLi toolchain comes from `https://pkgman.jieliapp.com/s/linux-toolchain`
and is pinned by SHA-256 in `tools/docker/toolchain.sha256`. The image build
fails if the download does not match. If it fails:

1. Download the archive yourself and check that the final URL is on a JieLi
   host and was served over HTTPS.
2. Check that its top-level directory is `jieli-linux-toolchains-<version>`.
3. Compute its SHA-256 and ask the project owner before changing the pin. Never
   replace the hash on your own.

Pinned now: `jieli-linux-toolchains-20260730.1`.
