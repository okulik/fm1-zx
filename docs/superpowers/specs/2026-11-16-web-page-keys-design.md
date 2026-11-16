# Browser test page: a readable guide, Spectrum labels on the FM-1, Spectrum keys on the computer

Date: 2026-11-16. Status: design, for the owner to check.

## Intent

The browser test page (`tools/web-demo`, our layer in `web/` over fm1-emu's page) is how the owner tries the firmware before there is hardware. Three things make it hard to use:

1. The guide under the synth (How to use it, Known limits, How it works, ...) is squeezed into a column about 200 pixels wide. Cause: upstream's page lays the guide out as a 200-pixel table of contents beside the text; we hide the table of contents (`web/fmzx.css`, `#guide > .toc`), so the text drops into the 200-pixel column.
2. Nothing on the FM-1 drawing says which Spectrum key a key gives; the owner has to read the side panel's table.
3. The computer keys press FM-1 keys by fm1-emu's piano layout (A to ' are notes, 1 to 0 the top buttons), not by Spectrum key, so typing 1 does not give Spectrum 1.

## Goals

1. The guide reads comfortably at any width.
2. Each FM-1 key and button on the drawing shows the Spectrum key it gives in the running game.
3. While a game runs, the computer keys type Spectrum keys: pressing a key presses whichever FM-1 key the running game maps to that Spectrum key (the owner's choice: option 1).

## Non-goals

- A Spectrum keyboard the FM-1 doesn't have: the firmware still receives only FM-1 key presses, so the page shows exactly what the device can do (the owner rejected option 2, a direct Spectrum keyboard).
- Changing the firmware, the key maps or the game packs.
- Restyling upstream's page beyond the guide and our panel.

## Anti-goals

- A computer key that does something the FM-1 can't (a press that reaches the Spectrum with no FM-1 key behind it).
- Losing a control: HOME, PLAY and the knobs must always be reachable from the computer keyboard.
- Labels that lie: a label must be the running game's mapping, not the one picked in the side list.

## Design

### 1. Layout

- When the table of contents is hidden, the guide is one column: `.guide` drops its two-column grid (`grid-template-columns: 1fr`).
- The text column is centred with a readable width (about 72 characters a line).
- On a wide screen (two cards of at least 420 px fit), the guide's sections (`<section>`s) sit as cards, two side by side; on a phone, one column.
- Our side panel (`.fmzx`: the game list, the key-map table) is checked at phone width and the widest desktop width: nothing cut off, no horizontal scroll.

### 2. The page knows the running game

- The emulator runs in the browser's audio thread (fm1-emu's worklet); it already sends the page a message each frame (screen, LEDs). A small patch to the emulator (`web/patch-emu.sh`, like the existing patches) adds two values to that message: the firmware's view (picker, game, menu, ...) and game index, read from the firmware's `fmzx_stats` (fields `view` and `game`).
- `tools/web-demo` reads `fmzx_stats`'s address from the build's symbol table (the ELF the package was made from) and writes it to `keymaps.json`; the page passes it to the worklet when it boots the package. A package without it (no address): the page keeps today's behaviour (keys and labels from the side list).
- The side panel's game list follows the running game (it still lets the owner look at another game's map; it snaps back when the running game changes).

### 3. Computer keys while a game runs

The page builds, for the running game, a table from Spectrum key to FM-1 input, from that game's key map (`keymaps.json`): an FM-1 input whose action is one Spectrum key (`key.X`) or a Kempston direction is reachable by the computer key below. When several FM-1 inputs give the same Spectrum key, the first in the key map is used.

| Computer key | Spectrum key | Notes |
|---|---|---|
| A to Z, 1 to 0 | the same letter or digit | by physical key (`event.code`), so a non-US layout still types the key printed in that place |
| Space | SPACE | |
| Enter | ENTER | in the picker and the HOME menu, Enter is PLAY instead (no game is reading keys there) |
| left Shift | CAPS SHIFT | |
| Alt (Option) | SYMBOL SHIFT | Ctrl is avoided: Ctrl+W and other browser shortcuts cannot be blocked |
| arrows | Kempston left, right, up, down | they also turn the SELECT and ALGORITHM knobs, as today, so menus work |
| right Shift | Kempston fire | *my call*; see Questions |

- An FM-1 input mapped to two actions (e.g. `key.CAPS+key.5`) is pressed by the matching chord (left Shift + 5).
- A Spectrum key the game doesn't map: nothing happens (the FM-1 couldn't do it either).
- Fixed, in every view: **Esc is HOME**, **Tab is PLAY** (pause in a game, OK in menus).
- fm1-emu's own piano letters (A to ' as notes, Z/X octave, R REC, 1 to 0 the top buttons) are switched off while a game runs; in the picker and the menus the arrows, Enter, Tab and Esc are all that's needed.
- The on-screen keyboard (opened from the HOME menu) still works with the knobs and PLAY; SAVE and REC (its CAPS and SYMBOL toggles) stay on their FM-1 keys.

### 4. Labels on the FM-1 drawing

- Each note key and button the running game maps shows its Spectrum key in small text on or just above the key (`1`, `Q`, `ENT`, `SPC`, `CS`, `SS`, `←` `↑` `→` `↓` for Kempston, `FIRE`); an unmapped key shows nothing.
- HOME and PLAY are labelled `HOME (Esc)` and `PLAY (Tab)`.
- Labels follow the running game and redraw when it changes; in the picker and the menus they show the menu controls only.
- The labels are drawn over the synth drawing by the page (the drawing itself comes from fm1-emu's code); where each key sits is found from the drawing's own hit test (`panel_hit`), so the labels stay aligned at any size.

## Verification

- The page's node tests (`web/fmzx.test.mjs`): the Spectrum-key table built from a key map (single actions, chords, duplicates, Kempston, unmapped keys); Esc/Tab/Enter in each view.
- The headless check (`tools/web-demo --check`): boots the package, starts a game, presses computer keys and sees the right FM-1 inputs reach the firmware (the firmware's key state), and that the running game is reported.
- By eye: screenshots of the guide at phone and desktop widths, and of the labels, for the owner.

## Questions for the owner

1. Kempston fire: right Shift (my pick, since Space is SPACE), or Space when the game doesn't map SPACE, or another key?
2. Tab for PLAY and Alt for SYMBOL SHIFT: fine?
