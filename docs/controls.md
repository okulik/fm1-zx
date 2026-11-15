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
| F3 | Kempston left |
| F#3, G#3 | Kempston up |
| G3 | Kempston down |
| A3 | Kempston right |
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
| Sound: *choice* | How the game's sound is played. **Beeper** (the default) is the Spectrum's own sound, with no delay. **FM: sound** plays music through the FM-1's synth where the program is sure of the notes (the rest stays on the beeper; a tune's first note too, since a lone tone could be an effect, so FM starts with the second note), and delays all sound, music and effects alike, by 60 ms (3 frames). **FM: engines** plays the music of known routines (Castaway and Colony 8 titles, BASIC `BEEP` notes of 20 ms or more, Cave Runner, Unison) as FM, reading the notes from memory and checking them against the beeper; anything it does not recognise, or that fails the check, stays on the beeper, and it switches to FM on its first recognised frame. All FM choices share the same 60 ms delay, and hold FM through a single unsure frame inside the music, so the music does not drop to the beeper for a moment. PLAY cycles Beeper, FM: sound, FM: engines. The choice stays through Reset game, Back to games and starting another game, but not through power-off. Turning FM on fades the sound out briefly and leaves a 60 ms gap before it returns; turning it off crossfades back in a few ms. |
| Screen: *choice* | How the 256-pixel-wide Spectrum picture fits the 240-pixel panel. **Crop** (the default) shows 240 of the 256 columns, from where each game's crop puts them, and no side border. **Fit** shows all 256 columns by leaving out one column in 16, evenly spaced; the colours stay the Spectrum's own. **Fit smooth** shows all 256 columns by blending neighbouring columns, so thin lines and text keep their weight. **Border** scales the picture with 8 border pixels each side down to 240 x 212, blended, with the border colour above and below. In every choice the menu, the keyboard and PAUSED look the same. PLAY cycles Crop, Fit, Fit smooth, Border. The choice stays through Reset game, Back to games and starting another game, but not through power-off. |
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

The browser test bench (`tools/web-demo`) maps the computer keyboard onto the FM-1 keys. The keys
below are claimed by this project and win over fm1-emu's page; the arrows also turn SELECT and
ALGORITHM, and Space is a note key (C5), not PLAY, so it can be the fire button. The page's side
panel lists the key map of the game picked in it (it cannot see the running game), with the computer key for each FM-1 input.

| Key | FM-1 input | Also |
|-----|------------|------|
| ← | F3 | SELECT counter-clockwise |
| → | A3 | SELECT clockwise |
| ↑ | F#3 | ALGORITHM counter-clockwise |
| ↓ | G3 | ALGORITHM clockwise |
| Space | C5 | |
| Enter | PLAY | |
| Esc | HOME | |
| C V B | G#3 A#3 B3 | |
| [ ] | F#5 G5 | |

fm1-emu's own keys stay as they are:

| Keys | FM-1 input |
|------|------------|
| A W S E D F T G Y H U J K O L P ; ' | notes C4 to F5, in order (C4 C#4 D4 ... F5) |
| Z / X | OCT- / OCT+ |
| R | REC |
| 1 2 3 4 5 6 7 8 9 0 | FX SCL ENV LFO EDIT GLO HOME SAVE ARP SEQ |
| , / . | ALGORITHM counter-clockwise / clockwise |
