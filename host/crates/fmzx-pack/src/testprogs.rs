// SPDX-FileCopyrightText: 2026 FM-1 ZX contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Our test programs, hand-assembled: `scroll.sna` changes every screen line every frame
//! (the LCD update path's worst case), `border.sna` draws border stripes and a beeper tone, and
//! `tape.tap` loads a screen image from the tape in the middle of its run (the LD-BYTES trap),
//! and `tune.sna` plays eight notes on the beeper.
//! `fmzx-pack testprogs <dir>` writes them; `games/test/` holds the committed copies.
//!
//! # Common layout of the snapshots
//!
//! All three run in uncontended RAM under IM 2 with their own vector table, so no ROM code or
//! system variable is involved and every frame takes the same time:
//!
//! | Address         | Contents                                                    |
//! |-----------------|-------------------------------------------------------------|
//! | `8000h`         | main loop (listings below)                                  |
//! | `FCFEh`-`FCFFh` | the stack (SP `FD00h`); the `.sna` keeps PC there           |
//! | `FDFDh`         | interrupt handler: `RET` (`C9`, 10 T)                       |
//! | `FE00h`-`FF00h` | 257 bytes of `FDh`: whatever the bus holds, the vector is `FDFDh` |
//!
//! I = `FEh`, IM 2, interrupts off at load. The main loop ends with `EI; HALT`; the interrupt
//! at T 0 costs 19 T (IM 2 response) plus the handler's `RET`, which returns to the
//! instruction after the `HALT` (address [`RESUME`]) with interrupts off again. So the work
//! starts at about T 32 of every frame and can never be interrupted.
//!
//! # `scroll.sna`
//!
//! A 32-pixel band (columns 14-17) on each of the 192 lines holds an 8-pixel stripe whose
//! position depends on the line (a diagonal). Every frame the band of every line rotates one
//! pixel right, so every line changes every frame. The border stays 1 (blue). Lines are visited
//! in memory order: bitmap rows are 32 bytes apart, and each 32-byte row is one screen line.
//!
//! ```text
//! 8000 FB        frame: EI                4
//! 8001 76               HALT              4n   IM 2 + RET return to 8002, interrupts off
//! 8002 21 11 40         LD HL,4011h       10   row 0, column 17 (band = columns 14-17)
//! 8005 11 20 00         LD DE,0020h       10   one bitmap row
//! 8008 06 C0            LD B,192          7
//! 800A 7E        line:  LD A,(HL)         7    column 17
//! 800B 1F               RRA               4    CF = its bit 0, wraps into column 14
//! 800C 2D               DEC L             4    DEC and INC keep CF
//! 800D 2D               DEC L             4
//! 800E 2D               DEC L             4    column 14
//! 800F CB 1E            RR (HL)           15
//! 8011 2C               INC L             4
//! 8012 CB 1E            RR (HL)           15
//! 8014 2C               INC L             4
//! 8015 CB 1E            RR (HL)           15
//! 8017 2C               INC L             4
//! 8018 CB 1E            RR (HL)           15   column 17
//! 801A 19               ADD HL,DE         11   next row
//! 801B 10 ED            DJNZ line         13/8
//! 801D 18 E1            JR frame          12
//! ```
//!
//! A line takes 119 T plus contention (9 screen accesses, at most 6 T each); the work ends at
//! T 23,919 (the test `scroll_fits_well_inside_a_frame` measures it).
//!
//! # `border.sna`
//!
//! The paper shows eight static colour bars (colours 0-7, left to right).
//!
//! One `OUT (FEh)` per 224-T line, 312 per frame: the border colour cycles 0-7 line by line
//! and the beeper (EAR, bit 4) flips every 8 lines, a square wave of
//! 3,500,000 / (2 x 8 x 224) = 976.5625 Hz. The 312 lines fill the frame exactly, so the
//! last line's `OUT` is followed directly by `EI; HALT` and the interrupt's timing takes the
//! place of that line's delay loop: the tone runs on across frames without a seam.
//!
//! `OUT (FEh)` is I/O-contended on paper lines unless its I/O cycle falls in the 96 T of
//! border and retrace at the end of each line. The first `OUT` starts at about T 146 of the
//! frame (the pre-delay `D` = 4 places it), and with exactly 224 T per line every `OUT`
//! starts at T 146 + 224k, whose I/O cycle (T 153 + 224k) is never contended. So the loop
//! keeps exact time, and the border changes 153 T into each line. The last `OUT` starts at
//! about T 69,810 and the `HALT` at about T 69,858, before the next interrupt.
//!
//! The output byte lives in E: colour in bits 0-2, EAR in bit 4, MIC (bit 3) 0. Setting bit 3
//! before `INC A` makes the carry out of the colour bits flip bit 4; `AND 17h` then clears
//! bit 3 and bits 5-7 (so the port's high byte stays below 40h, out of the contended range).
//!
//! ```text
//! 8000 FB        frame: EI                4
//! 8001 76               HALT              4n   IM 2 + RET return to 8002, interrupts off
//! 8002 01 38 01         LD BC,312         10   lines this frame
//! 8005 16 04            LD D,4            7    pre-delay: 7 + 4 x 16 - 5 = 66 T
//! 8007 15        pre:   DEC D             4
//! 8008 20 FD            JR NZ,pre         12/7
//! 800A 18 07            JR out            12   the first line skips its delay
//! 800C 16 09     line:  LD D,9            7    delay: 7 + 9 x 16 - 5 = 146 T
//! 800E 15        wait:  DEC D             4
//! 800F 20 FD            JR NZ,wait        12/7
//! 8011 00               NOP               4
//! 8012 29               ADD HL,HL         11   padding (HL unused)
//! 8013 7B        out:   LD A,E            4
//! 8014 F6 08            OR 08h            7
//! 8016 3C               INC A             4    colour + 1; every 8th time EAR flips
//! 8017 E6 17            AND 17h           7
//! 8019 5F               LD E,A            4
//! 801A D3 FE            OUT (FEh),A       11   I/O cycle at T +7
//! 801C 0B               DEC BC            6
//! 801D 78               LD A,B            4
//! 801E B1               OR C              4
//! 801F 20 EB            JR NZ,line        12/7 line total: 161 + 63 = 224 T
//! 8021 18 DD            JR frame          12
//! ```
//!
//! Measured on the host core (tests in `tests/testprogs.rs`): [`BORDER_EDGES_PER_FRAME`] beeper
//! edges in every frame, 1,792 T apart; the `HALT` reached at T 69,858 (scroll: T 23,919);
//! and a tone of 976.06 Hz ([`BORDER_TONE_HZ`]) from the zero crossings of one second
//! (50 frames) of `zx_beeper` output at 750000/17 Hz. The device's tone test compares against
//! that value (±1%).
//!
//! # `tape.tap`
//!
//! A tape that loads in several steps, the last one while the program runs. Blocks:
//!
//! 1. BASIC header (`boot`, autostart line 10) and program: `LOAD ""CODE: RANDOMIZE USR 32768`.
//! 2. CODE header (`tape`, 8000h) and the code below.
//! 3. A headerless data block (flag `FFh`, 6,912 bytes): a screen image whose bitmap bytes are
//!    the line number `y` (all 32 bytes of line `y`) and whose attributes are bright, with the
//!    ink always different from the paper.
//!
//! The code runs under the ROM's IM 1 with interrupts on. It sets the border to 1 (blue),
//! waits [`TAPE_WAIT_FRAMES`] frames, then calls LD-BYTES (`0556h`) for the data block straight
//! into the screen. The border becomes [`TAPE_DONE_BORDER`] (green) if the call returns with
//! carry set, [`TAPE_FAIL_BORDER`] (red) if not; the program then idles. The wait makes the
//! load a visible event in the middle of the run, and after a reset of the game the tape must
//! start again from its first block.
//!
//! ```text
//! 8000 3E 01        LD A,1
//! 8002 D3 FE        OUT (FEh),A
//! 8004 FB           EI
//! 8005 06 64        LD B,TAPE_WAIT_FRAMES
//! 8007 76    wait:  HALT                one frame
//! 8008 10 FD        DJNZ wait
//! 800A DD 21 00 40  LD IX,4000h
//! 800E 11 00 1B     LD DE,1B00h         6,912 bytes
//! 8011 3E FF        LD A,FFh            flag
//! 8013 37           SCF                 load, not verify
//! 8014 CD 56 05     CALL 0556h          LD-BYTES
//! 8017 3E 04        LD A,TAPE_DONE_BORDER
//! 8019 38 02        JR C,done
//! 801B 3E 02        LD A,TAPE_FAIL_BORDER
//! 801D D3 FE done:  OUT (FEh),A
//! 801F FB           EI
//! 8020 18 FE        JR $
//! ```
//!
//! # `tune.sna`
//!
//! A scale, C5 to C6 (the notes of [`TUNE_NOTES_HZ`]), each [`TUNE_NOTE_FRAMES`] frames long
//! (0.5 s), repeated forever, as a square wave on EAR with MIC 0. The paper shows eight
//! stair-step bars, one per note, of the colours 1-7 and then 1 again, each four columns wide.
//!
//! Every note starts with `EI; HALT`, so its first beeper edge is at the same T-state after
//! the frame interrupt every time (T 51-54); then `K` edges follow, `P` T-states apart, and
//! after the note's last edge come the next note's setup and its `HALT`. `K` and `P` are
//! chosen so that `K` edges of `P` T fill the 25 frames (1,747,200 T) to within the table's
//! "gap" column: the time from a note's last edge to the next note's first edge. A half-period
//! is `P` and a gap about `P`, so the seam is no more than a slightly long or short half-period
//! (note 7's is about 310 T long, the most; the test bounds it at 315 T).
//!
//! Why it does not drift: the interrupt that every `HALT` waits for happens at T 0 of a
//! frame, whatever the program did before, so the note boundaries are exactly 25 frames apart
//! and an error in one note (a rounded `P`, the I/O contention below) cannot add up into the
//! next. The last edge of every note is 1,658-3,250 T before that interrupt, far more than
//! the setup needs (about 100 T). The half-period itself is a fixed number of T-states: the
//! loop has no frame-dependent part, and interrupts are off inside it (the handler's `RET`
//! returns with interrupts off, as in `border.sna`).
//!
//! `OUT (FEh),A` is contended when its I/O cycle falls on a paper line's 128 pixel T-states
//! (the ULA holds the CPU for 0-6 T). The note starts at the same T of the frame every time,
//! so every pass has the same edges; the measured half-periods are `P` plus 0-2 T on average
//! (`P` + contention), and [`TUNE_HZ`] records the frequencies the host core produces.
//!
//! One edge costs `87 + 13n + 16m` T: the `DJNZ` loop is `13n - 5` T, the `DEC C; JR NZ` loop
//! `16m - 5` T, and two loops with coprime step sizes reach every `P` above 300 T. The table
//! at `802Fh` has one 4-byte record per note, `K` (16 bits), `n`, `m`, and a record of zeros
//! that restarts the tune. The `P` are within 0.5% of the nominal half-periods
//! 3,500,000 / (2 f); the gaps are those measured on the host core:
//!
//! | Note | Nominal Hz | `n`  | `m` | `P` T | `K`   | gap T |
//! |------|-----------:|-----:|----:|------:|------:|------:|
//! | C5   |     523.25 |  244 |   5 | 3,339 |   523 | 3,303 |
//! | D5   |     587.33 |  207 |  13 | 2,986 |   585 | 2,991 |
//! | E5   |     659.26 |  192 |   5 | 2,663 |   656 | 2,539 |
//! | F5   |     698.46 |  173 |  10 | 2,496 |   700 | 2,495 |
//! | G5   |     783.99 |  157 |   7 | 2,240 |   780 | 2,239 |
//! | A5   |     880.00 |  132 |  12 | 1,995 |   875 | 1,935 |
//! | B5   |     987.77 |  123 |   5 | 1,766 |   989 | 1,711 |
//! | C6   |    1046.50 |  113 |   7 | 1,668 | 1,047 | 1,979 |
//!
//! ```text
//! 8000 DD 21 2F 80 start: LD IX,802Fh    the first record
//! 8004 DD 6E 00    note:  LD L,(IX+0)    K, the number of edges
//! 8007 DD 66 01           LD H,(IX+1)
//! 800A 7C                 LD A,H
//! 800B B5                 OR L
//! 800C 28 F2              JR Z,start     the zero record: again from the first note
//! 800E FB                 EI
//! 800F 76                 HALT           the interrupt at T 0; returns to 8010, interrupts off
//! 8010 7A          tog:   LD A,D         D = the output level, bit 4 only
//! 8011 EE 10              XOR 10h
//! 8013 57                 LD D,A
//! 8014 D3 FE              OUT (FEh),A    the edge
//! 8016 2B                 DEC HL
//! 8017 7C                 LD A,H
//! 8018 B5                 OR L
//! 8019 28 0D              JR Z,next      that was the last edge of the note
//! 801B DD 46 02           LD B,(IX+2)    n
//! 801E DD 4E 03           LD C,(IX+3)    m
//! 8021 10 FE       d1:    DJNZ d1        13n - 5 T
//! 8023 0D          d2:    DEC C
//! 8024 20 FD              JR NZ,d2       16m - 5 T
//! 8026 18 E8              JR tog
//! 8028 01 04 00    next:  LD BC,4
//! 802B DD 09              ADD IX,BC      the next record
//! 802D 18 D5              JR note
//! ```
//!
//! # `noise.sna` (not in the pack)
//!
//! The decoder's worst case for the firmware's frame budget (`emu-gate`'s `fw_budget` builds it
//! into a pack of its own; it is not committed): [`NOISE_EDGES_PER_FRAME`] beeper edges every
//! frame at pseudo-random spacings of 121-233 T plus I/O contention (a 16-bit xorshift in HL
//! picks 1-8 turns of a delay loop), dense enough to spend most of the decoder's work cap and
//! few enough (at most 256 a frame) that the decoder analyses every frame. The paper is white,
//! the border black.
//!
//! ```text
//! 8000 21 E1 AC          LD HL,ACE1h       the seed (never 0)
//! 8003 FB         frame: EI                4
//! 8004 76                HALT              4n   IM 2 + RET return to 8005, interrupts off
//! 8005 06 FA             LD B,250          7    edges this frame
//! 8007 7C         edge:  LD A,H            4    xorshift: HL ^= HL << 7, >> 9, << 8
//! 8008 1F                RRA               4
//! 8009 7D                LD A,L            4
//! 800A 1F                RRA               4
//! 800B AC                XOR H             4
//! 800C 67                LD H,A            4
//! 800D 7D                LD A,L            4
//! 800E 1F                RRA               4
//! 800F 7C                LD A,H            4
//! 8010 1F                RRA               4
//! 8011 AD                XOR L             4
//! 8012 6F                LD L,A            4
//! 8013 AC                XOR H             4
//! 8014 67                LD H,A            4
//! 8015 E6 07             AND 07h           7
//! 8017 3C                INC A             4
//! 8018 4F                LD C,A            4    1-8 turns
//! 8019 0D         wait:  DEC C             4
//! 801A 20 FD             JR NZ,wait        12/7 16c - 5 T
//! 801C 7A                LD A,D            4    D = the output level, bit 4 only
//! 801D EE 10             XOR 10h           7
//! 801F 57                LD D,A            4
//! 8020 D3 FE             OUT (FEh),A       11   the edge
//! 8022 10 E3             DJNZ edge         13/8 an edge every 105 + 16c T
//! 8024 18 DD             JR frame          12
//! ```
//!
//! 250 edges take at most 250 x 233 = 58,250 T, so every frame starts at its interrupt.
//!
//! # `burst_sna(base)` (not in the pack)
//!
//! Method B's test engine (the "Test burst" entry of `core/sound/engines_test.c`): a tiny
//! music routine assembled at any address `base` that plays [`BURST_HZ`] as square-wave bursts
//! of about [`BURST_MS`] with [`BURST_GAP_MS`] of silence after each (a burst starts every
//! 78 ms, 4 frames less 2 ms: not in step with the frames), over and over, with interrupts off.
//! The burst's half-period count `n` is written into the operand of its loop's `LD B,n` (a
//! self-modified operand), and the melody is a table the code reads through the absolute
//! operand of `LD HL,table`; both absolute operands are relocated with `base`. A half-period is
//! `13n + 54` T (plus the I/O contention of the `OUT`), so f = 3,500,000 / (2 (13n + 54)).
//! The table has 5 bytes per note: `n`, the burst's edges (16 bits, even, so it ends low) and
//! the gap loop's count (16 bits, 26 T a turn), then a 0 that starts the melody again. The
//! paper is white, the border black, SP `FD00h`, PC `base`.
//!
//! ```text
//! +00 F3           start: DI
//! +01 21 tt tt     again: LD HL,table      the absolute operand (base + 2Ch)
//! +04 7E           next:  LD A,(HL)        n
//! +05 B7                  OR A
//! +06 28 F9               JR Z,again       n = 0: the melody again
//! +08 32 oo oo            LD (tone+1),A    the absolute operand (base + 13h): self-modifies
//! +0B 23                  INC HL
//! +0C 5E                  LD E,(HL)
//! +0D 23                  INC HL
//! +0E 56                  LD D,(HL)        DE = the burst's edges
//! +0F 23                  INC HL
//! +10 0E 00               LD C,0           C = the output level, bit 4 only
//! +12 06 nn        tone:  LD B,n           7    the self-modified operand
//! +14 10 FE        wait:  DJNZ wait        13n - 5
//! +16 79                  LD A,C           4
//! +17 EE 10               XOR 10h          7
//! +19 4F                  LD C,A           4
//! +1A D3 FE               OUT (FEh),A      11   the edge (the engine's one OUT site)
//! +1C 1B                  DEC DE           6
//! +1D 7A                  LD A,D           4
//! +1E B3                  OR E             4
//! +1F 20 F1               JR NZ,tone       12   13n + 54 T an edge
//! +21 4E                  LD C,(HL)
//! +22 23                  INC HL
//! +23 46                  LD B,(HL)        BC = the gap
//! +24 23                  INC HL
//! +25 0B           gap:   DEC BC           6
//! +26 78                  LD A,B           4
//! +27 B1                  OR C             4
//! +28 20 FB               JR NZ,gap        12   26 T a turn
//! +2A 18 D8               JR next
//! +2C              table
//! ```
//!
//! # `out_sites_sna(changing)` (not in the pack)
//!
//! Method B's matching worst case for the firmware's frame budget (`emu-gate`'s `fw_budget`
//! builds it into a pack of its own; it is not committed): every frame the speaker is driven from
//! [`OUT_SITES`] OUTs at four addresses (as many as a frame lists), none of them a known engine,
//! so B looks each one up. Each OUT changes the level (high, low, high, low), 4 edges a frame.
//! With `changing`, each routine also increments a byte beside its OUT every frame: the code
//! around each OUT is never the same two frames running, so no lookup is ever spared (with it
//! unchanged, method B remembers that the four addresses match nothing). The paper is white, the
//! border black.
//!
//! ```text
//! 8000 FB         frame: EI
//! 8001 76                HALT
//! 8002 CD 00 90          CALL 9000h        the routines, at OUT_SITES less 6
//! 8005 CD 00 A0          CALL A000h
//! 8008 CD 00 B0          CALL B000h
//! 800B CD 00 C0          CALL C000h
//! 800E 18 F0             JR frame
//!
//! +0 21 vv vv            LD HL,+9          its byte
//! +3 34 (or 00)          INC (HL)          (NOP when not `changing`)
//! +4 3E 10 (or 00)       LD A,10h          high from the first and third, low from the others
//! +6 D3 FE               OUT (FEh),A       the OUT site
//! +8 C9                  RET
//! +9 00                  the byte
//! ```
//!
//! # `beep_tap()` (not in the pack)
//!
//! Method B's ROM BEEP fixture (the "ROM BEEP" entry of `core/sound/engines_builtin.h`): an
//! autostarting BASIC tape (program `beep`, line 10) of `BEEP` statements that the ROM plays
//! through its BEEPER routine (03B5h):
//!
//! ```text
//! 10 BEEP d,p: ...      the melody, [`BEEP_NOTES`] (seconds, semitones above middle C)
//! 20 BEEP .008,p: ...   five blips of 8 ms, [`BEEP_BLIPS`] (shorter than 20 ms: the beeper's)
//! ```
//!
//! Each `BEEP` is a separate call of BEEPER, the interpreter's work between them a gap of
//! silence (about 68,000 T). The program then stops with its report (0 OK) and waits in the
//! editor.
//!
//! # TZX builders (not in the pack)
//!
//! [`tzx_header`] and one function per TZX block the tape signal player plays (0x10-0x14,
//! 0x20, 0x24/0x25), for the signal tests; [`wrap_tap_as_tzx`] turns a `.tap` into standard
//! blocks with the `.tap`'s 1000 ms pause.
//!
//! # `turbo_tzx()` and `hang_tzx()` (not in the pack)
//!
//! A tape with a loader of its own, for load to snapshot (`tapeload`). Blocks:
//!
//! 1. Standard blocks: the BASIC header (`turbo`, autostart line 10) and program
//!    `LOAD ""CODE: RANDOMIZE USR 32768`, then the CODE header (`loader`, 8000h) and the loader
//!    below; each with a 1000 ms pause.
//! 2. A turbo block (0x11, [`TURBO_TIMING`]: pilot 1500 T x 2000, syncs 400/450 T, bits 520/1040
//!    T, pause 0) with the first [`TURBO_SPLIT`] bytes of [`TURBO_PAYLOAD`].
//! 3. A 0x20 pause 0: the tape stops (load to snapshot presses play again after 1 s).
//! 4. The same signal as three blocks: a pure tone (0x12, the pilot), a pulse sequence (0x13,
//!    the syncs) and pure data (0x14, pause 0) with the rest of the payload. Nothing follows.
//!
//! The loader loads both parts to [`TURBO_PAYLOAD_AT`] and jumps there. The payload sets the
//! border to [`TAPE_DONE_BORDER`] and then, once a frame (`EI; HALT` under the ROM's IM 1),
//! increments the screen byte at 4000h. `hang_tzx()` is the same tape with its last block cut
//! to half its bytes: the loader waits for the rest for ever.
//!
//! The loader times edges (level changes on port FEh bit 6) the way the ROM's does, by counting
//! turns of a sampling loop of 43 T (more on a screen line, by the I/O contention), with
//! interrupts off. A pulse of 256 turns (about 3 ms) is too long: the search starts again. The
//! search wants [`TURBO_PILOT_PULSES_SEEN`] pilot pulses in a row (each 20-49 turns: 1500 T is
//! about 32), then a short one (sync 1, under 20 turns), then any (sync 2); then each bit is two
//! edges counted together: under [`TURBO_BIT_SPLIT`] turns a 0 (1040 T, about 20 turns), else a
//! 1 (2080 T, about 40 turns), most significant bit first. (The load still works with the
//! threshold anywhere from 24 to 36.)
//!
//! ```text
//! start: DI
//!        LD IX,TURBO_PAYLOAD_AT
//!        LD DE,TURBO_SPLIT
//!        CALL block
//!        LD IX,TURBO_PAYLOAD_AT+TURBO_SPLIT
//!        LD DE,TURBO_PAYLOAD.len()-TURBO_SPLIT
//!        CALL block
//!        JP TURBO_PAYLOAD_AT
//! block:
//! hunt:  LD H,256-TURBO_PILOT_PULSES_SEEN     pilot pulses still wanted (counts up to 0)
//! pilot: CALL edge0
//!        JR NC,hunt          too long: again
//!        LD A,B
//!        CP 20
//!        JR C,hunt           too short for the pilot
//!        CP 50
//!        JR NC,hunt          too long for the pilot
//!        INC H
//!        JR NZ,pilot
//! sync:  CALL edge0
//!        JR NC,hunt
//!        LD A,B
//!        CP 20
//!        JR NC,sync          still the pilot
//!        CALL edge0          sync 2
//!        JR NC,hunt
//! byte:  LD L,1              a marker bit: 8 shifts carry it out
//! bit:   CALL edge0
//!        JR NC,hunt
//!        CALL edge           the second edge, counted on
//!        JR NC,hunt
//!        LD A,B
//!        CP TURBO_BIT_SPLIT  carry: a 0
//!        CCF
//!        RL L
//!        JR NC,bit
//!        LD (IX+0),L
//!        INC IX
//!        DEC DE
//!        LD A,D
//!        OR E
//!        JR NZ,byte
//!        RET
//! edge0: LD B,0
//! edge:  INC B               4    B turns; 0 again (256): carry clear, too long
//!        RET Z               5
//!        IN A,(FEh)          11
//!        XOR C               4    C bit 6 = the level before
//!        AND 40h             7
//!        JR Z,edge           12   43 T a turn
//!        LD A,C
//!        XOR 40h
//!        LD C,A              the new level
//!        SCF
//!        RET
//! ```
//!
//! # `rom_copy_tap()` (not in the pack)
//!
//! A standard-speed tape that the ROM trap cannot load: the BASIC loader and the code as in
//! `tape.tap`, then a headerless data block (flag FFh) of [`rom_copy_payload`] that the code
//! loads to [`ROM_COPY_PAYLOAD_AT`] through its own copy of LD-BYTES in RAM, so 0556h is never
//! reached. The code copies the ROM's 0556h-0604h to A056h and moves its eight absolute
//! `CALL`/`JP` targets (05E3h, 05E7h, 05CAh) up by 9B00h, adding 9Bh to their high bytes (the
//! table: the low bytes of their addresses in A0xxh). Border [`TAPE_DONE_BORDER`] once loaded.
//!
//! ```text
//!        LD HL,0556h
//!        LD DE,A056h
//!        LD BC,00AFh
//!        LDIR
//!        LD H,A0h
//!        LD DE,table
//!        LD B,8
//! rel:   LD A,(DE)
//!        LD L,A
//!        LD A,(HL)
//!        ADD A,9Bh
//!        LD (HL),A
//!        INC DE
//!        DJNZ rel
//!        LD IX,ROM_COPY_PAYLOAD_AT
//!        LD DE,len
//!        LD A,FFh
//!        SCF
//!        CALL A056h          the copy of LD-BYTES
//!        LD A,TAPE_DONE_BORDER
//!        JR C,done
//!        LD A,TAPE_FAIL_BORDER
//! done:  OUT (FEh),A
//!        EI
//! spin:  JR spin
//! table: 6E 7D 84 93 9D CC D7 E5
//! ```
//!
//! # `stop_tap()`, `reset_tap()` and `halt_tap()` (not in the pack)
//!
//! Autostarting BASIC tapes of one line that fail load to snapshot's success test: `10 STOP`
//! ends at the ROM's report "9 STOP statement"; `10 RANDOMIZE USR 0` resets the machine to the
//! copyright screen; `10 POKE 32768,243: POKE 32769,118: RANDOMIZE USR 32768` runs `DI; HALT`.

