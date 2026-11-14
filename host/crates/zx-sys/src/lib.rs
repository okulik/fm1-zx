// SPDX-FileCopyrightText: 2026 FM-1 ZX contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Raw bindings to the C machine core, and the safe [`Machine`] wrapper.

use std::cell::UnsafeCell;
use std::marker::PhantomData;
use std::mem::MaybeUninit;

#[allow(
    non_camel_case_types,
    non_snake_case,
    non_upper_case_globals,
    dead_code,
    unnecessary_transmutes,
    clippy::all
)]
mod sys {
    include!(concat!(env!("OUT_DIR"), "/bindings.rs"));
}

pub use sys::{lcd_stats, zx_edge, zx_regs, ZX_ERR_FORMAT, ZX_OUT_SITES, ZX_ERR_UNSUPPORTED, ZX_FRAME_TSTATES, ZX_RAM_SIZE, ZX_ROM_SIZE};

/// An `OUT` that changed the speaker level during a frame (see `zx_out_sites`).
#[derive(Clone, Copy, Debug)]
pub struct OutSite {
    /// Address of the `OUT` instruction's first byte.
    pub pc: u16,
    /// Speaker edges it made this frame.
    pub count: u16,
    /// The registers at its last such `OUT`; `regs.pc` is the `OUT`'s address.
    pub regs: zx_regs,
}

static ROM_48: &[u8; ZX_ROM_SIZE as usize] = include_bytes!("../../../../third_party/zx-rom/48.rom");

/// A 48K Spectrum. Not `Sync`: the C core has no locking.
pub struct Machine {
    raw: Box<sys::zx_machine>,
    /// The inserted .tap; the C core keeps a pointer into this heap buffer.
    tape: Option<Box<[u8]>>,
    /// A ROM other than the built-in one ([`Machine::with_rom`]); the C core keeps a pointer to it.
    _rom: Option<Box<[u8]>>,
    /// The inserted tape signal: the player's state and a copy of the file; the C core keeps
    /// pointers to both.
    sig: Option<(Box<sys::zx_tapesig>, Box<[u8]>)>,
    /// The port FEh read counter of [`Machine::port_fe_probe`]; the C core keeps a pointer to it.
    probe: Option<Box<UnsafeCell<u32>>>,
    _not_sync: PhantomData<*const ()>,
}

impl Machine {
    /// A powered-on machine with the built-in 48K ROM.
    pub fn new() -> Self {
        // SAFETY: zx_machine is plain data (integers, arrays, nullable fn
        // pointers); all-zero is a valid value, and zx_init sets it up.
        let mut raw: Box<sys::zx_machine> = Box::new(unsafe { MaybeUninit::zeroed().assume_init() });
        // SAFETY: the ROM is 'static, so it outlives the machine; `raw` is
        // boxed, so the CPU's context pointer to it stays valid.
        unsafe { sys::zx_init(&mut *raw, ROM_48.as_ptr()) };
        Machine { raw, tape: None, _rom: None, sig: None, probe: None, _not_sync: PhantomData }
    }

    /// A powered-on machine with another 16K ROM (host tests: an altered copy of the 48K ROM).
    pub fn with_rom(rom: &[u8]) -> Self {
        assert_eq!(rom.len(), ZX_ROM_SIZE as usize, "ROM size");
        let rom: Box<[u8]> = rom.into();
        // SAFETY: as in `new`.
        let mut raw: Box<sys::zx_machine> = Box::new(unsafe { MaybeUninit::zeroed().assume_init() });
        // SAFETY: the ROM is a heap buffer the machine keeps (it does not move) for as long as
        // the C side may read it; `raw` is boxed, as in `new`.
        unsafe { sys::zx_init(&mut *raw, rom.as_ptr()) };
        Machine { raw, tape: None, _rom: Some(rom), sig: None, probe: None, _not_sync: PhantomData }
    }

    /// Runs one frame; an inserted tape signal is then advanced to the frame end.
    pub fn run_frame(&mut self) {
        // SAFETY: `raw` was initialised by zx_init.
        unsafe { sys::zx_run_frame(&mut *self.raw) }
        if self.sig.is_some() {
            // SAFETY: the player's state and data are alive in `self.sig`.
            unsafe { sys::zx_tapesig_frame(&mut *self.raw) }
        }
    }

    pub fn regs(&self) -> zx_regs {
        let mut r = MaybeUninit::<zx_regs>::zeroed();
        // SAFETY: both pointers are valid; zx_get_regs fills every field.
        unsafe {
            sys::zx_get_regs(&*self.raw, r.as_mut_ptr());
            r.assume_init()
        }
    }

    pub fn set_regs(&mut self, r: &zx_regs) {
        // SAFETY: both pointers are valid.
        unsafe { sys::zx_set_regs(&mut *self.raw, r) }
    }

    pub fn peek(&self, addr: u16) -> u8 {
        // SAFETY: any 16-bit address is valid.
        unsafe { sys::zx_peek(&*self.raw, addr) }
    }

    pub fn poke(&mut self, addr: u16, value: u8) {
        // SAFETY: any 16-bit address is valid; ROM writes are ignored.
        unsafe { sys::zx_poke(&mut *self.raw, addr, value) }
    }

    /// T-states already run into the next frame.
    pub fn frame_tstate(&self) -> u32 {
        // SAFETY: `raw` was initialised by zx_init.
        unsafe { sys::zx_frame_tstate(&*self.raw) }
    }

    pub fn frame_count(&self) -> u32 {
        // SAFETY: `raw` was initialised by zx_init.
        unsafe { sys::zx_frame_count(&*self.raw) }
    }

    /// Row r is address line A(8+r); a set bit is a pressed key.
    pub fn set_keys(&mut self, rows: [u8; 8]) {
        // SAFETY: `rows` is 8 bytes.
        unsafe { sys::zx_set_keys(&mut *self.raw, rows.as_ptr()) }
    }

    /// Bit 0 right, 1 left, 2 down, 3 up, 4 fire.
    pub fn set_kempston(&mut self, bits: u8) {
        // SAFETY: `raw` was initialised by zx_init.
        unsafe { sys::zx_set_kempston(&mut *self.raw, bits) }
    }

    /// Beeper level changes of the frame run last.
    pub fn edges(&self) -> &[zx_edge] {
        let mut n = 0u32;
        // SAFETY: the returned array holds `n` edges and lives as long as `self`.
        unsafe {
            let p = sys::zx_edges(&*self.raw, &mut n);
            std::slice::from_raw_parts(p, n as usize)
        }
    }

    /// The distinct `OUT`s that made the last frame's speaker edges (at most `ZX_OUT_SITES`).
    pub fn out_sites(&self) -> Vec<OutSite> {
        let mut n = 0u32;
        // SAFETY: the returned array holds `n` sites and lives as long as `self`.
        let sites = unsafe {
            let p = sys::zx_out_sites(&*self.raw, &mut n);
            std::slice::from_raw_parts(p, n as usize)
        };
        sites.iter().map(|s| OutSite { pc: s.pc, count: s.count, regs: s.regs }).collect()
    }

    /// Beeper level in effect at the start of the current edge list; feed it
    /// to [`Beeper::set_level`] before rendering that list.
    pub fn edges_start_level(&self) -> u8 {
        // SAFETY: `raw` was initialised by zx_init.
        unsafe { sys::zx_edges_start_level(&*self.raw) }
    }

    /// Border colour on a frame line (0-311).
    pub fn border_line(&self, line: usize) -> u8 {
        // SAFETY: `raw` was initialised by zx_init.
        unsafe { sys::zx_border_line(&*self.raw, line as u32) }
    }

