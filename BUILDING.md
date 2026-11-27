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
  carries (default `FM-1_048`, `fmzx_product`); `-DFMZX_LCD_RGB565=ON` makes RGB565
  the colour mode when no boot option is set (default OFF: RGB444).
- `third_party/felucca-loader/loader.bin`: felucca-bsp's update loader (target
  `felucca_loader`), built from `platform/fm1/bsp/loader/` with felucca-bsp's own commands;
  the build fails unless it is 15,060 bytes with the pinned SHA-256
  (`cmake/check_loader.cmake`).

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
    tools/web-demo --packages DIR   # DIR's packages instead (a tools/pack-studio pack's, below)
    tools/publish-page              # the public page: build, commit to gh-pages, push
    tools/publish-page --no-push    # build and commit to gh-pages only

puts the release package (`build/fm1/fmzx.fwsc`, as `tools/make-packages` makes it) into
fm1-emu's own web page: the emulator compiled to WebAssembly, with our one-entry catalog
(`web/firmwares.json`), the key maps (`fmzx-pack keymaps`) and the licences beside it
(`licenses/`: ours, fm1-emu's, `LICENSING.md` and every component's own licence files). It fetches
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
and the tone test). A Spectrum inside an emulated FM-1 inside wasm may not keep up in
a browser, and games cost more than the picker: the page is a preview, not a promise of real time.

**The public page.** `tools/publish-page` publishes the page on GitHub Pages, at
<https://dev-gh.nisdom.com/fm1-zx/>: always the built-in pack (`games/games.toml`), never a personal
one. From a clean working tree it runs `tools/web-demo --no-serve`, lays the site out in
`build/pages` (the page under `fm1-zx/`, the domain's `CNAME`, a root page forwarding to the page)
and makes one commit on the `gh-pages` branch whose files are exactly that (signed when
`commit.gpgsign` is set), then pushes it. The repository serves Pages from `gh-pages`' root on its
own custom domain, `dev-gh.nisdom.com`, whose DNS name points at `okulik.github.io`.

## Your own games

    tools/pack-studio ~/games                    # the studio: a terminal screen, music by ear, then the packs
    tools/pack-studio ~/games --build            # the packs, without the screen
    tools/pack-studio ~/games --build --try      # then the browser test bench with pack 1 (--try 2: pack 2)
    tools/pack-studio ~/games --status           # the games, their music and choices, as text
    tools/pack-studio ~/games --generate ai,sid,ay [--game ID]...   # takes for the scores, without the screen
    tools/pack-studio ~/games --assign ID=2 --auto-fill --build      # games put in packs, then the packs
    tools/pack-studio build/studio/games/all/personal-games.toml --build   # from the games file, as written

builds update packages with the games of a folder of yours instead of the built-in pack: as many
packs as the games need, each a pair `pack-<n>/fmzx.fwsc` and `pack-<n>/fmzx-preview.fwsc`, with
each game's music chosen by ear. They stay on your computer: everything goes under the workspace
`build/studio/<folder name>/` (`-o W` for another folder; `W` below), which git ignores, and the
folder of games is only read. It runs both device builds (in Docker, a no-op when they are up
to date, but a few seconds) before each build, and at start only when their app images are not
there yet, so it works from a clean `build/`; `FMZX_SKIP_DEVICE_BUILD=1` skips them.
`--offline` and `--cache DIR` work as for `tools/make-packages`. It needs Python 3.11 or later and
nothing else of its own: `tools/pack-studio` runs it through [uv](https://docs.astral.sh/uv/),
`python3 tools/pack-studio` with any Python 3.11 or later.

It learns every game (`fmzx-pack personal <folder> -o W/all ... --plan`, which measures every
game even when they are too many for one pack) and lists each game with how it went in, its
packed size and its music. A first learning takes minutes (three minutes of play per game,
emulated, several games at a time): the games list is there from the start, each game with a
glyph for its state as `fmzx-pack personal` reports them on stderr (`·` waiting, `▸` converting or
learning, `✓` learned, `✗` left out, with why) and its own bar while it is in hand (below); the
games open when the learning is done. Before a workspace's first learning, the learned music of other personal packs
under `build/` (each `sound-cache/` beside a `personal-games.toml` or `converted/manifest.toml`) is
copied into `W/all/sound-cache/` where that lacks it: a cache file names its game by content (the
converted file's CRC, its key script and overrides, the learner's version), so those games are not
learned again (`reused 27 learned games from build/zx-tunes` in the log). Per game, mark one or more ways to make the music of its scores:
**FM**, the FM-1 playing the music learned from the game's code and memory (always there: the
original); **AI**, takes [arranged](#arranged-music) by `tools/arrange-music generate`; **SID**,
the game's C64 tune, and **AY**, its Spectrum 128 tune ([music from other
versions](#music-from-other-versions)). A game with no score plays FM from its code, or the
beeper, and has nothing to arrange; one where no music was found says `no music` on the list
(`m`, `M` and `--auto` skip it, the log says so) and on its screen whether it makes any sound of
its own: a render that is silent is not played (`silent: this game makes no sound in its first
40 s`, the entry marked silent; the same for any silent take). Open a game to hear each score's candidates (the beeper, FM,
each AI take with its figures, each C64 or Spectrum 128 take with its source) and choose one.
What plays is `music-preview`'s render of what the FM-1 plays or, when that is not built, the
renders `tools/arrange-music` wrote, through `afplay` (macOS), `ffplay`, `paplay` or `aplay`.

The judge cuts down how much there is to hear. `m` on a game (`M` for every game) finds and
converts its C64 and Spectrum 128 versions (SID and AY, no AI model), then runs
[`tools/arrange-music judge --apply`](#arranged-music), which weighs every take of each score from
its notes and chooses the take where it is sure and you have not chosen. Each take on the game
screen then shows its place in the judge's order (`#1`, the judge's pick), its closeness to the
original (`close 97%`: green from 50%, the same tune; yellow from 20%, part of it; dim under, another
tune), its quality (`qual 62%`; polyphony, harmony and rhythm too on a wide screen) and its faults in
red (`refused by the renderer: ...`, `plays only a fragment: ...`); each score says whether it is
chosen by you or by the judge, or is to hear and why (`judge: chip-1 (low: close call)`). On the
games list a game shows `?2` (yellow) for two scores to hear, `✓` when every score with takes is
chosen, `m` when there are takes the judge has not chosen from yet; the header counts the games to
hear and `n` jumps to the next. The choice is marked `*` when you made it, `j` when the judge did;
the judge never changes yours.

Which games go in which pack is yours to choose before a build: `1` to `9` put the game in that
pack, `0` (or `x`) in none, `f` puts every game in no pack in the first pack with room for it (in
name order). The games list shows each game's pack (`P1`, a colour per pack, `—` for none) and its
packed size with its chosen music; a panel above it shows each pack's fill, `P1 312.4/416.7 KB`
and a bar against what a pack holds (the room less 2% and the pack's own overhead, as the build
splits), and the games in no pack. A game that would take its pack over is refused until you press
the same key again; that pack then shows red and no build starts. `p` lists the packs and their
games, where the same keys move a game. Sizes are the last learning's: after choosing a take the
panel says so, and `r` measures again.

Keys on the games screen: arrows or `j`/`k` move, Enter opens the game, `m`/`M` find and judge
(above), `n` the next game to hear, `1`-`9`/`0` as above, `f` fills, `p` the packs, `a`/`s`/`y`
mark AI/SID/AY for it, `A`/`S`/`Y` for every game, `g` generates what is marked, `J` lists the
jobs, `r` learns again, `b` builds, `l` shows the log, `?` the full help, `q` quits. On a game:
Enter or space plays the selected candidate and stops it when pressed again, Esc stops, `c`
chooses it (at once, as your choice; the chosen one is marked `*`), `m` and `a`/`s`/`y`/`g` as
before, Left, `h` or Backspace goes back. Long steps run one at a time in the background. Nothing
a tool prints shows on the screen: every line goes to `W/studio.log` (below). What shows is
progress, as bars of squares (`■` done, `□` to go; `#` and `.` without UTF-8): on the games list,
each game with work in hand or waiting has a bar in place of its columns, with what is in hand
(`learning`, `SID takes`, `judging`, `pack 2`, `waiting`), its share of all the work the studio
knows of for it in the current activity (the jobs since the queue was last empty): each step a
share, a step that reports how far it is (the learning's load, conversion, learning and check of
each game, a SID or AY job's candidates, an AI job's takes and passes, the judge's takes checked
and scores judged) by that measure, any other done or not; a game done shows its columns again, one
whose step failed a red mark (`✗ SID takes failed (l: the log)`). The bottom line of every screen
holds the bar of the whole activity, its games and its steps without a game (downloads, tool and
device builds): `learning games ■■■■■■■■■■■□□□□□□□□□□□□□□□ 12 of 45 games  ~4 min  · job 1 of 4`;
a step with nothing to count adds a spinner and the time it has run (`device build fm1 ⠹ 0:12
■■■□□□ ...`), and a lone such step shows only those. It is yellow while the activity runs, green
when it is done, red when a step failed (on a narrow terminal the queue and then the figures go
first). Messages take the key line above it for 3 s. `J`, on every screen, lists every job of the session, each with its state and a bar: the one
running (an AI job's take, pass and time left, a SID or AY job's candidates checked, a download's
MB), those waiting in the order they run, and those finished with their result, read from
`tools/arrange-music`'s summary line (`3 takes, best closeness 73% (C64: …)`, `no C64 tune
found by that name`, `0 takes, 4 songs skipped (48K beeper or CPC tunes)`, `12 scores: 3 chosen by the
judge (2 in this run), 4 by you, 5 to hear`); a failed one its reason in a line and where the rest is
(`exit 1: no music-preview here (the rest in studio.log: l)`); Enter shows a job's lines from the
log, `o` opens its game to listen, `x` takes a waiting job off the queue or, after `y`,
stops the running one. `g` says how many jobs it queued. The screen is in colour (`NO_COLOR` set: bold and
reverse only). It opens on a start screen for a moment, a game's loading screen in ZX pixels; a key ends it
(`FMZX_SPLASH=0`: no start screen).

`W/studio.log` reads on its own and with `tail -f`: a dated line per session (its command line),
then a line per event, each `HH:MM:SS <tag> <text>`, the tag the job's number, kind and game
(`#4 sid alpha`) or `studio`: each job queued, its start with its command line, every line it
prints (after `| `), its end with its state, exit code and time, and the studio's own events (the
keys that start work, the choices made, the messages). Past 4 MB it moves to `studio.log.1` (the
one there goes) and another starts. `l` shows it in a pager.