use std::path::{Path, PathBuf};

/// Beeper edges per frame of `border.sna`: 312 lines / 8.
pub const BORDER_EDGES_PER_FRAME: usize = 39;
/// Tone of `border.sna` measured from the host beeper renderer: 1,949 zero crossings in the
/// 50 frames (0.9984 s) after frame 10 (the design value is 976.5625 Hz).
pub const BORDER_TONE_HZ: f64 = 976.06;
/// Address of the `HALT` in `scroll.sna` and `border.sna` (`tune.sna`'s is at 800Fh).
pub const HALT: u16 = 0x8001;
/// Where the interrupt handler returns to in `scroll.sna` and `border.sna` (`tune.sna`: 8010h).
pub const RESUME: u16 = 0x8002;
/// Frames `tape.tap`'s code waits (one `HALT` each) before it loads the data block.
pub const TAPE_WAIT_FRAMES: u32 = 100;
/// Border of `tape.tap` once the data block has loaded.
pub const TAPE_DONE_BORDER: u8 = 4;
/// Border of `tape.tap` if the load failed.
pub const TAPE_FAIL_BORDER: u8 = 2;
/// The nominal pitches of `tune.sna`, C5 to C6.
pub const TUNE_NOTES_HZ: [f64; 8] = [523.25, 587.33, 659.26, 698.46, 783.99, 880.00, 987.77, 1046.50];
/// Frames each note of `tune.sna` lasts.
pub const TUNE_NOTE_FRAMES: u32 = 25;
/// The first frame (1-based, counted from loading the snapshot) with the first note's edges.
pub const TUNE_START_FRAME: u32 = 2;
/// The notes of `tune.sna` measured from the host beeper renderer: zero crossings over each
/// note's 21 middle frames, in both passes of the test (they agree to 0.02 Hz).
pub const TUNE_HZ: [f64; 8] = [524.67, 586.65, 657.01, 701.11, 781.02, 876.39, 989.70, 1049.28];
/// Beeper edges per frame of `noise.sna`.
pub const NOISE_EDGES_PER_FRAME: usize = 250;
/// Half-period counts `n` of `burst_sna`'s melody: C5 E5 G5 C6 G5 E5 D5 C5.
pub const BURST_N: [u8; 8] = [253, 200, 168, 124, 168, 200, 224, 253];
/// The pitches of `burst_sna`'s melody: 3,500,000 / (2 (13n + 54)) for each of [`BURST_N`].
pub const BURST_HZ: [f64; 8] = {
    let mut hz = [0.0; 8];
    let mut i = 0;
    while i < 8 {
        hz[i] = 3_500_000.0 / (2.0 * (13.0 * BURST_N[i] as f64 + 54.0));
        i += 1;
    }
    hz
};
/// A burst of `burst_sna` lasts about this long (the nearest even number of half-periods)...
pub const BURST_MS: f64 = 9.0;
/// ...and the silence after it this long: a burst starts every 78 ms.
pub const BURST_GAP_MS: f64 = 69.0;
/// Offset of `burst_sna`'s `OUT` from `base`.
pub const BURST_OUT: u16 = 0x1A;
/// `beep_tap`'s melody: each `BEEP` statement's duration (seconds) and pitch (semitones above
/// middle C, as `BEEP` takes it: 440 x 2^((p - 9) / 12) Hz). Every note lasts at least 20 ms.
pub const BEEP_NOTES: [(f64, i8); 9] =
    [(0.2, 0), (0.2, 4), (0.2, 7), (0.4, 12), (0.1, 9), (0.025, 14), (0.2, 7), (0.3, -5), (0.15, 24)];
