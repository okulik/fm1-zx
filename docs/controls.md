<!--
SPDX-FileCopyrightText: 2026 FM-1 ZX contributors
SPDX-License-Identifier: GPL-3.0-or-later
-->

# Controls

The FM-1's 14 buttons, 27 note keys and 7 encoders stand in for the Spectrum keyboard and a Kempston joystick.

## Reserved buttons

| Button | Use |
|--------|-----|
| HOME   | Opens the menu (and goes back) |
| PLAY   | Pauses and resumes the game; OK in the menu; presses the on-screen keyboard's key |

A game cannot map HOME or PLAY.

## Default mapping

Every game starts from this table. A game's manifest can replace any entry (see below).

| FM-1 input | Action |
|------------|--------|
| F3, G3, A3, B3 (the first four white keys) | Kempston down |
| F#3, G#3, A#3 (the first three black keys) | Kempston up |
| C#4, C#5 | Kempston left |
| D#4, D#5 | Kempston right |
| C5, D5, E5, F5, G5 | Kempston fire |
| C4 | ENTER |
| D4 | SPACE |
| E4 | Y |
| F4 | N |
| OCT-, OCT+, FX, SCL, ENV | 1, 2, 3, 4, 5 |
| LFO, EDIT, GLO | 6, 7, 8 |
| SAVE | 9 |
| ARP | 0 |
| SEQ | CAPS SHIFT |
| REC | SYMBOL SHIFT |
| Encoders | no action |

Other notes do nothing by default.

## Manifest key syntax

Each game in the pack manifest can have a `[game.keys]` table. Each entry names an input and gives one or two actions joined by `+`. A listed input replaces that input's default actions; all other inputs keep the default.

Inputs:

- `button.<OCT-|OCT+|FX|SCL|ENV|LFO|EDIT|GLO|SAVE|ARP|SEQ|REC>`
- `note.<F3 ... G5>`, chromatic, sharps written `F#3`
- `enc.<SELECT|ALGORITHM|K1|K2|K3|K4|PRESETS>.<cw|ccw>`

Actions:

- `key.<A-Z|0-9|ENTER|SPACE|CAPS|SYM>`
- `kempston.<left|right|up|down|fire>`
- `none`, which clears the input (it cannot be combined with another action)

An encoder detent presses its action for 3 frames; several detents in one poll queue up one after another.

Example:

```toml
[[game]]
id = "cave-runner"
name = "Cave Runner"
file = "games/cave-runner.sna"

[game.keys]
"note.F3" = "key.O"                    # left
"note.A3" = "key.P"                    # right
"note.C4" = "key.SPACE"                # jump
"button.FX" = "key.CAPS+key.SPACE"     # BREAK
"enc.K1.cw" = "kempston.right"
"enc.K1.ccw" = "kempston.left"
```

## Key ids

The ids fm1-emu (and the firmware's matrix scan) use, as in felucca-bsp's `FM1_KEYMAP`:

| Ids | Input |
|-----|-------|
| 0-13 | buttons: OCT- 0, OCT+ 1, FX 2, SCL 3, ENV 4, LFO 5, EDIT 6, GLO 7, HOME 8, SAVE 9, ARP 10, SEQ 11, PLAY 12, REC 13 |
| 14-40 | note keys, id = 14 + note (F3 = 14 ... G5 = 40) |
| encoders 0-6 | SELECT, ALGORITHM, K1, K2, K3, K4, PRESETS (separate numbering, clockwise = positive) |

## Picker, menu and on-screen keyboard

Buttons act on their press.

**Game picker.** SELECT and ALGORITHM turns and OCT-/OCT+ scroll (clockwise and OCT+ go to the
next game); PLAY starts the selected game.

**In the game.** Every frame, the held inputs go through the game's key map to the Spectrum
keyboard and the Kempston joystick. PLAY pauses the game ("PAUSED" in the top border) and resumes
it; HOME opens the menu.

While the game is paused or in the menu or on the key map page, it does not run: no Spectrum
frames, no sound, and every Spectrum key and Kempston direction is up. A key still held when the
game resumes counts again from the next frame.

**HOME menu.** SELECT and ALGORITHM turns and OCT-/OCT+ move (clockwise and OCT+ down), PLAY picks
the item, HOME resumes the game.

| Item | Does |
|------|------|
| Resume | Back to the game |
| Reset game | Loads the game again from the start |
| Keyboard | Opens the on-screen keyboard over the game |
| Key map | The game's key map, a page at a time (turns and OCT-/OCT+ page, HOME back to the menu) |
| LCD speed: *n* | The LCD's SPI clock, 60 MHz / (*n* + 1); each press steps 4, 2, 1, 0, then 4 again. Faster sends more of the screen each frame (150,000 / (*n* + 1) bytes). |
| Sound: *choice* | How the game's sound is played. **Beeper** is the Spectrum's own sound, with no delay. **FM: sound** plays music through the FM-1's synth where the program is sure of the notes (the rest stays on the beeper; a tune's first note too, since a lone tone could be an effect, so FM starts with the second note), and delays all sound, music and effects alike, by 60 ms (3 frames). **FM: engines** (the default) plays the music of known routines (Castaway and Colony 8 titles, BASIC `BEEP` notes of 20 ms or more, Cave Runner, Unison), and of the routines a personal pack learned for its games (BUILDING.md, "Your own games"), as FM, reading the notes from memory and checking them against the beeper; anything it does not recognise, or that fails the check, stays on the beeper, and it switches to FM on its first recognised frame. In FM, each tonal voice plays a bass, lead or pad instrument by its role in the tune (and the drum a drum): the FM-1 ranks the voices by pitch as they play (the lowest the bass, the highest the lead, a third the pad, a lone voice the lead), and a personal pack can store the roles of its games' routines, analysed when it is built or chosen by the owner (BUILDING.md, "Your own games"), which win; a voice changes instrument only when a note starts on it. All FM choices share the same 60 ms delay, and hold FM through a single unsure frame inside the music, so the music does not drop to the beeper for a moment. PLAY cycles Beeper, FM: sound, FM: engines. The choice stays through Reset game, Back to games and starting another game, but not through power-off. Turning FM on fades the sound out briefly and leaves a 60 ms gap before it returns; turning it off crossfades back in a few ms. |
| Screen: *choice* | How the 256-pixel-wide Spectrum picture fits the 240-pixel panel. **Crop** shows 240 of the 256 columns, from where each game's crop puts them, and no side border. **Fit** shows all 256 columns by leaving out one column in 16, evenly spaced; the colours stay the Spectrum's own. **Fit smooth** (the default) shows all 256 columns by blending neighbouring columns, so thin lines and text keep their weight. **Border** scales the picture with 8 border pixels each side down to 240 x 212, blended, with the border colour above and below. In every choice the menu, the keyboard and PAUSED look the same. PLAY cycles Crop, Fit, Fit smooth, Border. The choice stays through Reset game, Back to games and starting another game, but not through power-off. |
| Back to games | Stops the game and returns to the picker |