    /// Whether screen line `y` (0-191) changed since the last frame began.
    pub fn line_dirty(&self, y: usize) -> bool {
        // SAFETY: `raw` was initialised by zx_init; out-of-range lines return 0.
        unsafe { sys::zx_line_dirty(&*self.raw, y as u32) != 0 }
    }

    /// Palette indices (0-15, bright = +8) of screen line `y` (0-191).
    pub fn screen_line(&self, y: usize) -> [u8; 256] {
        let mut out = [0u8; 256];
        // SAFETY: `out` is 256 bytes.
        unsafe { sys::zx_screen_line(&*self.raw, y as u32, out.as_mut_ptr()) };
        out
    }

    /// LCD row `row` (0-239) as palette indices, cropped at `crop_x`.
    pub fn compose_row(&self, row: usize, crop_x: usize) -> [u8; 240] {
        let mut out = [0u8; 240];
        // SAFETY: `raw` was initialised by zx_init; `out` is LCD_W bytes.
        unsafe { sys::pix_compose_zx(&*self.raw, row as u32, crop_x as u32, out.as_mut_ptr()) };
        out
    }

    /// Loads a 48K `.sna` snapshot; `Err` carries the C error code and leaves
    /// the machine unchanged.
    pub fn load_sna(&mut self, data: &[u8]) -> Result<(), i32> {
        // SAFETY: the pointer and length describe `data`.
        match unsafe { sys::zx_load_sna(&mut *self.raw, data.as_ptr(), data.len() as u32) } {
            0 => {
                // the C side ejected the tape; free the buffer it was reading, and eject the
                // signal (a loaded game starts with no tape)
                self.tape = None;
                self.tape_signal_eject();
                Ok(())
            }
            e => Err(e),
        }
    }

    /// Loads a 48K `.z80` snapshot (v1, v2 or v3); errors as [`Machine::load_sna`].
    pub fn load_z80(&mut self, data: &[u8]) -> Result<(), i32> {
        // SAFETY: the pointer and length describe `data`.
        match unsafe { sys::zx_load_z80(&mut *self.raw, data.as_ptr(), data.len() as u32) } {
            0 => {
                // the C side ejected the tape; free the buffer it was reading, and eject the
                // signal (a loaded game starts with no tape)
                self.tape = None;
                self.tape_signal_eject();
                Ok(())
            }
            e => Err(e),
        }
    }

    /// Inserts a `.tap` (the machine keeps its own copy). `Err` carries the C
    /// error code (`ZX_ERR_FORMAT` for a block that overruns the data) and
    /// leaves the previous tape, if any, in place.
    pub fn tape_insert(&mut self, tap: &[u8]) -> Result<(), i32> {
        let buf: Box<[u8]> = tap.into();
        // SAFETY: the pointer and length describe `buf`, which `self.tape`
        // keeps alive (heap storage does not move) for as long as the C side
        // may read it; the C side only stores them on success.
        match unsafe { sys::zx_tape_insert(&mut *self.raw, buf.as_ptr(), buf.len() as u32) } {
            0 => {
                self.tape = Some(buf);
                Ok(())
            }
            e => Err(e),
        }
    }

    /// True while the machine owns a tape buffer.
    pub fn has_tape_buffer(&self) -> bool {
        self.tape.is_some()
    }

    /// Removes the tape and drops the buffer the machine was reading.
    pub fn tape_eject(&mut self) {
        // SAFETY: `raw` was initialised by zx_init; the C side forgets the pointer first.
        unsafe { sys::zx_tape_eject(&mut *self.raw) }
        self.tape = None;
    }

    /// Offset in the tape of the next block to read (the tape's length at its end).
    pub fn tape_pos(&self) -> u32 {
        self.raw.tape_pos
    }

    /// Inserts a tape signal: a `.tzx` or `.tap` played as a cassette plays it, into bit 6 of
    /// port FEh reads (the machine keeps its own copy). It starts now, and [`Machine::run_frame`]
    /// advances it. `Err` says what is wrong with the file (naming the TZX block) and leaves
    /// the previous signal, if any, in place.
    pub fn tape_signal_insert(&mut self, data: &[u8]) -> Result<(), String> {
        let buf: Box<[u8]> = data.into();
        // SAFETY: zx_tapesig is plain data (integers and a pointer); all-zero is a valid value.
        let mut st: Box<sys::zx_tapesig> = Box::new(unsafe { MaybeUninit::zeroed().assume_init() });
        let mut bad = 0u8;
        // SAFETY: the pointer and length describe `buf`; `st` and `buf` are heap storage that
        // `self.sig` keeps alive (it does not move) for as long as the C side may use them, and
        // the C side only stores the pointers on success.
        match unsafe { sys::zx_tapesig_insert(&mut *self.raw, &mut *st, buf.as_ptr(), buf.len() as u32, &mut bad) } {
            0 => {
                self.sig = Some((st, buf));
                self.probe = None; // the signal took the place of the probe
                Ok(())
            }
            e => Err(tape_signal_error(data, e, bad)),
        }
    }

    /// Removes the tape signal, and the port FEh probe if that is on (port FEh bit 6 is the
    /// last `OUT`'s EAR bit again).
    pub fn tape_signal_eject(&mut self) {
        // SAFETY: `raw` was initialised by zx_init; the C side forgets the pointers first.
        unsafe { sys::zx_tapesig_eject(&mut *self.raw) }
        self.sig = None;
        self.probe = None;
    }

    /// True once the signal is past its last pulse and pause (false with none inserted).
    pub fn tape_signal_done(&self) -> bool {
        // the C side reads m->tape_sig, which is the probe's counter while that is on
        if self.sig.is_none() {
            return false;
        }
        // SAFETY: `raw` was initialised by zx_init and its tape_sig is the signal's state.
        unsafe { sys::zx_tapesig_done(&*self.raw) != 0 }
    }

    /// True while the signal is held by a stop (a TZX pause 0).
    pub fn tape_signal_stopped(&self) -> bool {
        if self.sig.is_none() {
            return false;
        }
        // SAFETY: as in `tape_signal_done`.
        unsafe { sys::zx_tapesig_stopped(&*self.raw) != 0 }
    }

    /// Starts the tape again after a stop.
    pub fn tape_signal_play(&mut self) {
        if self.sig.is_none() {
            return;
        }
        // SAFETY: as in `tape_signal_done`.
        unsafe { sys::zx_tapesig_play(&mut *self.raw) }
    }

    /// True while the signal is in the middle of a pulse (it gives edges), false in a pause, a
    /// stop, at its end or with none inserted.
    pub fn tape_signal_in_pulse(&self) -> bool {
        self.sig.as_ref().is_some_and(|(s, _)| s.in_pulse != 0 && s.done == 0 && s.stopped == 0)
    }

    /// Turns the port FEh read counter on or off. While on, every read of an even port (the
    /// keyboard and EAR) is counted ([`Machine::take_port_fe_reads`]) and bit 6 reads as with no
    /// tape (the last `OUT`'s EAR bit). It uses the tape signal's place, so it stays off while a
    /// signal is inserted (`false`), and inserting or ejecting a signal turns it off.
    pub fn port_fe_probe(&mut self, on: bool) -> bool {
        if self.sig.is_some() {
            return false;
        }
        if on {
            if self.probe.is_none() {
                let counter = Box::new(UnsafeCell::new(0u32));
                self.raw.tape_sig = counter.get().cast();
                self.raw.tape_ear = Some(port_fe_counted);
                self.probe = Some(counter);
            }
        } else if self.probe.take().is_some() {
            self.raw.tape_ear = None;
            self.raw.tape_sig = std::ptr::null_mut();
        }
        true
    }