/// `beep_tap`'s blips after the melody: `BEEP .008,p` for each pitch p.
pub const BEEP_BLIPS: [i8; 5] = [12, 19, 12, 19, 12];
/// Duration of each of [`BEEP_BLIPS`] (seconds).
pub const BEEP_BLIP_S: f64 = 0.008;
/// The four OUT addresses of [`out_sites_sna`].
pub const OUT_SITES: [u16; 4] = [0x9006, 0xA006, 0xB006, 0xC006];
/// Builds one program's file.
pub type Generator = fn() -> Vec<u8>;
/// The programs `fmzx-pack testprogs` writes: file name and generator.
pub const PROGRAMS: [(&str, Generator); 4] =
    [("scroll.sna", scroll_sna), ("border.sna", border_sna), ("tape.tap", tape_tap), ("tune.sna", tune_sna)];

const CODE: u16 = 0x8000;
const STACK: u16 = 0xFD00;
const HANDLER: u16 = 0xFDFD;
const VECTORS: u16 = 0xFE00;
const RAM: u16 = 0x4000;

#[rustfmt::skip]
const SCROLL_CODE: [u8; 31] = [
    0xFB,             // frame: EI
    0x76,             //        HALT
    0x21, 0x11, 0x40, //        LD HL,4011h
    0x11, 0x20, 0x00, //        LD DE,0020h
    0x06, 0xC0,       //        LD B,192
    0x7E,             // line:  LD A,(HL)
    0x1F,             //        RRA
    0x2D,             //        DEC L
    0x2D,             //        DEC L
    0x2D,             //        DEC L
    0xCB, 0x1E,       //        RR (HL)
    0x2C,             //        INC L
    0xCB, 0x1E,       //        RR (HL)
    0x2C,             //        INC L
    0xCB, 0x1E,       //        RR (HL)
    0x2C,             //        INC L
    0xCB, 0x1E,       //        RR (HL)
    0x19,             //        ADD HL,DE
    0x10, 0xED,       //        DJNZ line
    0x18, 0xE1,       //        JR frame
];

