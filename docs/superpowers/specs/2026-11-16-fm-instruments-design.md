# FM instruments: an instrument per voice, by its role in the tune

Date: 2026-11-16. Status: design, for the owner's approval.

## Intent

FM music should sound arranged, not like three copies of one generic tone. Today every FM voice, in every game, plays the same 2-operator patch (`core/sound/fm.c`: a 1:1 ratio, the index a little higher for bass notes, a 4 ms attack, a fall to 0.75, a 50 ms release). The decoders already separate a tune's voices (method B reads each channel of a known or learned routine; method A finds up to three voices from the edges), so the missing piece is what happens after: knowing which voice is the bass, which the melody and which the accompaniment, and giving each a fitting sound.

The owner chose, by ear, the instrument set "D, chip-plus" from a listening test (four tunes, four candidate sets plus the beeper; the throwaway renderer and its WAVs: `build/web/listen/`, renderer `host/crates/music/examples/zz_instruments.rs`, untracked). D keeps the 8-bit character: a hollow square-ish lead with vibrato, a soft round bass, a mellow chorused accompaniment, a snare-like drum.

## Goals

1. Each FM voice plays one of three instruments by its role: **bass**, **lead** or **pad** (accompaniment), with the D sounds; the drum gets D's drum.
2. Roles are decided in two layers (the owner's choice, option 3):
   - **live, on the FM-1**, for every FM mode and every routine: a rule on the voices' pitch (below);
   - **stored in the pack** when known, which wins over the live rule: from an analysis of the whole tune when the personal pack is built, or from the owner's overrides in `personal-games.toml`.
3. Built in two steps: step 1 the room in fast memory, the synth, the D instruments and the live rule (the sound everywhere); step 2 the stored roles and the overrides.

## Non-goals

- More voices than the beeper played (arpeggios played as chords): a later project, on top of this one.
- Sound effects as FM (project 3), tapes on the device (project 4).
- Roles for the repository's built-in pack (Castaway, Colony 8, ...): the live rule covers them; `fmzx-pack games` writes no sound section, as today.
- A user-facing instrument editor or more than the three instruments.

## Anti-goals

- **Wrong notes or wrong timing.** This changes only how a voice sounds, never which notes play or when; no decoder decision (A, B, learned) may change.
- **An instrument changing in the middle of a note.** A voice's role may change only when a note starts on it.
- **Losing the FM-1's frame budget or fast memory to it.** The worst frame stays under 3.6M instructions, and RAMTEXT ends with at least as much free as today (288 bytes).
- **Louder or quieter music than today.** Overall level matches today's FM (a voice at level 255 peaks near 8160), so switching modes doesn't jump in volume.

## Constraints

- RAMTEXT 48,864 of 49,152 bytes (288 free). The synth (`snd_fm_render`) runs from RAMTEXT every sample.
- Data (patch tables, roles) can go in RAM (`.data`/`.bss`, ~42 KB free) or flash.
- Integer, fixed-point synth; the host and the device run the same C (`core/sound`), and the device's audio must stay sample-exact against the host's (emu-gate `fw_audio`, `fw_personal`).
- Pack format: learned entries reserve `u8 instrument[3]` (0 today, ignored by the decoder); the decoder rejects unknown entry kinds.
- Never flash, SysEx or open MIDI without the owner's explicit yes; the owner's games never enter the repository.

## Approaches considered

1. **Live rule only.** Works in every mode with no pack change; the first notes are a guess, and a bass line that climbs could swap roles.
2. **Stored roles only** (analysed at pack build). Right from the first note, but only for pack games in FM: engines; FM: sound would still need a rule, and the pack format grows.
3. **Both** (chosen): the live rule everywhere, stored roles where known. Built as step 1 (live rule, instruments, synth) then step 2 (stored roles, overrides), so the sound arrives early and step 2 only fixes what the rule gets wrong.

## Design

### Step 1a: room in fast memory

Move the sound library's start-up functions out of RAMTEXT into flash (XIP): `snd_mixer_init`, `snd_fm_init`, `snd_b_init`, `snd_decoder_init` (about 1.2 KB together; they run once per game start). They are called from `main.c` through the `fmzx_ram` pointer table today; the move must keep `check_reach.py` passing (no direct call between SRAM and XIP code). Anything they call that stays in RAMTEXT (`memset`, ...) is reached the way XIP code already reaches RAMTEXT code (wrappers, as `learned.c` does). Target: RAMTEXT at least 900 bytes lower before the synth grows.

*If it still doesn't fit* (my call): first move more cold code to flash (other functions that run once per game or per menu action); only then cut, in this order: the pad's second carrier (the least audible part of D), then the drum's tone component. The lead's 2:1 modulator and vibrato are what make D and are not cut. A cut is reported to the owner with the before/after listening page.

### Step 1b: the synth

A patch per instrument, as data. Each tonal voice plays the patch of its role. Patch fields (the builder picks the fixed-point forms):

| field | bass | lead | pad |
|---|---|---|---|
| modulator : carrier frequency | 1 | 2 | 1 |
| index at note start → settles at, time constant | 0.7 → 0.5, 100 ms | 1.6 → 1.3, 200 ms | 0.4 → 0.3, 200 ms |
| attack (linear) | 3 ms | 4 ms | 10 ms |
| decay to sustain: time constant, sustain level | 200 ms, 0.8 | 150 ms, 0.8 | 200 ms, 0.85 |
| release (linear) | 50 ms | 50 ms | 100 ms |
| vibrato: depth, rate, delay (ramped in over 200 ms) | none | 18 cents, 6.5 Hz, 120 ms | none |
| second carrier, detuned, mixed half and half | none | none | +4 cents |
| gain | 1.2 | 0.8 | 0.75 |

These are the listening test's values (`zz_instruments.rs`, set D); the gains are relative to each other, the overall level is calibrated to today's (anti-goals). Unchanged from today: the note restart rule (a new period more than 1/32 away, or a voice starting, restarts the note; smaller changes glide), the level glide, the 8 kHz control tick.

What the synth gains, to play these: a modulator at an integer multiple of the carrier (its own phase, or the carrier's phase times the ratio), a vibrato LFO per voice (on the 8 kHz tick: a phase increment scaled by a small sine), and the pad's second carrier (one more phase and sine per pad voice).

D's drum replaces today's noise drum: low-passed noise (one-pole, coefficient 0.5) decaying with a 70 ms time constant, weight 0.8, plus a 190 Hz sine decaying with a 30 ms time constant, weight 0.4; the strength scales with the decoder's drum level, the release as today.

### Step 1c: the live rule

Per tonal voice slot (the mixer's voices 0-2), the FM-1 keeps a slow running average of the voice's pitch while it sounds. At each note start on a voice, roles are assigned by ranking the slots that have sounded recently: the lowest average gets bass, the highest lead, a third the pad; a voice sounding alone gets lead. Two voices sounding: the lower gets bass, the higher lead. A role is held through its note (anti-goals). "Recently" is a window of a few seconds (my call; 4 s as a starting point, tuned on the corpus): a voice silent for longer drops out of the ranking, and its history is kept, so a bass line that pauses comes back as bass. Ranking is smoothed so a brief crossing doesn't swap roles (the builder picks the averaging time and the hysteresis, a few seconds and a margin of a few semitones as a starting point, checked on the corpus). Before a slot has any history, its first notes are ranked by their own pitch.

The rule runs the same in FM: sound (method A) and FM: engines (method B); in B a voice slot is a channel of the claiming routine.

### Step 2a: stored roles

- **Learned entries:** the reserved `instrument[c]` byte becomes channel c's role: 0 none (the live rule), 1 bass, 2 lead, 3 pad. The decoder stores it in the decoded entry; a value above 3 is treated as 0.
- **Built-in entries** (Cave Runner, Unison, ROM BEEP, Castaway, Colony 8) used by a personal pack's game: a new list entry **kind 1, "roles for a built-in entry"**: `u16 size, u8 kind = 1, u8 channels, u64 digest` (the built-in entry's digest, which identifies it), `u8 role[3]`, padding to a multiple of 4. It produces no engine; it gives the built-in entry with that digest these roles for this game. A kind-1 record naming no built-in digest is ignored. Kind-1 records count towards `SND_LEARNED_MAX` and the decoder's other bounds like any entry. `docs/pack-format.md` documents both. No pack version bump: the pack is always installed together with the firmware that reads it (both are in one `.fwsc` package), so old firmware never meets a new pack.
- **On the FM-1:** while a routine with stored roles claims the frame, its channels play their stored roles; any voice without one (role 0, or a frame no routine with stored roles claims) gets the live rule's role. The live rule keeps tracking every voice's pitch whatever decides its role, so its history is ready when stored roles stop applying (my call). The roles reach the synth through the frame the mixer renders (a role per tonal voice); a role still changes only at a note start.

### Step 2b: the analysis at pack build

The personal pack builder already runs each game for 3 minutes (the learner's listening). For each routine that claims frames (learned or built-in), it collects each channel's notes over the run and ranks the channels by their mean pitch over the whole run, as the listening test did: lowest bass, highest lead, a third pad, a lone channel lead. It stores the result (learned: `instrument[]`; built-in: a kind-1 record). A routine heard too little to rank (under the music gate's thresholds) gets no stored roles. `sound-report.txt` prints each routine's channels with their roles and where each came from (analysis, override).

### Step 2c: overrides in `personal-games.toml`

A per-game table names roles for a routine's channels, the routine named by the `OUT` address `sound-report.txt` shows for it:

```toml
[[game]]
name = "Some game"
file = "..."
[game.instruments]
"C694" = ["lead"]              # the routine whose OUT is at C694h: channel 0 lead
"881F" = ["bass", "lead"]      # channels 0 and 1
```

An override wins over the analysis; an unknown address or role name is an error the builder reports with the game's name (the pack is not written), so a typo isn't silently ignored. The report shows `override` for those channels.

## Verification

- Host: a synth test per instrument (the patch's spectrum and envelope against its table: e.g. the lead's second harmonic present, the pad's beating at +4 cents, the vibrato's depth and delay); the live rule on the corpus fixtures (roles stable, never switched mid-note); the decoder's kind-1 and `instrument[]` handling, including bad values; the builder's analysis and overrides (`fmzx-pack` tests).
- The pinned mixer and corpus hashes change (the sound changes) and are re-pinned; no decoder decision may change: the per-frame decisions of A and B are compared before and after on the corpus and must be identical.
- Firmware: the full emu-gate suite once at the end (sample-exact audio against the host, the frame budget with the new synth, RAMTEXT at most 48,864). Sample-exact means the device's C against the host's C, as today; the listening test's floating-point renderer is only the reference for the sound, not matched sample by sample.
- **Loudness** (my call): over the corpus, the FM output's peak and RMS stay within 1 dB of today's, measured on the mix (a whole tune), not per voice; the mix saturates as today, with no new clipping.
- **Listening:** before merging, the listening page is re-rendered from the real C synth (the mixer and synth through zx-sys, not the float renderer) (today vs the new instruments, same four tunes), for the owner to confirm it sounds like the D they chose.

## Left to the builder

The fixed-point forms of the patch fields, the vibrato and the second carrier; how the start-up functions reach RAMTEXT helpers from flash; the live rule's averaging time and hysteresis (starting points above); the loudness calibration; where the role per voice travels (a field of `snd_frame` or of the synth's voice); test names and fixtures.