    /// The port FEh reads counted since the last call (0 with the probe off).
    pub fn take_port_fe_reads(&mut self) -> u32 {
        // SAFETY: the counter is only written by `port_fe_counted` while the core runs, which
        // it does not do during this call.
        self.probe.as_ref().map_or(0, |c| unsafe { std::mem::replace(&mut *c.get(), 0) })
    }

    /// True while the CPU is halted (in a `HALT`, waiting for an interrupt).
    pub fn halted(&self) -> bool {
        self.raw.cpu.halt_line != 0
    }

    /// The R register as `LD A,R` reads it: the refresh counter's 7 bits and the bit 7 last
    /// written. ([`Machine::regs`] gives the core's raw counter, whose bit 7 is not R's.)
    pub fn r_register(&self) -> u8 {
        (self.raw.cpu.r & 0x7F) | (self.raw.cpu.r7 & 0x80)
    }

    /// The border colour (0-7) in effect now.
    pub fn border(&self) -> u8 {
        self.raw.border_col
    }

    /// Resets the machine and types `LOAD ""` + ENTER over the next frames. An inserted tape
    /// and tape signal stay inserted.
    pub fn tape_autostart(&mut self) {
        // the reset clears the signal's two fields; the signal plays on through it
        let (ear, sig) = (self.raw.tape_ear, self.raw.tape_sig);
        // SAFETY: `raw` was initialised by zx_init.
        unsafe { sys::zx_tape_autostart(&mut *self.raw) }
        self.raw.tape_ear = ear;
        self.raw.tape_sig = sig;
    }

    /// Host tests only: the current beeper level.
    pub fn test_level(&self) -> u8 {
        // SAFETY: `raw` was initialised by zx_init.
        unsafe { sys::zx_test_level(&*self.raw) }
    }

    /// Host tests only: sets the frame T-state the next instruction starts at.
    pub fn test_set_tstate(&mut self, t: u32) {
        // SAFETY: `raw` was initialised by zx_init.
        unsafe { sys::zx_test_set_tstate(&mut *self.raw, t) }
    }

    /// Host tests only: runs one instruction and returns its T-states,
    /// contention included.
    pub fn test_step(&mut self) -> u32 {
        // SAFETY: `raw` was initialised by zx_init.
        unsafe { sys::zx_test_step(&mut *self.raw) }
    }
}

/// The port FEh probe's EAR input: counts the read, and gives the level a read with no tape
/// gives.
unsafe extern "C" fn port_fe_counted(m: *mut sys::zx_machine, _frame_t: u32) -> u8 {
    // SAFETY: the core calls this only while `port_fe_probe` has installed it, with the machine
    // whose tape_sig points at the probe's live counter.
    unsafe {
        let n = (*m).tape_sig.cast::<u32>();
        *n = (*n).wrapping_add(1);
        (*m).ear_in
    }
}

/// Why the tape signal player refused `data` (C error `code`, TZX block `bad_id`).
fn tape_signal_error(data: &[u8], code: i32, bad_id: u8) -> String {
    if data.is_empty() {
        return "empty tape file".into();
    }
    match (code, data.starts_with(b"ZXTape!\x1a"), bad_id) {
        (sys::ZX_ERR_UNSUPPORTED, _, id) => format!("TZX block 0x{id:02X} is not loadable"),
        (_, true, 0) => "TZX header cut short".into(),
        (_, true, id) => format!("TZX block 0x{id:02X} has a zero or out-of-range length or count or runs past the end of the file"),
        _ => "bad .tap: a block runs past the end of the file".into(),
    }
}

/// Host tests only: the pulses the tape signal player plays for `data`, at most `max` entries
/// (pulse lengths in T-states, a pause as one entry, a stop as 0); fewer at the end of the tape.
pub fn test_tape_pulses(data: &[u8], max: usize) -> Result<Vec<u32>, String> {
    let mut m = Machine::new();
    m.tape_signal_insert(data)?;
    let (st, _) = m.sig.as_mut().expect("inserted");
    let mut out = vec![0u32; max];
    // SAFETY: `out` holds `max` entries; the player's state and data are alive in `m`.
    let n = unsafe { sys::zx_tapesig_test_pulses(&mut **st, out.as_mut_ptr(), max as u32) };
    out.truncate(n as usize);
    Ok(out)
}

/// Delay, in T-states, of a contended access that starts at frame T-state `tstate`.
pub fn contention(tstate: u32) -> u32 {
    // SAFETY: pure function of its argument.
    unsafe { sys::zx_contention(tstate) }
}

impl Default for Machine {
    fn default() -> Self {
        Self::new()
    }
}

/// The beeper renderer: turns each frame's edge list into audio samples.
pub struct Beeper {
    raw: Box<sys::zx_beeper>,
}

impl Beeper {
    /// A renderer for a sample rate of `num / den` Hz.
    pub fn new(num: u32, den: u32) -> Self {
        // SAFETY: zx_beeper is plain integers; all-zero is valid and zx_beeper_init sets it up.
        let mut raw: Box<sys::zx_beeper> = Box::new(unsafe { MaybeUninit::zeroed().assume_init() });
        // SAFETY: `raw` is a valid, exclusively owned zx_beeper.
        unsafe { sys::zx_beeper_init(&mut *raw, num, den) };
        Beeper { raw }
    }

    /// Samples the next `render` call produces.
    pub fn frame_samples(&self) -> u32 {
        // SAFETY: `raw` was initialised by zx_beeper_init.
        unsafe { sys::zx_beeper_frame_samples(&*self.raw) }
    }

    /// Renders one frame whose beeper level changes are `edges`.
    pub fn render(&mut self, edges: &[zx_edge]) -> Vec<i16> {
        let mut out = vec![0i16; self.frame_samples() as usize];
        // SAFETY: `out` holds frame_samples() entries, which render writes at most.
        let n = unsafe { sys::zx_beeper_render(&mut *self.raw, edges.as_ptr(), edges.len() as u32, out.as_mut_ptr()) };
        out.truncate(n as usize);
        out
    }

    /// Sets the level the next render starts from.
    pub fn set_level(&mut self, level: u8) {
        // SAFETY: `raw` was initialised by zx_beeper_init.
        unsafe { sys::zx_beeper_set_level(&mut *self.raw, level) }
    }

    /// Host tests only: the renderer's current level.
    pub fn test_level(&self) -> u8 {
        self.raw.level
    }

    /// Host tests only: switches the DC blocker on or off.
    pub fn test_dc_block(&mut self, on: bool) {
        // SAFETY: `raw` was initialised by zx_beeper_init.
        unsafe { sys::zx_beeper_test_dc_block(&mut *self.raw, on as i32) }
    }
}

// ---- FM music from the beeper (core/sound) -------------------------------