Without the screen: `--assign ID=N` (again for more) puts a game in pack N (0: in none; a pack it
would take over refuses it) and `--auto-fill` fills the packs as `f` does; `--generate` marks the
methods named for every game with scores (or for the games `--game` names, by the ids `--status`
shows) and runs the jobs `g` runs; `--auto` (with `--game`, or for every game with scores) does
what `m` does: the SID and AY jobs, then the judge, which chooses where it is sure and prints the
scores settled and those to hear (the rest is a choice by ear: the screen, or `tools/arrange-music
pick W/all`). `--build` builds, `--status` prints (the scores settled, by you or the judge, and to
hear; each score's state and the judge's verdict, each take's figures; each game's pack and the
packs' fill among the rest), `--try [N]` serves pack N (1 unless given) in the browser test bench;
given together they run in that order (assign, auto-fill, generate, auto, build, status, try), and
a step that fails stops the command (exit 1) with its reason in a line and the log's path. On a
terminal they draw the same bars in place while they run, a line per game in hand (at most four,
those running first) and the whole's bar under them, then clear them for the result: `--generate`
each takes job's result, `--auto` the scores settled and to hear (and the games skipped, with no
score), `--build` the summary, each with the log's path last. Printed elsewhere (a pipe, a file),
only those result lines: no bars, and never a tool's own output (that is in the log).