#[rustfmt::skip]
const BORDER_CODE: [u8; 35] = [
    0xFB,             // frame: EI
    0x76,             //        HALT
    0x01, 0x38, 0x01, //        LD BC,312
    0x16, 0x04,       //        LD D,4
    0x15,             // pre:   DEC D
    0x20, 0xFD,       //        JR NZ,pre
    0x18, 0x07,       //        JR out
    0x16, 0x09,       // line:  LD D,9
    0x15,             // wait:  DEC D
    0x20, 0xFD,       //        JR NZ,wait
    0x00,             //        NOP
    0x29,             //        ADD HL,HL
    0x7B,             // out:   LD A,E
    0xF6, 0x08,       //        OR 08h
    0x3C,             //        INC A
    0xE6, 0x17,       //        AND 17h
    0x5F,             //        LD E,A
    0xD3, 0xFE,       //        OUT (FEh),A
    0x0B,             //        DEC BC
    0x78,             //        LD A,B
    0xB1,             //        OR C
    0x20, 0xEB,       //        JR NZ,line
    0x18, 0xDD,       //        JR frame
];

#[rustfmt::skip]
const NOISE_CODE: [u8; 38] = [
    0x21, 0xE1, 0xAC,                    //        LD HL,ACE1h
    0xFB,                                // frame: EI
    0x76,                                //        HALT
    0x06, NOISE_EDGES_PER_FRAME as u8,   //        LD B,250
    0x7C, 0x1F, 0x7D, 0x1F, 0xAC, 0x67,  // edge:  LD A,H; RRA; LD A,L; RRA; XOR H; LD H,A
    0x7D, 0x1F, 0x7C, 0x1F, 0xAD, 0x6F,  //        LD A,L; RRA; LD A,H; RRA; XOR L; LD L,A
    0xAC, 0x67,                          //        XOR H; LD H,A
    0xE6, 0x07,                          //        AND 07h
    0x3C,                                //        INC A
    0x4F,                                //        LD C,A
    0x0D,                                // wait:  DEC C
    0x20, 0xFD,                          //        JR NZ,wait
    0x7A,                                //        LD A,D
    0xEE, 0x10,                          //        XOR 10h
    0x57,                                //        LD D,A
    0xD3, 0xFE,                          //        OUT (FEh),A
    0x10, 0xE3,                          //        DJNZ edge
    0x18, 0xDD,                          //        JR frame
];

#[rustfmt::skip]
const TAPE_CODE: [u8; 34] = [
    0x3E, 0x01,                   //        LD A,1
    0xD3, 0xFE,                   //        OUT (FEh),A
    0xFB,                         //        EI
    0x06, TAPE_WAIT_FRAMES as u8, //        LD B,TAPE_WAIT_FRAMES
    0x76,                         // wait:  HALT
    0x10, 0xFD,                   //        DJNZ wait
    0xDD, 0x21, 0x00, 0x40,       //        LD IX,4000h
    0x11, 0x00, 0x1B,             //        LD DE,1B00h
    0x3E, 0xFF,                   //        LD A,FFh
    0x37,                         //        SCF
    0xCD, 0x56, 0x05,             //        CALL 0556h
    0x3E, TAPE_DONE_BORDER,       //        LD A,TAPE_DONE_BORDER
    0x38, 0x02,                   //        JR C,done
    0x3E, TAPE_FAIL_BORDER,       //        LD A,TAPE_FAIL_BORDER
    0xD3, 0xFE,                   // done:  OUT (FEh),A
    0xFB,                         //        EI
    0x18, 0xFE,                   //        JR $
];

