<!-- SPDX-FileCopyrightText: 2026 FM-1 ZX contributors -->
<!-- SPDX-License-Identifier: GPL-3.0-or-later -->

# Building

## Host (tests)

The C core is built and tested from Rust. Fetch the third-party test data once
(it is checksummed by `tools/test-data.sha256` and lands in `build/test-data`;
the z80test and `.tap` tests fail without it), then, from the repository root:

    tools/fetch-test-data
    cd host && cargo test --workspace --exclude emu-gate

`emu-gate`'s firmware tests (`tests/fw_*.rs`) boot the device image in fm1-emu: build
the firmware first (below), then run the whole suite with

    cd host && FMZX_FM1_BUILD=$PWD/../build/fm1 cargo test --release --workspace

Without `FMZX_FM1_BUILD` they fail with the build commands rather than skip.
`FMZX_GOLDEN_UPDATE=1` rewrites their golden screens (`tests/golden/*.png`); on a
mismatch the screen is written to `build/fm1/<name>.actual.png`.

`[profile.test]` is optimised (opt-level 3), so the whole suite, including the
`z80full` conformance run (about a second), is fast in a plain `cargo test`;
`--release` gives the same. Lints:

    cd host && cargo clippy --workspace --all-targets

This needs a C compiler, libclang (for bindgen) and Rust 1.88 or newer (1.99 for `emu-gate`). The
Docker image below carries all of them.

To run a test under AddressSanitizer and UBSan (macOS clang):

    cd host && ZX_SANITIZE=1 cargo test -p zx-tests --test snapshot

`ZX_SANITIZE=1` makes `zx-sys/build.rs` compile the C core with
`-fsanitize=address,undefined`, copy clang's ASan runtime
(`libclang_rt.asan_osx_dynamic.dylib`, which also carries UBSan) into the build
output directory and link it into the test binaries. Switching the variable on
or off rebuilds the C core. Not supported on Linux hosts yet.

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
- `platform/fm1/fmzx_app.{elf,bin,dis,map}`: the firmware image (`fmzx_app.bin`
  is the bytes from `0x02000120`, the app slot's start). The game pack goes at the
  first 4 KiB boundary after it. `-DFMZX_LCD_DIVIDER=<0-15>` sets the LCD clock at
  boot (default 4); `-DFMZX_PRODUCT=FM-1_<3 digits>` the product identity the image
  carries (default `FM-1_948`, `fmzx_product`); `-DFMZX_LCD_RGB565=ON` makes RGB565
  the colour mode when no boot option is set (default OFF: RGB444).
- `third_party/felucca-loader/loader.bin`: Felucca 1.6.1's update loader (target
  `felucca_loader`), built from `third_party/felucca-loader/` with Felucca's own commands;
  the build fails unless it is 8,216 bytes with the pinned SHA-256
  (`third_party/felucca-loader/VENDORED.md`).

The preset `fm1-preview` builds the same firmware with `FMZX_LCD_RGB565=ON` into
`build/fm1-preview` (for the fm1-emu preview package):

    tools/fm1-docker cmake --preset fm1-preview
    tools/fm1-docker cmake --build --preset fm1-preview

## Update packages

    tools/make-packages