The workspace: `all/` the learning (`personal-games.toml`, which keeps your edits, the scores,
their takes and `choices.toml`, the cache), `studio.toml` the methods marked and each game's pack,
`studio.log` every step's output and the studio's events, `previews/` the renders (each with its
level measured beside it, `.level`). A build (`b`, or `--build`) learns again
(choices change the sizes) and builds exactly the packs you filled, each in `pack-<n>/` under its
own number (a game in no pack is not built; a pack over its room stops the build before it
starts). With no game in a pack, it splits the games in name order into packs filled to the room
less 2% instead, and a pack of that split that does not fit gives its last game to the next one.
Each pack is built from its own games file, those games' entries from `all/personal-games.toml` as
written there: `fmzx-pack personal` on that file (the pack and its table), then
`tools/make-packages`' steps for its packages and their verify (`FMZX_PACKAGES_PACK`,
`FMZX_PACKAGES_OUT`). The summary lists each pack's games, its room (`used 312.4 of 428.2 KB, 115.8 KB
free`, from `fmzx-pack`) and its package, and the games in no pack. The first SID job downloads the High Voltage SID Collection (about 85 MB; its address
from hvsc.c64.org) into `build/hvsc/<version>/`; bsdtar (macOS's `tar`) or 7-Zip lists its members
first (one whose path would land outside, a link or a device refuses the archive) and unpacks it.
The first AY job downloads the chris-y/AY collection on GitHub (Spectrum AY music; about 30 MB, its
newest commit from api.github.com) and unpacks only its `music/ay` folder into
`build/ay/<commit>/`, every member's path checked first (one that would land outside, or a link,
refuses the archive). The tunes belong to their authors: keep the collections and the packs made
with them to yourself. `--sid-dir PATH` and `--ay-dir PATH` use collections you have instead, and
`--offline` downloads nothing. `tools/pack-studio selftest` checks the tool with stand-ins for
every tool it runs, and the docstring at the top of `tools/pack-studio` is the full reference.

In what follows, `DIR` is a folder `fmzx-pack personal` writes: `W/all` for the learning,
`W/pack-<n>` for a pack.

The picker lists your games by name, A to Z (case does not count), whatever their order in the
games file.

**The games file.** Given a folder, the studio writes `W/all/personal-games.toml`, one
entry per loadable `.sna`, `.z80`, `.tap` or `.tzx` file of the folder (not its subfolders), in the
syntax of `games/games.toml` with `file` an absolute path. The name is the file name without its
extension, its `(...)` and `[...]` tags and anything from a ` - ` on (`Night Raid II - The
Return - 48k.tzx` is "Night Raid II"); the id is made from the name; there is no `[game.keys]` table,
so the game gets the default key map. Edit it: a name (at most 28 characters), a `[game.keys]`
table (docs/controls.md), a `crop`. On the next run from the folder your edits stay byte for byte,
new files are added at the end, and a game whose file is gone stays, marked with a `# missing:`
line above it; nothing is reordered. To leave a game out, comment out its lines with `#`: a file
whose path is anywhere in the games file, comment lines included, is never added again. Deleting
an entry works too when you build from the file itself (a run from the folder would add it back):
given a games file, the studio uses it as written (a relative `file` is taken from the file's
folder), changes nothing in it and takes the packs' entries from it. A workspace's own
`all/personal-games.toml` works in that workspace; another games file in
`build/studio/<its name without .toml>/` unless `-o` says otherwise.

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

**The table.** Each pack's build prints each game, how it went in, its packed size and its file; the
space used and left in the app slot (about 472 KB beside the current image); and the games left
out, with why:

      name                         how                     packed  file
      tape                         tape                    1.2 KB  tape.tap
      tune                         as is                   0.4 KB  tune.sna
      turbo                        loaded                  1.1 KB  turbo.tzx
    used 3.2 of 472.2 KB, 469.1 KB free
    left out:
      big.z80: 128K

**The music.** The learning also works out each game's music, so that the Sound choice FM: engines
(docs/controls.md) plays it as FM. It runs each game on the host for 3 minutes of emulated time,
as the FM-1 starts it: 40 s untouched, then ENTER, SPACE, 0, 1 and Kempston fire in turn, then
random play from a fixed seed (the same game gives the same run). Music demos often start on one
key press and stop or skip on the next, so the learner also tries nothing pressed and each start
key alone (held 5 frames at frame 500, then nothing), counts the frames of each run with at least
3 speaker flips, and uses the generic run unless another hears 1.5 times as many and 500 frames
more (then the one with the most); the details line `run:` names it. A key script beside the game
file, `<name>.keys` for `<name>.tzx` (the music corpus' format below, its times from the start),
replaces that run. The learner reads the code of the routines it heard and of the speaker `OUT`s
it finds in memory (with the calls a sound loop makes into code near it, and a loop that stops on
a key press read as running with no key down), and keeps a routine only when it is sure of it (the
same notes heard as read, and method B playing them back). A heard routine whose code it cannot
read or does not trust, such as one that mixes several channels into the one speaker bit, it
observes instead, if the routine sounded in at least 100 frames: it looks for the values the
routine's loop reads (bytes, words, registers) that hold each channel's pitch, by testing the
pitches they predict against the spectrum of the speaker's sound, and keeps the routine when
method B, replaying the run with it, plays at least half of the routine's frames as music and no
known routine claims any of them. On the FM-1 an observed routine's channels play from the values
it reads, without the check against the beeper's edges (its edges hold all its channels at
once).