/// Address of the note table of `tune.sna`.
const TUNE_TABLE_AT: u16 = 0x802F;

#[rustfmt::skip]
const TUNE_CODE: [u8; 47] = [
    0xDD, 0x21, 0x2F, 0x80, // start: LD IX,802Fh
    0xDD, 0x6E, 0x00,       // note:  LD L,(IX+0)
    0xDD, 0x66, 0x01,       //        LD H,(IX+1)
    0x7C,                   //        LD A,H
    0xB5,                   //        OR L
    0x28, 0xF2,             //        JR Z,start
    0xFB,                   //        EI
    0x76,                   //        HALT
    0x7A,                   // tog:   LD A,D
    0xEE, 0x10,             //        XOR 10h
    0x57,                   //        LD D,A
    0xD3, 0xFE,             //        OUT (FEh),A
    0x2B,                   //        DEC HL
    0x7C,                   //        LD A,H
    0xB5,                   //        OR L
    0x28, 0x0D,             //        JR Z,next
    0xDD, 0x46, 0x02,       //        LD B,(IX+2)
    0xDD, 0x4E, 0x03,       //        LD C,(IX+3)
    0x10, 0xFE,             // d1:    DJNZ d1
    0x0D,                   // d2:    DEC C
    0x20, 0xFD,             //        JR NZ,d2
    0x18, 0xE8,             //        JR tog
    0x01, 0x04, 0x00,       // next:  LD BC,4
    0xDD, 0x09,             //        ADD IX,BC
    0x18, 0xD5,             //        JR note
];

/// `tune.sna`'s notes: edges `K`, `DJNZ` count `n`, `DEC C` count `m` (see the module docs).
const TUNE_TABLE: [(u16, u8, u8); 8] =
    [(523, 244, 5), (585, 207, 13), (656, 192, 5), (700, 173, 10), (780, 157, 7), (875, 132, 12), (989, 123, 5), (1047, 113, 7)];

/// 48K of RAM (from 4000h) with `code` at 8000h, the IM 2 handler and vector table, and a
/// screen whose attributes are all `attr`.
fn ram_with(code: &[u8], attr: u8) -> Vec<u8> {
    let mut ram = vec![0u8; 0xC000];
    let at = |a: u16| (a - RAM) as usize;
    ram[at(CODE)..at(CODE) + code.len()].copy_from_slice(code);
    ram[at(HANDLER)] = 0xC9; // RET
    ram[at(VECTORS)..=at(VECTORS) + 256].fill((HANDLER >> 8) as u8);
    ram[0x1800..0x1B00].fill(attr);
    ram
}

/// Offset of screen line `y`'s byte `col` in the bitmap.
fn bitmap_offset(y: usize, col: usize) -> usize {
    ((y & 0xC0) << 5) | ((y & 7) << 8) | ((y & 0x38) << 2) | col
}

/// A 48K `.sna` that starts at 8000h with SP FD00h, I FEh, IM 2, interrupts off, every other
/// register 0, and the given border.
fn sna(ram: Vec<u8>, border: u8) -> Vec<u8> {
    sna_at(ram, border, CODE)
}

/// As [`sna`], starting at `pc`.
fn sna_at(mut ram: Vec<u8>, border: u8, pc: u16) -> Vec<u8> {
    let sp = STACK - 2; // PC is popped from the stack
    let s = (sp - RAM) as usize;
    ram[s..s + 2].copy_from_slice(&pc.to_le_bytes());
    let mut h = vec![0xFEu8]; // I
    h.extend_from_slice(&[0; 18]); // HL' DE' BC' AF' HL DE BC IY IX
    h.push(0); // IFF2 (bit 2) clear
    h.push(0); // R
    h.extend_from_slice(&[0, 0]); // AF
    h.extend_from_slice(&sp.to_le_bytes());
    h.push(2); // IM
    h.push(border);
    debug_assert_eq!(h.len(), 27);
    h.extend_from_slice(&ram);
    h
}

/// The scroller: border 1, white paper, a yellow-on-blue band whose stripe on line `y`
/// starts at pixel `y % 32` of the band.
pub fn scroll_sna() -> Vec<u8> {
    let mut ram = ram_with(&SCROLL_CODE, 0x38);
    for y in 0..192 {
        let band = 0xFF00_0000u32.rotate_right(y as u32 % 32).to_be_bytes();
        for (i, b) in band.iter().enumerate() {
            ram[bitmap_offset(y, 14 + i)] = *b;
        }
    }
    for row in 0..24 {
        ram[0x1800 + row * 32 + 14..0x1800 + row * 32 + 18].fill(0x0E); // ink 6, paper 1
    }
    sna(ram, 1)
}

/// The border stripes and 976.5625 Hz tone; the paper shows eight static bars of the colours
/// 0-7, 4 columns each, for comparison with the border.
pub fn border_sna() -> Vec<u8> {
    let mut ram = ram_with(&BORDER_CODE, 0x38);
    for (i, a) in ram[0x1800..0x1B00].iter_mut().enumerate() {
        *a = ((i % 32 / 4) as u8) << 3; // paper = colour, ink 0
    }
    sna(ram, 0)
}

/// The scale: eight notes of 25 frames on the beeper, and eight stair-step bars on the screen.
pub fn tune_sna() -> Vec<u8> {
    let mut code = TUNE_CODE.to_vec();
    for (k, n, m) in TUNE_TABLE {
        code.extend_from_slice(&k.to_le_bytes());
        code.extend_from_slice(&[n, m]);
    }
    code.extend_from_slice(&[0; 4]); // K = 0 restarts the tune
    debug_assert_eq!(CODE as usize + TUNE_CODE.len(), TUNE_TABLE_AT as usize);
    let mut ram = ram_with(&code, 0x38);
    for row in 0..24 {
        for note in 0..8 {
            // bar `note` is `note + 1` character rows high, rising from the bottom
            let lit = row >= 23 - note;
            let colour = (note % 7 + 1) as u8;
            ram[0x1800 + row * 32 + note * 4..0x1800 + row * 32 + note * 4 + 4].fill(if lit { colour << 3 } else { 0x38 });
        }
    }
    sna(ram, 0)
}

/// The decoder's busiest sound: [`NOISE_EDGES_PER_FRAME`] edges a frame at pseudo-random
/// spacings (see the module docs). Not one of [`PROGRAMS`]: the pack does not carry it.
pub fn noise_sna() -> Vec<u8> {
    sna(ram_with(&NOISE_CODE, 0x38), 0)
}

/// Method B's matching worst case: four unknown OUT sites a frame, the code around them
/// changing every frame if `changing` (see the module docs). Not one of [`PROGRAMS`].
pub fn out_sites_sna(changing: bool) -> Vec<u8> {
    let mut main = vec![0xFB, 0x76]; // frame: EI; HALT
    for site in OUT_SITES {
        main.push(0xCD); // CALL
        main.extend_from_slice(&(site - 6).to_le_bytes());
    }
    main.extend_from_slice(&[0x18, 0xF0]); // JR frame
    let mut ram = ram_with(&main, 0x38);
    for (k, site) in OUT_SITES.into_iter().enumerate() {
        let at = (site - 6 - RAM) as usize;
        let var = (site + 3).to_le_bytes();
        let level = if k % 2 == 0 { 0x10 } else { 0x00 };
        let routine = [0x21, var[0], var[1], if changing { 0x34 } else { 0x00 }, 0x3E, level, 0xD3, 0xFE, 0xC9, 0x00];
        ram[at..at + routine.len()].copy_from_slice(&routine);
    }
    sna(ram, 0)
}

#[rustfmt::skip]
const BURST_CODE: [u8; 0x2C] = [
    0xF3,             // start: DI
    0x21, 0x00, 0x00, // again: LD HL,table (relocated)
    0x7E,             // next:  LD A,(HL)
    0xB7,             //        OR A
    0x28, 0xF9,       //        JR Z,again
    0x32, 0x00, 0x00, //        LD (tone+1),A (relocated)
    0x23,             //        INC HL
    0x5E,             //        LD E,(HL)
    0x23,             //        INC HL
    0x56,             //        LD D,(HL)
    0x23,             //        INC HL
    0x0E, 0x00,       //        LD C,0
    0x06, 0x00,       // tone:  LD B,n (self-modified)
    0x10, 0xFE,       // wait:  DJNZ wait
    0x79,             //        LD A,C
    0xEE, 0x10,       //        XOR 10h
    0x4F,             //        LD C,A
    0xD3, 0xFE,       //        OUT (FEh),A
    0x1B,             //        DEC DE
    0x7A,             //        LD A,D
    0xB3,             //        OR E
    0x20, 0xF1,       //        JR NZ,tone
    0x4E,             //        LD C,(HL)
    0x23,             //        INC HL
    0x46,             //        LD B,(HL)
    0x23,             //        INC HL
    0x0B,             // gap:   DEC BC
    0x78,             //        LD A,B
    0xB1,             //        OR C
    0x20, 0xFB,       //        JR NZ,gap
    0x18, 0xD8,       //        JR next
];

