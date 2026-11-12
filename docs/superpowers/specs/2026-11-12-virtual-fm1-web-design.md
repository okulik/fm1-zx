<!--
SPDX-FileCopyrightText: 2026 FM-1 ZX contributors
SPDX-License-Identifier: GPL-3.0-or-later
-->

# Virtual FM-1 in the browser (Plan 3)

**Status:** approved by the owner, 2026-11-12. Milestone 4 of the main design
(`2026-11-11-zx48k-fm1-design.md`, §5.9).

## 1. Intent

A browser page where a virtual FM-1 (fm1-emu) runs **the firmware package we will flash**, so
the owner can pick a game in our picker and play it from the computer before any hardware
arrives.

**Who it is for (the owner's words): "public demo eventually, but my test bench first."** This
plan builds the test bench, shaped so that publishing it later is a deployment step and not a
rewrite.

## 2. Goals, non-goals, anti-goals

**Goals**
- `tools/web-demo` builds the firmware and its package, fetches fm1-emu at the revision the
  repository already pins, builds fm1-emu's web crate to wasm, puts our files from `web/` on top,
  and serves the result on localhost.
- The page runs the **release package** (`fmzx.fwsc`, RGB444), with correct colours and sound,
  and starts our firmware without a firmware choice or a Start-button hunt.
- Every FM-1 control the firmware uses can be reached from the computer keyboard, and the
  common game actions (joystick, fire, start/OK, menu) sit on the keys a player expects.
- A controls panel next to the device shows, for the game the owner picks in the panel, which
  computer key presses which FM-1 control and what that does in the Spectrum.
- The emulator's own speed readout (upstream already shows "N× real time" and audio load) stays
  visible, so the owner can tell whether a game runs at full speed in the browser.
- `tools/web-demo --check`: a headless smoke test that boots the built page's wasm with our
  package and fails on a broken build.

**Non-goals (this plan)**
- Publishing to GitHub Pages, and the Moon Caves keep/drop decision that publishing needs. Both come
  with the deployment plan.
- The plain Spectrum page (milestone 5), USB upload, MIDI input.
- Forking fm1-emu, or sending patches upstream (the latter is an outward-facing step for the
  owner to decide later).
- Changing the firmware for the page's sake.

**Anti-goals**
- A page that runs a different build from the one we flash (wrong colours hidden by a special
  package, for example).
- Games that can't be played from the keyboard because a control the game needs has no key.
- A build that silently picks up a different fm1-emu than the one emu-gate tests against.

## 3. Constraints and facts found

- fm1-emu is pinned at `93cb7ee3f7008b27959ba73073b864e37efd7289` in
  `host/crates/emu-gate/Cargo.toml`; the web build uses the same revision (read from that file).
- fm1-emu's web build is `web/build.sh <outdir>`: the `fm1-web` crate to `wasm32-unknown-unknown`
  plus its page (`index.html`, `main.js`, `worklet.js`, `jit.js`, `fwsc.js`, `persistence.js`,
  `style.css`, `firmwares.json`). On macOS it uses rustup's stable toolchain.
- The page boots a firmware from its whole `.fwsc` package listed in `firmwares.json`; a
  relative `package` path is served next to the page.
- **Colours:** fm1-soc's LCD (`crates/fm1-soc/src/lcd.rs`, `pixel_byte`) ignores COLMOD and
  always takes two bytes per pixel as RGB565. Our release firmware sets COLMOD 0x03 (12-bit,
  three bytes per two pixels) unless the boot option or the preview build asks for RGB565.
- **Sound:** on the package path the page leaves the audio PLL unset, so the firmware runs
  silent. emu-gate writes the stock values (`fw.rs` `AUDIO_PLL_STOCK`: `0x119A0 = 0x45400203`,
  `0x119A4 = 0x3F503026`, felucca-bsp's `FM1_AUDIO_PLL_STOCK0/1`).
- **Clock:** the page defaults to 100 MHz (`?mhz=` or a stored choice override it); our frame
  budgets assume 240 MHz (`emu-gate/src/budgets.rs`).
- **Keyboard:** upstream maps `awsedftgyhujkolp;'` to C4..F5, `z x` to OCT-/OCT+, Space to PLAY,
  `r` to REC, `1..0` to FX SCL ENV LFO EDIT GLO HOME SAVE ARP SEQ; ←/→ turn SELECT, ↑/↓
  PRESETS, `,`/`.` ALGORITHM. The page's `turn()` takes **panel** encoder indices
  (`panel::ENCS` in `crates/fm1-live/src/panel.rs`, read through `panel_enc_id` in
  `crates/fm1-web/src/lib.rs`), not the firmware's encoder ids: SELECT is panel 0, ALGORITHM
  panel 2, so our layer translates. The notes F3..B3, F#5 and G5 have **no key**, and our default map puts the
  Kempston directions on F3..A3 and fire on C5..G5; with upstream's keys alone the joystick is
  unreachable.
- Key ids (docs/controls.md): buttons 0-13, notes 14 + n (F3 = 14 … G5 = 40), encoders 0-6.
- The page cannot see which game the firmware is running; the panel needs its own game choice.
- **Kept packages:** after the first boot upstream's page saves the package in IndexedDB under the
  catalog id and boots that saved copy on every later load (`packageOf()` → `keptEntry`),
  fetching again only on "Update firmware". Unchanged, a reload after a rebuild would run the
  previous build. `python3 -m http.server` also leaves files to the browser's cache heuristics.
- upstream's `build.sh` also builds an `smk37/` page with upstream's community catalog, and the
  main page carries the firmware selector, guide cards, "Open .fwsc…", DX7 bank, MIDI, "Update
  firmware" and save/backup controls.
- Licences: fm1-emu is MIT; its files are copied into `build/`, never into the repository; the
  served site carries its LICENSE.
- **Speed is the open risk (main design §5.9):** a Spectrum emulated inside an emulated FM-1 in
  wasm may not reach real time. Then the page is a preview, not a way to play; this plan reports
  the speed and does not promise it.

## 4. Approaches

- **A. Wrap upstream's page, run the release package (chosen by the owner).** Fetch fm1-emu,
  apply small checked text patches to the fetched copy (LCD honours COLMOD, stock audio PLL,
  240 MHz default, start at once, our keyboard layer), add our catalog, keyboard layer and
  controls panel from `web/`. The browser tests exactly the package we flash, including the
  RGB444 path.
- **B. Same, but serve the RGB565 preview package and leave the LCD alone.** One patch fewer;
  the browser then tests a build we don't ship and RGB444 stays unproven until hardware.
- **C. Our own page around fm1-web's wasm, upstream's HTML/JS dropped.** Full layout control,
  at the cost of rewriting audio, input, persistence and the package loader. Too much for a test
  bench; still possible later if the public page needs it.

## 5. The design

### 5.1 Files

```
tools/web-demo          the build-and-serve script (options below)
web/firmwares.json      our one-entry catalog (version filled in by the script)
web/fmzx.js             our layer: keyboard map, controls panel, wiring into upstream's page
web/fmzx.css            the panel's styles
web/patch-edit.sh       the checked text-replace helper the patch scripts use
web/patch-emu.sh        patches to the fetched fm1-emu sources (LCD, audio PLL)
web/patch-page.sh       patches to the built page (default clock, auto start, our layer's hooks)
web/check.mjs           the headless smoke test
web/serve.py            the local server (127.0.0.1, Cache-Control: no-store)
host/crates/fmzx-pack   new subcommand: the resolved key maps as JSON for the panel
```

Output goes to `build/web/` (served) and `build/fm1-emu/` (the fetched checkout); both are
under the already git-ignored `build/`.

### 5.2 `tools/web-demo`

```
tools/web-demo                 build everything into build/web and serve on localhost:8642
tools/web-demo --check         build, then run web/check.mjs and exit with its status
tools/web-demo --no-serve      build only
tools/web-demo --skip-firmware reuse build/fm1's firmware and package (no Docker)
tools/web-demo --port N        another port
```

Steps:
1. Unless `--skip-firmware`: the release firmware and package as `tools/make-packages` makes
   them (Docker, `fm1` preset). The page serves `build/fm1/fmzx.fwsc` as `fmzx.fwsc`.
2. Read the fm1-emu revision from `host/crates/emu-gate/Cargo.toml`; fail if absent. Fetch that
   exact commit into `build/fm1-emu` (shallow), re-fetch only when HEAD differs, verify HEAD.
3. Apply `web/patch-emu.sh`, run upstream `web/build.sh build/web` with `RUSTUP_TC` set to the
   active toolchain's sysroot (`rustc --print sysroot`, 1.99.0 here: the owner added
   `wasm32-unknown-unknown` to it). upstream's default (`~/.rustup/toolchains/stable-…`) has no
   wasm target on this machine. If the sysroot has no `lib/rustlib/wasm32-unknown-unknown`, the
   script stops and prints the `rustup target add` command; it never installs anything itself.
4. Copy our `web/` files over, write `firmwares.json` (version = short commit, `+dirty` when the
   tree is dirty) and `version.json` (our commit, fm1-emu revision, build time), generate
   `keymaps.json` with fmzx-pack, apply `web/patch-page.sh` to the built `main.js`/`index.html`,
   copy licences (ours, fm1-emu's MIT) into `build/web/licenses/`.
5. Serve with `web/serve.py` on 127.0.0.1, or run the check.

Every patch is an exact-text replacement that **fails the build** when the text is not found
exactly once, so a revision bump that moves upstream code breaks loudly instead of silently
dropping a patch.

### 5.3 Patches to fm1-emu (fetched copy only)

- **LCD colour mode:** the LCD remembers COLMOD's argument; with 0x03 (12-bit) it takes three
  bytes per two pixels and stores each as RGB565 (each 4-bit channel widened: R4→R5, G4→G6,
  B4→B5 by bit replication); with 0x55 (or never set) it behaves as today. emu-gate's native
  tests keep using the unpatched crate from cargo's git checkout and the RGB565 boot option, as
  now.
- **Audio PLL:** after `skip_idle` is set in `Machine::new`, write the two stock PLL words
  (the same values as emu-gate's `AUDIO_PLL_STOCK`).

### 5.4 Patches to the page

- **Always the fresh build (the builder's call, from the anti-goal):** our entry is fetched from
  the server on every load and never taken from or saved to upstream's IndexedDB package store;
  the local server sends `Cache-Control: no-store` (a small `web/serve.py`, not plain
  `http.server`). Our firmware never writes flash, so nothing of upstream's save machinery is
  needed. How a published page keeps packages is the deployment plan's question.
- **Clock pinned at 240 MHz (the builder's call):** the speed readout only says "the game runs at
  full speed" at the clock the frame budgets assume, so the page boots at 240 MHz whatever
  `?mhz=` or the stored `fm1.mhz` say, and the clock picker and the "Use N MHz" suggestion are
  hidden. At 240 MHz, "≥ 1× real time" means the browser keeps up with the device.
- **Upstream extras hidden, not patched out (the builder's call):** `fmzx.css` hides the firmware
  selector, guide cards, "Open .fwsc…", DX7 bank, MIDI, "Update firmware", save/backup and the
  SMK-37 link by their ids/classes (hiding by CSS can't break the build when upstream moves
  code); `tools/web-demo` deletes the built `smk37/` directory. Only behaviour (kept packages,
  clock, start, keyboard hooks) is changed by text patches.
- Start our firmware on load (or with one click where the browser's audio policy needs a user
  gesture: the page's existing Start button is enough, as long as it is the only thing to press).
- Hooks for our layer: expose the page's `setKey(id, down)` and `turn(encoder, step)` to
  `fmzx.js`, and hand keyboard events to our map instead of upstream's (upstream's handler stays
  for any key our map does not claim, or is disabled entirely: the builder's call, as long as no
  key presses two unrelated controls by accident).
- Load `fmzx.js` and `fmzx.css`; place the panel beside the device (below it on narrow screens).

### 5.5 Keyboard map (the builder's proposal; the owner reviews it on the page)

Principles: every note F3..G5, every button, and the SELECT and ALGORITHM encoders have a key;
game actions sit where a player looks for them; upstream's piano rows stay where upstream puts
them so its on-screen labels stay true.

| Computer | FM-1 | Why |
|----------|------|-----|
| ← → ↑ ↓ | F3, A3, F#3, G3 **and** SELECT ccw/cw, ALGORITHM ccw/cw (panel indices 0 and 2) | joystick in games (default map), and navigation in picker, menu and on-screen keyboard |
| Space | C5 | Kempston fire (default map) |
| Enter | PLAY | start a game, OK in the menu, pause |
| Esc | HOME | the menu |
| `a w s e d f t g y h u j k o l p ; '` | C4..F5 | as upstream |
| free keys, chosen by the builder | G#3, A#3, B3, F#5, G5 | reach the remaining notes (Moon Caves uses B3) |
| `z x`, `r`, `1..0` | OCT-, OCT+, REC, FX…SEQ as upstream except that `7` (HOME upstream) | as upstream; the builder resolves HOME and PLAY duplicates |

Known trade-off (the builder's call): an arrow presses a note and turns an encoder together. In
a game the encoders do nothing by default, and in the picker/menu the notes do nothing, so each
place sees only the half it uses; under the on-screen keyboard both act (cursor and player
move), which is acceptable on a test bench. A game whose manifest maps `enc.SELECT` or
`enc.ALGORITHM` would see both; none does today, and the panel would show it.

### 5.6 Controls panel

- Data: `fmzx-pack keymaps games/games.toml -o keymaps.json` writes, per game in pack order, the
  resolved map (default plus the game's overrides) as input → actions, using the same parser and
  default table the pack uses, so the panel can't drift from the firmware.
- The panel has a game selector (pack order, the picker's names) and a table: computer key, FM-1
  control, Spectrum action; unmapped inputs are left out; reserved ones (HOME, PLAY) and the
  picker/menu controls are listed in a short fixed section above it.
- The selector does not follow the firmware; it is the owner's choice (the page can't see the
  running game). Its choice is remembered in `localStorage` (wrapped in try/catch).

### 5.7 Smoke test (`web/check.mjs`, node)

Boots `build/web/fm1_web.wasm` with `build/web/fmzx.fwsc` as upstream's own smoke test does
(JIT on, `fm1_boot_fwsc` at 240 MHz, the PLL patch active), then:
1. runs ~3 emulated seconds: not halted, the picker drawn (lit pixels over a threshold), and the
   frame's colours decode as RGB444 would (e.g. the picker's background or text colour matches
   the expected RGB565 value; a wrong decoder gives a different value);
2. presses PLAY on the first game (the scroll test) or selects the border-and-tone test and
   starts it, runs ~2 s: audio output level rises above silence;
3. prints emulated seconds per wall second, as a rough hint of browser speed.

4. checks the page side: the built `main.js` boots our entry without the IndexedDB package
   store and at 240 MHz (a text check on the patched file is enough).

Exit non-zero on any failure. The exact thresholds and which test game gives the sound check are
the builder's call.

### 5.8 Docs

- BUILDING.md: a "Browser demo" section (command, options, what it fetches, the speed caveat).
- README: one line pointing at it.
- docs/controls.md: the computer keyboard map.

## 6. Decided by the owner

- Purpose: test bench first, public demo later (publishing and Moon Caves are a later plan).
- Approach A: wrap upstream's page and run the release package with an LCD colour-mode patch.
- The wasm target is installed by the owner, not by the script.

## 7. Left to the builder

- The remaining key letters (5.5), and whether upstream's keyboard handler is replaced or
  narrowed.
- How `fmzx.js` hooks into upstream's page (minimal text patches; no copy of `main.js` in the
  repository).
- Panel look, within the page's existing style.
- Smoke-test thresholds and the sound-check game.
- Whether `version.json` is shown on the page.
