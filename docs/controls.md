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
| PLAY   | Pauses the game; OK in the menu |

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

In the game picker, SELECT and ALGORITHM turns and OCT-/OCT+ scroll (clockwise and OCT+ go to the
next game) and PLAY starts the selected game. Buttons act on their press.