/// Method B's test engine assembled at `base` (8000h-FB00h), playing [`BURST_HZ`] in bursts
/// (see the module docs). Not one of [`PROGRAMS`].
pub fn burst_sna(base: u16) -> Vec<u8> {
    assert!((0x8000..=0xFB00).contains(&base), "burst_sna at {base:04X}");
    let mut code = BURST_CODE.to_vec();
    code[2..4].copy_from_slice(&(base + BURST_CODE.len() as u16).to_le_bytes());
    code[9..11].copy_from_slice(&(base + 0x13).to_le_bytes());
    debug_assert_eq!(code[BURST_OUT as usize], 0xD3);
    // a burst: the nearest even number of half-periods to BURST_MS; then the gap loop's count
    // that makes the note last 78 ms: 98 T of setup, the edges and 26 T a turn of the gap
    let period = ((BURST_MS + BURST_GAP_MS) * 3_500.0) as u32;
    for n in BURST_N {
        let half = 13 * n as u32 + 54;
        let edges = 2 * ((BURST_MS * 3_500.0 / half as f64 / 2.0).round() as u32).max(1);
        let gap = (period - 98 - edges * half) / 26;
        code.push(n);
        code.extend_from_slice(&(edges as u16).to_le_bytes());
        code.extend_from_slice(&(gap as u16).to_le_bytes());
    }
    code.push(0);
    let mut ram = vec![0u8; 0xC000];
    let at = |a: u16| (a - RAM) as usize;
    ram[at(base)..at(base) + code.len()].copy_from_slice(&code);
    ram[0x1800..0x1B00].fill(0x38);
    sna_at(ram, 0, base)
}

/// A `.tap` block: length, flag, data, XOR checksum.
fn tap_block(flag: u8, data: &[u8]) -> Vec<u8> {
    let mut b = vec![flag];
    b.extend_from_slice(data);
    b.push(b.iter().fold(0, |x, &y| x ^ y));
    let mut out = (b.len() as u16).to_le_bytes().to_vec();
    out.extend(b);
    out
}

/// A header block: `kind` 0 program, 3 code; the name is padded to 10 characters.
fn tap_header(kind: u8, name: &str, len: u16, p1: u16, p2: u16) -> Vec<u8> {
    let mut d = vec![kind];
    d.extend_from_slice(format!("{name:<10}").as_bytes());
    for w in [len, p1, p2] {
        d.extend_from_slice(&w.to_le_bytes());
    }
    tap_block(0, &d)
}

/// The BASIC program `10 LOAD ""CODE: RANDOMIZE USR 32768`.
fn usr_prog() -> Vec<u8> {
    let mut line = vec![0xEF, 0x22, 0x22, 0xAF, 0x3A, 0xF9, 0xC0];
    line.extend_from_slice(b"32768");
    line.extend_from_slice(&[0x0E, 0x00, 0x00, 0x00, 0x80, 0x00]);
    let mut prog = 10u16.to_be_bytes().to_vec();
    prog.extend_from_slice(&((line.len() + 1) as u16).to_le_bytes());
    prog.extend_from_slice(&line);
    prog.push(0x0D);
    prog
}

/// The multi-load tape: the autostart BASIC loader, the code, and the screen image the code
/// loads while it runs.
pub fn tape_tap() -> Vec<u8> {
    let prog = usr_prog();
    let mut image = vec![0u8; 0x1B00];
    for y in 0..192 {
        image[bitmap_offset(y, 0)..bitmap_offset(y, 0) + 32].fill(y as u8);
    }
    for (i, a) in image[0x1800..].iter_mut().enumerate() {
        let paper = (i % 32 / 4) as u8;
        let ink = (paper + 1 + (i / 32 % 7) as u8) % 8;
        *a = 0x40 | paper << 3 | ink;
    }
    let mut tap = tap_header(0, "boot", prog.len() as u16, 10, prog.len() as u16);
    tap.extend(tap_block(0xFF, &prog));
    tap.extend(tap_header(3, "tape", TAPE_CODE.len() as u16, CODE, 0));
    tap.extend(tap_block(0xFF, &TAPE_CODE));
    tap.extend(tap_block(0xFF, &image));
    tap
}

/// A number as the ROM stores it after its text in a BASIC line: an integer up to 65535 in the
/// small form, else the 5-byte floating-point form (exponent + 128, then the mantissa, its top
/// bit the sign). Non-negative only (a BASIC minus is an operator).
fn zx_number(x: f64) -> [u8; 5] {
    assert!((0.0..1e30).contains(&x), "{x}");
    if x.fract() == 0.0 && x <= 65535.0 {
        let v = (x as u16).to_le_bytes();
        return [0, 0, v[0], v[1], 0];
    }
    let mut exp = x.log2().floor() as i32 + 1; // x = m 2^exp, 0.5 <= m < 1
    let mut m = (x / 2f64.powi(exp) * 4_294_967_296.0).round() as u64;
    if m >= 1 << 32 {
        m >>= 1;
        exp += 1;
    }
    let m = (m as u32 & 0x7FFF_FFFF).to_be_bytes();
    [(128 + exp) as u8, m[0], m[1], m[2], m[3]]
}

/// A number in a BASIC line: its text, then 0Eh and its stored form (a leading minus as text).
fn basic_number(out: &mut Vec<u8>, x: f64) {
    if x < 0.0 {
        out.push(b'-');
    }
    out.extend_from_slice(format!("{}", x.abs()).as_bytes());
    out.push(0x0E);
    out.extend_from_slice(&zx_number(x.abs()));
}

/// A BASIC line: number, length, the statements `BEEP d,p` joined by `:`, ENTER.
fn beep_line(number: u16, beeps: &[(f64, i8)]) -> Vec<u8> {
    let mut body = Vec::new();
    for (k, &(d, p)) in beeps.iter().enumerate() {
        if k > 0 {
            body.push(b':');
        }
        body.push(0xD7); // BEEP
        basic_number(&mut body, d);
        body.push(b',');
        basic_number(&mut body, f64::from(p));
    }
    body.push(0x0D);
    let mut line = number.to_be_bytes().to_vec();
    line.extend_from_slice(&(body.len() as u16).to_le_bytes());
    line.extend(body);
    line
}

/// The ROM BEEP fixture: an autostarting BASIC tape of `BEEP` statements, the melody
/// [`BEEP_NOTES`] then the blips [`BEEP_BLIPS`] (see the module docs). Not one of [`PROGRAMS`].
pub fn beep_tap() -> Vec<u8> {
    let blips: Vec<(f64, i8)> = BEEP_BLIPS.iter().map(|&p| (BEEP_BLIP_S, p)).collect();
    let mut prog = beep_line(10, &BEEP_NOTES);
    prog.extend(beep_line(20, &blips));
    let mut tap = tap_header(0, "beep", prog.len() as u16, 10, prog.len() as u16);
    tap.extend(tap_block(0xFF, &prog));
    tap
}

/// The 10-byte TZX file header, version 1.20. TZX blocks follow it.
pub fn tzx_header() -> Vec<u8> {
    let mut d = b"ZXTape!\x1a".to_vec();
    d.extend_from_slice(&[1, 20]);
    d
}

/// TZX block 0x10, a standard-speed block: `block` is one `.tap` block without its length
/// (flag, data, checksum), then `pause_ms` of silence.
pub fn tzx_standard(pause_ms: u16, block: &[u8]) -> Vec<u8> {
    let mut d = vec![0x10];
    d.extend_from_slice(&pause_ms.to_le_bytes());
    d.extend_from_slice(&(block.len() as u16).to_le_bytes());
    d.extend_from_slice(block);
    d
}

/// The timings of a TZX block 0x11 (turbo speed), in T-states.
#[derive(Clone, Copy, Debug)]
pub struct TzxTurbo {
    pub pilot: u16,
    pub sync1: u16,
    pub sync2: u16,
    pub zero: u16,
    pub one: u16,
    pub pilot_pulses: u16,
    /// Bits of the last data byte that are played (1-8), from its top bit down.
    pub last_bits: u8,
    pub pause_ms: u16,
}

/// TZX block 0x11, a turbo-speed block.
pub fn tzx_turbo(t: &TzxTurbo, data: &[u8]) -> Vec<u8> {
    let mut d = vec![0x11];
    for w in [t.pilot, t.sync1, t.sync2, t.zero, t.one, t.pilot_pulses] {
        d.extend_from_slice(&w.to_le_bytes());
    }
    d.push(t.last_bits);
    d.extend_from_slice(&t.pause_ms.to_le_bytes());
    d.extend_from_slice(&(data.len() as u32).to_le_bytes()[..3]);
    d.extend_from_slice(data);
    d
}