/// The FM choice's fixed delay in frames (60 ms), also the decoder's look-ahead.
pub const SND_DELAY_FRAMES: u32 = sys::SND_DELAY_FRAMES;
/// Tonal voices in a [`SndFrame`] (plus one drum voice).
pub const SND_TONAL: usize = sys::SND_TONAL as usize;
/// Most samples a mixer frame may have.
pub const SND_MAX_FRAME_SAMPLES: u32 = sys::SND_MAX_FRAME_SAMPLES;
/// The decoder's work cap per frame (`snd_decoder.work` units).
pub const SND_WORK_MAX: u32 = sys::SND_WORK_MAX;
/// The mixer's hysteresis: FM after this many confident decisions in a row...
pub const SND_ENTER_FRAMES: u32 = sys::SND_ENTER_FRAMES;
/// ...and the beeper after this many unconfident ones.
pub const SND_LEAVE_FRAMES: u32 = sys::SND_LEAVE_FRAMES;
/// Crossfades last 1/`SND_XFADE_DIV` s (4 ms).
pub const SND_XFADE_DIV: u32 = sys::SND_XFADE_DIV;
pub const SND_MODE_BEEPER: u8 = sys::snd_mode_SND_MODE_BEEPER as u8;
pub const SND_MODE_FM_A: u8 = sys::snd_mode_SND_MODE_FM_A as u8;
pub const SND_MODE_FM_B: u8 = sys::snd_mode_SND_MODE_FM_B as u8;
pub const SND_MODE_COUNT: u8 = sys::snd_mode_SND_MODE_COUNT as u8;
pub const SND_STYLE_NONE: u8 = sys::snd_style_SND_STYLE_NONE as u8;
pub const SND_STYLE_SLOTS: u8 = sys::snd_style_SND_STYLE_SLOTS as u8;
pub const SND_STYLE_FLIPS: u8 = sys::snd_style_SND_STYLE_FLIPS as u8;
pub const SND_STYLE_PULSES: u8 = sys::snd_style_SND_STYLE_PULSES as u8;
pub const SND_STYLE_ENGINE: u8 = sys::snd_style_SND_STYLE_ENGINE as u8;
/// Most note events in a [`SndFrame`].
pub const SND_EVENTS: usize = sys::SND_EVENTS as usize;

/// One voice of a decoder decision (`snd_voice`).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SndVoice {
    /// Full period in T-states x 256; 0 = voice off. Frequency = 3,500,000 x 256 / period_q8.
    pub period_q8: u32,
    /// Loudness 0-255.
    pub level: u8,
}

/// A voice change inside a frame (`snd_event`).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SndEvent {
    /// Frame T-state, 0 .. ZX_FRAME_TSTATES - 1, ascending within a frame.
    pub t: u32,
    /// Tonal: the voice's new period (T-states x 256; 0 = off). Drum: ignored.
    pub period_q8: u32,
    /// 0 .. SND_TONAL - 1 a tonal voice, SND_TONAL the drum.
    pub voice: u8,
    /// Tonal: loudness. Drum: strength (0 = drum off).
    pub level: u8,
}

/// What a method concluded about one past frame (`snd_frame`).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SndFrame {
    /// The voices at the frame's start.
    pub tone: [SndVoice; SND_TONAL],
    /// 0 = none, else drum strength 1-255.
    pub drum: u8,
    /// One of the `SND_STYLE_*` values.
    pub style: u8,
    /// 1 = play FM for this frame.
    pub confident: u8,
    /// 0 = none, else 1 + the engine's index in method B's table (see [`SndEngines::name`]).
    pub engine: u8,
    /// Events in `ev`, applied in order after the frame's start (method A: 0).
    pub n_ev: u8,
    pub ev: [SndEvent; SND_EVENTS],
}

impl SndFrame {
    /// The frame's note events.
    pub fn events(&self) -> &[SndEvent] {
        &self.ev[..(self.n_ev as usize).min(SND_EVENTS)]
    }
}

const _: () = {
    assert!(std::mem::size_of::<SndVoice>() == std::mem::size_of::<sys::snd_voice>());
    assert!(std::mem::align_of::<SndVoice>() == std::mem::align_of::<sys::snd_voice>());
    assert!(std::mem::size_of::<SndFrame>() == std::mem::size_of::<sys::snd_frame>());
    assert!(std::mem::align_of::<SndFrame>() == std::mem::align_of::<sys::snd_frame>());
    assert!(std::mem::offset_of!(SndFrame, drum) == std::mem::offset_of!(sys::snd_frame, drum));
    assert!(std::mem::offset_of!(SndFrame, style) == std::mem::offset_of!(sys::snd_frame, style));
    assert!(std::mem::offset_of!(SndFrame, confident) == std::mem::offset_of!(sys::snd_frame, confident));
    assert!(std::mem::offset_of!(SndFrame, engine) == std::mem::offset_of!(sys::snd_frame, engine));
    assert!(std::mem::offset_of!(SndFrame, n_ev) == std::mem::offset_of!(sys::snd_frame, n_ev));
    assert!(std::mem::offset_of!(SndFrame, ev) == std::mem::offset_of!(sys::snd_frame, ev));
    assert!(std::mem::size_of::<SndEvent>() == std::mem::size_of::<sys::snd_event>());
    assert!(std::mem::align_of::<SndEvent>() == std::mem::align_of::<sys::snd_event>());
    assert!(std::mem::offset_of!(SndEvent, t) == std::mem::offset_of!(sys::snd_event, t));
    assert!(std::mem::offset_of!(SndEvent, period_q8) == std::mem::offset_of!(sys::snd_event, period_q8));
    assert!(std::mem::offset_of!(SndEvent, voice) == std::mem::offset_of!(sys::snd_event, voice));
    assert!(std::mem::offset_of!(SndEvent, level) == std::mem::offset_of!(sys::snd_event, level));
};

/// Allocates a zeroed C struct on the heap.
fn boxed_zeroed<T>() -> Box<T> {
    // SAFETY: only used for the core/sound structs, which are plain integers and arrays: all-zero
    // is a valid value, and their init functions set every field.
    Box::new(unsafe { MaybeUninit::zeroed().assume_init() })
}

/// The beeper music decoder: feed it each frame's edges, get its decision about the frame
/// `SND_DELAY_FRAMES` earlier.
pub struct SndDecoder {
    raw: Box<sys::snd_decoder>,
}

impl SndDecoder {
    pub fn new() -> Self {
        let mut raw: Box<sys::snd_decoder> = boxed_zeroed();
        // SAFETY: `raw` is a valid, exclusively owned snd_decoder.
        unsafe { sys::snd_decoder_init(&mut *raw) };
        SndDecoder { raw }
    }

    /// Feeds frame t's edges (and the level they start from); returns the decision for frame
    /// t - SND_DELAY_FRAMES.
    pub fn frame(&mut self, edges: &[zx_edge], start_level: u8) -> SndFrame {
        let mut out = SndFrame::default();
        // SAFETY: `raw` was initialised by snd_decoder_init; the edge pointer and count describe
        // `edges`; SndFrame has snd_frame's layout (checked above).
        unsafe {
            sys::snd_decoder_frame(
                &mut *self.raw,
                edges.as_ptr(),
                edges.len() as u32,
                start_level,
                (&mut out as *mut SndFrame).cast(),
            )
        };
        out
    }
}

impl SndDecoder {
    /// A decoder whose frame counter starts at `frame`, for tests: its absolute times (frame x
    /// 69,888 + T-state) wrap at 2^32, 256 T into frame 61,455, as on a machine running 20 min.
    pub fn starting_at(frame: u32) -> Self {
        let mut d = Self::new();
        d.raw.frames = frame;
        d
    }

    /// For tests of the time-slot decoder's channel carry-over (`snd_slots_rechannel`): a decoder
    /// with channels of weights `old` (1/256), channel c holding `rises` rises at times
    /// c x 1,000,000 + i, re-learns the weights `new`; returns each new channel's rise count and
    /// newest rise time (0 for none).
    pub fn slots_rechannel(old: &[u8], rises: u32, new: &[u32]) -> Vec<(u32, u32)> {
        assert!(old.len() <= sys::SND_SLOTS as usize && new.len() <= sys::SND_SLOTS as usize);
        let mut d = Self::new();
        let sl = &mut d.raw.sl;
        sl.k = old.len() as u8;
        for (c, &w) in old.iter().enumerate() {
            sl.w[c] = w;
            for i in 0..rises {
                sl.rise[c][(i % sys::SND_RISES) as usize] = c as u32 * 1_000_000 + i;
            }
            sl.nrise[c] = rises;
        }
        // SAFETY: `raw` is an initialised snd_decoder; `new` holds new.len() weights (at most
        // SND_SLOTS).
        unsafe { sys::snd_slots_rechannel(&mut *d.raw, new.as_ptr(), new.len() as u32) };
        let sl = &d.raw.sl;
        (0..new.len())
            .map(|c| {
                let n = sl.nrise[c];
                (n, if n > 0 { sl.rise[c][((n - 1) % sys::SND_RISES) as usize] } else { 0 })
            })
            .collect()
    }