**On-screen keyboard.** The Spectrum keyboard in the bottom five text rows, while the game keeps
running above it.

| Control | Does |
|---------|------|
| SELECT turn | Cursor left / right (counter-clockwise / clockwise) |
| ALGORITHM turn | Cursor up / down |
| PLAY | Presses the key under the cursor; it stays down while PLAY is held |
| SAVE | CAPS SHIFT for the next key (press again to cancel) |
| REC | SYMBOL SHIFT for the next key |
| HOME | Closes the keyboard |

The game's other mapped inputs keep working under the keyboard; PLAY, SAVE, REC and the SELECT
and ALGORITHM encoders belong to the keyboard while it is open.

## Computer keyboard (browser page)

The browser test bench (`tools/web-demo`) knows which game runs (the firmware's view and game index,
read from `fmzx_stats` by the emulator). While a game runs, the computer keys type Spectrum keys:
a key presses the FM-1 input the running game maps to that Spectrum key. Keys are matched by their
place (`event.code`), so a non-US layout types the key printed in that place on a US keyboard.

| Computer key | Spectrum key |
|--------------|--------------|
| A to Z, 0 to 9 | the same letter or digit |
| Space | SPACE |
| Enter | ENTER (PLAY in the picker and the menus) |
| Left Shift | CAPS SHIFT |
| Alt (Option) | SYMBOL SHIFT |
| ← → ↑ ↓ | Kempston left, right, up, down |
| Right Shift | Kempston fire |

- When several FM-1 inputs give the same Spectrum key, the first in the key map is used (a key or
  note before an encoder; an encoder turns one detent per key press and repeat).
- An input mapped to two actions (`key.CAPS+key.5`) is pressed by holding both computer keys (Left
  Shift and 5).
- A Spectrum key the game does not map does nothing, as on the FM-1.
- In every view **Esc is HOME** and **Tab is PLAY** (pause in a game, OK in the menus).
- In the picker, the HOME menu, the key map page and messages the arrows turn SELECT (← →) and
  ALGORITHM (↑ ↓) and Enter is PLAY; the letters do nothing there. On the on-screen keyboard the arrows
  move its cursor and the other keys still reach the game; SAVE and REC (its CAPS and SYMBOL toggles)
  are clicked on the drawing.
- fm1-emu's own piano keys (A to ' as notes, Z / X octave, R REC, 1 to 0 the top buttons) are off.

The FM-1 drawing shows each mapped key's Spectrum key (`ENT`, `SPC`, `CS`, `SS`, `←` `↑` `→` `↓`
for Kempston, `FIRE`; encoder turns under the knob, counter-clockwise first), HOME and PLAY as `HOME
(Esc)` and `PLAY (Tab)`, and in the picker and the menus only the menu controls. The key map under the synth lists
a game's key map with the computer key that reaches each input; it follows the running game (another
game can be picked to look at; it snaps back when a game starts).

A package whose `fmzx_stats` address the page does not have (`tools/web-demo` warns) keeps the fixed
keys: Esc HOME, Tab and Enter PLAY, the arrows C#4 / D#4 / F#3 / F3 (Kempston by default) and turning
SELECT and ALGORITHM, Space C5, C V B G#3 A#3 B3, [ ] F#5 G5, and fm1-emu's piano keys for the rest;
the labels then show the game picked in the key map under the synth.
