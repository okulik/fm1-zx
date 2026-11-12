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

