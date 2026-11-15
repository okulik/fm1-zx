# Screen modes

Status: designed 2026-11-15, owner approved ("yes, go ahead"). Plan to follow (Plan 8).

## Intent

The FM-1's panel is 240 × 240. The Spectrum picture is 256 × 192 inside a border. Today the firmware
crops 16 columns from the width, using each game's `crop_x`, and shows no side border.

The owner compared five ways of fitting the picture, rendered from their own games (a throwaway
spike, 2026-11-15), and chose four of them. All four must be choices in the HOME menu.

## Approaches

One sensible approach: compose each LCD row differently per mode, on the device, in
`app/pixels.c`. The firmware already draws row by row there.

Rejected alternatives:
- Pre-scaling at pack time does not work, because the picture is live.
- Scaling in the panel is not possible, because the ST7789 has no scaler.

## Behaviour (owner's choice)

A new HOME-menu item, **Screen**. PLAY cycles Crop → Fit → Fit smooth → Border → Crop. The default is
Crop. The choice holds through Reset game, Back to games and starting another game, and is lost at
power-off, as Sound does.

| Choice | Mode (spike name) | Picture |
|---|---|---|
| **Crop** | A | Today's picture: 240 of the 256 columns from the game's `crop_x`; 24 border rows above and below. Byte-identical to today. |
| **Fit** | B | All 256 columns to 240 by dropping one column in 16, evenly spaced. Rows and border as Crop. Spectrum colours only. |
| **Fit smooth** | C | All 256 columns to 240 by area weighting: each output pixel is the weighted average, in RGB, of the source columns it covers. Rows and border as Crop. |
| **Border** | E | The 272-column region (8 border pixels each side), with the matching rows above and below the paper, scaled uniformly by 240/272 in both axes with an area filter, in RGB. Centred vertically. Rows outside the picture take the border colour of the nearest frame line. |

**Colour.** Blended colours are computed in 8-bit RGB from the palette in `pixels.c`, then
reduced to the panel's format:
- RGB444 in the release build, by truncation as today (`>> 4`);
- RGB565 in the preview build.

**Unchanged in every mode:**
- the game picker;
- the HOME menu;
- the on-screen keyboard overlay;
- the "PAUSED" border text.

They draw as they do today.

**Partial updates.** The LCD scheduler sends only changed rows. In Border, one Spectrum line
feeds one or two LCD rows, so a dirty line marks every LCD row it feeds. Border colour changes
mark rows as they do today.

**Browser page.** It runs the firmware, so it shows the same modes.

## Constraints

- Crop must be byte-identical to today: the same pixels and the same panel bytes. Every existing
  golden image and test passes unchanged.
- Integer arithmetic only, no allocation, `-Wall -Wextra -Werror`. Host and device produce identical
  pixels. The host composes rows with the same C, which tests check.
- **Budget.** Every game's worst frame stays within `fw_budget`'s cap (3,600,000) in every Screen
  mode combined with every Sound mode. RAMTEXT ≤ 49,152 bytes; about 1 KB is free today, and
  `pixels.c` lives in RAMTEXT. If the new modes don't fit, Crop stays in RAMTEXT and the other modes'
  filters run from flash (XIP). The firmware's RAMTEXT/XIP reach rules apply. The budget must still
  hold. This is the builder's call; the result goes in the plan's report.
- Commits are a single line with no trailers. Never name other projects in files or commits.
- **Device rule.** Never flash a device, send SysEx or open MIDI ports.

## Tests

- **Host golden images per mode.** These are composed rows for `scroll.sna` and `border.sna`, two of
  our own test programs. Crop's goldens are today's.
- Fit drops exactly 16 columns, evenly spaced. Fit smooth's weights for each output pixel sum to 1.
  On a flat area, Fit smooth and Border reproduce the flat colour exactly.
- **Firmware:**
  - The Screen item cycles through all four choices, and the choice holds through reset, the picker
    and a new game.
  - The LCD shows each mode's golden image for a test program.
  - `fw_budget` passes with every Screen mode × Sound mode.

## Left to the builder

- The row pipeline's shape. Blended modes produce colours outside the 16-colour palette, so a mode
  may compose straight to panel bytes instead of palette indices.
- Weight tables: precomputed, and where they live.
- Which code goes in RAMTEXT.
- The dirty-row mapping for Border.
- The menu label width on the LCD.
