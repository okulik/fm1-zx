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
    tools/web-demo --packages DIR   # DIR's packages instead (tools/make-personal-pack's, below)

puts the release package (`build/fm1/fmzx.fwsc`, as `tools/make-packages` makes it) into
fm1-emu's own web page: the emulator compiled to WebAssembly, with our one-entry catalog
(`web/firmwares.json`), the key maps (`fmzx-pack keymaps`) and the licences beside it. It fetches
fm1-emu from `https://github.com/joakimeriksson/fm1-emu`, at the revision
`host/crates/emu-gate/Cargo.toml` pins, into `build/fm1-emu` (a shallow fetch, repeated only when
that revision changes), patches the fetched copy and builds it with upstream's `web/build.sh` into
`build/web`. Nothing from fm1-emu is committed. Every patch is exact text that must occur once, so a
changed upstream fails the build loudly (`web/patch-edit.sh`). In the emulator (`web/patch-emu.sh`): the
stock audio PLL, so the firmware plays sound, an LCD that honours COLMOD, so RGB444 shows, and the
running game: an export reads a word of RAM (`fm1_peek32`) and the worklet adds the firmware's view
and game index (`fmzx_stats`, words 3 and 4) to each frame message. The address of `fmzx_stats` comes
from the ELF the packages were made from (`build/fm1/platform/fm1/fmzx_app.elf`, read by
`web/elf-symbol.py`) and is written into the served `keymaps.json`; it is left out, with a warning,
when that ELF is missing or newer than the package, and the page then keeps the fixed keys. In the
page (`web/patch-page.sh`): the fresh package is never read from or saved to the browser's IndexedDB,
the clock is pinned at 240 MHz, input hooks let our keyboard layer (`web/fmzx.js`: Spectrum keys by
the running game's key map, docs/controls.md, "Computer keyboard"; under the synth, the computer
keys and a game's key map from `keymaps.json`, which follows the running game) claim keys, the boot message passes
the `fmzx_stats` address to the worklet, each frame goes to our layer, our labels (each key's
Spectrum key, placed by the drawing's own hit test) are drawn over the synth after each frame, the
draw loop takes the panel pointer before the memory buffer, and the SMK-37 catalog is not fetched;
`web/fmzx.css` hides the upstream extras (other devices, firmware list, MIDI, plugin and the like)
and lays the guide out as cards, two side by side on a wide screen. The server (`web/serve.py`)
listens on 127.0.0.1 only and sends every file with `Cache-Control: no-store`. `--check` runs the node
unit tests (`web/fmzx.test.mjs`) and the smoke test (`web/check.mjs`), which boots the built wasm at
240 MHz and checks that the package boots, the picker draws, the border-and-tone test sounds, the
running game is reported (view and game after PLAY), computer keys through `web/fmzx.js` reach the
firmware (Esc opens the menu, Enter resumes, Tab pauses and resumes), the colours match the preview
package, and every page patch is in place; it needs node.

The wasm build uses the active Rust toolchain (`rustc --print sysroot` in `host/`), which needs
the `wasm32-unknown-unknown` target. The script stops if it is missing and never installs
anything; add it with `rustup target add wasm32-unknown-unknown`.

Speed: the smoke test prints emulated seconds per wall second (about 1.7 in node on the picker
and the tone test, 2026-11-12). A Spectrum inside an emulated FM-1 inside wasm may not keep up in
a browser, and games cost more than the picker: the page is a preview, not a promise of real time.

## Your own games

    tools/make-personal-pack ~/games            # build/personal/fmzx.fwsc and fmzx-preview.fwsc
    tools/make-personal-pack build/personal/personal-games.toml   # from the games file, as written
    tools/make-personal-pack ~/games --try      # then the browser test bench with these packages

builds the two update packages with the games of a folder of yours instead of the built-in pack.
They stay on your computer: everything goes under `build/personal/` (`-o DIR` for another
folder), which git ignores, and the folder of games is only read. It needs the device builds
(`build/fm1` and `build/fm1-preview`: run `tools/make-packages` once first); `--offline` and
`--cache DIR` work as for `tools/make-packages`.

**The games file.** Given a folder, the command writes `build/personal/personal-games.toml`, one
entry per loadable `.sna`, `.z80`, `.tap` or `.tzx` file of the folder (not its subfolders), in the
syntax of `games/games.toml` with `file` an absolute path. The name is the file name without its
extension, its `(...)` and `[...]` tags and anything from a ` - ` on (`Night Raid II - Avenging
Angel - 48k.tzx` is "Night Raid II"); the id is made from the name; there is no `[game.keys]` table,
so the game gets the default key map. Edit it: a name (at most 28 characters), a `[game.keys]`
table (docs/controls.md), a `crop`. On the next run from the folder your edits stay byte for byte,
new files are added at the end, and a game whose file is gone stays, marked with a `# missing:`
line above it; nothing is reordered. To leave a game out, comment out its lines with `#`: a file
whose path is anywhere in the games file, comment lines included, is never added again. Deleting
an entry works too when you build from the file itself (a run from the folder would add it back):
given a games file, the command uses it as written (a relative `file` is taken from the file's
folder) and changes nothing in it.

**How each game goes in.** `.sna` and 48K `.z80` files go in as they are ("as is"). A `.tap`, or a
`.tzx` of standard blocks only, stays a tape ("tape") only when a trial load through the ROM's
tape trap on the host loads the whole tape with no one at the keys: on the FM-1 it then loads
instantly. Any other `.tzx` (a turbo loader, say) is loaded on the host by playing its tape signal
into the emulated Spectrum until the tape ends, and goes in as a snapshot of the loaded machine
("loaded"). So does a standard tape whose trial fails, and the table says why: "loaded (own
loader)" when the trap took none of its blocks, "loaded (the ROM trap took 4 of 5 blocks)" when it
took some, the rest being read by a loader of the game's own or wanted only later. A game that
loads later parts after you start playing (a multi-load game) is such a tape: it goes in as a
snapshot of its first part, and its later levels are not available on the FM-1. A standard tape
too big for the pack ("loaded (tape too big)") goes in as its snapshot as well. 128K files, a
tape whose program pages a 128K's memory (RAM banks 1-7 or the second screen), and files that
fail to load (the tape never finishes, or the program ends at the BASIC editor, an error report
or a reset: `host/crates/fmzx-pack/src/tapeload.rs`), are left out with the reason.

**The table.** The command prints each game, how it went in, its packed size and its file; the
space used and left in the app slot (about 472 KB beside the current image); and the games left
out, with why:

      name                         how                     packed  file
      tape                         tape                    1.2 KB  tape.tap
      tune                         as is                   0.4 KB  tune.sna
      turbo                        loaded                  1.1 KB  turbo.tzx
    used 3.2 of 472.2 KB, 469.1 KB free
    left out:
      big.z80: 128K

**The music.** The command also learns each game's music, so that the Sound choice FM: engines
(docs/controls.md) plays it as FM. It runs each game on the host for 3 minutes of emulated time,
as the FM-1 starts it: 40 s untouched, then ENTER, SPACE, 0, 1 and Kempston fire in turn, then
random play from a fixed seed (the same game gives the same run). A key script beside the game
file, `<name>.keys` for `<name>.tzx` (the music corpus' format below, its times from the start),
replaces that run. The learner reads the code of the routines it heard and of the speaker `OUT`s
it finds in memory (with the calls a sound loop makes into code near it, and a loop that stops on
a key press read as running with no key down), and keeps a routine only when it is sure of it (the
same notes heard as read, and method B playing them back); anything else stays on the beeper. After the table it prints one
line per game:

    tune: music learned (1 routine, heard, 1 channel)
    border: no music learned (OUTs that do not always flip the speaker at 801Ah)

`DIR/sound-report.txt` holds the same lines and, per routine, the speaker `OUT` it was read from,
the routine, each channel's half period as a formula, the fit and replay figures, why a
routine was left out, and each channel's instrument. The games are learned in parallel, a few
seconds each; the results are cached in `DIR/sound-cache/` (by the converted game's CRC, its key
script, its instrument overrides and the learner's version), which runs keep, so a re-run learns
only new or changed games.

**Instruments.** In FM, each voice plays a bass, lead or pad instrument by its role in the tune.
The FM-1 decides the roles live, by pitch; for the routines a game's music uses (learned ones,
and the known routines of FM: engines), the command also works them out over the whole 3-minute
run and stores them in the pack: per routine, its lowest channel the bass, its highest the lead,
a third the pad, a lone channel the lead (none for a routine heard too little). The report shows
each channel's role and where it came from:

    OUT at 8014h, heard in 8999 frames (276934 edges)
      ...
      learned
      roles: channel 0 lead (analysis)

To choose the roles yourself, give the game an `[game.instruments]` table in
`personal-games.toml`: a routine's `OUT` address as the report shows it (4 hex digits; any `OUT`
of the routine the report shows), then its channels' roles from channel 0 (`"bass"`, `"lead"`
or `"pad"`; a channel left out keeps the analysis' role). It wins over the analysis, and the
report says `override` for those channels:

    [[game]]
    id = "some-game"
    name = "Some game"
    file = "..."
    [game.instruments]
    "C694" = ["lead"]            # the routine whose OUT is at C694h: channel 0 lead
    "881F" = ["bass", "lead"]    # channels 0 and 1

Each of these fails the run with the game's name and the bad key, and no pack is written: a
key that is not 4 hex digits; the same address given twice (in any case); a list of fewer than
1 or more than 3 roles; a role name not in the list; an address that names no learned or known
routine of the game (the error lists the game's routine `OUT`s); an address that names a
routine another key names too (two of its `OUT`s); more roles than the routine has channels;
and an `[game.instruments]` table in a games file that is not a personal one.

A channel with no stored role (the report says `by the live rule` for it) is ranked by the live
rule only among the other voices without a stored role, so it may play the same instrument as a
channel with one.

A pack that does not fit fails: the command lists every game by packed size, largest first, and
writes no pack or packages (those of an earlier run are removed). Then comment out some entries in
`personal-games.toml` and run again. The output folder must be a personal pack's: one that holds
packages or a `games.bin` but no `personal-games.toml` (such as `build/fm1`) is refused, since a
run clears those files first. The packages are made and verified by
`tools/make-packages`' own steps, given the pack and the output folder (`FMZX_PACKAGES_PACK`,
`FMZX_PACKAGES_OUT`).

**Trying and installing.** `--try` serves the browser demo with `DIR/fmzx.fwsc` and the games'
key maps (`tools/web-demo --packages DIR`, which builds nothing of the firmware). Install
`build/personal/fmzx.fwsc` as `docs/install.md` describes for `build/fm1/fmzx.fwsc`, with the same
warnings: FM-1 ZX has run on a real FM-1 only once so far (`docs/hardware-test.md`, "Results").

The pieces: `fmzx-pack personal <folder | games.toml> -o DIR --app <app.bin> [--app <app.bin>]...`
(the games file, the converted games in `DIR/converted/` with a manifest of their own,
`DIR/games.bin` with the learned music, `DIR/keymaps.json`, `DIR/sound-report.txt` and the table;
`--app`, required, gives the images, whose size sets the room), then
those steps. `emu-gate`'s `tests/fw_personal.rs` runs the script on a folder of our own generated
test programs and boots the result in fm1-emu.

## Music corpus

`music-corpus` measures the FM music decoder (`core/sound`, the same C code as the firmware) over a
local folder of games. It is for games that must not enter the repository (Cave Runner, Unison, any
file of yours): it reads the folder you give it and writes only under the output directory, never
beside the games.

    cd host
    cargo run --release -p music --bin music-corpus -- <folder> [--seconds N] [--wav] [--no-learn] [-o <dir>]

Every `.tap`, `.tzx`, `.sna` and `.z80` in the folder is loaded and run for N seconds (default
40). A standard tape (a `.tap`, or a `.tzx` of standard-speed blocks) that loads through the ROM
trap, as the device will load it, autostarts, and its run starts at the autostart. Any other
`.tzx` (a turbo or custom loader, any 48K loader), and a standard tape the trap cannot load, is
loaded first through the tape signal (as `fmzx-pack` does for the personal pack): its run, and its
key script's times, start at the loaded snapshot, 2 s after the end of the tape. A game whose load
fails the success test, and a 128K file, report "not loadable" with the reason, and are skipped. The output goes to `build/music-corpus` (the repository's `build/`) unless
`-o` says otherwise: `report.txt`, `report.csv`, and with `--wav` `<name>_<ext>_beeper.wav`,
`<name>_<ext>_fm.wav` (the mixer's output in FM: sound) and `<name>_<ext>_fm_b.wav` (in FM: engines), both 3 frames
late to listen to the difference (`foo.tap` writes `foo_tap_beeper.wav`, so a `foo.tzx`
beside it keeps its own).

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
- `FM %`: of those frames, the share the mixer plays as FM in FM: sound (method A), as the firmware
  does: the decoder's confident decisions after the mixer's hysteresis (FM from the second
  confident decision in a row, the beeper from the first unconfident one); the rest plays the
  beeper.
- `B FM %`: the same in FM: engines (method B, the known music engines; FM from its first confident
  decision).
- `L FM %`: the same with the music the personal pack would learn for the game (its learned
  entries after the built-in ones; the same as `B FM %` when nothing was learned). The learner runs
  as `fmzx-pack personal` runs it, on the game as the pack converts it (3 minutes of emulated time,
  the key script if there is one), and its results are cached in `<dir>/sound-cache/` by the
  converted game's CRC, its key script and the learner's version, so a second run of the same
  folder does not learn again; `--no-learn` skips it. Its line per game follows the table, and
  `<dir>/sound-report.txt` holds the details, as the personal pack's does.
- `styles s/f/p %`: of those frames, the share decoded as time-slot music, speaker-flip music and
  short-pulse music; the remainder is sound with no music in it.
- truth, for games whose music engine is recognised (by the code of its sound loop in memory:
  Castaway, Colony 8, Cave Runner's title tune and in-game loop, Unison's melody): the engine's own
  notes are read from RAM and compared with what the decoder found: frame precision and recall,
  notes found of the true notes, and decoded notes that match no true note (`wrong`). After `B:`,
  the engines method B recognised and the same numbers for its notes (per voice, the last pitch it
  sounded in each frame it is confident about). After `learned:`, the frames with sound a learned
  entry claims and the same numbers with the learned entries. The CSV has the same columns.

The run is fast: the games run in parallel, and the owner's folder of 29 games takes about 10 s on an
Apple Silicon Mac when it learns, under 5 s from the cache. The Cave Runner and Unison entries of
method B were made from the owner's local copies of those games, which are not in the repository;
they are tested only locally, with this tool.

### Drops and clicks

When FM: engines "clicks here and there", the `drops` example finds where and why. It runs each game
as `music-corpus` does (key script, learned entries) through the mixer in FM: engines and lists:
- every audible drop to the beeper inside the music, with method B's decision and the frame's edges
  (count, OUT addresses, gaps) for each frame. A rejected frame of the engine's own OUT is the
  check's doing; another OUT among them is other code's sound;
- every frame's playback in `--show`: `FM` (method B's music), `fx` (the effects follower's FM, for
  a frame the mixer left on the beeper) or `bp` (the beeper). A `bp` frame the follower's gate kept
  says why: `two-voice` (many very short gaps, two voices mixed), `busy` (over 256 edges),
  `irregular` (no steady pitch for several frames) or `B-rejected` (method B's own frame that its
  check refused, in a run of 1-2 frames: a brief rejection inside a known tune stays on the
  beeper; a longer one is followed). The summary adds "follower for N of M frames with sound";
- every jump in the output (a frame's largest second difference over three times the typical FM
  frame's). A jump at an FM/beeper switch is a click; note attacks at big pitch steps show too. `fx` frames
  count as not FM here: they are not method B's music.

    tools/drops <game file>... [--seconds N] [--show A-B] [--wav FILE] [--cache DIR]

`tools/drops` runs the example from any directory (`cd host && cargo run --release -p music --example
drops -- …` does the same, with paths taken from `host/`).

`--show A-B` prints frames A to B in full, and `--wav` writes the mixer's output. Learned entries are
cached in `build/drops-cache`.

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
| 2026-11-17 | fe28ebc | first run on a real FM-1 (stock firmware 015, flash 0x856014), installed with M-Flasher 0.12.1 | boots to the picker |

Re-run with `cd host && FMZX_FM1_BUILD=$PWD/../build/fm1 cargo test --release -p emu-gate -- --ignored --nocapture`.

### Firmware budgets and the display frame-rate report

Both need the device build (see Firmware above) and run in fm1-emu:

    cd host
    FMZX_FM1_BUILD=$PWD/../build/fm1 cargo test --release -p emu-gate --test fw_budget --test fw_report -- --nocapture

`fw_budget` measures the CPU per frame in the two heaviest Sound and Screen pairs by default (FM: sound in
Crop, FM: engines in Border; a few minutes). `FMZX_BUDGET_FULL=1` runs the full matrix (about 12
minutes): before a release, or after a change to per-frame code that only one mode runs. In full it runs every game, and a generated fixture pack (`build/fm1/fixture-pack`, never committed: `noise.sna`, which stresses the decoder, and two programs with 4 unknown OUT sites per frame), in Crop with each of the three sound modes (Beeper, FM: sound and FM: engines) and in the other screen modes (Fit, Fit smooth and Border) with FM: engines (screen and sound work are independent and add up, so not every pair is run; the worst unmeasured pair, estimated from the tables below, is Border with ZX Sopwith under FM: sound at about 2.37M, or Border scroll at about 2.50M, both well under the 3,600,000 cap), and checks RAMTEXT (`.ram_text` <= 49,152 bytes), image + game pack
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

| game | Beeper max | Beeper avg | FM: sound max | FM: sound avg | FM: engines max | FM: engines avg |
| --- | --- | --- | --- | --- | --- | --- |
| scroll | 1,470,140 | 1,452,158 | 1,582,199 | 1,564,189 | 1,582,198 | 1,564,188 |
| border | 1,379,646 | 1,373,714 | 1,491,705 | 1,485,745 | 1,491,704 | 1,485,744 |
| castaway | 1,500,531 | 467,992 | 1,386,204 | 574,740 | 1,612,506 | 579,271 |
| sopwith | 1,507,011 | 1,232,085 | 1,618,987 | 1,344,116 | 1,569,667 | 1,356,075 |
| colony8 | 1,494,749 | 394,987 | 1,606,808 | 507,018 | 1,606,807 | 507,008 |
| dino | 1,550,940 | 889,847 | 1,662,916 | 1,001,878 | 1,662,567 | 1,001,542 |
| oneway | 693,673 | 616,982 | 805,732 | 729,013 | 805,648 | 729,020 |
| tape | 807,047 | 801,194 | 919,106 | 913,225 | 919,105 | 913,224 |
| tune | 1,160,417 | 950,105 | 1,307,041 | 1,096,735 | 1,272,477 | 1,062,138 |
| noise | 1,739,934 | 1,316,336 | 1,851,881 | 1,428,367 | 1,851,880 | 1,428,366 |
| sites (4 unknown OUT sites, code unchanged) | 201,725 | 195,901 | 313,784 | 307,932 | 313,783 | 307,931 |
| sites-var (4 unknown OUT sites, code changing every frame) | 807,569 | 801,771 | 919,628 | 913,803 | 919,627 | 913,802 |

(Crop, 2026-11-15.) The other screen modes, with FM: engines:

| game | Fit max | Fit avg | Fit smooth max | Fit smooth avg | Border max | Border avg |
| --- | --- | --- | --- | --- | --- | --- |
| scroll | 1,546,318 | 1,528,308 | 1,830,478 | 1,812,468 | 2,497,596 | 2,478,295 |
| border | 1,491,704 | 1,485,744 | 1,491,704 | 1,485,744 | 1,498,550 | 1,492,590 |
| castaway | 1,606,920 | 578,395 | 1,710,938 | 598,425 | 1,892,263 | 631,456 |
| sopwith | 1,559,202 | 1,353,933 | 1,860,252 | 1,364,422 | 2,324,954 | 1,418,852 |
| colony8 | 1,602,403 | 506,403 | 1,710,187 | 522,659 | 1,912,307 | 555,895 |
| dino | 1,650,602 | 993,698 | 1,769,645 | 1,065,477 | 2,143,331 | 1,280,515 |
| oneway | 799,630 | 725,383 | 963,710 | 825,984 | 1,233,919 | 991,872 |
| tape | 919,105 | 913,224 | 919,105 | 913,224 | 926,143 | 920,262 |
| tune | 1,272,477 | 1,062,138 | 1,272,477 | 1,062,138 | 1,279,323 | 1,068,984 |
| noise | 1,851,880 | 1,428,366 | 1,851,880 | 1,428,366 | 1,858,778 | 1,435,268 |
| sites | 313,783 | 307,931 | 313,783 | 307,931 | 320,821 | 314,969 |
| sites-var | 919,627 | 913,802 | 919,627 | 913,802 | 926,665 | 920,840 |

Fit, Fit smooth and Border compose from flash (`app/pixmodes.c`, XIP); Crop stays on its SRAM path.
A Border row costs about 18,000 instructions when its source lines are not flat, against about
4,500 for a Crop row; a run of 17 columns of one colour in every source line takes a short cut
(exact: every pixel of it is the column's blend). scroll, every paper line changing every frame,
is the worst Border frame. A screen of fine detail that changes everywhere every frame, or a border
that changes on every line every frame (loading stripes), would cost more in Border: at LCD speed 0
up to all 240 rows can be redrawn each frame (about 205 at speed 1); none of the games does. Going
over the cap does not slow the game: game frames run first whenever the audio buffer is low
(`platform/fm1/main.c`) and each LCD band is at most 8 rows, so the picture updates fewer rows a
second while the game's speed and the sound hold.

(CPU per frame, max and average, from `fw_budget`; the cap is 3,600,000. In Crop the noise
program is the worst frame in every sound mode (Fit was measured with FM: engines only, where it is also the worst); in Fit smooth it is ZX Sopwith, in Border scroll. Method B remembers the OUT addresses that matched no
entry and looks them up again only when the code around them changes; a lookup costs about 152,000
instructions. The reachable worst case is about 7 lookups of unknown, changing addresses in one
frame, about 2.9M estimated, under the cap.)

The LCD compose path uses about 3 KB of the 8 KB user stack, measured from the disassembly: `pix_compose_rows` has a frame of 2,524 bytes plus its callees, and is called only from the main loop.

Audio interrupt, max 1,574 instructions per block. RAMTEXT 48,396 of 49,152 bytes (2026-11-15). Image
+ pack 179,904 of 581,564 (0x8DFBC) bytes. POOL 272,076 bytes, 96,564 below the stack guard. The demos play the whole 20 s window: Castaway's player steps left and right on the ship's
platform, out of the crawler's reach, so the energy does not run out; ZX Sopwith flies and
crashes through its five planes; Escape from Colony 8's player walks and shoots in the first room; ZX Dino jumps and restarts after each crash; A World of One-Way's heroes keep sliding in the first level.
