// SPDX-FileCopyrightText: 2026 FM-1 ZX contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Raw bindings to the C machine core, and the safe [`Machine`] wrapper.

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

pub use sys::{lcd_stats, zx_edge, zx_regs, ZX_ERR_FORMAT, ZX_ERR_UNSUPPORTED, ZX_FRAME_TSTATES, ZX_RAM_SIZE, ZX_ROM_SIZE};

static ROM_48: &[u8; ZX_ROM_SIZE as usize] = include_bytes!("../../../../third_party/zx-rom/48.rom");

/// A 48K Spectrum. Not `Sync`: the C core has no locking.
pub struct Machine {
    raw: Box<sys::zx_machine>,
    /// The inserted .tap; the C core keeps a pointer into this heap buffer.
    tape: Option<Box<[u8]>>,
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
        Machine { raw, tape: None, _not_sync: PhantomData }
    }

    pub fn run_frame(&mut self) {
        // SAFETY: `raw` was initialised by zx_init.
        unsafe { sys::zx_run_frame(&mut *self.raw) }
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
                // the C side ejected the tape; free the buffer it was reading
                self.tape = None;
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
                // the C side ejected the tape; free the buffer it was reading
                self.tape = None;
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

    /// Resets the machine and types `LOAD ""` + ENTER over the next frames.
    pub fn tape_autostart(&mut self) {
        // SAFETY: `raw` was initialised by zx_init.
        unsafe { sys::zx_tape_autostart(&mut *self.raw) }
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