    /// The work units (`snd_decoder.work`) the last [`SndDecoder::frame`] call spent.
    pub fn work(&self) -> u32 {
        self.raw.work
    }
}

impl Default for SndDecoder {
    fn default() -> Self {
        Self::new()
    }
}

/// The FM synth: three 2-operator tonal voices and a noise drum.
pub struct SndFm {
    raw: Box<sys::snd_fm>,
}

impl SndFm {
    /// A synth for a sample rate of `num / den` Hz.
    pub fn new(num: u32, den: u32) -> Self {
        let mut raw: Box<sys::snd_fm> = boxed_zeroed();
        // SAFETY: `raw` is a valid, exclusively owned snd_fm.
        unsafe { sys::snd_fm_init(&mut *raw, num, den) };
        SndFm { raw }
    }

    /// Renders `n` samples of the frame `fr`.
    pub fn render(&mut self, fr: &SndFrame, n: usize) -> Vec<i16> {
        let mut out = vec![0i16; n];
        // SAFETY: `raw` was initialised by snd_fm_init; SndFrame has snd_frame's layout (checked
        // above); `out` holds `n` samples.
        unsafe { sys::snd_fm_render(&mut *self.raw, (fr as *const SndFrame).cast(), out.as_mut_ptr(), n as u32) };
        out
    }
}

/// An engine table of method B: (entries, count).
/// Method B's engine tables.
#[derive(Clone, Copy)]
enum Table {
    /// The built-in engines (the firmware's).
    Builtin,
    /// The built-in engines plus the test fixtures'.
    Test,
    /// The ROM BEEP entry with its period read a semitone off.
    BeepMisread,
}

fn engine_table(which: Table) -> (*const sys::snd_engine, u32) {
    // SAFETY: the tables and counts are immutable C globals.
    unsafe {
        match which {
            Table::Builtin => (std::ptr::addr_of!(sys::snd_engine_table).cast(), sys::snd_engine_count),
            Table::Test => (std::ptr::addr_of!(sys::snd_engine_test_table).cast(), sys::snd_engine_test_count),
            Table::BeepMisread => (std::ptr::addr_of!(sys::snd_engine_misread_table).cast(), sys::snd_engine_misread_count),
        }
    }
}

/// Method B, the known music engines: feed it each frame's machine and edges, get its decision
/// about the frame `SND_DELAY_FRAMES` earlier.
pub struct SndEngines {
    raw: Box<sys::snd_b>,
}

impl SndEngines {
    /// With the built-in engine table (the firmware's).
    pub fn new() -> Self {
        Self::with_table(Table::Builtin)
    }

    /// With the test table: the built-in engines plus the test fixtures'.
    pub fn new_test() -> Self {
        Self::with_table(Table::Test)
    }

    /// With one entry, ROM BEEP's with its reader's period a semitone too long (host tests: a
    /// misread the check must reject).
    pub fn new_beep_misread() -> Self {
        Self::with_table(Table::BeepMisread)
    }

    fn with_table(which: Table) -> Self {
        let (table, n) = engine_table(which);
        let mut raw: Box<sys::snd_b> = boxed_zeroed();
        // SAFETY: `raw` is a valid, exclusively owned snd_b; the table is a 'static C array of n
        // entries.
        unsafe { sys::snd_b_init(&mut *raw, table, n) };
        SndEngines { raw }
    }

    /// Feeds frame t (`m` right after its `run_frame`, its edges and the level they start
    /// from); returns the decision for frame t - SND_DELAY_FRAMES.
    pub fn frame(&mut self, m: &Machine, edges: &[zx_edge], start_level: u8) -> SndFrame {
        let mut out = SndFrame::default();
        // SAFETY: `raw` was initialised by snd_b_init; `m.raw` is an initialised machine; the
        // edge pointer and count describe `edges`; SndFrame has snd_frame's layout (checked
        // above).
        unsafe {
            sys::snd_b_frame(
                &mut *self.raw,
                &*m.raw,
                edges.as_ptr(),
                edges.len() as u32,
                start_level,
                (&mut out as *mut SndFrame).cast(),
            )
        };
        out
    }

    /// The name of a frame's `engine` (1 + index in this table); "" for 0 or an unknown one.
    pub fn name(&self, engine: u8) -> String {
        let i = u32::from(engine);
        if i == 0 || i > self.raw.n {
            return String::new();
        }
        // SAFETY: the table holds `n` entries, i - 1 < n; each name is a static C string.
        unsafe { std::ffi::CStr::from_ptr(sys::snd_engine_name(self.raw.table, i - 1)) }.to_string_lossy().into_owned()
    }

    /// The signature digest of this table's entry `index` with its anchor at `anchor` in `m`'s
    /// memory (`snd_engine_digest`): what the entry's `digest` must be to match there.
    pub fn digest(&self, m: &Machine, index: usize, anchor: u16) -> u64 {
        assert!(index < self.raw.n as usize, "entry {index}");
        // SAFETY: the table holds `n` entries and index < n; `m.raw` is an initialised machine.
        unsafe { sys::snd_engine_digest(&*m.raw, self.raw.table.add(index), anchor) }
    }

    /// The shape of this table's entries (host tests: the table's own consistency).
    pub fn entries(&self) -> Vec<SndEngineEntry> {
        (0..self.raw.n as usize)
            .map(|i| {
                // SAFETY: the table holds `n` entries; each `sites` array holds `n_sites` entries
                // and, for an entry with a pass, `pos` one per channel (engines_builtin.h checks
                // the built-in ones at compile time).
                unsafe {
                    let en = &*self.raw.table.add(i);
                    let sites = (0..en.n_sites as usize).map(|s| *en.sites.add(s)).map(|s| SndEngineSite { off: s.off, role: s.role, chan: s.chan }).collect();
                    let pos = if en.pass != 0 { (0..en.channels as usize).map(|c| *en.pos.add(c)).collect() } else { Vec::new() };
                    SndEngineEntry { name: self.name(i as u8 + 1), len: en.len, channels: en.channels, pass: en.pass, flips: en.flips != 0, sites, pos }
                }
            })
            .collect()
    }

    /// The voices entry `index`'s reader gives with its anchor at `anchor` in `m`, `site` the tonal
    /// site whose registers `regs` are (host tests: a reader's formula).
    pub fn read(&self, m: &Machine, index: usize, anchor: u16, site: u32, regs: &zx_regs) -> [SndVoice; SND_TONAL] {
        assert!(index < self.raw.n as usize, "entry {index}");
        // SAFETY: as in `digest`; snd_b_read is plain data, zero a valid start.
        unsafe {
            let en = &*self.raw.table.add(index);
            assert!((site as usize) < en.n_sites as usize, "site {site}");
            let mut out: sys::snd_b_read = MaybeUninit::zeroed().assume_init();
            (en.read.expect("a reader"))(&*m.raw, anchor, site, regs, &mut out);
            out.v.map(|v| SndVoice { period_q8: v.period_q8, level: v.level })
        }
    }
}

/// One site of a [`SndEngineEntry`] (`snd_engine_site`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SndEngineSite {
    pub off: u16,
    /// [`SND_SITE_TONAL`], [`SND_SITE_DRUM`] or [`SND_SITE_STOP`].
    pub role: u8,
    /// The tonal channel, or [`SND_CHAN_ANY`].
    pub chan: u8,
}