/// TZX block 0x12, a pure tone: `count` pulses of `pulse` T-states.
pub fn tzx_tone(pulse: u16, count: u16) -> Vec<u8> {
    let mut d = vec![0x12];
    d.extend_from_slice(&pulse.to_le_bytes());
    d.extend_from_slice(&count.to_le_bytes());
    d
}

/// TZX block 0x13, a sequence of pulses of the given lengths (at most 255).
pub fn tzx_pulses(lens: &[u16]) -> Vec<u8> {
    let mut d = vec![0x13, lens.len() as u8];
    for l in lens {
        d.extend_from_slice(&l.to_le_bytes());
    }
    d
}

/// TZX block 0x14, pure data: bits only, no pilot or sync.
pub fn tzx_pure_data(zero: u16, one: u16, last_bits: u8, pause_ms: u16, data: &[u8]) -> Vec<u8> {
    let mut d = vec![0x14];
    d.extend_from_slice(&zero.to_le_bytes());
    d.extend_from_slice(&one.to_le_bytes());
    d.push(last_bits);
    d.extend_from_slice(&pause_ms.to_le_bytes());
    d.extend_from_slice(&(data.len() as u32).to_le_bytes()[..3]);
    d.extend_from_slice(data);
    d
}

/// TZX block 0x20: a pause of `ms` milliseconds, or with 0 a stop of the tape.
pub fn tzx_pause(ms: u16) -> Vec<u8> {
    let mut d = vec![0x20];
    d.extend_from_slice(&ms.to_le_bytes());
    d
}

/// TZX blocks 0x24 and 0x25 around `body`: the body played `count` times.
pub fn tzx_loop(count: u16, body: &[u8]) -> Vec<u8> {
    let mut d = vec![0x24];
    d.extend_from_slice(&count.to_le_bytes());
    d.extend_from_slice(body);
    d.push(0x25);
    d
}

/// A `.tap` as a TZX: each block becomes a block 0x10 with a 1000 ms pause. The `.tap`'s block
/// lengths must fit it.
pub fn wrap_tap_as_tzx(tap: &[u8]) -> Vec<u8> {
    let mut d = tzx_header();
    let mut i = 0;
    while i < tap.len() {
        let n = u16::from_le_bytes([tap[i], tap[i + 1]]) as usize;
        d.extend(tzx_standard(1000, &tap[i + 2..i + 2 + n]));
        i += 2 + n;
    }
    d
}

