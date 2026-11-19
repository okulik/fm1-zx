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
`FM-1_048`:

| Package | For | Colour mode without boot options |
| --- | --- | --- |
| `build/fm1/fmzx.fwsc` | the FM-1 | RGB444 (12-bit colour, a quarter less to send to the panel) |
| `build/fm1/fmzx-preview.fwsc` | fm1-emu's web page | RGB565 (the build option `FMZX_LCD_RGB565`, preset `fm1-preview`) |

Each is a JieLi UFW container (chip `AC791N`, about 610 KB) with

- `flash.bin`: the flash head (JieLi's SDK boot loader `uboot.boot`, the flash header and an
  `isd_config` blob that gives the chip key), then the app area: the FM-1 ZX image at offset
  0x120, the game pack at the next 4 KiB boundary after it (both in the 581,564-byte app slot),
  then the SDK's `cfg_tool.bin` and `eq_cfg_hw.bin`;
- `ota.bin`: felucca-bsp's update loader (`platform/fm1/bsp/loader`), a separate program
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
        -o ../build/fm1/fmzx.fwsc [--product FM-1_048] [--cache DIR] [--offline]
    cargo run --release -p fmzx-pack -- verify ../build/fm1/fmzx.fwsc

`--product` must match the image's identity (the CMake cache variable `FMZX_PRODUCT`, default
`FM-1_048`). The loader must be the pinned felucca-bsp loader that the `felucca_loader`
target builds (15,060 bytes, `cmake/check_loader.cmake`).

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

## Installing on an FM-1 with a USB-MIDI installer

FM-1 firmware is updated through the firmware that is running: a USB-MIDI installer sends a
`.fwsc` to the running firmware, which hands it to the update loader in the package (`ota.bin`),
which writes the app area. For a local package file the installer is felucca-bsp's
`tools/fm1_install.py` (<https://github.com/hugelton/felucca-bsp>; it needs Python with `mido` and
`python-rtmidi`). The FM-1 Firmware Hub installs the packages it lists, and Felucca's web
installer installs Felucca (it takes a local file only for the official V15). Tested on the FM-1:
`fm1_install.py` installs this package (identity `FM-1_048`, with felucca-bsp's loader, which
presents itself to update hosts as `ota-FM-1_900`) from SLOOP and from FM-1 ZX itself.
*Unverified:* from the stock firmware.

1. Start the FM-1 with its current firmware (stock or another community firmware) and connect
   it by USB.
2. Run `python3 tools/fm1_install.py build/fm1/fmzx.fwsc` from a felucca-bsp checkout (with the
   path to the package; `--info` first shows the FM-1's identity).
3. Wait until the installer reports success and the FM-1 restarts into the game picker.

## Installing and updating over USB from FM-1 ZX

Whenever the USB cable is in (in the picker, the HOME menu, a game running or paused), FM-1 ZX is
a class-compliant USB-MIDI device named `FM-1 ZX`. It answers the installers' handshake with its
identity `FM-1_048` and runs their update session, with felucca-bsp's USB-MIDI device and update
code (`platform/fm1/usbota.c`). So the same installers
that reach the stock firmware and Felucca reach FM-1 ZX, for a newer FM-1 ZX, another firmware or
the stock firmware:

- the FM-1 Firmware Hub (<https://thefirmwarehub.com>): **Connect FM-1** marks FM-1 ZX's card as
  the firmware on your FM-1; **Install** on any card installs that firmware from FM-1 ZX;
- a local installer for a local package, for example felucca-bsp's `tools/fm1_install.py`
  (<https://github.com/hugelton/felucca-bsp>) with a `.fwsc` file.

What the FM-1 does: a running game pauses and goes silent, and the panel shows `UPDATE` and the
step: `PACKAGE` (the package's header read and checked), `CHECK HEAD` (another writer's flash head
checked), `LOADER` (the package's update loader written to the flash's OTA area,
0xE0000-0xE4FFF, and read back), `CONFIRM` (the installer's go-ahead), `RESTART`. Then the FM-1
restarts into the package's update loader (felucca-bsp's presents itself to the installer as
`ota-FM-1_900`), which writes the app area, and the new firmware starts. Until the installer's
go-ahead nothing is committed: a failure (the installer gone, a damaged read, a refused package)
shows `FAILED -<n>` for 1.5 s, erases the OTA area and returns to where you were, the picker or
the game paused. While FM-1 ZX runs it writes the flash only in the OTA area and in the saved
game's area (0xEA000-0xF6FFF, above the app area: [save format](save-format.md)), which an
update leaves as it is; installing another firmware may overwrite it. It refuses, before
anything is erased:

- a package with felucca-bsp's update loader whose `flash.bin` that loader would refuse after the
  restart (none, shorter than 0x93000 bytes, the flash up to the app area's end, or longer than
  the 1 MiB flash): `FAILED -15` (a package with another loader goes on to the session's own
  checks);
- any session on a flash chip other than the two FM-1s ship with (JEDEC `85 60 14`, Puya, and
  `B3 60 14`, UC25HQ80): the handshake is answered, the update message ignored.

If an install stops while the loader writes (the cable pulled, the power gone), the FM-1 starts
into the loader again at the next power-on; **Install** again on the hub (or the local installer
again) resumes it. The hub's rescue key (the soft UBOOT key, `F0 22 24 35 7D F7`) puts FM-1 ZX
into the chip's update mode, as the UBOOT gesture below does. When nothing over USB works, the
gesture (OCT- held at power-on) and M-Flasher remain.

What is proven: in fm1-emu (`host/crates/emu-gate/tests/fw_usb.rs`), against the release package,
the session stages the loader and reaches the restart on both flash chips with no flash write
outside the OTA area; an abandoned session, corrupted reads (of the loader's head and of its
body) and a `flash.bin` the loader refuses (missing, too short, past the flash) leave the OTA
area erased and FM-1 ZX where it was, the panel as it was before the session, while a `flash.bin`
with a longer app area (0xA9000) passes the check; the soft UBOOT key enters the update mode.
Tested on the FM-1 (flash chip 0x856014): the hub's **Connect FM-1** finds FM-1 ZX; the hub installs
SLOOP from FM-1 ZX; `fm1_install.py` installs FM-1 ZX over itself and back from SLOOP, and an
install unplugged while the loader writes resumes when it is run again; the soft UBOOT key; a game
plays without clicks with the cable in and out. *Unverified:* the other flash chip (0xB36014).

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