Music that no routine plays (none learned, none known) is recorded instead, as a score, when its
`OUT` sounded in at least 100 frames: each stretch from the first speaker edge of that `OUT` after
at least 25 frames without one, for as long as the keys stay as they were (a key press may change
the music) and the `OUT` is not silent for 100 frames, to the end of the 3-minute run at most, so
a run that plays several tunes with silence between gives a score each; a stretch under 100 frames
(2 s) is not recorded. Every heard routine is looked at this way, learned, known or neither, and
each stretch is judged by what plays it: replayed with the game's learned entries and the known
routines, a stretch they claim at most a tenth of the sounding frames of is music nothing plays,
and gets a score. So a routine inside a known routine (the ROM's BEEP routine at 03B5h, say) gets
a score for music the known entry does not take (BEEP notes under a frame long). A stretch they
claim more of is music a routine already plays live: it gets a *shadow score*, recorded and
verified as a score is but not packed, so that it can be [arranged](#arranged-music); a take
chosen for it puts it in the pack, where it then plays in place of the live version. When the run
gives a score or a shadow score, the learner also listens to the game once per key, 1-9, 0, ENTER
and SPACE (but the key of its own run), each held 5 frames at frame 500 and then nothing, as a menu
of tunes plays another behind each key, and takes those runs' stretches too, from the same `OUT`s
and from any other `OUT` that sounded there in 100 frames and lies in no routine already known. Two stretches whose speaker edges agree frame by frame over their first 100
frames (2 s) are the same tune: the longer is kept. Each extra run costs a replay of the game, and
each new tune a transcription and the replays that verify it, so a menu of tunes takes several
times as long as a game with one. Most tunes loop: when a stretch's speaker edges repeat (from
some point on, every edge comes again one loop later, to the stretch's end, with a whole repeat
heard and the loop at least a second long), the score is an intro and one pass of the loop, the
shortest that repeats, and the FM-1 goes round the loop for as long as the beeper keeps to it;
music that does not repeat is all intro. A score's notes are read from the speaker's sound alone
(up to 3 voices, by their spectrum; a held tone, under 8 notes or 3 pitches, is not music), and
the pack stores them compactly: each pitch once in a table, each note a byte or two and a short
time, and the speaker edges of each frame-long stretch of the intro and of the loop as small
changes from the stretch before. Method B, replaying the score's own run with the game's whole
list, must play the score over the whole stretch (round its loop as often as the run repeats
it) while every score kept before still plays in its run; a score whose loop it does not follow
is recorded with no loop instead, and one with no loop it stops following is cut to where it
held, if 100 frames remain. At most 8 scores a game, the longest first, within 128 KB in all;
shadow scores are not counted (at most 8 of them as well, the longest first, none playing with a
score in its run), and a chosen take of one counts when it goes into the pack. On
the FM-1, FM: engines starts a score when its `OUT` makes that first edge after the same silence
with the same code around it (several scores of one `OUT` start together, and the one whose
recorded edges the beeper follows plays), and plays it in step with the beeper, comparing the
beeper's edges with the recorded ones as it goes: when the game does something else (a key
pressed during the tune, another tune), the score stops within a few frames and the beeper
plays. Its voices take their instruments by pitch, as a routine's channels do. A score yields
the frames a learned or known routine claims; the chosen take of a shadow score keeps them (the
routine's own notes are not used there). A recorded score or a shadow score can also play with a
bass, chords and drums written around its melody, as a take you choose
([arranged music](#arranged-music)).

Anything else stays on the beeper. After the table it prints one line per game:

    tune: music learned (1 routine, heard, 1 channel; 1 shadow score)
    mixer: music learned (1 routine, heard, observed, 2 channels; 1 shadow score)
    demo: music recorded (1 score, 3 voices)
    border: no music learned (OUTs that do not always flip the speaker at 801Ah)

`DIR/sound-report.txt` holds the same lines and, per routine, the speaker `OUT` it was read from,
the routine, each channel's half period as a formula, the fit and replay figures, why a
routine was left out, and each channel's instrument. A routine observed shows why its code was
not used, the routine observed, each channel's value (`word (9004h)`, `DE`, `IYh`), how the value
gives the pitch (`counter x1`: that many passes of the loop; `16-bit accumulator`: a step added
each pass) and its score (the share of the music in which the predicted pitch was heard), its
replay, and `learned by observation: 2 channels` or `observation:` and why not. A routine that
could have a score ends with its scores, `recorded score: 169.5 s from frame 506 (intro 4.2 s,
loop 41.6 s), 3 voices, 1074 notes, 4.1 KB (score 8951-1a2b3c4d)` (its stretch's length, the
frame of its first edge, its intro and loop when it loops, its voices, the notes stored, its size
and its key, which names its folder `DIR/arrange/<game>/<key>/`, see
[arranged music](#arranged-music)), the
run named when it is an extra one (`recorded score (key 3 once): ...`), its shadow scores (`shadow
score: 39.9 s from frame 1 (intro 0.0 s, loop 8.0 s), 1 voice, 16 notes, 0.4 KB (score
8014-cc5bc889; packed only with a chosen take)`), and its stretches left out, `no score:` or `no
shadow score:` and why not; an `OUT` heard only in the extra runs has a block of its own (`OUT at
9A12h, heard only in other runs`) unless it was found in the code; the game's line counts the
scores and the shadow scores (`music recorded (5 scores, 3 voices)`, `music learned (1 routine,
heard, 2 channels; 1 score; 2 shadow scores)`, `no music learned (built-in entry at 03B5h; 1
shadow score)`). The games are learned in parallel, a few seconds each (a menu of tunes longer); the results are cached in `DIR/sound-cache/` (by the
converted game's CRC, its key script, its instrument overrides and the learner's version), which
runs keep, so a re-run learns only new or changed games.

**Instruments.** In FM, each voice plays a bass, lead or pad instrument by its role in the tune.
The FM-1 decides the roles live, by pitch; for the routines a game's music uses (learned ones,
and the known routines of FM: engines), the learning also works them out over the whole 3-minute
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

A pack that does not fit makes `fmzx-pack personal` fail: it lists every game by packed size,
largest first, and writes no pack or packages (those of an earlier run are removed); the studio
then gives the pack's last game to the next pack and builds it again, and a game too big for a
pack of its own goes in no pack (the summary names it). To leave games out, comment out their
entries in `personal-games.toml`. The output folder must be a personal pack's: one that holds
packages or a `games.bin` but no `personal-games.toml` (such as `build/fm1`) is refused, since a
run clears those files first.

**Trying and installing.** `--try` serves the browser demo with `W/pack-1/fmzx.fwsc` (`--try N`:
pack N's) and its games' key maps (`tools/web-demo --packages W/pack-<n>`, which builds nothing of
the firmware). Install a pack's `fmzx.fwsc` as `docs/install.md` describes for
`build/fm1/fmzx.fwsc`.

The pieces: `fmzx-pack personal <folder | games.toml> -o DIR --app <app.bin> [--app <app.bin>]...`
(the games file, the converted games in `DIR/converted/` with a manifest of their own,
`DIR/games.bin` with the learned music and the chosen takes, `DIR/keymaps.json`,
`DIR/sound-report.txt`, the scores' `DIR/arrange/<game>/<key>/score.toml` and the table;
`--app`, required, gives the images, whose size sets the room), then
`tools/make-packages`' steps. `emu-gate`'s `tests/fw_personal.rs` runs
`tools/pack-studio <folder> --build` on a folder of our own generated test programs and boots the
pack in fm1-emu.

With `--plan` the command goes as far as measuring the pack: the games converted, their music
learned and the chosen takes applied, `sound-report.txt` and the scores written, but no
`games.bin` or `keymaps.json` (those of an earlier run are removed), and a pack over the room is
no error. It writes `DIR/plan.json` instead: `room` and `used` (the room and the pack's size, in
bytes), and `games`, the packed games in pack order and then those left out, each with its `id`,
`name`, `file`, `how`, `packed` (its bytes in the pack with its music; `null` when left out),
`reason` (why left out, else `null`), `music` (its line in the table) and `scores`: each score
and shadow score with its `key`, `shadow`, `seconds` (intro and one loop pass) and `status`
(`original`, `take 2 (<generator>)`, `chip-1 (<source>)`, `ay-1 (<source>)` or `refused: <why>`).
Any run removes the
`plan.json` of an earlier one. Packs built from games files into folders whose `sound-cache` and
`arrange` are symlinks to one shared pair use that pair: each run writes its own games' scores and
leaves the others' as they are.

### Arranged music

A recorded score can play with a band: a bass, a pad (chords, one note at a time) and a drum,
written around the score's melody by a music model, as several takes that you listen to and choose
from by ear, or keep the original. Music a routine plays live (learned or built in) can be
arranged too, through its shadow score ([the music](#your-own-games)): its takes are made and
chosen like any score's, and with none chosen the routine plays it live as before; a chosen take
goes into the pack as a score that plays in place of the routine's notes.

    tools/pack-studio ~/games --generate ai       # takes for every game's scores (a and g on the screen)
    tools/arrange-music judge build/studio/games/all --apply   # choose where the judge is sure
    tools/arrange-music pick build/studio/games/all   # listen and choose the rest on a page (or c on the screen)
    tools/pack-studio ~/games --build             # the packs, with the chosen takes

`tools/arrange-music` is the engine `tools/pack-studio` runs for AI, SID and AY takes, on its
workspace's `W/all` (`DIR` below); its commands work on their own too, for experiments.

**What it needs.** [uv](https://docs.astral.sh/uv/) (`tools/pack-studio` checks for it before an
AI, SID or AY job): `tools/arrange-music` is a uv script, and its first run installs its Python packages
(about 1 GB: PyTorch, Hugging Face Transformers and the model's code, pinned in the script) into
uv's cache, with a Python 3.11 or 3.12 if you have none. The first run that generates with the
model downloads it, the Anticipatory Music Transformer (`stanford-crfm/music-medium-800k`, about
1.4 GB), to the Hugging Face cache. FluidSynth renders the takes to listen to (`brew install
fluid-synth`; the tool takes the first soundfont in FluidSynth's folders, `--soundfont PATH`
another); without it there is nothing to hear on the page, only MIDI files. ffmpeg, when it is
installed, matches the renders' loudness. None of this is part of the repository or the packages
([LICENSING.md](LICENSING.md#the-arranging-tool)).

**The scores.** Every `fmzx-pack personal` run writes each recorded score's notes, and each
shadow score's, to `DIR/arrange/<game>/<key>/score.toml` (`<game>` the game's id; `<key>` the
score's `OUT` address in 4 hex digits and a CRC-32 of its recorded speaker edges, such as
`8951-1a2b3c4d`, the same for as long as the learner records the same music; a shadow score's
says `shadow = true`), only when the text changes, and leaves the rest of the folder alone. The
studio's AI jobs run `tools/arrange-music generate W/all --game ID`; a take goes into the packs
once it is chosen, at the next build.

**The commands.**

- `tools/arrange-music generate DIR [--takes N] [--game ID]... [--force] [--jobs J] [--model NAME]
  [--soundfont PATH] [--seed S] [--offline]` writes N takes (default 4) for each score that has
  none, every score with `--force`, only the scores of the games `--game` names. `--model amt` (the
  default) runs the model in three passes, drums, then bass, then chords, each hearing the original
  and the passes before it; `--model rules` is a quick generator without a model (a bass on the
  melody's chord roots, a triad pad and a plain drum pattern). The same seed (`--seed`, default 1)
  gives the same takes again on the same machine. `--jobs J` arranges J scores at once, each in a
  process of its own that loads the model. A score has its takes when `takes.toml` beside it lists
  takes 1 to N for its `score.toml` as it is; a changed `score.toml` gets new takes in place of the
  old ones (choose again), and an interrupted run resumes at the missing takes.
- `tools/arrange-music pick DIR [--port P]` serves a page at `http://127.0.0.1:8765/` (or a free
  port; only this computer can open it): per score, the judge's verdict, the original and each
  take, the take as the FM-1 plays it and the whole arrangement the model wrote, with its figures
  (below) and the judge's (closeness, quality, its place in the judge's order, its faults); Save
  writes `DIR/arrange/choices.toml`. `--set GAME/KEY=N` chooses take N without the page (0: the
  original), and `--list` prints each score, its takes, what made them, their figures and the
  choice (`(the judge)` after one the judge made); other choices in the file are kept, and a choice
  made here is yours. Before the page or the list it judges the scores (quick once they are
  judged) and warns of each faulty take (`warning: <game>/<key> chip-2: refused by the renderer:
  ...`). Both mark a shadow score `(replaces the live version when chosen)`; its original is the
  routine playing live.
- `tools/arrange-music judge DIR [--game ID]... [--apply]` weighs each score's takes and its
  original from their notes and prints a table per score: per take its closeness to the original's
  tune (0 to 1, key, octave and tempo not counting, calibrated against 30 takes of other games),
  its quality over the seconds the FM-1 plays of it (polyphony, harmony, rhythm, times a fault
  multiplier), its faults in words (refused by the renderer, mostly silent, a silent gap, stuck
  notes, extreme registers, plays only a fragment) and a score, quality x (0.3 + 0.7 x closeness),
  closeness raised for the game's own period music; the highest is the judge's pick (the original
  can win), with a confidence (high, medium, low) and the reasons. `--apply` writes the pick to
  `choices.toml` for each score it is sure of (high) that you have not chosen, marked in a
  `[judged]` table so it is told apart from yours and judged again later (changed, or taken back
  when no longer sure); yours are never changed. The renderer check runs `music-preview`
  (`FMZX_MUSIC_PREVIEW`, else `host/target/release/music-preview`); without it the check is skipped
  and the judge says so. `generate`, `chip` and `ay` end by judging the scores they worked on
  (without `--apply`). The figures go to `judge.toml` beside each `score.toml`, reused while the
  score and its takes stay the same.
- `tools/arrange-music selftest` checks `generate` (with `rules`), the files it writes, `chip`,
  `ay`, `pick --set`, `--list`, the page's save and the judge (its measures on made-up tunes, the
  renderer's refusals through a stand-in, the cache, `--apply`) in a temporary folder, with no model
  (and, once uv has installed the packages, no network).

The renders play General MIDI instruments through FluidSynth: they show the parts, not the FM-1's
sound. To hear a take as the FM-1 plays it, render it with `music-preview` (below), or choose it,
build the packages and try them (`--try`, or on the FM-1).

**Hearing a take as the FM-1 plays it.** `music-preview` renders a game of a learning workspace
(the output folder of `fmzx-pack personal`, such as the studio's `W/all`, `DIR` here) to a 16-bit
WAV through the firmware's sound code, as `music-corpus` renders `_fm_l.wav`:

    cd host && cargo build --release -p music --bin music-preview
    target/release/music-preview DIR --game ID [--score KEY [--take T]] [--seconds N] [--beeper] -o OUT.wav

It runs the game as the learner ran it and plays FM: engines with the game's music as the pack
would hold it, the takes `choices.toml` chooses in, each checked as a run checks it. Without
`--score`: the learner's run from its start, 40 s. With `--score KEY`: the run that recorded the
score, from where it starts (what the run played before it would sound the same in every take), for the score (its intro and one pass of its loop) and 2 s
(at most 40 s), with the score playing take T in place of its choice: `0` or `original` (a shadow
score: the routine, live), `<n>` (`take-<n>`), `chip-<n>` or `ay-<n>`; without `--take`, what `choices.toml`
chooses. `--seconds N` sets the length (up to 600), `--beeper` renders the beeper alone over the
same frames. The game's music comes from `DIR/sound-cache/` (a game not there is learned and cached
first, which takes a while); from the cache a render takes about a second. It prints one line
when the WAV is written; an unknown game, score or take, or no workspace, is one line on stderr and
exit 1, a take the pack would refuse `refused: <why>` and exit 3.

**The files**, in `DIR/arrange/` (`W/all/arrange/` under `build/studio/`, which git ignores):

    choices.toml                     [choices] "<game>/<key>" = <take> (0 or absent: the original;
                                     "chip-<n>": a C64 take, "ay-<n>": a Spectrum 128 take,
                                     see "Music from other versions"); [judged] the same keys
                                     for the choices judge --apply made (fmzx-pack reads only
                                     [choices])
    <game>/<key>/score.toml          the score's notes (fmzx-pack personal)
                 original.mid, original.wav   the score alone
                 takes.toml          the takes made, for which score.toml, by what, how long each took
                 judge.toml          the judge's figures per take, its pick, confidence and reasons
                 take-<n>/arranged.toml       the take as the FM-1 plays it
                 take-<n>/fm1.mid, fm1.wav    the same, to listen to (the drum's level picks a drum)
                 take-<n>/full.mid, full.wav  the original with everything the model wrote
                 chip-<n>/...        a C64 take (see "Music from other versions")
                 ay-<n>/...          a Spectrum 128 take (the same)

`arranged.toml` has `score.toml`'s header, `take`, `generator` (the model and its settings), a
`[metrics]` table and notes on 4 voices: 0 the lead (the score's voice with the role lead, else its
highest voice, as it is), 1 the bass, 2 the pad, 3 the drum (`period_t = 0`, its `level` the hit's
strength). The score's other voices give way to the bass and the pad. The figures: the key the
original's notes suggest; the share of the bass's and the pad's sounding time in that key; their
consonance with the lead (the share of 10 ms steps where they sound a unison, third, fourth, fifth
or sixth apart; random notes give about 0.58); the share of onsets within 30 ms of a lead onset;
each voice's notes a second. They help to compare takes; the ear decides.
[docs/music-generation.md](docs/music-generation.md) describes the formats in full, and how to try
other generators.

**What the next build checks.** Every `fmzx-pack personal` run reads `choices.toml` (the cache
holds the learned music, so a changed choice learns nothing again) and puts each chosen take into
the pack in place of the score's notes; the score keeps its trigger and recorded edges, so it
starts and plays in step with the beeper as before. The take must have the score's key and
timeline (`intro_t`, `loop_t`), every note within that timeline on voices 0-3 with none
overlapping another on its voice, a tonal period of 64 to 16,777,215 T, a drum period of 0 and
levels 1-255. A frame of music holds at most 8 events (note ons and offs of every voice): where a
frame has more, drum notes give way first, then pad notes, the latest first, and the report says
how many; a frame with more on the lead and the bass alone refuses the take. The game's scores must
stay within 128 KB with it, method B's decoder must take the list, and a replay of the run that
recorded the score (the cache keeps each score's run) must still play the score where it played. A
shadow score's take goes into the list after the scores, marked to keep the frames the routine
claims; it must leave the game at 8 scores at most and within 128 KB with the scores packed, and
the replay must play it over its whole stretch, at least 90% of the frames it sounds in. A take
that fails any of these leaves the original (for a shadow score, the routine playing live).
`sound-report.txt` says what plays, and why a take is refused, under the `recorded score` (or
`shadow score`) line that ends with the score's key:

      arranged: take 2 (amt stanford-crfm/music-medium-800k staged drums,bass,chords top_p=0.98 seed=1)
      arranged: original
      arranged: take 3 refused: notes 14 and 15 overlap on voice 1

(`arranged: original`: takes exist, none chosen; a shadow score's `arranged: the live version (no
take chosen)`). The game's line counts the takes that play (`music recorded (2 scores, 3 voices, 1
arranged)`), and the table ends with `<name>: score <key> plays take 2 (...)` or `<name>: score
<key>: take 3 refused, the original plays: <why>` per choice (`<name>: shadow score <key> plays take
2 (...)`, `... refused, the live version plays: <why>`).
A choice that names no score of the pack (a game left out, music the learner no longer records) is
ignored, with a note; a `choices.toml` that does not read fails the run. A take's bytes count
toward the pack as the score's did.

**Limits.** The FM-1 plays 3 tonal voices and one drum sound, so a take is a lead, one bass note
and one pad note at a time, and one drum whose hits differ only in strength (the drum kinds the
model writes are only heard in `fm1.wav`). A drum hit lasts at most 50 ms, hits are at least 40 ms
apart, and each ends before the next starts (the FM-1's drum strikes only from silence). The model
writes the loop as plain music, with nothing telling it that the pass's end leads back to its
start, so the seam can jump; the FM-1 plays that pass round for as long as the beeper repeats it.
Takes differ widely in how many notes they hold.

**Speed.** `rules` takes a moment a score. The model runs on Apple silicon's GPU (MPS), else
CUDA, else the CPU; on an Apple M4 Max it took 0.11 to 0.15 s a note it wrote, which for a
14-second score was 37 s for a sparse take and 3 minutes for a dense one. A minute of music is
roughly 5 minutes a take, 20 minutes for 4 takes; `generate` prints an estimate first and its
progress every 15 seconds. The selftest takes about 20 s.

**Offline.** `tools/pack-studio ... --offline` runs `UV_OFFLINE=1 tools/arrange-music generate
W/all --offline` for its AI jobs: uv uses only its cached packages and the tool only the
cached model, and it stops with how to get the model when it is not cached (one run without
`--offline` downloads it). Run directly, `tools/arrange-music generate --offline` keeps the model
offline, and `UV_OFFLINE=1` keeps uv offline too.

### Music from other versions

Many games also came out on the Commodore 64, often with a tune of their own on its SID chip, and
many have a Spectrum 128 version, or a rip of its music, playing the 128K's AY chip. A recorded
score (or a shadow score) can take its game's C64 or Spectrum 128 tune as a take, next to the AI
takes: the FM-1 then plays that tune's notes from the moment the Spectrum tune starts, on the
tune's own timeline (its own intro and loop), and stops when the beeper stops matching the
Spectrum tune (the game has moved on), as any score does.

    tools/pack-studio ~/games --generate sid,ay                       # candidates as chip-<n> and ay-<n> takes
    tools/pack-studio ~/games --auto                                  # the same, then the judge chooses where sure
    cd host && cargo build --release -p fmzx-pack && cd ..          # or directly: the converter (SID and AY),
    tools/arrange-music chip build/studio/games/all --sid-dir ~/C64Music   # on collections you have
    tools/arrange-music ay build/studio/games/all --ay-dir ~/AY/music/ay
    tools/arrange-music pick build/studio/games/all                   # listen, choose chip-1 or ay-1, or --set
    tools/pack-studio ~/games --build                                 # the packs, with the chosen take

**What it needs.** A SID collection, such as the High Voltage SID Collection (its `C64Music`
folder; any folder of `.sid` files works, and its `DOCUMENTS/STIL.txt`, when there is one, helps to
find the tunes), or a collection of `.ay` files (ZXAYEMUL, the Project AY format), such as the
chris-y/AY repository on GitHub (its `music/ay` folder: `games/` holds the games' tunes, `misc/`
more). `tools/arrange-music` never downloads a collection: give it one you have, or let
[`tools/pack-studio`](#your-own-games) download both into `build/hvsc/` and `build/ay/`.
The converter is `fmzx-pack chip-notes` (`cd host && cargo build --release -p fmzx-pack`): it
plays a SID file's tune in a 6502 and C64 model of its own and writes the notes. `tools/arrange-music`
runs under uv as for the AI takes, and FluidSynth renders the takes; `chip.wav` (the C64's own
sound) is written only when `sidplayfp` is on the PATH.

**Finding the tune.** `tools/arrange-music chip DIR --sid-dir PATH [--game ID]... [--max N]
[--force]` looks, for each score, for SID files whose file name or header title is the game's
name, or whose STIL entry names it (more so when the entry mentions the Spectrum), converts 180 s
of each candidate subtune and measures its closeness to the score's tune, as the judge does
(above): about 1 for the same tune in another key, tempo or arrangement, under 0.2 for another
tune. Candidates are ranked by closeness, then by how well the title matches, and the best N
(default 3) are written as `DIR/arrange/<game>/<key>/chip-<n>/` (`arranged.toml`, `fm1.mid`,
`fm1.wav`, and `chip.wav` when it can be made). Most C64 versions play a different piece than the
Spectrum game, so many scores find nothing close; the judge then weighs how each take sounds and
where it comes from (the game's own C64 music counts for more than a later remix), and says when
it is not sure.

**From a Spectrum 128 tune.** `tools/arrange-music ay DIR --ay-dir PATH [--game ID]... [--max N]
[--force]` does the same with the `.ay` files at PATH: candidates are the files whose file name or
misc string is the game's name, or one of whose songs' names is (a file under `games/` before one
under `misc/` at a tie), each song checked and ranked as above, and the best written as
`DIR/arrange/<game>/<key>/ay-<n>/` (`generator = "chip ay"`, `ay_file`, `subtune` the song,
1-based, and the source `Spectrum 128: <title> by <author>, song N, <file>`). The converter runs
an `.ay` song's Z80 player in the Spectrum machine core, captures the AY registers it writes and
writes the notes in the same form. Many `.ay` files in such collections hold a 48K game's beeper tune, or
an Amstrad CPC or SAM Coupé song; the converter refuses those, and the tool skips them without a
line each and counts them. `ay.wav` (the Spectrum 128's own sound) is written only when `ffmpeg`
reads `.ay` files (its libgme demuxer).

**Choosing.** `tools/arrange-music judge DIR --apply` (`m` or `--auto` in the studio) chooses
where the judge is sure. The `pick` page lists each chip take with its source (`C64: <title> by
<author> (<released>), subtune N, <file>`) and each ay take with its own, with the judge's figures;
choosing one saves `"<game>/<key>" = "chip-1"` (or `"ay-1"`) in `choices.toml`, and
`tools/arrange-music pick DIR --set GAME/KEY=chip-1` does the same without the page (`--list`
prints the chip and ay takes too). The next build packs the
chosen take as a replacement score: the score's trigger and recorded edges are kept, so it starts
with the Spectrum tune and follows the beeper as before, and its notes play on their own timeline
(docs/pack-format.md, "replacement"). The take is checked as an AI take is, against its own
timeline: the score's key, `timeline = "own"`, every note within `own_intro_t + own_loop_t`, voices
0-3 with no overlaps, tonal periods 64 T or more, drum periods 0, levels 1-255; the same 8 events a
frame (counted on its own sections), 128 KB and replay checks follow. `sound-report.txt` says:

      arranged: C64 take chip-1 (C64: <title> by <author> (<released>), subtune 2, <file>)
      arranged: C64 take chip-2 refused: note 6: it ends at 630000064 T, past the take's own end at 630000000 T
      arranged: Spectrum 128 take ay-1 (Spectrum 128: <title> by <author>, song 2, <file>)

and the table ends with `<name>: score <key> plays C64 take chip-1 (...)`.

**What carries over, and what does not.** The notes of the SID's three voices (the highest on
average plays the lead, the lowest the bass, the other the pad), their gates and pitch changes,
noise as drum hits, and the volume and sustain as levels. Fast arpeggios become their lowest note
held. The filter, pulse width, ring modulation and sync are not carried over, nor drums played as
samples through the volume register; tunes for 2 or 3 SID chips, and tunes that call the C64's
KERNAL, are refused. **Loops**: a loop is found only where the SID's registers repeat exactly,
twice within the 180 s converted, with a pass of at least a second; tunes with free-running
modulation never repeat exactly, and loops longer than 90 s are not found. Without a loop the take
plays its notes once (up to 180 s) and then rests while the score still follows the beeper. Either
way the C64 tune plays only as long as the Spectrum tune does: a score whose recording has no loop
ends where its recording ends, however long the C64 tune is, and the pack keeps only the C64
notes up to that end. An AY song carries over the same way: its three channels' tones (by
average pitch: lead, bass, pad), their volumes as levels (a channel on the envelope at full
level), a repeating envelope with the tone off as a buzzer note at its pitch, and noise as drum
hits; the envelope's shape within a note is not carried over. Its loops are found where the AY's
registers, as heard, repeat exactly, as for a SID, and a Spectrum 128 take ends with its score
the same way.

**Rights.** The tunes belong to their authors. A chip or ay take is for your own personal pack:
keep the collections, the takes and the packs made with them to yourself. Nothing from a SID or AY
collection is part of the repository or its packages (the tests build their own made-up SID and
`.ay` files).

## Music corpus

`music-corpus` measures the FM music decoder (`core/sound`, the same C code as the firmware) over a
local folder of games. It is for games that must not enter the repository (any file of
yours): it reads the folder you give it and writes only under the output directory, never
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
`<name>_<ext>_fm.wav` (the mixer's output in FM: sound), `<name>_<ext>_fm_b.wav` (in FM: engines, the
built-in entries) and, for a game with learned music, `<name>_<ext>_fm_l.wav` (in FM: engines with its
learned entries, as the personal pack plays it), all 3 frames late to listen to the difference (`foo.tap` writes `foo_tap_beeper.wav`, so a `foo.tzx`
beside it keeps its own).

Key scripts: a file `<name>.keys` beside the game holds steps `ACTION@ms[+hold]` separated by commas
or line breaks (`#` starts a comment line), in the syntax of the `demo` strings of `games/games.toml`
but with Spectrum inputs: `key.ENTER@8000`, `key.SPACE@9000+200`, `kempston.fire@12000`. Hold is
100 ms if omitted; times count from the start of the run.

    # cave-runner.keys
    key.ENTER@8000+500, key.SPACE@9000+200

Some games read the keyboard only now and then (some only between the notes of their title tune),
so a press may need a longer hold.

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
  Castaway, Colony 8, the two-count, burst and unison tunes): the engine's own
  notes are read from RAM and compared with what the decoder found: frame precision and recall,
  notes found of the true notes, and decoded notes that match no true note (`wrong`). After `B:`,
  the engines method B recognised and the same numbers for its notes (per voice, the last pitch it
  sounded in each frame it is confident about). After `learned:`, the frames with sound a learned
  entry claims and the same numbers with the learned entries. The CSV has the same columns.

The run is fast: the games run in parallel, and a folder of 29 games takes about 10 s on an
Apple Silicon Mac when it learns, under 5 s from the cache. The two-count, burst and unison
entries of method B come from local copies of games that are not in the repository; they are
tested only locally, with this tool.

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

| Measurement | Figure |
| --- | --- |
| Z80 core on pi32v2 in fm1-emu (`emu-gate`, 150 frames of the bench; limit 30) | 13.90 instructions per T-state |

Re-run with `cd host && FMZX_FM1_BUILD=$PWD/../build/fm1 cargo test --release -p emu-gate -- --ignored --nocapture`.

### Firmware budgets and the display frame-rate report

Both need the device build (see Firmware above) and run in fm1-emu:

    cd host
    FMZX_FM1_BUILD=$PWD/../build/fm1 cargo test --release -p emu-gate --test fw_budget --test fw_report -- --nocapture

`fw_budget` measures the CPU per frame in the two heaviest Sound and Screen pairs by default (FM: sound in
Crop, FM: engines in Border; about 8 minutes), each with the USB cable out and in (a host enumerates the
firmware and polls it while the game runs). `FMZX_BUDGET_FULL=1` runs the full matrix (about 25
minutes, estimated): before a release, or after a change to per-frame code that only one mode runs. In full it runs every game, and a generated fixture pack (`build/fm1/fixture-pack`, never committed: `noise.sna`, which stresses the decoder, and two programs with 4 unknown OUT sites per frame), in Crop with each of the three sound modes (Beeper, FM: sound and FM: engines) and in the other screen modes (Fit, Fit smooth and Border) with FM: engines (screen and sound work are independent and add up, so not every pair is run; the worst unmeasured pair, estimated from the tables below, is Border with ZX Sopwith under FM: sound at about 2.37M, or Border scroll at about 2.50M, both well under the 3,600,000 cap), and checks RAMTEXT (`.ram_text` <= 49,152 bytes), the flash driver's SRAM code (`.ram_flash`) in the RAM region with `.data` and `.bss` (<= 65,536 bytes), image + game pack
<= 0x8DFBC bytes, POOL below the user stack, the audio interrupt (`fm1_alnk0_irq` <= 8,000
instructions per block) and the CPU per Spectrum frame (`game_frame` plus every
`lcd_compose_band` call of that frame <= 3,600,000 instructions, 75% of a 20 ms frame at 240 MHz).
`fw_report` (about 5 to 6 minutes (318 s measured), the 27 runs in parallel) boots every game in `games/games.toml`
at LCD dividers 4, 2 and 1 in RGB444, plays the manifest's `demo` (`demo = "<key>@<ms>[+<hold ms>],..."`,
keys as in the key syntax plus `PLAY`/`HOME`, ms after the game started), measures 20 s from
second 16 on and prints the table below. It also checks that each game with a demo is being
played: rows must go out in at least 90% of the 1-second sub-windows (at least 8 rows each; gameplay
sends 45-90 rows a second, a title, menu or game-over screen sends none, though a blinking menu can still pass the share check, so the games' demos have also been checked by eye on the frames), and a second test
(`title_screens_are_not_gameplay`) shows that Castaway at its title menu fails it. It writes `build/fm1/fmzx-report.json` and asserts:
scroll 20-23 fps at divider 4 and >= 49 at divider 1; every game with a changing screen >= 45
fps at divider 1. fps is `50 * row_sends / pending_frames_sum` (how often a changed row reaches
the panel; 50 = every changed row goes out in the frame that changed it); a static screen
(`border`) sends nothing and has no rate.

The report (20 s windows; CPU at divider 1; the instruction counts are what the
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

(Crop.) The other screen modes, with FM: engines:

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

Audio interrupt, max 1,574 instructions per block. RAMTEXT 49,108 of 49,152 bytes. The flash
driver 2,932 bytes in the RAM region, which holds 30,708 of 65,536 bytes. Image
+ pack 216,768 of 581,564 (0x8DFBC) bytes. POOL 275,800 bytes, 92,840 below the stack guard. The demos play the whole 20 s window: Castaway's player steps left and right on the ship's
platform, out of the crawler's reach, so the energy does not run out; ZX Sopwith flies and
crashes through its five planes; Escape from Colony 8's player walks and shoots in the first room; ZX Dino jumps and restarts after each crash; A World of One-Way's heroes keep sliding in the first level.
