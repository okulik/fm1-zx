# ZX Sopwith

<img src="sopwith.png" alt="ZX Sopwith running on a ZX Spectrum 48K" width="768" height="576" />

Sopwith for the ZX Spectrum 48K, by [Neil Rackett](https://neilrackett.com/zxspectrum)

## About

ZX Sopwith is a port of the game "Sopwith", which was originally by BMB
Compuscience Canada. The original author David L. Clark has kindly released
the source code under the GNU GPL.

This is a port of [ST Sopwith](https://github.com/neilrackett/atarist-sopwith),
which came in turn from [Fragglet](https://github.com/fragglet/sdl-sopwith)'s
SDL port of the DOS original. You fly a biplane, and you have until the fuel
runs out to flatten every building the other side owns before they flatten
yours.

The flight model is the original one, angle for angle: the same sixteen
headings, the same gravity table, the same four-tic throttle, the same map of
three thousand pixels. What is different is everything underneath, which had
to be rebuilt for a machine with a monochrome bitmap, no multiply instruction
and 41K to put a whole game in.

## Run

Load `sopwith.z80` or `spowith.sn` in your favourite emulator, including
[MD/Speccy](https://neilrackett.com/atarist) if you have an Atari ST with a
SidecarTridge Multi-device, or `sopwith.tap` on real hardware.

## Controls

| Key       | Kempston | Action              |
| --------- | -------- | ------------------- |
| **Q**     | Up       | Pull up             |
| **A**     | Down     | Pull down           |
| **P**     | Right    | Throttle up         |
| **O**     | Left     | Throttle down       |
| **SPACE** | Fire     | Machine gun         |
| **F**     |          | Flip the plane over |
| **B**     |          | Drop a bomb         |
| **H**     |          | Fly home            |
| **M**     |          | Sound on and off    |
| **1**     |          | Abandon the flight  |

The DOS original uses comma, full stop and slash, which on a Spectrum are all
symbol shifted and so cannot be held down alongside anything else.

A **Kempston joystick** flies and fires, and the keyboard stays live alongside
it for the four things one button cannot reach. Throttle sits on left and
right because a throttle is not a direction anyone expects to steer with, and
up and down are wanted for the stick itself. The button also starts a novice
game from the title screen, so a stick alone can get you into the air. With no
interface fitted the port floats, so a reading that could not have come from a
joystick is thrown away and an unattached machine sees nothing.

Land back on your own runway to refuel and rearm. You get five planes.

## Build

1. Install [zxspectrum-toolkit-docker](https://github.com/neilrackett/zxspectrum-toolkit-docker)
1. Open the command line and navigate to the project directory
1. Run `zxcmd make` to compile the code
1. Files will be created in the `dist` folder

| Target              | Output                                     |
| ------------------- | ------------------------------------------ |
| `make` / `make all` | `sopwith.tap` and `sopwith.z80`            |
| `make tap`          | tape image, for real hardware              |
| `make z80`          | snapshot, via `.sna` and `snapconv`        |
| `make sna`          | the snapshot on its own                    |
| `make size`         | where the memory went, and whether it fits |
| `make profile`      | cycle counts from `z88dk-ticks`            |
| `make clean`        | remove the dist directory                  |

## Making it fit in 48K

Birds, oxen, bomb craters, the computer pilot and its shooting model, the
scoring, and the difficulty scaling that runs from level to level are all
still here, as are all three ways to play: novice, expert, and against the
computer's planes.

But with only 48KB to work with, a few things had to go:

- **Custom missions.** They need a file system to load from and a parser to
  read them with. The parser alone is bigger than the whole collision system.
- **Network play.** There is no socket to open, and the two-player code
  carries a lag compensator, a command ring buffer and a second player's
  worth of state with it.
- **Missiles and starbursts**, which the PC version ships switched off, and
  observation balloons, which no level uses.
- **Medals and ribbons.** Sixteen more sprites and a status bar with nowhere
  to put them; the bar is 32 characters wide and already carrying the score,
  the fuel, the ammunition, the bombs, the lives and the map.
- **Wounded planes**, another PC option that defaults to off.
- **Multiple palettes and solid-ground mode.** One is meaningless on a
  monochrome bitmap and the other is now how the ground is always drawn.
- **The high score table**, which has no disk to live on. A single best score
  is kept for as long as the machine is on.

## More info

More information about the original Sopwith is available at:

- http://www.sopwith.org/
- https://fragglet.github.io/sdl-sopwith
- https://en.wikipedia.org/wiki/Sopwith_(video_game)

## Credits

- Original game by David L. Clark, (c) 1984, 1985, 1987 BMB Compuscience
- SDL port by [Simon Howard](https://github.com/fragglet/sdl-sopwith)
- Atari ST and ZX Spectrum ports by [Neil Rackett](https://neilrackett.com)

## Licence

Released under the [GNU General Public License v3.0](LICENSE).

The original game is distributed under the GNU GPL version 2 or later, which
permits this.