/// The shape of a method B table entry (`snd_engine`).
#[derive(Clone, Debug)]
pub struct SndEngineEntry {
    pub name: String,
    /// Signature bytes from the anchor.
    pub len: u16,
    pub channels: u8,
    /// T-states of a pass (0: none).
    pub pass: u16,
    pub flips: bool,
    pub sites: Vec<SndEngineSite>,
    /// Per channel, its OUT's T-state in a pass (only for an entry with a pass).
    pub pos: Vec<u16>,
}

pub const SND_SITE_TONAL: u8 = sys::snd_site_role_SND_SITE_TONAL as u8;
pub const SND_SITE_DRUM: u8 = sys::snd_site_role_SND_SITE_DRUM as u8;
pub const SND_SITE_STOP: u8 = sys::snd_site_role_SND_SITE_STOP as u8;
pub const SND_CHAN_ANY: u8 = sys::SND_CHAN_ANY as u8;

impl Default for SndEngines {
    fn default() -> Self {
        Self::new()
    }
}

/// The mixer: Beeper mode copies the beeper samples; an FM mode plays all sound 3 frames late,
/// FM where its method is confident, the beeper elsewhere. Owns both methods and a synth.
pub struct SndMixer {
    raw: Box<sys::snd_mixer>,
}

impl SndMixer {
    /// A mixer for a sample rate of `num / den` Hz.
    pub fn new(num: u32, den: u32) -> Self {
        let mut raw: Box<sys::snd_mixer> = boxed_zeroed();
        // SAFETY: `raw` is a valid, exclusively owned snd_mixer.
        unsafe { sys::snd_mixer_init(&mut *raw, num, den) };
        SndMixer { raw }
    }

    /// Method B with the test engine table instead of the built-in one.
    pub fn set_test_engines(&mut self) {
        let (table, n) = engine_table(Table::Test);
        // SAFETY: `raw` was initialised by snd_mixer_init; the table is a 'static C array of n
        // entries.
        unsafe { sys::snd_mixer_set_engines(&mut *self.raw, table, n) };
    }

    /// One frame: the machine right after its `run_frame` (None: nothing is method B's), its
    /// edges, the level they start from, and its beeper samples (from [`Beeper::render`]);
    /// returns as many output samples in `mode` (`SND_MODE_*`).
    pub fn frame(&mut self, mode: u8, m: Option<&Machine>, edges: &[zx_edge], start_level: u8, beeper: &[i16]) -> Vec<i16> {
        let mut out = vec![0i16; beeper.len()];
        // SAFETY: `raw` was initialised by snd_mixer_init; the machine, if given, is initialised;
        // each pointer and count describes its slice, and `out` is as long as `beeper`.
        unsafe {
            sys::snd_mixer_frame(
                &mut *self.raw,
                mode,
                m.map_or(std::ptr::null(), |m| &*m.raw as *const sys::zx_machine),
                edges.as_ptr(),
                edges.len() as u32,
                start_level,
                beeper.as_ptr(),
                beeper.len() as u32,
                out.as_mut_ptr(),
            )
        };
        out
    }

    /// Whether the mixer's hysteresis plays FM for the frame the last [`SndMixer::frame`] decided
    /// (the one `SND_DELAY_FRAMES` before it), in an FM mode.
    pub fn fm_on(&self) -> bool {
        self.raw.fm_on != 0
    }
}

/// Pixel formats of the LCD.
#[derive(Clone, Copy)]
pub enum PixFormat {
    Rgb444,
    Rgb565,
}

impl PixFormat {
    fn raw(self) -> i32 {
        match self {
            PixFormat::Rgb444 => sys::PIX_RGB444 as i32,
            PixFormat::Rgb565 => sys::PIX_RGB565 as i32,
        }
    }

    /// Bytes of one 240-pixel row.
    pub fn row_bytes(self) -> usize {
        // SAFETY: pure function.
        unsafe { sys::pix_row_bytes(self.raw()) as usize }
    }

    /// Packs 240 palette indices into panel bytes.
    pub fn pack(self, idx: &[u8; 240]) -> Vec<u8> {
        let mut out = vec![0u8; self.row_bytes()];
        // SAFETY: `idx` is LCD_W bytes and `out` holds one packed row.
        unsafe { sys::pix_pack(self.raw(), idx.as_ptr(), out.as_mut_ptr()) };
        out
    }
}

/// `150000 / (divider + 1)` bytes per frame.
pub fn lcd_budget_bytes(divider: u32) -> u32 {
    // SAFETY: pure function.
    unsafe { sys::lcd_budget_bytes(divider) }
}

/// The LCD row scheduler.
pub struct Sched {
    raw: Box<sys::lcd_sched>,
}

impl Sched {
    pub fn new(fmt: PixFormat) -> Self {
        // SAFETY: lcd_sched is plain integers and arrays; all-zero is valid and
        // lcd_sched_init sets it up.
        let mut raw: Box<sys::lcd_sched> = Box::new(unsafe { MaybeUninit::zeroed().assume_init() });
        // SAFETY: `raw` is a valid lcd_sched.
        unsafe { sys::lcd_sched_init(&mut *raw, fmt.row_bytes() as u32) };
        Sched { raw }
    }

    pub fn mark(&mut self, row: usize, frame: u32) {
        // SAFETY: `raw` was initialised by lcd_sched_init; out-of-range rows are ignored.
        unsafe { sys::lcd_sched_mark(&mut *self.raw, row as u32, frame) }
    }

    pub fn mark_all(&mut self, frame: u32) {
        // SAFETY: `raw` was initialised by lcd_sched_init.
        unsafe { sys::lcd_sched_mark_all(&mut *self.raw, frame) }
    }

    pub fn begin_frame(&mut self, frame: u32, budget: u32) {
        // SAFETY: `raw` was initialised by lcd_sched_init.
        unsafe { sys::lcd_sched_begin_frame(&mut *self.raw, frame, budget) }
    }

    /// The next band as `(row0, rows)`, or `None` when nothing fits.
    pub fn next_band(&mut self) -> Option<(usize, usize)> {
        let (mut row0, mut rows) = (0u32, 0u32);
        // SAFETY: `raw` was initialised by lcd_sched_init; both outputs are valid.
        let got = unsafe { sys::lcd_sched_next_band(&mut *self.raw, &mut row0, &mut rows) };
        (got != 0).then_some((row0 as usize, rows as usize))
    }

    pub fn pending(&self) -> usize {
        // SAFETY: `raw` was initialised by lcd_sched_init.
        unsafe { sys::lcd_sched_pending(&*self.raw) as usize }
    }

    /// The frame a row has been pending since, if it is pending.
    pub fn pending_since(&self, row: usize) -> Option<u32> {
        (self.raw.pending[row] != 0).then_some(self.raw.pending_since[row])
    }

    pub fn stats(&self) -> lcd_stats {
        self.raw.stats
    }
}

/// Number of FM-1 inputs the key map covers (buttons, notes, encoder directions).
pub const KM_INPUTS: usize = sys::KM_INPUTS as usize;
pub const KM_NONE: u8 = sys::KM_NONE as u8;
pub const KM_PULSE_FRAMES: u32 = sys::KM_PULSE_FRAMES;

/// The built-in key map: two action bytes per input.
pub fn km_default() -> [[u8; 2]; KM_INPUTS] {
    let mut k = MaybeUninit::<sys::keymap>::zeroed();
    // SAFETY: km_default fills the whole table.
    unsafe {
        sys::km_default(k.as_mut_ptr());
        k.assume_init().act
    }
}

