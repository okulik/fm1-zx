# Tape signal loading and the personal pack

Status: designed 2026-11-14; plan to follow (Plan 7).

## 1. Intent

The owner wants to play any of their own 48K games on the FM-1, which they say Fuse on macOS loads
without trouble. Two gaps stand in the way today.

- **Custom loaders.** 18 of the 31 files in the owner's folder (`~/Downloads/zx`) do not load.
  They use turbo, pure-tone, pulse-sequence and pure-data TZX blocks (0x11, 0x12, 0x13, 0x14),
  usually behind their own loader code. Our emulator only loads tapes by trapping the ROM's
  LD-BYTES and copying `.tap` blocks into memory. The host analysis tool converts standard-speed
  `.tzx` to `.tap` (`host/crates/music/src/tzx.rs`) and refuses everything else.
- **Getting games onto the FM-1.** Games come only from the built-in pack (`games/games.toml`,
  built by `tools/make-packages`). The owner has no way to build a pack of their own games.

USB upload stays out of this stage (owner, 2026-11-14: "let's wait with USB for now").

## 2. What the owner decided

- **Approach A (owner's choice, 2026-11-14).** Games load on the Mac when the pack is built. The
  emulator core plays the real tape signal into the emulated Spectrum's tape input, the way
  Fuse does. The pack builder runs each game until its tape has finished, then stores the
  machine as a snapshot. On the FM-1 every game starts instantly, and its speed and memory
  budgets do not change. The analysis tool uses the same loader, so every one of the owner's
  48K games can be analysed.
- **Approaches considered:**
  - **B:** play the tape on the FM-1 itself. Multi-load games would work, but every start would
    wait for the tape, the firmware would need more code in nearly full memory, and the device
    test would be harder.
  - **C:** A by default, B for marked multi-load games.

  B and C are not built now. Because the player lives in the shared core, they stay possible.
- **Personal pack, as proposed and accepted:**
  - One command builds firmware packages from a folder (or list) of the owner's games.
  - Each game gets the default key map. The owner can override a game's keys and name in a local
    file that never enters the repository.
  - A pack that does not fit is refused, with a list of game sizes, so the owner knows what to
    drop.
  - Games, snapshots and the built packages stay on the Mac (under `build/`, untracked).

## 3. Goals

1. **Tape signal player in `core/zx`.** It plays TZX blocks 0x10 (standard speed, as a signal),
   0x11, 0x12, 0x13, 0x14 and 0x20 (pause, including "stop the tape"), plus the loop blocks
   0x24/0x25. Info and group blocks (0x21, 0x22, 0x30, 0x32 and the others `tzx.rs` already
   skips) are skipped. The tape level appears on bit 6 of port FEh reads, timed to the T-state.
   The game's own loader then runs untouched.
2. **Load to snapshot (host).** Given a `.tzx` or `.tap`, autostart it (as `zx_tape_autostart`
   does), play the signal until the tape ends, run a short settle time, then capture the machine
   as a 48K `.z80` that our own `zx_load_z80` loads back identically.
3. **The analysis tool** (`music-corpus`) loads every `.tzx` that the trap cannot load this way, so
   turbo-loaded games are analysed like any other. Standard-block files keep today's trap path
   there too, so existing `.keys` files and the Plan 6 numbers stay valid. For a signal-loaded game,
   time 0 of the run (the 40 s window and its `.keys` steps) is the loaded snapshot, after the tape
   and the settle time. No such key files exist yet, so nothing needs migrating.
4. **Personal pack command.** It takes a folder (or a games file) and builds `fmzx.fwsc` and
   `fmzx-preview.fwsc` with those games.
   - `.sna` and 48K `.z80` go in as they are.
   - `.tap`, and `.tzx` with only standard blocks, go in as tapes, so they keep the instant ROM
     trap and multi-load, but only if a trial load through the trap on the host succeeds
     (section 6, "Successful load").
   - If the trial fails (for example a headerless standard-speed loader of the game's own, which
     the trap cannot serve), the tape is loaded through the signal instead and goes in as its
     snapshot.
   - Every other `.tzx` goes in as its loaded snapshot.
   - 128K files and files that fail to load are reported and left out.
   - The browser test bench can run the personal packages, so the owner can try a pack before
     installing it.

## 4. Non-goals and anti-goals

- **Non-goals:**
  - USB upload.
  - Tape playback on the device (B/C).
  - 128K games, AY music, `.tzx` direct-recording (0x15), CSW and generalized-data (0x19) blocks
    (none occur in the owner's files).
  - Saving games.
  - Typing BASIC.
- **Anti-goals:**
  - Any change to how games in the built-in pack load or play.
  - Beeper output or emulation timing changes for programs that never use the tape signal.
  - Any of the owner's games, their snapshots or a personal package entering the repository.
  - A snapshot that does not run like the game did in the emulator at the moment it was taken.

## 5. Constraints

- `core/` C rules as for every stage:
  - integer only, no allocation;
  - `-Wall -Wextra -Werror`;
  - compiled into the host, the firmware and wasm;
  - host and device identical where both run it.
- The tape player must not cost the firmware RAMTEXT it does not use. Today 1,008 bytes are free.
  The firmware never plays a signal in this stage, so the player is kept out of the firmware build
  (as `engines_test.c` is), except for whatever the port-read check needs.
- The port FEh read path is hot. With no signal playing, the result must be exactly as today
  (bit 6 = the last OUT's EAR bit), and the extra cost is at most a compare.
- Untrusted input: every TZX length and count is checked, as `tzx.rs` does.
- Third-party games never enter the repository. Tests use generated tapes: a turbo-block tape
  and a pure-tone and pulse-sequence tape built from our own test program, in the style of
  `testprogs.rs`.
- Commit messages are a single line with no trailers. Never name other projects in files or
  commits.
- Device rule: the owner installs packages. Nothing is flashed or sent over MIDI.

## 6. Behaviour

**Tape signal.**
- The player keeps a position in the block list and the current pulse's remaining T-states. It
  advances by the T-states the machine runs, flipping the level at each pulse end.
- Block timings follow the TZX 1.20 specification.
  - Standard block: pilot 2168 T × 8063 or 3223 pulses, syncs 667/735 T, bits 855/1710 T.
  - Turbo block: its own values.
  - Pause: level low, then the next block.
  - Pause 0: stop the tape until it is started again. For load-to-snapshot, a stop means the
    tape is done if no data block follows. Otherwise playback resumes after 1 s, as a person
    would press play again.
- The level starts low at insert. Each pulse flips it.

**Load to snapshot.**
1. Insert the tape.
2. Autostart (`LOAD ""`, as the ROM trap path does today), with the trap off.
3. Run frames until the player reaches the end of the tape.
4. Run the settle time, 2 s (the designer's call).
5. Capture.

**Successful load (the designer's call), for both the trap trial and the signal load:**
- The whole tape is used. On the trap path, every data block was taken by LD-BYTES. On the signal
  path, the player reached the end.
- Then, over the 2 s settle time:
  - the CPU runs code in RAM (PC at 4000h or above) in at least half of the sampled frames;
  - it is not halted with interrupts off;
  - it is not sitting in the ROM's error report or back at the copyright screen.
- A tape still being read when the time runs out (the loader polling port FEh) is a failure.
- The command's table shows each outcome.

Timeouts and failures:
- The load gives up after 15 minutes of emulated time, which is well over any 48K tape. On the
  host that is a few seconds of wall time.
- A game that halts with interrupts off, or that sits in the ROM's error report, is reported as
  failed.

**Snapshot.**
- A `.z80` version 3, 48K: all registers, IFF1/2, IM, border, R. Memory is stored with the
  format's own run-length compression or uncompressed (builder's call).
- `zx_load_z80` of it must restore a machine whose next frames match the original's frame for
  frame: screen, edges and registers. This is a test.
- Where the core keeps state the format cannot carry (for example an interrupt pending
  mid-frame), capture at a frame boundary, where today's snapshot loads already start.

**Personal pack command.** `tools/make-personal-pack <folder | games.toml> [-o DIR]`
- Given a folder, it writes (or updates) `<DIR>/personal-games.toml`. There is one entry per
  loadable file, with the name taken from the file name minus extension and bracketed tags, and
  no keys table (the default map). On the next run the owner's edits to names and keys are kept.
- Given a toml, it uses it as written. The syntax is that of `games/games.toml`, so docs/controls.md
  applies.
- It converts or loads each game (section 3, goal 4), builds the pack and the two packages under
  `DIR` (default `build/personal/`), and verifies them as `make-packages` does.
- It prints a table:
  - each game, how it went in (as is, tape or loaded snapshot), and its packed size;
  - the space used and left;
  - every game left out and why.
- If the pack does not fit, it fails and lists the games by size.
- A flag runs the browser test bench on the personal packages.

## 7. Tests (committed, no third-party bytes)

1. **Signal timing.** A generated tape of each block kind is played. A Z80 test program samples
   port FEh and records the level-change times, which must match the block's pulse timings
   within 1 T plus the sampling loop's period.
2. **Turbo load.** A generated program saved as a turbo block (custom timings), loaded by a
   generated custom loader (our own code, in the style of `testprogs.rs`), reaches its code with
   the right bytes in memory.
3. **Unchanged without a signal.** Every existing golden (screen, beeper, method A and B) and
   every existing test passes unchanged. A program reading port FEh with no tape gets today's
   values.
4. **Snapshot round trip.** Load to snapshot, then `zx_load_z80`. The next 200 frames of the
   restored machine match those of the original that kept running.
5. **Standard tape as signal.** `tape.tap` (our multi-load test), wrapped as standard 0x10 blocks
   and loaded through the signal with the trap off, gives the same memory as the trap path.
6. **Personal pack.** A folder of our own test programs (the generated turbo tape, `tune.sna`,
   `tape.tap`) builds packages that pass `fmzx-pack verify`. The firmware tests boot one in
   fm1-emu and start each game.
7. **Local corpus run (not committed).** All 27 `.tzx` in the owner's folder load in the analysis
   tool or are reported with a reason. The expectation is that every one listed in §1 loads,
   since Fuse loads them. The run is recorded in the plan's report.

## 8. Calls the designer made (the owner may overrule)

- **Standard-only tapes keep the trap path,** so multi-load and instant loading still work.
  Signal loading is used only where the trap cannot load the tape.
- **Settle time 2 s** after the tape ends.
- **Fallback.** A standard tape that the trap cannot load falls back to signal loading. It goes in
  as a snapshot, so its later levels from tape are not available.
- **Success test.** As in section 6: the whole tape is used, then the CPU runs RAM code, is not
  halted with interrupts off, and is not in the ROM's error report or at the copyright screen.
- **Analysis tool.** It keeps the trap for standard tapes. A signal-loaded game's run starts at its
  snapshot.
- **Personal output** goes under `build/personal/` (untracked), with the editable
  `personal-games.toml` beside the packages.
- **No loader acceleration** (Fuse's skip-ahead in edge loops). The host runs about 150 times
  faster than real time, so a 5-minute tape loads in about 2 s. Acceleration matters only for
  device playback (B), which is not built.

## 9. Left to the builder

- Where the TZX block parser lives. One parser should serve the C player and the host. Keeping it
  in C, with the Rust side calling it, is the obvious choice.
- How the player is kept out of the firmware build.
- The exact `.z80` compression.
- How the command reuses `make-packages` and `fmzx-pack`.
- The test program and generated loader details.
- The command's table format.
