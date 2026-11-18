<!--
SPDX-FileCopyrightText: 2026 FM-1 ZX contributors
SPDX-License-Identifier: GPL-3.0-or-later
-->

# Installing FM-1 ZX, and the way back

> **FM-1 ZX runs on the real FM-1.** "Installing with M-Flasher", the UBOOT gesture and going back
> to the stock firmware by restoring an M-Flasher backup work on hardware; the routes marked
> *unverified* below have not been tried. Installing any custom firmware is at your own risk.

## The packages

`tools/make-packages` builds two update packages (`.fwsc`), both with the product identity
`FM-1_948`:

| Package | For | Colour mode without boot options |
| --- | --- | --- |
| `build/fm1/fmzx.fwsc` | the FM-1 | RGB444 (12-bit colour, a quarter less to send to the panel) |
| `build/fm1/fmzx-preview.fwsc` | fm1-emu's web page | RGB565 (the build option `FMZX_LCD_RGB565`, preset `fm1-preview`) |

Each is a JieLi UFW container (chip `AC791N`, about 610 KB) with

- `flash.bin`: the flash head (JieLi's SDK boot loader `uboot.boot`, the flash header and an
  `isd_config` blob that gives the chip key), then the app area: the FM-1 ZX image at offset
  0x120, the game pack at the next 4 KiB boundary after it (both in the 581,564-byte app slot),
  then the SDK's `cfg_tool.bin` and `eq_cfg_hw.bin`;
- `ota.bin`: Felucca 1.6.1's update loader (`third_party/felucca-loader`), a separate program
  that the FM-1 runs from RAM to write the update.

`fmzx-pack verify <package>` checks a package: the identity, every CRC, the sizes, the SDK files'
and the loader's SHA-256, that the image carries the identity, and that the game pack opens at
the offset the image's size gives.

## Building them

You need Docker (for the device build, `BUILDING.md`), Rust and curl on the host.

    tools/make-packages

It runs both device builds (`tools/fm1-docker cmake --preset fm1` and `--preset fm1-preview`,
then `cmake --build`), builds the game pack from `games/games.toml`, and writes and verifies the
two packages, and copies the SDK files' licence text next to them
(`build/fm1/AC79-SDK-Apache-2.0.txt`, from `docs/third-party/`; pass it on with the packages). The
first run fetches three files of JieLi's AC79 SDK (Apache-2.0) with curl into
`build/ac79/`:

    https://gitee.com/Jieli-Tech/fw-AC79_AIoT_SDK/raw/AC79NN_SDK_V1.2.1_2023-12-13/cpu/wl82/tools/
      uboot.boot          4e3b4c220dc96641cb5a723f41e68ce41d5261ae9434bb33fbd7f2c59976ded4
      cfg_tool.bin        276579954f076886a6a7694f65dc71c034a63a2c204b76749065c0ac7b010d1b
      cfg/eq_cfg_hw.bin   41167491bffed4651750719c973d2758adeb9021a5670d02d6a53c85ed80ea7d

A file is used only when its SHA-256 matches (`host/crates/fmzx-pack/inputs.toml`); a cached
file that does not match is an error and is never replaced on its own. `tools/make-packages
--offline` uses the cache only; `--cache DIR` uses another directory (for example the
`cpu/wl82/tools` directory of an SDK checkout at that tag). If gitee.com cannot be reached,
put the three files into `build/ac79/` (same names, `cfg/` included) by any other means: the
hashes decide.

By hand, the same steps:

    cd host
    cargo run --release -p fmzx-pack -- games ../games/games.toml -o ../build/fm1/games.bin
    cargo run --release -p fmzx-pack -- fwsc --app ../build/fm1/platform/fm1/fmzx_app.bin \
        --pack ../build/fm1/games.bin --loader ../build/fm1/third_party/felucca-loader/loader.bin \
        -o ../build/fm1/fmzx.fwsc [--product FM-1_948] [--cache DIR] [--offline]
    cargo run --release -p fmzx-pack -- verify ../build/fm1/fmzx.fwsc

`--product` must match the image's identity (the CMake cache variable `FMZX_PRODUCT`, default
`FM-1_948`). The loader must be the pinned Felucca 1.6.1 loader that the `felucca_loader`
target builds (8,216 bytes, `third_party/felucca-loader/VENDORED.md`).

With your own games: `tools/make-personal-pack <folder>` builds the same two packages with the
games of a folder of yours, as `build/personal/fmzx.fwsc` and `build/personal/fmzx-preview.fwsc`
(BUILDING.md, "Your own games"). They install, and try in the browser, as the packages here do;
`tools/web-demo --packages build/personal` runs them in the local browser demo.

## Trying it in the browser first

`tools/web-demo` runs the release package itself in a local browser page, with sound and the
correct colours (see BUILDING.md, "Browser demo"). The rest of this section is the quick way
with fm1-emu's public page and the preview package.