/// One poll of the FM-1 controls.
#[derive(Clone, Copy, Default)]
pub struct KmInput {
    pub buttons: u32,
    pub notes: u32,
    pub enc: [i8; 7],
}

/// The key mapper: a table plus the encoder pulse timers.
pub struct KeyMapper {
    map: sys::keymap,
    state: sys::km_state,
}

impl KeyMapper {
    /// A mapper with the default table.
    pub fn new() -> Self {
        // SAFETY: both are plain byte arrays; all-zero is valid, and the C calls set them up.
        let mut m = KeyMapper { map: unsafe { MaybeUninit::zeroed().assume_init() }, state: unsafe { MaybeUninit::zeroed().assume_init() } };
        // SAFETY: both pointers are valid.
        unsafe {
            sys::km_default(&mut m.map);
            sys::km_reset(&mut m.state);
        }
        m
    }

    /// Replaces the table.
    pub fn set_map(&mut self, act: &[[u8; 2]; KM_INPUTS]) {
        self.map.act = *act;
    }

    pub fn reset(&mut self) {
        // SAFETY: the state is valid.
        unsafe { sys::km_reset(&mut self.state) }
    }

    /// Returns `(rows, kempston)` for this poll.
    pub fn apply(&mut self, input: &KmInput) -> ([u8; 8], u8) {
        let inp = sys::km_input { buttons: input.buttons, notes: input.notes, enc: input.enc };
        let mut rows = [0u8; 8];
        let mut kemp = 0u8;
        // SAFETY: all pointers are valid; rows holds 8 bytes.
        unsafe { sys::km_apply(&self.map, &mut self.state, &inp, rows.as_mut_ptr(), &mut kemp) };
        (rows, kemp)
    }
}

impl Default for KeyMapper {
    fn default() -> Self {
        Self::new()
    }
}

/// Largest raw game the pack holds.
pub const PACK_MAX_RAW: u32 = sys::PACK_MAX_RAW;
pub const PACK_HEADER_SIZE: usize = sys::PACK_HEADER_SIZE as usize;
pub const PACK_ENTRY_SIZE: usize = sys::PACK_ENTRY_SIZE as usize;
pub const PACK_NAME_LEN: usize = sys::PACK_NAME_LEN as usize;
pub const PACK_SNA: u8 = sys::PACK_SNA as u8;
pub const PACK_Z80: u8 = sys::PACK_Z80 as u8;
pub const PACK_TAP: u8 = sys::PACK_TAP as u8;
pub const PACK_STORED: u8 = sys::PACK_STORED as u8;
pub const PACK_LZ4: u8 = sys::PACK_LZ4 as u8;

/// CRC-32 (IEEE) as the pack uses it.
pub fn pack_crc32(data: &[u8]) -> u32 {
    // SAFETY: the pointer and length describe `data`.
    unsafe { sys::pack_crc32(data.as_ptr(), data.len() as u32) }
}

/// Decodes one raw LZ4 block into at most `cap` bytes; `Err` carries the C error code.
pub fn lz4_decode_raw_cap(src: &[u8], cap: u32) -> Result<(), i32> {
    // Only for testing the cap limit: the output buffer is a single byte, and the decoder must
    // refuse before touching it.
    let mut out = [0u8; 1];
    let mut len = 0u32;
    // SAFETY: the decoder rejects any cap above its limit without writing; for a valid cap the
    // caller must pass a buffer that large, so this is only used with an oversized cap.
    assert!(cap > 0x7FFF_FFFF, "lz4_decode_raw_cap is for oversized caps only");
    match unsafe { sys::lz4_block_decode(src.as_ptr(), src.len() as u32, out.as_mut_ptr(), cap, &mut len) } {
        0 => Ok(()),
        e => Err(e),
    }
}

/// Decodes one raw LZ4 block into at most `cap` bytes; `Err` carries the C error code.
pub fn lz4_decode(src: &[u8], cap: usize) -> Result<Vec<u8>, i32> {
    let mut out = vec![0u8; cap];
    let mut len = 0u32;
    // SAFETY: both buffers are valid for the lengths given; `out` holds `cap` bytes.
    match unsafe { sys::lz4_block_decode(src.as_ptr(), src.len() as u32, out.as_mut_ptr(), cap as u32, &mut len) } {
        0 => {
            out.truncate(len as usize);
            Ok(out)
        }
        e => Err(e),
    }
}

/// Validates the header of a pack image and returns the game count.
pub fn pack_open(pack: &[u8]) -> Result<u32, i32> {
    let mut count = 0u32;
    // SAFETY: the pointer and length describe `pack`.
    match unsafe { sys::pack_open(pack.as_ptr(), pack.len() as u32, &mut count) } {
        0 => Ok(count),
        e => Err(e),
    }
}

/// One pack entry, borrowing the image it was read from.
pub struct PackGame<'a> {
    raw: sys::pack_game,
    _pack: PhantomData<&'a [u8]>,
}

impl PackGame<'_> {
    pub fn name(&self) -> String {
        let n = self.raw.name.iter().position(|&c| c == 0).unwrap_or(self.raw.name.len());
        self.raw.name[..n].iter().map(|&c| c as u8 as char).collect()
    }
    pub fn kind(&self) -> u8 {
        self.raw.kind
    }
    pub fn codec(&self) -> u8 {
        self.raw.codec
    }
    pub fn crop_x(&self) -> u8 {
        self.raw.crop_x
    }
    pub fn keys(&self) -> [[u8; 2]; KM_INPUTS] {
        self.raw.keys.act
    }
    pub fn stored_len(&self) -> u32 {
        self.raw.stored_len
    }
    pub fn raw_len(&self) -> u32 {
        self.raw.raw_len
    }
    pub fn crc32(&self) -> u32 {
        self.raw.crc32
    }

    /// Decompresses the game into a buffer of `cap` bytes.
    pub fn extract(&self, cap: usize) -> Result<Vec<u8>, i32> {
        let mut out = vec![0u8; cap];
        let mut len = 0u32;
        // SAFETY: `raw.data` points into the image `'a` borrows, for stored_len bytes (checked
        // by pack_game_at); `out` holds `cap` bytes.
        match unsafe { sys::pack_extract(&self.raw, out.as_mut_ptr(), cap as u32, &mut len) } {
            0 => {
                out.truncate(len as usize);
                Ok(out)
            }
            e => Err(e),
        }
    }
}

/// Reads entry `index` of a pack image. Validates the header first (the C call needs that).
pub fn pack_game_at(pack: &[u8], index: u32) -> Result<PackGame<'_>, i32> {
    pack_open(pack)?;
    // SAFETY: pack_game is plain data; all-zero is valid and pack_game_at fills it on success.
    let mut raw: sys::pack_game = unsafe { MaybeUninit::zeroed().assume_init() };
    // SAFETY: the header was validated against `pack.len()` just above.
    match unsafe { sys::pack_game_at(pack.as_ptr(), index, &mut raw) } {
        0 => Ok(PackGame { raw, _pack: PhantomData }),
        e => Err(e),
    }
}

// ---- text UI and menus ---------------------------------------------------

/// Events the device layer feeds the menus.
pub mod ev {
    use super::sys;
    pub const UP: i32 = sys::EV_UP as i32;
    pub const DOWN: i32 = sys::EV_DOWN as i32;
    pub const LEFT: i32 = sys::EV_LEFT as i32;
    pub const RIGHT: i32 = sys::EV_RIGHT as i32;
    pub const OK: i32 = sys::EV_OK as i32;
    pub const BACK: i32 = sys::EV_BACK as i32;
    pub const HOME: i32 = sys::EV_HOME as i32;
    pub const PAUSE: i32 = sys::EV_PAUSE as i32;
    pub const CAPS: i32 = sys::EV_CAPS as i32;
    pub const SYM: i32 = sys::EV_SYM as i32;
}

