<!-- SPDX-FileCopyrightText: 2026 FM-1 ZX contributors -->
<!-- SPDX-License-Identifier: GPL-3.0-or-later -->

# Music generation

A personal pack can play a game's recorded tune with a bass, a pad and a drum written around its
melody by a music model ([BUILDING.md, "Arranged music"](../BUILDING.md#arranged-music) says how
to use it). This guide is for running experiments with that generation: trying another model,
other settings or another way to reduce a band to the FM-1's voices. It is written so that a
person or an AI agent can start from it with no other context: the pipeline and where each step
lives, the files between the steps, what the FM-1 can play, how to add and evaluate a generator,
what is known so far, and the rules for working on it.

## Rules for working on it

- **Personal games never enter the repository.** Not the game files, not their `score.toml`, not
  the takes, renders or reports made from them. Work under `build/` (git ignores it; the personal
  pack's default folder is `build/personal/`) or in a scratch folder outside the repository. Tests
  and fixtures are synthetic: invented tunes with invented names (the selftest's `test-game`).
- **Never name commercial games in repository files**, including the tunes of the owner's games:
  describe a tune by its shape (its length, its loop, its voices, the kind of routine that plays
  it) or by its score key.
- **Repository text describes the current state only**: no history, no dates, no "new" or
  "added". An experiment's story belongs in its report outside the repository; what ships is
  described as it is.
- **The owner runs anything on the device**, and makes every commit. An experiment ends with files
  and figures the owner can listen to and judge, not with a claim that it sounds good.
- **Keep the tool offline-capable and pinned**: dependencies go in the script's PEP 723 block at
  exact versions (a git dependency at a commit), `--offline` must keep working from the caches, and
  every third-party package or model the tool downloads is listed in
  [LICENSING.md](../LICENSING.md#the-arranging-tool) with its licence as its source declares it.

## The pipeline

| Step | What happens | Where |
|---|---|---|
| 1. Listening run | The game runs on the host for 3 minutes of emulated time under fixed inputs (or the key script `<name>.keys`); a run per start key is compared and the one that hears the most music is kept. | `host/crates/soundlearn/src/listen.rs`, `learn.rs` |
| 2. Routines | The learner reads a routine's code (`model/`, `gate.rs`), or observes a routine that mixes channels into the one speaker bit by testing the pitches its loop's values predict against the speaker's spectrum (`observe.rs`, `spectrum.rs`), or the routine is one of the FM-1's built-in entries (`core/sound/engines_builtin.c`). The FM-1 reads those notes live. | `host/crates/soundlearn/src/` |
| 3. Recorded scores | Every heard routine whose `OUT` sounded in 100 frames or more (learned, built in or neither) has its music recorded: each stretch of at least 100 frames (2 s) from a trigger edge; a run per key for a menu of tunes; the same tune kept once; the loop found on the speaker edges (an intro and one pass of the loop); the notes transcribed from the speaker's sound (up to 3 voices, at least 8 notes over 3 pitches); encoded as a kind-2 record and verified by replaying the run through method B. A stretch the table (learned and built-in entries) claims at most a tenth of the sounding frames of is a *score*, packed; one it claims more of is a *shadow score*: verified with the precedence bit set, kept in the learner's result and cache, not packed. | `scores.rs`, `learn.rs`, `transcribe.rs`, `entry.rs` |
| 4. `score.toml` | Every `tools/make-personal-pack` run writes each recorded score's notes, decoded from the learned list, and each shadow score's (with `shadow = true`) to `DIR/arrange/<game>/<key>/score.toml`. | `host/crates/fmzx-pack/src/arrange.rs` (`export`), called from `personal.rs` |
| 5. Generator | `tools/arrange-music generate DIR` runs a generator per take, reduces what it wrote to the FM-1's four voices, measures it and writes `take-<n>/` with MIDI and renders. | `tools/arrange-music` (`GENERATORS`, `arrange`, `metrics`) |
| 6. Choice | The owner listens on `tools/arrange-music pick DIR` and chooses a take or the original. | `tools/arrange-music` (`cmd_pick`), `DIR/arrange/choices.toml` |
| 7. Pack | The next `tools/make-personal-pack` run checks each chosen take, turns its notes into the record's events in place of the score's (trigger and fingerprints kept), caps each frame's events, checks the size, has method B's decoder take the list and replays the run that recorded the score (the learner's cache keeps each score's run) to see that it still plays. A shadow score's chosen take is added after the scores with the precedence bit set, within 8 packed scores and 128 KB, and verified the same way. | `arrange.rs` (`arrange`, `apply`), `soundlearn::scores::verify`, `soundlearn::replace_scores`, `soundlearn::append_score` |
| 8. Device | FM: engines arms the score at its trigger edge, follows the beeper's edges against the recorded fingerprint, and plays the events on the FM synth with each voice's stored role (lead, bass, pad) and the FM drum. A score yields the frames a table entry claims; a score with precedence keeps them, the entry's voices unused there. | `core/sound/learned.c` (`snd_b_scores`), `core/sound/fm.c` |

The learner's results, each score's run among them, are cached (`DIR/sound-cache/`), so after the first build a run that only
changes choices learns nothing again, and `generate` needs nothing but the `score.toml` files: an
experiment can copy a pack's `DIR/arrange/` tree into a scratch folder and run there, leaving the
pack's own takes alone.

## The files

### score.toml (written by tools/make-personal-pack)

    # A recorded score's notes, for tools/arrange-music (written by tools/make-personal-pack).
    game = "some-game"          # the game's id in personal-games.toml
    name = "Some game"
    key = "8951-1a2b3c4d"       # the score key, below
    clock_hz = 3500000
    intro_t = 434175            # T-states from T0 (the trigger edge)
    loop_t = 3125448            # one pass of the loop; 0: no loop
    voices = 3                  # tonal voices, 1-3
    roles = ["lead", "bass", "pad"]   # one per tonal voice, "" for none
    shadow = true               # a shadow score's alone (comment lines above it say so)

    [[note]]
    voice = 0                   # 0-2
    start_t = 1234              # T-states since T0
    end_t = 56789               # exclusive
    period_t = 26746            # the full period in T-states: the pitch is clock_hz / period_t Hz
    level = 116                 # 1-255

One timeline: the intro from 0 to `intro_t`, then one pass of the loop from `intro_t` to `intro_t +
loop_t` (a loop note at `intro_t` plus its time in the loop). A note runs from its note on to its
voice's next note on or note off, or to its section's end; a voice sounding at the loop's start
shows a note starting at `intro_t` (the device restates it there). Notes are sorted by start, then
voice. Times are multiples of 64 T within each section, levels multiples of 4 (the record's steps),
periods the record's pitch table entries: exactly what the FM-1 plays.

A **shadow score** (`shadow = true`) is music a learned or built-in routine plays live: the same
file and the same takes as any score's; with no take chosen the routine plays it, and a chosen take
goes into the pack as a score with precedence that plays in its place. `tools/arrange-music`
generates its takes like any score's, and `pick` marks it `(replaces the live version when
chosen)`.

The **score key** is `<site>-<crc>`: the score's `OUT` address in 4 uppercase hex digits and the
CRC-32 (IEEE) of its intro fingerprint's bytes followed by its loop fingerprint's, 8 lowercase hex
digits (`soundlearn::score_key`). It stays the same while the learner records the same music, and
ends the score's `recorded score` line in `sound-report.txt` (`(score 8951-1a2b3c4d)`; a shadow
score's `shadow score` line, `(score 8951-1a2b3c4d; packed only with a chosen take)`).

### arranged.toml (written by tools/arrange-music, one per take)

`score.toml`'s header (with `voices = 3` and `roles = ["lead", "bass", "pad"]`), then `take` (n),
`generator` (the model and its settings, as one string), a `[metrics]` table and the notes:

    take = 1
    generator = "amt stanford-crfm/music-medium-800k staged drums,bass,chords top_p=0.95 seed=1"

    [metrics]
    key = "D# maj"
    key_r = 0.686
    bass_in_key = 1.0
    bass_consonance = 0.791
    pad_in_key = 0.976
    pad_consonance = 0.561
    on_lead_onsets = 0.219
    lead_nps = 3.86
    bass_nps = 11.21
    pad_nps = 14.71
    drums_nps = 14.14
    generated = 1176

    [[note]]
    voice = 1
    start_t = 210000
    end_t = 385000
    period_t = 142859
    level = 122

Voices: 0 the lead (the score's voice whose role is lead, the longest-sounding of several; else its
highest voice on average; its notes copied as they are, trimmed so they do not overlap), 1 the
bass, 2 the pad, 3 the drum (`period_t = 0`; `level` the hit's strength; a hit from `start_t` to
`end_t`). Notes on one voice never overlap, all lie in `[0, intro_t + loop_t)`, and are sorted by
start, then voice. The file holds no timings, so the same take is the same bytes; timings go to
`takes.toml`. fmzx-pack reads `key`, `intro_t`, `loop_t`, `take` (if present) and the notes; it
ignores the other header fields.

### takes.toml (written by tools/arrange-music, per score)

`score` (the first 16 hex digits of `score.toml`'s SHA-256), `model`, `seed`, and `[done.<n>]` per
take with its `generator`, `seconds` and the generator's `info` (JSON). A score has its takes when
the digest matches and takes 1 to N are done; a changed `score.toml` gets new takes.

### choices.toml (written by tools/arrange-music pick)

    [choices]
    "some-game/8951-1a2b3c4d" = 2     # the take; 0 or absent: the original

### The record

A chosen take becomes a kind-2 record's events: [pack-format.md, "Entry list"](pack-format.md#entry-list)
gives the record byte by byte, with the drum events and the player's rules.

## What the FM-1 can play

- **Voices.** 3 tonal voices (`SND_TONAL`) and one drum. A tonal voice plays one note at a time; its
  instrument is its stored role (lead, bass or pad), which the record holds per voice. There is one
  drum sound: a hit has a strength (63 steps) and nothing else, so drum kinds cannot be told apart
  on the device.
- **The drum strikes only from silence** (`fm.c`'s `drum_set`): a hit while the drum sounds does not
  strike again, so each drum note is a hit and an off. `tools/arrange-music` keeps each hit to at
  most 50 ms, at least 40 ms after the one before (the stronger kind wins a clash: kick, snare,
  clap, tom, hat, cymbal), ending at least 128 T before the next; fmzx-pack moves an off to 64 T
  before the next hit where needed and leaves out a hit with no room. At the start of each pass of
  the loop the player releases the drum if it sounds, so a hit at a pass's start always strikes;
  the drum is never restated at the loop's start.
- **At most 8 events a frame** (`SND_EVENTS`, every voice's note ons and offs together, per
  69,888 T); the player drops the events past that. fmzx-pack counts the take's events per frame
  of each section and drops drum notes, then pad notes, the latest first, until each frame fits; a
  frame over 8 on the lead and the bass alone refuses the take. The player's own releases and
  restatements at a pass's start are not counted, so a dense take can still lose an event there.
  `tools/arrange-music` keeps the count down by dropping bass and pad pieces under 30 ms.
- **Levels.** Files hold 1-255; the record stores a step, (level + 2) / 4 clamped to 1-63, and the
  FM-1 plays step x 4 + 3. `tools/arrange-music` gives the bass the lead's median level and the pad
  0.7 of it, and the drum a level by kind (kick 200, snare 170, clap and tom 130, cymbal 96, hat 80,
  others 110).
- **Pitches.** A tonal note's period is 64 to 16,777,215 T (a full period at 3.5 MHz). Each record
  stores up to 255 distinct periods; past that they are merged to the nearest kept one by cents.
  The lead keeps the transcribed periods (not snapped to a scale); generated notes get equal-tempered
  periods from their MIDI pitch.
- **Timing.** T-states at 3.5 MHz from the trigger edge; a frame is 69,888 T (about 50 a second);
  the record stores event times in 64-T units from each section's start, and fmzx-pack rounds to
  the nearest. A note reaching its section's end is cut there (the player releases or restates it).
- **Why the timeline must stay the original's.** The FM-1 does not follow a score by its notes but
  by its speaker edges: it arms at the trigger edge, compares each frame-long bucket of the
  beeper's edges with the recorded fingerprint (within max(2, recorded / 8)), stops after three
  buckets in a row disagree, and goes round the loop pass after pass for as long as the beeper
  agrees. The notes play by the clock from T0, each pass from `intro_t + k x loop_t`. So the FM
  music lines up with the game only because the take's times are the beeper's: a take that changed
  the tempo, the intro's length or the loop's would drift from the game, and fmzx-pack refuses a
  take whose key, `intro_t` or `loop_t` differs from the score's. The replay at pack time confirms
  that the decoder takes the list and the score still plays where it did; it cannot judge what the
  notes sound like.
- **Size.** A game's packed scores take at most 128 KB together (`SCORES_BYTES_MAX`), and a game
  packs at most 8 (`SND_SCORES_MAX`); shadow scores count only once a take of theirs is packed. A
  take usually holds more notes than the score, and its bytes count toward the pack's room.
- **Shortest.** A stretch is recorded from 100 frames (2 s) on; a stretch with no repeat heard
  (a loop needs two passes of at least 50 frames each) is all intro.
- **Precedence.** A shadow score's take is packed with bit 7 of its record's voices byte set
  (docs/pack-format.md): the FM-1 then gives it every frame it plays, those its routine claims
  too. Without the bit the routine's entry would keep its frames, and the replay at pack time
  (which needs at least 90% of the stretch's sounding frames for the score) would refuse it.

## Adding a generator

A generator is one function in `tools/arrange-music`, registered by name:

```python
@generator("mine")
def mine_take(score: Score, take_index: int, rng: random.Random, settings: Settings) -> Take:
    notes = []
    for s, e, midi in ...:                       # seconds from the timeline's start
        notes.append(GenNote(s, e, 33, midi, "bass"))         # GM program, MIDI pitch, part
    notes.append(GenNote(1.0, 1.05, 128, 36, "drums"))       # program 128: drums; pitch: GM drum key
    return Take(notes, f"mine seed={settings.seed} ...", {"anything": "for takes.toml"})
```

- `score`: the parsed `score.toml` (`Score`: `header`, `notes` as `Note(voice, start_t, end_t,
  period_t, level)`, and helpers: `seconds`, `sec(t)`, `tstate(s)`, `tonal()`, `midi(note)`,
  `melody_voice()`, `roles`, `total_t`).
- `take_index` counts from 1. `rng` is `random.Random(f"{model}/{seed}/{game}/{key}/{take}")`: draw
  every random choice from it (and seed any library's generator from it) so that the same seed
  gives the same take. The `amt` generator samples on the CPU from a seeded `torch.Generator`,
  because sampling on the GPU is not reproducible.
- `settings` (`Settings`): `model`, `takes`, `seed`, `offline`, `soundfont`, `amt_model`, and
  `log(msg)` for progress lines. A generator that downloads anything must honour `offline`.
- It returns `Take(notes, generator, info)`: `GenNote(start, end, program, pitch, part, velocity=90)`
  with times in seconds from the timeline's start (0 is T0, `score.seconds` its end), `part` one of
  `"bass"`, `"chords"`, `"drums"`. Write as many notes as you like, overlapping: the shared
  `arrange()` makes voice 1 the lowest bass note and voice 2 the highest chord note at each 10 ms
  step, voice 3 one drum hit at a time within the drum rules above, and copies the lead.
- `--model mine` then picks it; an unknown `--model` lists the names. `estimate_seconds` gives the
  first time estimate (only `amt` has one).

The shared code does the rest: the reduction, the metrics, `arranged.toml`, the MIDI files, the
renders, `takes.toml`, resuming and `--jobs`. To change the reduction itself (another voice
choice, levels, drum handling), change `arrange()`; the selftest checks its output's invariants.

The two generators to compare against:

- **`rules`**: no model. It estimates a beat from the melody's onsets, picks a chord span of 2 or
  4 beats, puts the bass on each span's strongest pitch class and a triad above it, and plays a
  rock, half-time or four-on-the-floor pattern with eighth-note hats. Any experiment should beat
  it by ear.
- **`amt`**: the Anticipatory Music Transformer (`stanford-crfm/music-medium-800k`, the
  `anticipation` package) in three passes, drums `{128}`, then bass (GM 32-39), then chords (GM
  0-7, 24-31, 48-51). Each pass is held to its class by a mask on the instrument tokens and hears
  every original note (as GM 80, a square lead) and the passes before it as anticipated controls.
  It generates in 30 s windows (a pass that stops early resumes at the next), with times moved
  0.1 s later because the library drops controls in its first 50 ms. `top_p` cycles 0.95, 0.98,
  0.92, 0.96 by take. Its `add_token` replacement reuses the model's key-value cache for the
  second and third token of each note, which matched a full recompute to within 3e-4 on logits up
  to 122 (the same choices) in under half the time.

## Evaluating

- **The metrics** (`[metrics]`, printed by `generate` and `pick --list`): `key` and `key_r` (a
  Krumhansl-Schmuckler key from the original's notes and its correlation); `bass_in_key`,
  `pad_in_key` (the share of the voice's sounding time on the key's scale); `bass_consonance`,
  `pad_consonance` (the share of 10 ms steps where the voice and the lead sound a unison, third,
  fourth, fifth or sixth apart; random notes give about 0.58); `on_lead_onsets` (the share of
  bass, pad and drum onsets within 30 ms of a lead onset); `lead_nps`, `bass_nps`, `pad_nps`,
  `drums_nps` (notes a second); `generated` (notes the generator wrote). They catch failures (a
  chord part droning a wrong note, an empty voice, a runaway density); they do not say which take
  sounds best.
- **The selftest**, `tools/arrange-music selftest`: about 100 checks on two invented scores with
  `rules` (the files, the four voices, the drum rules, no overlaps, the lead copied, determinism,
  resuming, `pick --set`, `--list`, the page's save). It needs no network and no model; keep it
  passing and add checks for what a change guarantees.
- **The pack side**: `cd host && cargo test -p fmzx-pack` runs `arrange.rs`'s unit tests and
  `tests/personal_arrange.rs` (a synthetic game whose take goes into the pack, with the refusals).
- **Listening**: `tools/arrange-music pick DIR` plays the original, each take on the FM-1's voices
  (`fm1.wav`) and everything the model wrote (`full.wav`), rendered with General MIDI instruments.
  The FM-1's own sound comes only from the packages: choose a take, run `tools/make-personal-pack`
  again and `--try` it in the browser test bench, or the owner installs it.
- **The source material**: `music-corpus --wav` (BUILDING.md, "Music corpus") writes each game's
  beeper and FM renders, which show what the learner made of its music before any arranging;
  `DIR/sound-report.txt` shows each recorded score (length, intro, loop, voices, notes, bytes) and
  what plays for it.

### An experiment, step by step

1. **Set up** a folder outside the repository's tracked files: build a personal pack into it
   (`tools/make-personal-pack <folder> -o build/exp`), or copy an existing pack's `arrange/` tree
   into a scratch folder. Note each score's key, length, loop and voices.
2. **Change one thing**: a generator (a new `@generator`, or the settings of one), or the
   reduction. Keep everything else as it is, so the comparison means something.
3. **Run** `tools/arrange-music selftest`, then on the same scores, with the same seed:
   `tools/arrange-music generate DIR --model rules --takes 4 --force`, the baseline you compare
   against (`amt`, or the generator before your change), and yours. `tools/arrange-music pick DIR
   --list` prints every take's figures.
4. **Pack** a few takes (`pick DIR --set GAME/KEY=N`, then `tools/make-personal-pack ... -o DIR`)
   and read `sound-report.txt`: takes refused, notes dropped for full frames.
5. **Report**, outside the repository: what changed and why; the generator strings; the scores by
   key, length and shape (never by game); a table of the figures per take for each generator; the
   time per take and the machine; the refusals and drops; the files to listen to (`fm1.wav`,
   `full.wav`, `original.wav` paths). The owner listens and decides.
6. **To keep it**: pin any dependency in the PEP 723 block, add selftest checks, describe it in the
   tool's docstring, BUILDING.md ("Arranged music") and this guide, and list any downloaded
   package or model in LICENSING.md.

## What is known so far

### Transcription: what the generator hears

The score's notes are read from the speaker's one bit by `transcribe.rs`, so the generator works
from a transcription, not from the game's data:

- **Counter engines** (channels as pulse trains from counters reloaded each note) transcribe well:
  on a four-channel counter tune, 86% of the audible notes found frame by frame and 99% of what
  was found real (counting the fourth channel).
- **Phase-accumulator (pulse-width) engines** transcribe worse: 66% of the audible notes found and
  54-72% of what was found real; octave errors in 16-24% of the frames, and harmonics of real
  voices read as extra notes. The jitter of a pass-quantised pulse train smears the spectrum.
- **3 voices at most**: on a four-channel tune the fourth competes for a voice, and which channel
  keeps it can change from frame to frame.
- **Timing**: an 8-frame (160 ms) window and a 2-frame shortest note, so notes under about 4 frames
  are smeared and an arpeggio of 1-2 frame notes reads as a chord.
- **Pitches are measured, not snapped**: the speaker's real pitch, often about 19 cents under the
  nominal on interrupt-driven engines (the interrupt takes time from the music loop). Narrow-pulse
  basses come out quiet, because they are.
- Stray notes (very high or low pitches, octave slips) muddy the key the model infers.

### The model's behaviour

From a trial on four beeper tunes (15-35 s each, dense to sparse, 174 to 690 original notes),
generating with the original as controls:

- **Unconstrained, the model adds another lead.** In all 8 takes it wrote one more square or chiff
  synth lead line: no bass, no chords, no drums; on one tune almost nothing (1 note a second). It
  reads a dense square-wave line as a solo chiptune and continues the style.
- **Banning synth leads** gave one solo line on another patch (crystal, shakuhachi, brass,
  saxophone); one copied the melody note for note (99% of its onsets on melody onsets).
- **One mask allowing a band** (pianos, guitars, basses, strings, drums): the model stays with the
  first instrument it picks, so drums only, or drums and bass, or 34 piano notes.
- **Staged passes, drums then bass then chords**, each held to its class and hearing the passes
  before it, is the only setting that gave all three parts. Consonance with the melody 0.60-0.79
  against 0.58 for random notes; bass lines followed the melody's pitch centre. Chord parts were hit
  or miss: one drew 256 notes on a single pitch class outside the melody's key (in key 0.17).
- **Rhythm**: 50-60% of generated onsets within 30 ms of a melody onset (73% on one tune); drum
  grids were steady and found the tempo, but one drum pass stopped at 14 s of 35 and another was
  mostly one conga.
- **top_p 0.95 against 0.98** mattered less than the instrument constraint; within a setting,
  density varies more by chance than quality does. In the tool's run on one 14 s score, two takes
  gave 1176 and 180 generated notes.
- **Figures cannot pick the take.** The trial's choices were made on the figures alone and some
  are doubtful by construction (a "bass" taken from a guitar's low notes, a conga for a drum part);
  takes have to be picked by ear, which is why `pick` exists.
- **Speed**: the `anticipation` library recomputes the whole 1024-token context for every token
  (3 a note): 0.25-0.4 s a note on Apple M4 Max MPS. With the cached `add_token`, 0.11-0.15 s. The
  model download is about 1.4 GB.

### Open questions in the current tool

- The loop pass is generated as plain continuous music; nothing makes its end lead back to its
  start, so the seam can jump.
- Scores up to 60 s have run with `amt`; longer ones are untested. The model's time tokens span
  100 s, and the library keeps times relative to its context window.
- The model can keep writing notes at one moment without moving its clock on, so a pass would
  never end: a generation window ends where it is after `AMT_STALL` (16) notes in a row at one
  moment, or past `AMT_NOTES_PER_S_MAX` (40) notes a second of music (the take's `stages` say how
  many windows were cut). Below that cap there is no density control: a dense take can hold over 14 pad notes
  a second, which the frame cap then thins.
- Voice 0 keeps only the score's lead voice; an original bass or pad voice is replaced.

## Ideas

Ideas to try, not plans:

- **Other models**: MIDI-GPT (built for infilling bars and tracks under attribute controls, which
  fits "write a bass for these bars"); MIDI-RWKV (an RWKV-family symbolic model, cheaper on long
  sequences); text-conditioned symbolic models (a style asked for in words). Check each model's
  licence and its time resolution against the 10 ms the reduction works on.
- **Use the loop structure**: generate over two passes and keep the second, or give the pass's
  start as the context of its end, so the seam joins; generate the loop once and the intro around
  it.
- **Per-game style prompts**: a short history prompt in the target style before the controls (the
  trial did not try one), or a per-game choice of instruments and density.
- **Density and fit**: cap notes a second per part, align bass and drum onsets to the melody's beat
  grid, reject chord parts outside the key before writing a take, spend the 8 events a frame on
  purpose instead of letting the cap drop them.
- **Keep more of the original**: map an original bass voice to voice 1 when the generator writes
  none, or offer a take that only adds drums.
- **More voices**: the device plays 3 tonal voices and one drum; more would be a firmware and
  format change (the player's state, the event format, the frame's event budget) on a firmware whose
  RAM is nearly full.
- **Drum kinds**: the FM-1 has one drum sound with a strength; kinds (kick, snare, hat) would need
  drum sounds in `fm.c` and room in the drum event (its 6 bits are the strength).