fm1-emu's web page (<https://joakimeriksson.github.io/fm1-emu/>) can open a local package: use
its control for opening your own `.fwsc` file (something like "open a local package"; the
label may differ) and pick `build/fm1/fmzx-preview.fwsc`. The file is read in your browser;
nothing is uploaded. The FM-1's keys are on the computer keyboard (the page shows the layout):
the picker, the HOME menu and the games work as in `docs/controls.md`.

Open the **preview** package, not `fmzx.fwsc`: fm1-emu's LCD model reads every pixel as RGB565,
so the release package's RGB444 picture comes out scrambled there. The preview build differs
only in that default (`FMZX_LCD_RGB565=ON`). The emu-gate test
`fw_package::preview_package_boots_without_boot_options` boots it the way the web page does and
checks the game picker. Two things to know:

- There is no sound on the web page: the firmware starts its audio only on the audio PLL that the
  FM-1's boot leaves running, and the emulator's package boot does not set it up. The games run
  silently, paced by the timer.
- It is not verified that the public page's emulator version (it may be newer or older than the
  revision the tests pin, `93cb7ee`) boots the package; if it does not, run fm1-emu's web build
  at that revision locally.

## Installing with M-Flasher (verified on hardware)

[M-Flasher](https://github.com/jvitkauskas/m-flasher) (a community tool, not M-VAVE's) writes a
package over the chip's own USB download mode, whatever firmware is installed. It saves a backup
of the whole flash before every write (`~/Documents/M-Flasher/backups`), writes the package's app
area only (from 0x4000: the unit's bootloader stays as it is) and checks what it wrote.

1. Get the unit into download mode (the computer sees a USB disk named `WL82 UBOOT1.00`, which is
   not opened):
   - from the stock firmware or another community firmware that answers it: M-Flasher's DOWNLOAD
     MODE button (a SysEx);
   - with [m-bootloader](https://github.com/jvitkauskas/m-bootloader) installed: hold **REC** while
     switching on (**PLAY** opens its menu instead, which also has DOWNLOAD MODE);
   - from FM-1 ZX: the UBOOT gesture below.
2. In M-Flasher, install `build/fm1/fmzx.fwsc`.
3. Switch off and on: FM-1 ZX starts into the game picker.

To go back, restore one of M-Flasher's backups the same way, or install another firmware's package.

## Installing on an FM-1 with a USB-MIDI installer (unverified)

FM-1 firmware is updated through the firmware that is running: the community installers (for
example Felucca's web installer, or the FM-1 Firmware Hub) send a `.fwsc` over USB-MIDI to the
running firmware, which hands it to the update loader in the package (`ota.bin`), which writes
the app area. *Unverified:* that the stock firmware or Felucca accepts this package (identity
`FM-1_948`, with Felucca's loader, which presents itself to update hosts as `ota-FM-1_900`), and
that a given installer takes it. The first device test checks this.

1. Start the FM-1 with its current firmware (stock or another community firmware) and connect
   it by USB.
2. Open a community installer, choose "install a local file" (or the installer's equivalent)
   and pick `build/fm1/fmzx.fwsc`.
3. Wait until the installer reports success and the FM-1 restarts into the game picker
   (expected, unverified).

**FM-1 ZX itself has no USB stack in this version.** Once it runs, the computer does not see
the FM-1 as a MIDI device, so no installer can reach it while FM-1 ZX is running. To replace it
(with a newer FM-1 ZX, another firmware or the stock firmware), use the update mode below. The
in-app update over USB comes with a later version.

## The update mode (UBOOT gesture)

Hold **OCT-** while switching the FM-1 on, and keep holding it for one second. FM-1 ZX then calls
the boot loader's update-mode entry (`fmzx_enter_uboot`, which calls felucca-bsp's
`fm1_enter_uboot`: it sets `usb_update_mode` and resets the chip) instead of starting. What is
proven: in fm1-emu, `fmzx_enter_uboot` is reached just after one second with OCT- held, and a
short tap of OCT- at power-on does not reach it (FM-1 ZX starts normally). On hardware, behind
M-VAVE's stock bootloader, the computer sees the boot loader on USB and M-Flasher installs a
package from there. *Unverified:* the other recovery tools (for example
FM-1-transporter, <https://github.com/kurogedelic/FM-1-transporter>) from there.

If the FM-1 ZX image itself does not start at all, the gesture cannot run either. The chip's own
update mode, below every firmware, is reported by the community to be reached by holding
**OCT- and OCT+ together for about five seconds** at power-on, with the same tools working from
there (community report, not tried by this project).

## Back to the stock firmware (unverified)

M-VAVE's workbench (their updater application) installs M-VAVE's stock package (`FM-1.fwsc`).
With FM-1 ZX installed:

1. Put the FM-1 into the update mode (UBOOT gesture above).
2. Install the stock firmware with M-VAVE's workbench, or write M-VAVE's `FM-1.fwsc` with a
   community recovery tool (unverified: whether the workbench works from this update mode).
3. Switch the FM-1 off and on: the stock firmware starts.

These steps are part of the first device test (`docs/hardware-test.md`) and are not verified:
treat them as the plan, not as tested instructions.
