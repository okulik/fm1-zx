<!-- SPDX-FileCopyrightText: 2026 FM-1 ZX contributors -->
<!-- SPDX-License-Identifier: GPL-3.0-or-later -->

# Music generation

A personal pack can play a game's recorded tune with a bass, a pad and a drum written around its
melody by a music model ([BUILDING.md, "Arranged music"](../BUILDING.md#arranged-music) says how
to use it). This guide is for running experiments with that generation: trying another model,
other settings or another way to reduce a band to the FM-1's voices. It is written so that a
person or an AI agent can start from it with no other context: the pipeline and where each step
lives, the files between the steps, what the FM-1 can play, how to add and evaluate a generator,
what is known so far, and the rules for working on it. A score can also take its game's tune
from the game's C64 version, from a SID collection the owner has, or from its Spectrum 128
version, from a collection of `.ay` files ([BUILDING.md, "Music from other
versions"](../BUILDING.md#music-from-other-versions)); those paths share the files, the choice
and the pack step, and are described here alongside.

## Rules for working on it

- **Personal games never enter the repository.** Not the game files, not their `score.toml`, not
  the takes, renders or reports made from them. Work under `build/` (git ignores it; `tools/pack-studio`'s
  workspaces are `build/studio/<folder name>/`) or in a scratch folder outside the repository. Tests
  and fixtures are synthetic: invented tunes with invented names (the selftest's `test-game`).
- **Never name commercial games in repository files**, including the tunes of the owner's games:
  describe a tune by its shape (its length, its loop, its voices, the kind of routine that plays
  it) or by its score key. Do not name the composers of their tunes either.
- **Nothing from a SID or AY collection enters the repository**: no `.sid` or `.ay` file, no notes
  converted from one, no chip or ay take. The tunes belong to their authors and go only into the
  owner's personal packs. Tests build their own made-up SID and `.ay` files.
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
| 4. `score.toml` | Every `fmzx-pack personal` run (`tools/pack-studio`'s learning and each pack's build) writes each recorded score's notes, decoded from the learned list, and each shadow score's (with `shadow = true`) to `DIR/arrange/<game>/<key>/score.toml`. | `host/crates/fmzx-pack/src/arrange.rs` (`export`), called from `personal.rs` |
| 5. Generator | `tools/arrange-music generate DIR` runs a generator per take, reduces what it wrote to the FM-1's four voices, measures it and writes `take-<n>/` with MIDI and renders. | `tools/arrange-music` (`GENERATORS`, `arrange`, `metrics`) |
| 5b. C64 takes | `tools/arrange-music chip DIR --sid-dir PATH` finds the game's SID files in the owner's collection (file names, header titles, STIL), converts each candidate subtune with `fmzx-pack chip-notes` (a 6502 and C64 model capturing the SID registers once per play call, turned into notes), ranks them by their closeness to the score's tune (the judge's, step 6), and writes the best as `chip-<n>/` takes on their own timeline. `tools/arrange-music ay DIR --ay-dir PATH` does the same with the owner's `.ay` files (file names, misc strings, song names), each song's Z80 player run in the Spectrum machine core (its AY registers captured once per frame), as `ay-<n>/` takes. | `tools/arrange-music` (`cmd_chip`, `cmd_ay`, `own_score`), `host/crates/chipmusic`, `fmzx-pack chip-notes` |
| 6. Choice | `tools/arrange-music judge DIR` weighs every take and the original from their notes (closeness to the tune, quality, faults, provenance) and, with `--apply`, chooses where it is sure and the owner has not; the owner listens to the rest on `tools/arrange-music pick DIR` (or the studio) and chooses a take, a C64 or Spectrum 128 take, or the original. | `tools/arrange-music` (`judge_score`, `cmd_judge`, `cmd_pick`), `DIR/arrange/<game>/<key>/judge.toml`, `DIR/arrange/choices.toml` |
| 7. Pack | The next build (`tools/pack-studio`'s `fmzx-pack personal` runs) checks each chosen take, turns its notes into the record's events in place of the score's (trigger and fingerprints kept), caps each frame's events, checks the size, has method B's decoder take the list and replays the run that recorded the score (the learner's cache keeps each score's run) to see that it still plays. A chosen C64 or Spectrum 128 take becomes a replacement record instead: the score's trigger and fingerprints, precedence set, and the take's events on its own sections. A shadow score's chosen take is added after the scores with the precedence bit set, within 8 packed scores and 128 KB, and verified the same way. | `arrange.rs` (`arrange`, `apply`), `soundlearn::scores::verify`, `soundlearn::replace_scores`, `soundlearn::append_score` |
| 8. Device | FM: engines arms the score at its trigger edge, follows the beeper's edges against the recorded fingerprint, and plays the events on the FM synth with each voice's stored role (lead, bass, pad) and the FM drum. A score yields the frames a table entry claims; a score with precedence keeps them, the entry's voices unused there. | `core/sound/learned.c` (`snd_b_scores`), `core/sound/fm.c` |

The learner's results, each score's run among them, are cached (`DIR/sound-cache/`), so after the first build a run that only
changes choices learns nothing again, and `generate` needs nothing but the `score.toml` files: an
experiment can copy a pack's `DIR/arrange/` tree into a scratch folder and run there, leaving the
pack's own takes alone.

## The files

### score.toml (written by fmzx-pack personal)

    # A recorded score's notes, for tools/arrange-music (written by fmzx-pack personal).
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

### Notes from a SID or AY file (written by fmzx-pack chip-notes)

`fmzx-pack chip-notes <file.sid | file.ay> [--subtune N] [--seconds S] -o <notes.toml>` (S default
180; `fmzx-pack chip-notes --info <file>` prints the header) plays a PSID or RSID tune, or a song
of a ZXAYEMUL `.ay` file, told apart by the file's magic, and writes:

    file = "SOME/Some_Tune.sid"   # the file as named on the command line
    format = "PSID"               # PSID, RSID or AY
    subtune = 1                   # 1-based (the header's start song by default)
    title = "Some Tune"           # the header's (an .ay's: the song's name, else its misc string)
    author = "Someone"
    released = "1985 Somebody"    # an .ay's: its misc string when that is not the title
    clock_hz = 985248             # the chip's: 985,248 PAL or 1,022,727 NTSC; the 128K's AY 1,773,400
    seconds = 180
    intro_t = 1234560             # T-states at 3.5 MHz, from the first sounding row
    loop_t = 4567808              # one pass of a loop found; 0: none, the notes end at intro_t

    [[note]]
    voice = 0                     # 0 the lead, 1 the bass, 2 the pad, 3 the drum
    start_t = 0
    end_t = 488768
    period_t = 26956              # 3,500,000 / Hz; 0 for the drum
    level = 151                   # 1-255

The conversion (`host/crates/chipmusic`, `notes.rs`): a SID voice's frequency register gives its
pitch (register x clock / 2^24 Hz); a gate rise or retrigger starts a note, a pitch move of more
than 0.6 semitone while gated starts a new one (legato), and the gate going off, the test bit, no
waveform, volume 0 or a pitch outside 16-8,000 Hz ends it. Noise is a drum hit on voice 3 (up to
0.06 s, at least 2,048 T before the next). A run of 3 or more notes of at most 2.5 frames cycling
2-4 pitches (an arpeggio) becomes one note at its lowest pitch. Level: 255 x sqrt(volume / 15 x
(0.35 + 0.65 x sustain / 15)). The tonal SID voices take the roles by their mean pitch (highest
the lead, lowest the bass); a comment line says which SID voice became which. A loop is the
register stream repeating exactly to the end, at least twice, with a pass of a second or more and
a note in it; the notes then cover the intro and one pass, cut at the loop's end. Times are
rounded to 64 T, and at most 8 events a frame are kept with the pack's counting (drums dropped
first, then the pad, the bass, the lead). Filter, pulse width, ring modulation and sync are not
read; multi-SID tunes, BASIC RSIDs and tunes that call the KERNAL are refused with the reason.

An `.ay` song runs its Z80 player in the Spectrum machine core (the song's blocks in memory as
the format lays them out, its init, then its interrupt routine once a frame; the AY's ports
decoded as a 128K decodes them), its AY registers captured once a frame. A channel's tone period
gives its pitch (clock / (16 x period)); with the tone off, a repeating envelope plays as a buzzer
at the envelope's pitch. A level rising from 0, a retrigger, an envelope restart or a pitch move of
more than 0.6 semitone starts a note; noise is a drum hit as for a SID. Level: 255 for the
envelope, else 255 x sqrt(10^((volume - 15) x 3 / 20)) (the AY's steps are about 3 dB), at least
32. Arpeggios, roles, loops (the registers as heard: those a channel does not use count as 0),
rounding and the frame cap are as for a SID. A song is refused, with the reason, when it plays the
speaker and not the AY (a 48K beeper tune), drives an Amstrad CPC's or a SAM Coupé's ports, needs
a special player, or stays silent. `host/crates/chipmusic/src/notes.rs` holds the full rules.

### A C64 or Spectrum 128 take's arranged.toml (written by tools/arrange-music chip or ay)

The same file as an AI take's, in `chip-<n>/` (or `ay-<n>/`), with the score's header (its `key`, `intro_t` and
`loop_t`: the fingerprint's), voices 0-3 as an AI take's, and these fields:

    take = "chip-1"               # or the number 1
    generator = "chip sid"
    timeline = "own"              # the notes are on the tune's own timeline
    own_intro_t = 1234560         # T-states: its intro
    own_loop_t = 4567808          # one pass of its loop; 0: none, the notes end at own_intro_t
    source = "C64: Some Tune by Someone (1985 Somebody), subtune 1, SOME/Some_Tune.sid"
    sid_file = "SOME/Some_Tune.sid"
    subtune = 1
    stil = "..."                  # the STIL text that named the game, if any

    [metrics]
    closeness = 0.97              # to the score's tune, as the judge measures it (below)
    title_match = 1.0
    rank = 1.07                   # closeness + 0.1 title_match + 0.1 for stil_spectrum
    stil_spectrum = false
    lead_nps = 4.1                # notes a second per voice, as an AI take's

A Spectrum 128 take (`ay-<n>/`) differs only in `take = "ay-1"`, `generator = "chip ay"`,
`source = "Spectrum 128: <title> by <author>, song N, <file>"` (the song's name, else the file's
misc string), `ay_file` in place of `sid_file`, `subtune` the song (1-based), and no `stil` or
`stil_spectrum`.

The notes lie in `[0, own_intro_t + own_loop_t)`: the intro, then
one pass of the loop. Without a loop the converter's `intro_t` is the end (or 180 s). When the
score itself has no loop and the take's intro reaches past the score's end, fmzx-pack packs the
take's timeline only up to that end (its loop and later notes would never play). fmzx-pack
reads `key`, `intro_t`, `loop_t`, `take`, `timeline`, `own_intro_t`, `own_loop_t`, `source` and
the notes.

### takes.toml (written by tools/arrange-music, per score)

`score` (the first 16 hex digits of `score.toml`'s SHA-256), `model`, `seed`, and `[done.<n>]` per
take with its `generator`, `seconds` and the generator's `info` (JSON). A score has its takes when
the digest matches and takes 1 to N are done; a changed `score.toml` gets new takes.

### choices.toml (written by tools/arrange-music pick and judge --apply, and tools/pack-studio)

    [choices]
    "some-game/8951-1a2b3c4d" = 2           # the take; 0 or absent: the original
    "some-game/9A00-0b1c2d3e" = "chip-1"    # a C64 take, by its folder
    "other-game/3B10-1c2d3e4f" = "ay-1"     # a Spectrum 128 take, by its folder

    [judged]
    "some-game/9A00-0b1c2d3e" = "chip-1"    # a choice judge --apply made

Any other value in `[choices]` (a negative number, `"chip-0"`, `"ay-0"`, `"take-1"`, a float) fails
the pack's run with the file and the key named. fmzx-pack reads only `[choices]`. A choice is the
judge's while `[judged]` holds the same value for its key: `judge --apply` changes or takes back
only those, and a choice the owner makes (pick, its page, the studio's `c`) drops the key from
`[judged]`; an entry whose value no longer matches (the owner edited the file) is dropped at the
next write.

### judge.toml (written by tools/arrange-music judge, per score)

    version = 1
    score = "1a2b3c4d5e6f7a8b"   # score.toml's digest, as takes.toml's
    plays = 60.0                 # seconds the FM-1 plays of a take (the quality window)
    search = 150.0               # seconds of a take the closeness search looks through
    checkable_z = 16.2           # the original aligned with itself, in null standard deviations
    null_real = 30               # controls from real music (other games' takes, then originals)
    null_built = 0               # controls built from the score (pitches shuffled, made-up tunes)
    null = [0.12, 0.08, ...]     # the controls' raw alignments
    renderer = "music-preview"   # or "not checked: no <path> ..."
    pick = "chip-1"              # "0": the original
    confidence = "high"          # high, medium or low
    flags = []                   # the reasons it is not high
    margin = 0.42                # the pick's lead over the next, as a share of its combined
    order = ["chip-1", "0", "chip-2"]

    [[take]]
    name = "chip-1"
    digest = "..."               # its arranged.toml's (score.toml's for "0")
    closeness = 0.998
    z = 9.5                      # absent for the original
    raw = 1.43
    provenance = 1.0
    quality = 0.89
    polyphony = 0.93
    harmony = 0.79
    rhythm = 0.95
    fault = 1.0                  # the fault multiplier
    faults = []                  # in words: "refused by the renderer: ...", "plays only a fragment: ..."
    combined = 0.888
    rank = 1
    refused = ""                 # music-preview's refusal; "" when it plays or was not checked
    rendered = "..."             # the music-preview it was checked with; "": not checked
    source = "C64: ..."

The measures (`tools/arrange-music`'s docstring, "judge", has every constant):

- **Closeness**: the best Smith-Waterman local alignment of any of the original's melody lines
  (each tonal voice's and the top line, ornaments under 45 ms and stray notes over 14 semitones
  from the line's middle dropped, repeated pitches merged, its first 160 notes) with any of the
  take's (its loop repeated over the search window), by pitch class under all 12 transpositions,
  1 a match, -1 a mismatch, 1 a gap, plus 0.5 where the notes' lengths over their line's median
  agree within a factor of 1.5; per original note (a perfect copy about 1.5). As a z against the
  null and a logistic, 0.5 at z 3.
- **The null**: 30 takes of other games (another family, none from a file of the score's own
  takes), sampled with the score's id as the seed; with fewer in the workspace, other games'
  originals, then built controls, copies of the original with each voice's pitches shuffled and
  made-up tunes (a lead of varied phrases on a scale over a bass), alternately, their spread taken
  as at least 0.4 x their mean. Fewer than 10 real controls is a flag.
- **Quality**, over the window the FM-1 plays (the loop repeated): polyphony (voices sounding at
  once, 1 to 2.5, and whether they hold different pitch classes), harmony (graded consonance of
  each pair of voices sounding together, the Krumhansl-Schmuckler key fit, minor-second clashes of
  150 ms or more), rhythm (0.45 a moving bass below the lead, 0.25 drum hits a second, 0.3 the
  onsets' autocorrelation at beat lags), combined 0.3 / 0.35 / 0.35, times the fault multiplier.
- **Faults**, each a factor: refused by the renderer (0), mostly silent (sounding under 60% of
  the time), a silent gap over 2 s, notes held over 8 s, sound below MIDI 26 or above 100, a take
  shorter than 85% of the window (plays only a fragment).
- **Provenance**: 1 for a STIL entry naming the Spectrum; else the title match (1, 0.5 from 0.8,
  0.2) times the era (a C64 release year to 1990 1, later 0.15, a DEMOS file at most 0.3; a
  Spectrum 128 file under games/ 1, else 0.2).
- **Combined**: quality x (0.3 + 0.7 x max(closeness, 0.6 x provenance)); the original's
  closeness is 1.
- **Confidence**: low on a close call (the pick under 15% ahead of the next), or the original too
  plain to check (checkable_z under 5) with another take winning, or a refused pick; medium with
  any flag (the winner not shown to be the same tune, closeness under 0.5; a take with a partial
  match, closeness 0.2 to 0.8; fewer than 10 real controls); else high.

### The summary line (printed by tools/arrange-music generate, chip, ay and judge)

Each of these commands ends with one line of `key=value` fields, kept stable for
`tools/pack-studio`'s jobs view and for scripts:

    summary: kind=ai game=some-game scores=1 pending=1 takes=4 model=amt seconds=2172
    summary: kind=sid game=some-game scores=1 pending=1 takes=3 candidates=4 checked=3 refused=1 best=0.73 seconds=41 source=C64: <title> by <author> (<released>), subtune 1, <file>
    summary: kind=judge game=all tunes=67 owner=20 picked=4 applied=4 sure=4 flagged=43 faults=46 renderer=checked seconds=153

`kind` is `ai`, `sid` or `ay`; `game` the `--game` ids, comma-separated, or `all`; `scores` the
scores found, `pending` those worked on (the others had their takes), `takes` the takes written;
for `sid` and `ay`, `candidates` the (file, subtune) pairs that name the game, `checked` those
converted and compared, `refused` those the converter refused, `best` the closeness of the
take written with the best one (`-` with none) and `source` that take's source line, last and to
the end of the line (absent with no take). For `judge`: `tunes` the scores with takes judged,
`owner` those the owner chose for, `picked` those with the judge's choice, `applied` the
choices `--apply` wrote in this run, `sure` those without the owner's choice the judge is sure of, `flagged` those
with no choice it is not sure of (they need the owner's ear), `faults` the takes with a fault,
`renderer` `checked` or `skipped`. No other value holds a space. A command that stops on an error
prints none.

### The record

A chosen take becomes a kind-2 record's events: [pack-format.md, "Entry list"](pack-format.md#entry-list)
gives the record byte by byte, with the drum events and the player's rules. A chosen C64 or
Spectrum 128 take becomes a *replacement* record (bit 6 of the voices byte, with bit 7, the precedence): the score's
trigger, code, quiet and fingerprints (its key stays the score's), and its events on sections of
their own, `ev_intro_t = own_intro_t` and `ev_loop_t = own_loop_t` (`soundlearn::ScoreRecord`'s
`replacement`; `event_lengths()` gives the sections a record's events are timed in, a
replacement's or the fingerprint's). fmzx-pack builds the events as for an AI take on those
sections: notes cut at each section's end, tonal voices sounding at the loop's start restated
there, drum offs at least 64 T before the next hit, the 8-events cap counted on those sections'
frames, the size counted with them.

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
  notes sound like. A C64 or Spectrum 128 take is the one exception, by design: it is another arrangement with its
  own tempo and loop, so it plays on its own timeline (a replacement record) from the score's
  trigger, for as long as the beeper keeps to the recorded tune; it is not meant to line up with
  the beeper beat for beat.
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
- **The judge** (`tools/arrange-music judge DIR`, its table, and `judge.toml` above): which take
  each score would play, from the notes alone, with the reasons when it is not sure. It ranks
  takes; it does not replace listening where it says so.
- **The selftest**, `tools/arrange-music selftest`: about 100 checks on two invented scores with
  `rules` (the files, the four voices, the drum rules, no overlaps, the lead copied, determinism,
  resuming, `pick --set`, `--list`, the page's save). It needs no network and no model; keep it
  passing and add checks for what a change guarantees.
- **The pack side**: `cd host && cargo test -p fmzx-pack` runs `arrange.rs`'s unit tests and
  `tests/personal_arrange.rs` (a synthetic game whose take, and a C64 and a Spectrum 128 take on
  their own timeline, go into the pack and pass the replay, with the refusals); `cargo test -p
  chipmusic` runs the converter on SID and `.ay` files its tests assemble.
- **Listening**: `tools/arrange-music pick DIR` plays the original, each take on the FM-1's voices
  (`fm1.wav`) and everything the model wrote (`full.wav`), rendered with General MIDI instruments.
  The FM-1's own sound comes only from the packages: choose a take, build again
  (`tools/pack-studio <folder> --build --try`) and hear it in the browser test bench, or the owner
  installs it.
- **The source material**: `music-corpus --wav` (BUILDING.md, "Music corpus") writes each game's
  beeper and FM renders, which show what the learner made of its music before any arranging;
  `DIR/sound-report.txt` shows each recorded score (length, intro, loop, voices, notes, bytes) and
  what plays for it.

### An experiment, step by step

1. **Set up** a folder outside the repository's tracked files: build a personal pack into it
   (`tools/pack-studio <folder> --build -o build/exp`; `DIR` is then `build/exp/all`), or copy an
   existing pack's `arrange/` tree into a scratch folder. Note each score's key, length, loop and voices.
2. **Change one thing**: a generator (a new `@generator`, or the settings of one), or the
   reduction. Keep everything else as it is, so the comparison means something.
3. **Run** `tools/arrange-music selftest`, then on the same scores, with the same seed:
   `tools/arrange-music generate DIR --model rules --takes 4 --force`, the baseline you compare
   against (`amt`, or the generator before your change), and yours. `tools/arrange-music pick DIR
   --list` prints every take's figures.
4. **Pack** a few takes (`pick DIR --set GAME/KEY=N`, then `tools/pack-studio <folder> --build -o build/exp`)
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

### C64 and Spectrum 128 versions of the games' music

From a survey of 27 Spectrum games against the High Voltage SID Collection and chip music for the
AY chip (Spectrum 128K rips, Amstrad CPC register dumps):

- **Most games have a SID, few have the same tune.** 24 of the 27 had a C64 tune in the
  collection, 23 from the game's own C64 version. But the C64 tune is often another piece, or by
  another composer: "has a SID" does not mean "has the Spectrum tune". One game's tune is in the
  SID of its predecessor's C64 version (STIL says so); a few STIL entries name a Spectrum tune
  outright ("[from the Spectrum game]", a cover of a Spectrum title).
- **Telling the same tune from another.** Closeness (the judge's alignment, above) against an
  interval 4-gram overlap of the top lines, on synthetic changes of 42 recorded scores: a
  transposition, a tempo change with another key, the tune after 6 s of other music, the lead
  moved under another band, 15% of the notes dropped with octave slips and grace notes (all the
  same tune); an unrelated score and the same notes with their pitches shuffled (not). Closeness
  separates them with an AUC of 0.99 (the same tune scores about 1.0 in 90-95% of the cases), the
  4-gram overlap 0.72 (0.2 or more in under half of them). Heavily ornamented or very plain
  originals give little to align: the judge flags an original whose alignment with itself is
  under 5 standard deviations above the null.
- **The judge against the owner's choices.** On 20 scores the owner had chosen for (by ear,
  among the original and up to three C64 takes), the judge's pick was the owner's in 13, as often
  as always taking the first C64 take; where it was sure (high confidence) it agreed in 6 of 7,
  the seventh the owner's choice of a take the renderer refuses (the pack plays the original in
  its place), where the judge picks the original. It ranks well but does not replace the ear: hence
  `--apply` only where it is sure and the owner has not chosen.
- **Built controls for the null.** On 190 real takes, a null of built controls (half the
  original with its pitches shuffled, half made-up tunes) with its spread taken as at least 0.4 x
  its mean gave the same verdict (same tune or not) as the null of 30 real takes for every take;
  without that floor it called 5 unrelated takes the same tune.
- **AY sources.** AY sources exist for about half of the games (4 with a 128K AY rip, 13 with a
  CPC register dump, often closer to the Spectrum tune than the SID). The `.ay` path runs a
  Spectrum 128K rip's Z80 code in the Spectrum machine core and writes the same notes file
  (`fmzx-pack chip-notes`, `tools/arrange-music ay`); CPC register dumps are not read.
- **Most `.ay` files of the games are not 128K tunes.** Of the 799 game files of the chris-y/AY
  collection (`music/ay/games/`), 483 convert at their start song, 158 of those with a loop; 307
  are refused as 48K beeper tunes (their player drives the speaker, not the AY), and the rest are
  Amstrad CPC songs, speech or silent. None crashed the converter, and none took more than 0.82 s.
  A 48K game whose collection files are all beeper rips gets no ay take: its own beeper tune is
  what the score already holds.
- **The conversion holds up.** Against another SID emulator's register captures of 8 tunes, the
  converter's registers matched 100% in the median 2 s window, and its notes a second agreed
  mostly within a few percent; all 164 subtunes of 38 real files played, in 0.01-1.2 s per 180 s
  tune. Loops were found for about 10 of the 38 files at their start song, matching the
  collection's song lengths where it has them; tunes with free-running filter or pulse-width
  modulation never repeat exactly.

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
- A C64 or Spectrum 128 take's voices take their roles by mean pitch over the whole tune; a SID
  voice or AY channel that swaps between melody and accompaniment keeps one role. The 8-events cap is counted per section, so
  where a pass of the take's own loop ends and the next begins within one frame, the player can
  still drop an event.

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