/// A small Z80 code builder for the generated loaders: bytes, labels, and references to
/// labels from relative (`JR`, `DJNZ`) and absolute (`JP`, `CALL`, `LD rr,nn`) operands, which
/// `finish` fills in.
struct Asm {
    org: u16,
    code: Vec<u8>,
    labels: Vec<(&'static str, u16)>,
    /// Operand offset, label, relative.
    refs: Vec<(usize, &'static str, bool)>,
}

impl Asm {
    fn new(org: u16) -> Self {
        Asm { org, code: Vec::new(), labels: Vec::new(), refs: Vec::new() }
    }

    fn op(&mut self, bytes: &[u8]) -> &mut Self {
        self.code.extend_from_slice(bytes);
        self
    }

    fn label(&mut self, name: &'static str) -> &mut Self {
        assert!(self.labels.iter().all(|l| l.0 != name), "label {name} twice");
        self.labels.push((name, self.org + self.code.len() as u16));
        self
    }

    /// `JR`/`JR cc`/`DJNZ` (`opcode`) to `name`.
    fn rel(&mut self, opcode: u8, name: &'static str) -> &mut Self {
        self.code.push(opcode);
        self.refs.push((self.code.len(), name, true));
        self.code.push(0);
        self
    }

    /// An instruction (`opcode`, prefix included) whose last two bytes are `name`'s address.
    fn abs(&mut self, opcode: &[u8], name: &'static str) -> &mut Self {
        self.code.extend_from_slice(opcode);
        self.refs.push((self.code.len(), name, false));
        self.code.extend_from_slice(&[0, 0]);
        self
    }

    fn finish(&self) -> Vec<u8> {
        let mut code = self.code.clone();
        for &(at, name, rel) in &self.refs {
            let target = self.labels.iter().find(|l| l.0 == name).unwrap_or_else(|| panic!("no label {name}")).1;
            if rel {
                let from = self.org as i32 + at as i32 + 1;
                let d = target as i32 - from;
                assert!((-128..128).contains(&d), "JR to {name} out of range");
                code[at] = d as i8 as u8;
            } else {
                code[at..at + 2].copy_from_slice(&target.to_le_bytes());
            }
        }
        code
    }
}

/// Where `turbo_tzx()` loads [`TURBO_PAYLOAD`].
pub const TURBO_PAYLOAD_AT: u16 = 0x9000;
/// Bytes of [`TURBO_PAYLOAD`] in the turbo block; the rest is in the pure data block.
pub const TURBO_SPLIT: usize = 128;
/// The timings of `turbo_tzx()`'s blocks, in T-states (the 0x12-0x14 part uses the same).
pub const TURBO_TIMING: TzxTurbo =
    TzxTurbo { pilot: 1500, sync1: 400, sync2: 450, zero: 520, one: 1040, pilot_pulses: 2000, last_bits: 8, pause_ms: 0 };
/// Pilot pulses in a row the turbo loader wants before it looks for the sync.
pub const TURBO_PILOT_PULSES_SEEN: u8 = 200;
/// The turbo loader's bit threshold, in turns of its sampling loop (both edges of a bit).
pub const TURBO_BIT_SPLIT: u8 = 30;

const TURBO_PAYLOAD_BYTES: [u8; 256] = {
    #[rustfmt::skip]
    let code = [
        0x3E, TAPE_DONE_BORDER, //       LD A,TAPE_DONE_BORDER
        0xD3, 0xFE,             //       OUT (FEh),A
        0xFB,                   // loop: EI
        0x76,                   //       HALT
        0x21, 0x00, 0x40,       //       LD HL,4000h
        0x34,                   //       INC (HL)
        0x18, 0xF8,             //       JR loop
    ];
    let mut a = [0u8; 256];
    let mut i = 0;
    while i < 256 {
        a[i] = if i < code.len() { code[i] } else { (i * 151 + 7) as u8 };
        i += 1;
    }
    a
};
/// What `turbo_tzx()` loads to [`TURBO_PAYLOAD_AT`]: the code that runs once loaded (border
/// [`TAPE_DONE_BORDER`], then a screen byte incremented every frame), then a pattern.
pub const TURBO_PAYLOAD: &[u8] = &TURBO_PAYLOAD_BYTES;

/// `turbo_tzx()`'s loader, at 8000h (see the module docs).
fn turbo_loader() -> Vec<u8> {
    let at = TURBO_PAYLOAD_AT.to_le_bytes();
    let second = (TURBO_PAYLOAD_AT + TURBO_SPLIT as u16).to_le_bytes();
    let first_len = (TURBO_SPLIT as u16).to_le_bytes();
    let second_len = ((TURBO_PAYLOAD.len() - TURBO_SPLIT) as u16).to_le_bytes();
    let mut a = Asm::new(CODE);
    a.op(&[0xF3]) // DI
        .op(&[0xDD, 0x21, at[0], at[1]]) // LD IX,payload
        .op(&[0x11, first_len[0], first_len[1]]) // LD DE,first part
        .abs(&[0xCD], "block")
        .op(&[0xDD, 0x21, second[0], second[1]])
        .op(&[0x11, second_len[0], second_len[1]])
        .abs(&[0xCD], "block")
        .op(&[0xC3, at[0], at[1]]); // JP payload
    a.label("block")
        .label("hunt")
        .op(&[0x26, 0u8.wrapping_sub(TURBO_PILOT_PULSES_SEEN)]) // LD H,256-n
        .label("pilot")
        .abs(&[0xCD], "edge0")
        .rel(0x30, "hunt") // JR NC
        .op(&[0x78, 0xFE, 20]) // LD A,B; CP 20
        .rel(0x38, "hunt") // JR C
        .op(&[0xFE, 50]) // CP 50
        .rel(0x30, "hunt")
        .op(&[0x24]) // INC H
        .rel(0x20, "pilot") // JR NZ
        .label("sync")
        .abs(&[0xCD], "edge0")
        .rel(0x30, "hunt")
        .op(&[0x78, 0xFE, 20])
        .rel(0x30, "sync")
        .abs(&[0xCD], "edge0") // sync 2
        .rel(0x30, "hunt")
        .label("byte")
        .op(&[0x2E, 0x01]) // LD L,1
        .label("bit")
        .abs(&[0xCD], "edge0")
        .rel(0x30, "hunt")
        .abs(&[0xCD], "edge")
        .rel(0x30, "hunt")
        .op(&[0x78, 0xFE, TURBO_BIT_SPLIT, 0x3F, 0xCB, 0x15]) // LD A,B; CP split; CCF; RL L
        .rel(0x30, "bit")
        .op(&[0xDD, 0x75, 0x00, 0xDD, 0x23, 0x1B, 0x7A, 0xB3]) // LD (IX+0),L; INC IX; DEC DE; LD A,D; OR E
        .rel(0x20, "byte")
        .op(&[0xC9]); // RET
    a.label("edge0")
        .op(&[0x06, 0x00]) // LD B,0
        .label("edge")
        .op(&[0x04, 0xC8, 0xDB, 0xFE, 0xA9, 0xE6, 0x40]) // INC B; RET Z; IN A,(FEh); XOR C; AND 40h
        .rel(0x28, "edge") // JR Z
        .op(&[0x79, 0xEE, 0x40, 0x4F, 0x37, 0xC9]); // LD A,C; XOR 40h; LD C,A; SCF; RET
    a.finish()
}

/// Standard-speed blocks as TZX blocks 0x10 with a 1000 ms pause (the `.tap` blocks given with
/// their lengths).
fn tzx_standard_blocks(blocks: &[Vec<u8>]) -> Vec<u8> {
    blocks.iter().flat_map(|b| tzx_standard(1000, &b[2..])).collect()
}

/// The turbo tape (see the module docs); with `cut`, the last block has half its bytes.
fn turbo_tape(cut: bool) -> Vec<u8> {
    let prog = usr_prog();
    let code = turbo_loader();
    let mut d = tzx_header();
    d.extend(tzx_standard_blocks(&[
        tap_header(0, "turbo", prog.len() as u16, 10, prog.len() as u16),
        tap_block(0xFF, &prog),
        tap_header(3, "loader", code.len() as u16, CODE, 0),
        tap_block(0xFF, &code),
    ]));
    let (first, rest) = TURBO_PAYLOAD.split_at(TURBO_SPLIT);
    let t = TURBO_TIMING;
    d.extend(tzx_turbo(&t, first));
    d.extend(tzx_pause(0));
    d.extend(tzx_tone(t.pilot, t.pilot_pulses));
    d.extend(tzx_pulses(&[t.sync1, t.sync2]));
    let rest = if cut { &rest[..rest.len() / 2] } else { rest };
    d.extend(tzx_pure_data(t.zero, t.one, 8, 0, rest));
    d
}

/// A tape with a turbo loader of its own (see the module docs). Not one of [`PROGRAMS`].
pub fn turbo_tzx() -> Vec<u8> {
    turbo_tape(false)
}

/// [`turbo_tzx`] with its last block cut short: the loader never finishes. Not one of
/// [`PROGRAMS`].
pub fn hang_tzx() -> Vec<u8> {
    turbo_tape(true)
}

/// Where `rom_copy_tap()`'s code loads [`rom_copy_payload`].
pub const ROM_COPY_PAYLOAD_AT: u16 = 0xC000;
/// Where `rom_copy_tap()`'s code copies LD-BYTES (0556h-0604h): the targets move by 9B00h.
const ROM_COPY_AT: u16 = 0xA056;
/// The low bytes of the addresses, in the copy, of the high bytes of its absolute targets.
const ROM_COPY_RELOCS: [u8; 8] = [0x6E, 0x7D, 0x84, 0x93, 0x9D, 0xCC, 0xD7, 0xE5];

/// The headerless block of `rom_copy_tap()`: 768 bytes of a pattern.
pub fn rom_copy_payload() -> Vec<u8> {
    (0..768u32).map(|i| (i * 73 + 29) as u8).collect()
}

/// `rom_copy_tap()`'s code, at 8000h (see the module docs).
fn rom_copy_code() -> Vec<u8> {
    let at = ROM_COPY_AT.to_le_bytes();
    let dest = ROM_COPY_PAYLOAD_AT.to_le_bytes();
    let len = (rom_copy_payload().len() as u16).to_le_bytes();
    let mut a = Asm::new(CODE);
    a.op(&[0x21, 0x56, 0x05]) // LD HL,0556h
        .op(&[0x11, at[0], at[1]]) // LD DE,copy
        .op(&[0x01, 0xAF, 0x00, 0xED, 0xB0]) // LD BC,00AFh; LDIR
        .op(&[0x26, (ROM_COPY_AT >> 8) as u8]) // LD H,A0h
        .abs(&[0x11], "table") // LD DE,table
        .op(&[0x06, ROM_COPY_RELOCS.len() as u8]) // LD B,8
        .label("rel")
        .op(&[0x1A, 0x6F, 0x7E]) // LD A,(DE); LD L,A; LD A,(HL)
        .op(&[0xC6, ((ROM_COPY_AT - 0x0556) >> 8) as u8, 0x77, 0x13]) // ADD A,9Bh; LD (HL),A; INC DE
        .rel(0x10, "rel") // DJNZ
        .op(&[0xDD, 0x21, dest[0], dest[1]]) // LD IX,payload
        .op(&[0x11, len[0], len[1], 0x3E, 0xFF, 0x37]) // LD DE,len; LD A,FFh; SCF
        .op(&[0xCD, at[0], at[1]]) // CALL copy
        .op(&[0x3E, TAPE_DONE_BORDER])
        .rel(0x38, "done") // JR C
        .op(&[0x3E, TAPE_FAIL_BORDER])
        .label("done")
        .op(&[0xD3, 0xFE, 0xFB]) // OUT (FEh),A; EI
        .label("spin")
        .rel(0x18, "spin")
        .label("table")
        .op(&ROM_COPY_RELOCS);
    a.finish()
}

/// A standard-speed tape whose last block only a loader of its own reads (see the module docs).
/// Not one of [`PROGRAMS`].
pub fn rom_copy_tap() -> Vec<u8> {
    let prog = usr_prog();
    let code = rom_copy_code();
    let mut tap = tap_header(0, "copy", prog.len() as u16, 10, prog.len() as u16);
    tap.extend(tap_block(0xFF, &prog));
    tap.extend(tap_header(3, "copy", code.len() as u16, CODE, 0));
    tap.extend(tap_block(0xFF, &code));
    tap.extend(tap_block(0xFF, &rom_copy_payload()));
    tap
}

/// An autostarting BASIC tape (program `name`) of one line 10 with the statements `body`.
fn basic_tap(name: &str, mut body: Vec<u8>) -> Vec<u8> {
    body.push(0x0D);
    let mut prog = 10u16.to_be_bytes().to_vec();
    prog.extend_from_slice(&(body.len() as u16).to_le_bytes());
    prog.extend(body);
    let mut tap = tap_header(0, name, prog.len() as u16, 10, prog.len() as u16);
    tap.extend(tap_block(0xFF, &prog));
    tap
}

/// An autostarting BASIC tape whose program is `10 STOP`. Not one of [`PROGRAMS`].
pub fn stop_tap() -> Vec<u8> {
    basic_tap("stop", vec![0xE2])
}

/// An autostarting BASIC tape whose program is `10 REM`: it ends with "0 OK" and the machine
/// waits at the BASIC editor. Not one of [`PROGRAMS`].
pub fn ok_tap() -> Vec<u8> {
    basic_tap("ok", vec![0xEA])
}

/// An autostarting BASIC tape whose program is `10 PAUSE 0`: the program runs, in the ROM,
/// waiting for a key. Not one of [`PROGRAMS`].
pub fn pause_tap() -> Vec<u8> {
    let mut body = vec![0xF2]; // PAUSE
    basic_number(&mut body, 0.0);
    basic_tap("pause", body)
}

/// An autostarting BASIC tape whose program is `10 INPUT a`: the program runs, in the ROM's
/// editor in INPUT mode. Not one of [`PROGRAMS`].
pub fn input_tap() -> Vec<u8> {
    basic_tap("input", vec![0xEE, b'a'])
}

/// An autostarting BASIC tape whose program is `10 RANDOMIZE USR 0`: the machine resets to
/// the copyright screen. Not one of [`PROGRAMS`].
pub fn reset_tap() -> Vec<u8> {
    let mut body = vec![0xF9, 0xC0]; // RANDOMIZE USR
    basic_number(&mut body, 0.0);
    basic_tap("reset", body)
}

/// An autostarting BASIC tape whose program is
/// `10 POKE 32768,243: POKE 32769,118: RANDOMIZE USR 32768`: `DI; HALT` at 8000h, halted with
/// interrupts off for ever. Not one of [`PROGRAMS`].
pub fn halt_tap() -> Vec<u8> {
    let mut body = Vec::new();
    for (a, v) in [(32768.0, 243.0), (32769.0, 118.0)] {
        body.push(0xF4); // POKE
        basic_number(&mut body, a);
        body.push(b',');
        basic_number(&mut body, v);
        body.push(b':');
    }
    body.extend_from_slice(&[0xF9, 0xC0]);
    basic_number(&mut body, 32768.0);
    basic_tap("halt", body)
}

/// Writes every program into `dir` (created if needed); returns the paths written.
pub fn write_all(dir: &Path) -> Result<Vec<PathBuf>, String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    PROGRAMS
        .iter()
        .map(|(name, make)| {
            let path = dir.join(name);
            std::fs::write(&path, make()).map_err(|e| format!("{}: {e}", path.display()))?;
            Ok(path)
        })
        .collect()
}