/// Views of the menu state machine.
pub mod view {
    use super::sys;
    pub const PICKER: i32 = sys::VIEW_PICKER as i32;
    pub const GAME: i32 = sys::VIEW_GAME as i32;
    pub const MENU: i32 = sys::VIEW_MENU as i32;
    pub const KEYS: i32 = sys::VIEW_KEYS as i32;
    pub const OSK: i32 = sys::VIEW_OSK as i32;
    pub const PAUSED: i32 = sys::VIEW_PAUSED as i32;
    pub const MESSAGE: i32 = sys::VIEW_MESSAGE as i32;
}

/// Commands the menu state machine returns.
pub mod cmd {
    use super::sys;
    pub const NONE: i32 = sys::CMD_NONE as i32;
    pub const START: i32 = sys::CMD_START as i32;
    pub const RESUME: i32 = sys::CMD_RESUME as i32;
    pub const RESET: i32 = sys::CMD_RESET as i32;
    pub const LEAVE: i32 = sys::CMD_LEAVE as i32;
    pub const DIVIDER: i32 = sys::CMD_DIVIDER as i32;
    pub const OSK_KEY: i32 = sys::CMD_OSK_KEY as i32;
    pub const SOUND: i32 = sys::CMD_SOUND as i32;
}

pub const UI_COLS: usize = sys::UI_COLS as usize;
pub const UI_ROWS: usize = sys::UI_ROWS as usize;

/// The C side has one text grid, which `Ui` and the menu calls that invalidate it (`menu_event`,
/// `menu_set_view`, `menu_message`) all touch; the lock serialises them (tests run in parallel).
/// A `Ui` holds it for its lifetime; every `Menu` call holds it for the call, unless its thread
/// already holds a `Ui` (UI_HELD), so a test can use a `Ui` and a `Menu` together.
static UI_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

thread_local! {
    /// This thread holds a `Ui` (and so UI_LOCK). `Ui` is not `Send`: it stays on this thread.
    static UI_HELD: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Runs `f` holding UI_LOCK: taken for the call, or already held by this thread's `Ui`.
fn with_ui_lock<R>(f: impl FnOnce() -> R) -> R {
    if UI_HELD.with(|h| h.get()) {
        f()
    } else {
        let _guard = UI_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        f()
    }
}

/// Exclusive access to the text grid. Taking it re-initialises the grid.
pub struct Ui {
    _guard: std::sync::MutexGuard<'static, ()>,
}

impl Ui {
    /// Panics if this thread already holds a `Ui` (waiting for it would never end).
    pub fn new() -> Self {
        assert!(!UI_HELD.with(|h| h.get()), "this thread already holds the Ui");
        let guard = UI_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        UI_HELD.with(|h| h.set(true));
        // SAFETY: the ROM is a full 16 KiB image.
        unsafe { sys::ui_init(ROM_48.as_ptr()) };
        Ui { _guard: guard }
    }

    pub fn clear(&mut self, ink: u8, paper: u8) {
        // SAFETY: the grid is initialised.
        unsafe { sys::ui_clear(ink, paper) }
    }

    pub fn text(&mut self, col: usize, row: usize, s: &str, ink: u8, paper: u8) {
        let c = std::ffi::CString::new(s).expect("no NUL in UI text");
        // SAFETY: `c` is NUL-terminated.
        unsafe { sys::ui_text(col as u32, row as u32, c.as_ptr(), ink, paper) }
    }

    /// The characters of grid row `row`, trailing spaces kept.
    pub fn row_text(&self, row: usize) -> String {
        let mut buf = [0i8; UI_COLS + 1];
        // SAFETY: the buffer holds UI_COLS + 1 bytes.
        unsafe { sys::ui_get_text(row as u32, buf.as_mut_ptr()) };
        buf[..UI_COLS].iter().map(|&c| c as u8 as char).collect()
    }

    /// 240 palette indices of LCD row `row`.
    pub fn compose_row(&self, row: usize) -> [u8; 240] {
        let mut out = [0u8; 240];
        // SAFETY: the buffer is LCD_W bytes.
        unsafe { sys::ui_compose_row(row as u32, out.as_mut_ptr()) };
        out
    }

    /// `(count, bitmap)` of the LCD rows changed since the last call.
    pub fn dirty_rows(&mut self) -> (u32, [u8; 30]) {
        let mut rows = [0u8; 30];
        // SAFETY: the bitmap is LCD_ROWS / 8 bytes.
        let n = unsafe { sys::ui_dirty_rows(rows.as_mut_ptr()) };
        (n, rows)
    }

    /// Draws the menu's current view into the grid.
    pub fn draw(&mut self, m: &Menu, pack: Option<&[u8]>) {
        let p = pack.map_or(std::ptr::null(), |p| p.as_ptr());
        // SAFETY: a non-null pack was validated by pack_open in the caller (the C side needs that).
        unsafe { sys::menu_draw(&*m.raw, p) }
    }
}

impl Default for Ui {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for Ui {
    fn drop(&mut self) {
        // runs before `_guard` is dropped, so UI_HELD never claims a lock this thread lacks
        UI_HELD.with(|h| h.set(false));
    }
}

/// The menu state machine. Its calls hold UI_LOCK (see there): some of them invalidate the grid.
pub struct Menu {
    raw: Box<sys::menu>,
}

impl Menu {
    pub fn new(count: u32) -> Self {
        // SAFETY: menu is plain integers and chars; all-zero is valid and menu_init sets it up.
        let mut raw: Box<sys::menu> = Box::new(unsafe { MaybeUninit::zeroed().assume_init() });
        // SAFETY: `raw` is a valid menu.
        unsafe { sys::menu_init(&mut *raw, count) };
        Menu { raw }
    }

    /// Feeds an event; returns `(command, arg)`.
    pub fn event(&mut self, ev: i32) -> (i32, u32) {
        let mut arg = 0u32;
        // SAFETY: both pointers are valid; the lock covers the grid menu_event may invalidate.
        let c = with_ui_lock(|| unsafe { sys::menu_event(&mut *self.raw, ev, &mut arg) });
        (c, arg)
    }

    pub fn view(&self) -> i32 {
        // SAFETY: `raw` was initialised by menu_init.
        unsafe { sys::menu_view(&*self.raw) }
    }

    pub fn set_view(&mut self, v: i32) {
        // SAFETY: `raw` was initialised by menu_init; the lock covers the grid it invalidates.
        with_ui_lock(|| unsafe { sys::menu_set_view(&mut *self.raw, v) })
    }

    pub fn message(&mut self, text: &str) {
        let c = std::ffi::CString::new(text).expect("no NUL in text");
        // SAFETY: `c` is NUL-terminated; the lock covers the grid menu_message invalidates.
        with_ui_lock(|| unsafe { sys::menu_message(&mut *self.raw, c.as_ptr()) })
    }

    pub fn set_divider(&mut self, d: u32) {
        // SAFETY: `raw` was initialised by menu_init.
        unsafe { sys::menu_set_divider(&mut *self.raw, d) }
    }

    pub fn runs_game(&self) -> bool {
        // SAFETY: `raw` was initialised by menu_init.
        unsafe { sys::menu_runs_game(&*self.raw) != 0 }
    }

    pub fn row_is_ui(&self, lcd_row: usize) -> bool {
        // SAFETY: `raw` was initialised by menu_init.
        unsafe { sys::menu_row_is_ui(&*self.raw, lcd_row as u32) != 0 }
    }
}