runs both device builds, then `fmzx-pack` on the host, and writes `build/fm1/fmzx.fwsc` (the
release package) and `build/fm1/fmzx-preview.fwsc` (the preview for fm1-emu's web page), each
checked with `fmzx-pack verify`. The first run fetches three pinned files of JieLi's AC79 SDK into
`build/ac79/` (`--offline` fetches nothing). Details, and installing: `docs/install.md`.
`emu-gate`'s `tests/fw_package.rs` boots both packages and needs them made first.

Size of `zx_core` (Release, pi32v2, summed over the sections of `libzx_core.a`):
`.text` 12,752 bytes plus 4,160 bytes of read-only data (the Z80 library's
extra-clock table is 4,096 of them); it has no `.data` or `.bss`. The benchmark
image (`zx_bench.bin`) is 45,320 bytes.

The first image build can take several minutes (the toolchain and Rust are
downloaded; on Apple silicon the image runs under amd64 emulation).

## Browser demo

    tools/web-demo                  # build into build/web, serve on http://127.0.0.1:8642
    tools/web-demo --check          # build, then run the headless smoke test (web/check.sh)
    tools/web-demo --no-serve       # build only
    tools/web-demo --skip-firmware  # no Docker: repack build/fm1's device build offline
    tools/web-demo --port N         # serve on another port

puts the release package (`build/fm1/fmzx.fwsc`, as `tools/make-packages` makes it) into
fm1-emu's own web page: the emulator compiled to WebAssembly, with our one-entry catalog
(`web/firmwares.json`), the key maps (`fmzx-pack keymaps`) and the licences beside it. It fetches
fm1-emu from `https://github.com/joakimeriksson/fm1-emu`, at the revision
`host/crates/emu-gate/Cargo.toml` pins, into `build/fm1-emu` (a shallow fetch, repeated only when
that revision changes), patches the fetched copy and builds it with upstream's `web/build.sh` into
`build/web`. Nothing from fm1-emu is committed. Every patch is exact text that must occur once, so a
changed upstream fails the build loudly (`web/patch-edit.sh`). In the emulator (`web/patch-emu.sh`): the
stock audio PLL, so the firmware plays sound, and an LCD that honours COLMOD, so RGB444 shows. In the
page (`web/patch-page.sh`): the fresh package is never read from or saved to the browser's IndexedDB,
the clock is pinned at 240 MHz, input hooks let our keyboard layer (`web/fmzx.js`, with the controls
panel that lists a game's key map from `keymaps.json`) claim keys, the draw loop takes the panel
pointer before the memory buffer, and the SMK-37 catalog is not fetched; `web/fmzx.css` hides the
upstream extras (other devices, firmware list, MIDI, plugin and the like). The server (`web/serve.py`)
listens on 127.0.0.1 only and sends every file with `Cache-Control: no-store`. `--check` runs the node
unit tests (`web/fmzx.test.mjs`) and the smoke test (`web/check.mjs`), which boots the built wasm at
240 MHz and checks that the package boots, the picker draws, the border-and-tone test sounds, the
colours match the preview package, and every page patch is in place; it needs node.

The wasm build uses the active Rust toolchain (`rustc --print sysroot` in `host/`), which needs
the `wasm32-unknown-unknown` target. The script stops if it is missing and never installs
anything; add it with `rustup target add wasm32-unknown-unknown`.

Speed: the smoke test prints emulated seconds per wall second (about 1.7 in node on the picker
and the tone test, 2026-11-12). A Spectrum inside an emulated FM-1 inside wasm may not keep up in
a browser, and games cost more than the picker: the page is a preview, not a promise of real time.

## Music corpus

`music-corpus` measures the FM music decoder (`core/sound`, the same C code as the firmware) over a
local folder of games. It is for games that must not enter the repository (Cave Runner, Unison, any
file of yours): it reads the folder you give it and writes only under the output directory, never
beside the games.

    cd host
    cargo run --release -p music --bin music-corpus -- <folder> [--seconds N] [--wav] [-o <dir>]

Every `.tap`, `.tzx`, `.sna` and `.z80` in the folder is loaded (tapes autostart; `.tzx` must be
standard-speed, any other block type reports "not loadable" and the game is skipped) and run for N
seconds (default 40). The output goes to `build/music-corpus` (the repository's `build/`) unless
`-o` says otherwise: `report.txt`, `report.csv`, and with `--wav` `<name>_beeper.wav` and
`<name>_fm.wav` (the mixer's FM-mode output, 3 frames late) to listen to the difference.

Key scripts: a file `<name>.keys` beside the game holds steps `ACTION@ms[+hold]` separated by commas
or line breaks (`#` starts a comment line), in the syntax of the `demo` strings of `games/games.toml`
but with Spectrum inputs: `key.ENTER@8000`, `key.SPACE@9000+200`, `kempston.fire@12000`. Hold is
100 ms if omitted; times count from the start of the run.

    # cave.keys
    key.ENTER@8000+500, key.SPACE@9000+200

Some games read the keyboard only now and then (Cave Runner only between title-tune notes), so a
press may need a longer hold.

What the report says, one row per game:

- `sound s`: seconds of the run with any speaker activity.
- `FM %`: of those frames, the share the decoder is confident enough about to play as FM (the rest
  plays the beeper).
- `styles s/f/p %`: of those frames, the share decoded as time-slot music, speaker-flip music and
  short-pulse music; the remainder is sound with no music in it.
- truth, for games whose music engine is recognised (Castaway and Colony 8 by the code of their
  sound loops, Cave Runner by its file name): the engine's own notes are read from RAM and compared
  with what the decoder found: frame precision and recall, notes found of the true notes, and
  decoded notes that match no true note (`wrong`). The CSV has the same columns.

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

The base image is pinned by digest in `tools/docker/Dockerfile` (`ubuntu:24.04@sha256:2198e4c6…`,
the multi-platform index). To move to a newer one, look up the digest with
`docker buildx imagetools inspect ubuntu:24.04`, rebuild the image and rerun the device build and
the test suite.

## Measured budgets

| Date | Commit | Measurement | Figure |
| --- | --- | --- | --- |
| 2026-11-12 | 6c1f0e6 | Z80 core on pi32v2 in fm1-emu (`emu-gate`, 150 frames of the bench; limit 30) | 13.81 instructions per T-state |
| 2026-11-12 | n/a | same, after the final-review fix wave | 13.90 instructions per T-state |

Re-run with `cd host && FMZX_FM1_BUILD=$PWD/../build/fm1 cargo test --release -p emu-gate -- --ignored --nocapture`.

### Firmware budgets and the display frame-rate report

Both need the device build (see Firmware above) and run in fm1-emu:

    cd host
    FMZX_FM1_BUILD=$PWD/../build/fm1 cargo test --release -p emu-gate --test fw_budget --test fw_report -- --nocapture

`fw_budget` (about 2.8 minutes) runs every game, and a noise program that stresses the decoder, in both sound modes (Beeper and FM: sound), and checks RAMTEXT (`.ram_text` <= 49,152 bytes), image + game pack
<= 0x8DFBC bytes, POOL below the user stack, the audio interrupt (`fm1_alnk0_irq` <= 8,000
instructions per block) and the CPU per Spectrum frame (`game_frame` plus every
`lcd_compose_band` call of that frame <= 3,600,000 instructions, 75% of a 20 ms frame at 240 MHz).
`fw_report` (about 5 to 6 minutes (318 s measured), the 27 runs in parallel) boots every game in `games/games.toml`
at LCD dividers 4, 2 and 1 in RGB444, plays the manifest's `demo` (`demo = "<key>@<ms>[+<hold ms>],..."`,
keys as in the key syntax plus `PLAY`/`HOME`, ms after the game started), measures 20 s from
second 16 on and prints the table below. It also checks that each game with a demo is being
played: rows must go out in at least 90% of the 1-second sub-windows (at least 8 rows each; gameplay
sends 45-90 rows a second, a title, menu or game-over screen sends none, though a blinking menu can still pass the share check, so the four new games' demos were also checked by eye on the frames, Task 8), and a second test
(`title_screens_are_not_gameplay`) shows that Castaway at its title menu fails it. It writes `build/fm1/fmzx-report.json` and asserts:
scroll 20-23 fps at divider 4 and >= 49 at divider 1; every game with a changing screen >= 45
fps at divider 1. fps is `50 * row_sends / pending_frames_sum` (how often a changed row reaches
the panel; 50 = every changed row goes out in the frame that changed it); a static screen
(`border`) sends nothing and has no rate.

Latest report (2026-11-14, 20 s windows; CPU at divider 1; the instruction counts are what the
emulator counts, one per instruction, interrupts excluded):

| game | divider | fps | LCD bytes per frame | worst row age (frames) |
| --- | --- | --- | --- | --- |
| scroll | 4 | 21.4 | 29,641 | 4 |
| scroll | 2 | 35.9 | 49,881 | 2 |
| scroll | 1 | 50.0 | 69,384 | 1 |
| border | 4, 2, 1 | static | 0 | 0 |
| castaway | 4, 2, 1 | 50.0 | 1,488 | 1 |
| sopwith | 4, 2, 1 | 50.0 | 2,090 | 1 |
| colony8 | 4, 2, 1 | 50.0 | 1,222 | 1 |
| dino | 4, 2, 1 | 50.0 | 16,122 | 1 |
| oneway | 4, 2, 1 | 50.0 | 7,165 | 1 |
| tape | 4, 2, 1 | static | 0 | 0 |
| tune | 4, 2, 1 | static | 0 | 0 |

| game | Beeper max | Beeper avg | FM: sound max | FM: sound avg |
| --- | --- | --- | --- | --- |
| scroll | 1,465,703 | 1,447,623 | 1,574,233 | 1,556,223 |
| border | 1,361,439 | 1,355,503 | 1,469,966 | 1,464,007 |
| castaway | 1,337,485 | 470,179 | 1,376,829 | 569,070 |
| sopwith | 1,502,114 | 1,209,485 | 1,610,561 | 1,332,740 |
| colony8 | 1,367,653 | 397,448 | 1,449,235 | 498,704 |
| dino | 1,541,056 | 883,313 | 1,649,503 | 992,850 |
| oneway | 690,832 | 614,159 | 799,362 | 722,808 |
| tape | 800,910 | 795,057 | 909,440 | 903,560 |
| tune | 1,146,602 | 938,842 | 1,288,812 | 1,078,636 |
| noise | 1,711,471 | 1,288,037 | 1,819,893 | 1,396,540 |

(CPU per frame, in both sound modes; the noise program is the worst case, the mixer alone at most
685,944 in Beeper and 794,366 in FM. The scroll to tune rows are from `fw_report`, noise from `fw_budget`.)

Audio interrupt, max 1,574 instructions per block. RAMTEXT 38,600 of 49,152 bytes. Image
+ pack 171,712 of 581,564 (0x8DFBC) bytes. POOL 270,804 bytes, 97,836 below the stack guard. The demos play the whole 20 s window: Castaway's player steps left and right on the ship's
platform, out of the crawler's reach, so the energy does not run out; ZX Sopwith flies and
crashes through its five planes; Escape from Colony 8's player walks and shoots in the first room; ZX Dino jumps and restarts after each crash; A World of One-Way's heroes keep sliding in the first level.
