// SPDX-FileCopyrightText: 2026 FM-1 ZX contributors
// SPDX-License-Identifier: GPL-3.0-or-later
//
// The browser page's computer keyboard, the labels over the FM-1 drawing and the controls panel.
//
// The worklet reports the firmware's view and running game each frame (web/patch-emu.sh, from
// fmzx_stats at the address tools/web-demo writes into keymaps.json). While a game runs, the
// computer keys type Spectrum keys: a key presses the FM-1 input the running game maps to that
// Spectrum key (docs/controls.md, "Computer keyboard"). Esc is HOME and Tab is PLAY in every view;
// in the picker, the menus and the on-screen keyboard Enter is PLAY and the arrows turn SELECT and
// ALGORITHM. Without the
// running game (a package without the address, or before the first frame) the keys are the
// fixed table below (LEGACY) and the labels follow the key map's game.

export const VIEW = { PICKER: 0, GAME: 1, MENU: 2, KEYS: 3, OSK: 4, PAUSED: 5, MESSAGE: 6 };
export const HOME = 8;
export const PLAY = 12;

// The views where the game's keys count: the game, paused (PLAY resumes it) and the on-screen
// keyboard (the game runs under it, its other inputs keep working)
export const gameView = (v) => v === VIEW.GAME || v === VIEW.PAUSED || v === VIEW.OSK;

// ---------------------------------------------------------------- FM-1 inputs

const BUTTONS = ["OCT-", "OCT+", "FX", "SCL", "ENV", "LFO", "EDIT", "GLO", "HOME", "SAVE", "ARP", "SEQ", "PLAY", "REC"];
const NOTES = ["F", "F#", "G", "G#", "A", "A#", "B", "C", "C#", "D", "D#", "E"];
// the drawing's encoder order (panel_hit 0..6; fm1-emu's turn() takes it)
const ENCODERS = { SELECT: 0, PRESETS: 1, ALGORITHM: 2, K1: 3, K2: 4, K3: 5, K4: 6 };

export function inputId(name) {
  const b = BUTTONS.indexOf(name);
  if (b >= 0) return b;
  const m = /^([A-G]#?)(\d)$/.exec(name);
  if (!m) return -1;
  // semitones above F3: the scale runs F..E, so C..E belong to the octave number's previous span
  const pc = NOTES.indexOf(m[1]);
  const oct = Number(m[2]) - (pc >= 7 ? 1 : 0);
  return 14 + (oct - 3) * 12 + pc;
}

// A key map input ("note.F3", "button.FX", "enc.K1.cw") as a target: { key: id } or { enc, step }
export function target(input) {
  const [kind, name, dir] = input.split(".");
  if (kind === "enc") {
    const enc = ENCODERS[name];
    return enc === undefined ? null : { enc, step: dir === "cw" ? 1 : -1 };
  }
  const id = inputId(name);
  return id < 0 ? null : { key: id };
}

// ---------------------------------------------------------------- Spectrum keys

// The computer key (event.code, the physical key) for each Spectrum key or Kempston direction
export function tokenOf(code) {
  let m = /^Key([A-Z])$/.exec(code);
  if (m) return `key.${m[1]}`;
  m = /^Digit(\d)$/.exec(code);
  if (m) return `key.${m[1]}`;
  return CODES[code] ?? null;
}
const CODES = {
  Space: "key.SPACE",
  Enter: "key.ENTER",
  NumpadEnter: "key.ENTER",
  ShiftLeft: "key.CAPS",
  AltLeft: "key.SYM",
  AltRight: "key.SYM",
  ArrowLeft: "kempston.left",
  ArrowRight: "kempston.right",
  ArrowUp: "kempston.up",
  ArrowDown: "kempston.down",
  ShiftRight: "kempston.fire",
};
// the arrows' knobs in the picker and the menus (and the on-screen keyboard's cursor)
const ARROW_TURNS = {
  ArrowLeft: { enc: 0, step: -1 },
  ArrowRight: { enc: 0, step: 1 },
  ArrowUp: { enc: 2, step: -1 },
  ArrowDown: { enc: 2, step: 1 },
};

// The running game's table: Spectrum key -> the FM-1 input that gives it (single), and the
// inputs that give two at once (chords, pressed by holding both computer keys). The first input
// in the key map wins; a key or note wins over an encoder.
export function buildTable(game) {
  const single = new Map();
  const chords = [];
  for (const { input, actions } of game?.keys ?? []) {
    const t = target(input);
    if (!t || actions.includes("none")) continue;
    if (actions.length === 1) {
      const old = single.get(actions[0]);
      if (!old || ("enc" in old && "key" in t)) single.set(actions[0], t);
    } else if (actions.length === 2) {
      chords.push({ tokens: actions, target: t });
    }
  }
  return { single, chords };
}

// The FM-1 keys to hold for the Spectrum keys held: chords first (each token used once), then
// the single keys of the rest; encoder targets are turned on the key press instead (pressFor)
export function resolve(held, table) {
  const ids = new Set();
  const used = new Set();
  for (const c of table.chords) {
    if (c.tokens.every((t) => held.has(t) && !used.has(t)) && "key" in c.target) {
      ids.add(c.target.key);
      c.tokens.forEach((t) => used.add(t));
    }
  }
  for (const t of held) {
    if (used.has(t)) continue;
    const s = table.single.get(t);
    if (s && "key" in s) ids.add(s.key);
  }
  return ids;
}

// The target a press of `token` reaches with `held` (it included): a completed chord, else its own
export function pressFor(token, held, table) {
  for (const c of table.chords) if (c.tokens.includes(token) && c.tokens.every((t) => held.has(t))) return c.target;
  return table.single.get(token) ?? null;
}

// ---------------------------------------------------------------- the keyboard

// Without the running game: the fixed table (the joystick arrows, Space as fire, Enter PLAY, Esc
// HOME and a few notes below C4), by event.key; everything else is left to fm1-emu's piano map.
// The demo video's id on YouTube ("" for none: the page then has no video card).
export const VIDEO = "Is4KYSKhlqY";

// Where a video id plays: embedded (the host's no-cookie player, started by the click) and on
// its own page; null for no id or one that is not an id.
export function videoLinks(id) {
  if (!/^[A-Za-z0-9_-]{11}$/.test(id || "")) return null;
  return { embed: `https://www.youtube-nocookie.com/embed/${id}?autoplay=1&rel=0`, watch: `https://youtu.be/${id}` };
}

export const LEGACY = {
  ArrowLeft: [{ key: 22 }, { enc: 0, step: -1 }],   // C#4 (the default map's Kempston left)
  ArrowRight: [{ key: 24 }, { enc: 0, step: 1 }],   // D#4 right
  ArrowUp: [{ key: 15 }, { enc: 2, step: -1 }],     // F#3 up
  ArrowDown: [{ key: 14 }, { enc: 2, step: 1 }],    // F3 down
  " ": [{ key: 33 }],
  Enter: [{ key: PLAY }],
  Tab: [{ key: PLAY }],
  Escape: [{ key: HOME }],
  c: [{ key: 17 }],
  v: [{ key: 19 }],
  b: [{ key: 20 }],
  "[": [{ key: 39 }],
  "]": [{ key: 40 }],
};

export function legacyHandle(e, down, input) {
  const actions = LEGACY[e.key.length === 1 ? e.key.toLowerCase() : e.key];
  if (!actions) return false;
  e.preventDefault?.(); // upstream's handler is skipped for claimed keys: no scrolling, no button activation
  for (const a of actions) {
    if ("key" in a) {
      if (down && e.repeat) continue;
      input.setKey(a.key, down);
    } else if (down) {
      input.turn(a.enc, a.step);
    }
  }
  return true;
}

// fm1-emu's piano keys that are not Spectrum keys, kept quiet while the page knows the view
const QUIET = new Set(["Semicolon", "Quote", "BracketLeft", "BracketRight"]);

// The keyboard state: `input` is fm1-emu's { setKey, turn }. setRunning(view, game) with the
// running game's key map entry (or view null: not known, the legacy table).
export function createKeys(input) {
  const held = new Set();       // Spectrum keys held on the computer
  let ours = new Set();         // FM-1 keys held for them
  let view = null;
  let table = buildTable(null);
  const asPlay = new Set();     // Enter keys held as PLAY
  const sync = () => {
    const want = view !== null && gameView(view) ? resolve(held, table) : new Set();
    for (const id of ours) if (!want.has(id)) input.setKey(id, false);
    for (const id of want) if (!ours.has(id)) input.setKey(id, true);
    ours = want;
  };
  const keys = {
    setRunning(v, game) {
      view = v;
      table = buildTable(game);
      sync();
    },
    get view() { return view; },
    releaseAll() {
      held.clear();
      asPlay.clear();
      sync();
      input.setKey(HOME, false);
      input.setKey(PLAY, false);
    },
    handle(e, down) {
      const code = e.code;
      if (code === "Escape" || code === "Tab") {
        e.preventDefault?.();
        if (!(down && e.repeat)) input.setKey(code === "Escape" ? HOME : PLAY, down);
        return true;
      }
      if (view === null) return legacyHandle(e, down, input);
      // a key let go is let go whatever the view now (one held in the game, let go in the menu)
      if (!down && held.delete(tokenOf(code))) sync();
      const inGame = gameView(view);
      if (ARROW_TURNS[code] && view !== VIEW.GAME && view !== VIEW.PAUSED) {
        // the picker, the menus and the on-screen keyboard: the arrows turn SELECT and ALGORITHM
        e.preventDefault?.();
        if (down) input.turn(ARROW_TURNS[code].enc, ARROW_TURNS[code].step);
        return true;
      }
      const enter = code === "Enter" || code === "NumpadEnter";
      // Enter is PLAY where the arrows move a cursor: the picker, the menus, the on-screen keyboard
      if (enter && (down ? !inGame || view === VIEW.OSK : asPlay.has(code))) {
        // (a key let go acts as it was pressed: Enter that started a game lets PLAY go)
        e.preventDefault?.();
        if (down) asPlay.add(code);
        else asPlay.delete(code);
        if (!(down && e.repeat)) input.setKey(PLAY, down);
        return true;
      }
      const token = tokenOf(code);
      if (!token) {
        if (QUIET.has(code)) { e.preventDefault?.(); return true; }
        return false;
      }
      e.preventDefault?.();
      if (!inGame) return true; // no game reads keys here
      if (down) {
        if (!e.repeat) held.add(token);
        const t = pressFor(token, held, table);
        if (t && "enc" in t) input.turn(t.enc, t.step);
      } else {
        held.delete(token);
      }
      sync();
      return true;
    },
  };
  return keys;
}

// ---------------------------------------------------------------- names

const SHORT = { ENTER: "ENT", SPACE: "SPC", CAPS: "CS", SYM: "SS" };
const ARROWS = { left: "←", right: "→", up: "↑", down: "↓", fire: "FIRE" };
export function shortName(action) {
  const [kind, name] = action.split(".");
  if (kind === "kempston") return ARROWS[name] ?? name;
  if (kind === "key") return SHORT[name] ?? name;
  return "";
}

const COMPUTER = { ENTER: "Enter", SPACE: "Space", CAPS: "Left Shift", SYM: "Alt" };
const COMPUTER_JOY = { left: "←", right: "→", up: "↑", down: "↓", fire: "Right Shift" };
export function computerName(action) {
  const [kind, name] = action.split(".");
  if (kind === "kempston") return COMPUTER_JOY[name] ?? "";
  if (kind === "key") return COMPUTER[name] ?? name;
  return "";
}

const SHIFTS = { CAPS: "CAPS SHIFT", SYM: "SYMBOL SHIFT" };
function actionName(a) {
  const [kind, name] = a.split(".");
  if (kind === "kempston") return `Kempston ${name}`;
  if (kind === "none") return "none";
  return SHIFTS[name] ?? name;
}

const same = (a, b) => a && b && a.key === b.key && a.enc === b.enc && a.step === b.step;

// fm1-emu's piano map (the legacy panel only): C4..F5, OCT-/OCT+, REC, the top buttons, , . ALGORITHM
const UPSTREAM = (() => {
  const m = { z: 0, x: 1, r: 13 };
  [..."awsedftgyhujkolp;'"].forEach((k, i) => { m[k] = 21 + i; });
  [2, 3, 4, 5, 6, 7, 8, 9, 10, 11].forEach((id, i) => { m[String((i + 1) % 10)] = id; });
  return m;
})();
const UPSTREAM_ENC = { ",": { enc: 2, step: -1 }, ".": { enc: 2, step: 1 } };
const SHOWN = { ArrowLeft: "←", ArrowRight: "→", ArrowUp: "↑", ArrowDown: "↓", " ": "Space", Enter: "Enter", Tab: "Tab", Escape: "Esc" };
const show = (k) => SHOWN[k] ?? k.toUpperCase();
function legacyKeys(t) {
  const ours = Object.entries(LEGACY).filter(([, acts]) => acts.some((a) => same(a, t))).map(([k]) => show(k));
  const theirs = "key" in t
    ? Object.entries(UPSTREAM).filter(([, v]) => v === t.key).map(([k]) => show(k))
    : Object.entries(UPSTREAM_ENC).filter(([, a]) => same(a, t)).map(([k]) => show(k));
  return [...ours, ...theirs];
}

// The key map's rows for a game: the computer key(s) that press the input, the FM-1 input, the
// Spectrum action. `following`: the page sees the running game (Spectrum keys), else the legacy map.
export function panelRows(game, following = true) {
  const table = buildTable(game);
  return game.keys.map(({ input, actions }) => {
    const [kind, ...rest] = input.split(".");
    const fm1 = kind === "enc" ? `${rest[0]} ${rest[1]}` : rest.join(".");
    const t = target(input);
    let computer = "—";
    if (t && following) {
      let reach;
      if (actions.length === 1) reach = same(table.single.get(actions[0]), t);
      else reach = same(table.chords.find((c) => c.tokens.join() === actions.join())?.target, t);
      const names = actions.map(computerName);
      if (reach && names.every(Boolean)) computer = names.join(" + ");
    } else if (t) {
      const keys = legacyKeys(t);
      if (keys.length) computer = keys.join(" / ");
    }
    return { computer, fm1, spectrum: actions.map(actionName).join(" + ") };
  });
}

// ---------------------------------------------------------------- labels on the drawing

// The on-screen keyboard's own buttons (the game's key map does not get them while it is open):
// OCT- / OCT+ move it to the top / bottom of the screen, SAVE and REC shift the next key.
const OSK_BUTTON_LABELS = { 0: "KEYS ↑", 1: "KEYS ↓", 9: "CS", 13: "SS" };

// What to write on the FM-1 drawing: [{ target, text }]. HOME and PLAY always; in a game view
// (or game given with view null: the key map's game) each mapped key, button and encoder turn;
// in the picker and the menus the knobs the arrows turn.
export function labels(game, view) {
  const out = [{ target: { key: HOME }, text: "HOME (Esc)" }, { target: { key: PLAY }, text: "PLAY (Tab)" }];
  if (view !== null && !gameView(view)) {
    out.push({ target: { enc: 0, step: 0 }, text: "← →" }, { target: { enc: 2, step: 0 }, text: "↑ ↓" });
    return out;
  }
  if (view === VIEW.OSK) out.push({ target: { enc: 0, step: 0 }, text: "← →" }, { target: { enc: 2, step: 0 }, text: "↑ ↓" },
    ...Object.entries(OSK_BUTTON_LABELS).map(([key, text]) => ({ target: { key: Number(key) }, text })));
  const turns = new Map();
  for (const { input, actions } of game?.keys ?? []) {
    const t = target(input);
    const text = actions.map(shortName).filter(Boolean).join("+");
    if (!t || !text) continue;
    if ("key" in t) {
      if (!(view === VIEW.OSK && t.key in OSK_BUTTON_LABELS)) out.push({ target: t, text });
    } else if (!(view === VIEW.OSK && (t.enc === 0 || t.enc === 2))) {
      const both = turns.get(t.enc) ?? {};
      both[t.step > 0 ? "cw" : "ccw"] = text;
      turns.set(t.enc, both);
    }
  }
  for (const [enc, { ccw, cw }] of turns) out.push({ target: { enc, step: 0 }, text: `${ccw ?? "·"} ${cw ?? "·"}` });
  return out;
}

// Where each control sits on the drawing, from its own hit test: hit value -> box
export function scanPanel(hit, W, H, step = 2) {
  const boxes = new Map();
  for (let y = step / 2; y < H; y += step) {
    for (let x = step / 2; x < W; x += step) {
      const h = hit(x, y);
      if (h < 0) continue;
      const b = boxes.get(h);
      if (!b) boxes.set(h, { x0: x, y0: y, x1: x, y1: y });
      else {
        if (x < b.x0) b.x0 = x;
        if (x > b.x1) b.x1 = x;
        if (y < b.y0) b.y0 = y;
        if (y > b.y1) b.y1 = y;
      }
    }
  }
  return boxes;
}

// a button with another button right above it (the lower row): its label goes below it
function lowerRow(boxes, b) {
  for (let id = 100; id < 114; id++) {
    const o = boxes.get(id);
    if (o && o !== b && o.y1 < b.y0 && o.x0 < b.x1 && o.x1 > b.x0) return true;
  }
  return false;
}

function drawLabels(ctx, boxes, list) {
  ctx.save();
  ctx.textAlign = "center";
  ctx.textBaseline = "middle";
  for (const { target: t, text } of list) {
    const b = boxes.get("key" in t ? 100 + t.key : t.enc);
    if (!b) continue;
    const isNote = "key" in t && t.key >= 14;
    // (the arrows are small in a monospace font: a larger sans one)
    const arrows = /[←→↑↓]/.test(text);
    const size = (isNote ? 17 : 16) + (arrows ? 5 : 0);
    ctx.font = arrows ? `700 ${size}px system-ui, sans-serif`
      : `700 ${size}px ui-monospace, SFMono-Regular, Menlo, Consolas, monospace`;
    // wider than its key: "HOME (Esc)" on two lines
    let lines = [text];
    if (ctx.measureText(text).width + 10 > b.x1 - b.x0 + 12 && text.includes(" (")) {
      const i = text.indexOf(" (");
      lines = [text.slice(0, i), text.slice(i + 1)];
    }
    const w = Math.max(...lines.map((l) => ctx.measureText(l).width)) + 10;
    const h = lines.length * (size + 2) + 4;
    const cx = (b.x0 + b.x1) / 2;
    // note keys: near the key's bottom; buttons: just above (the lower row: just below); knobs: below
    let cy;
    if (isNote) cy = b.y1 - h / 2 - 8;
    else if ("key" in t) cy = lowerRow(boxes, b) ? b.y1 + h / 2 + 6 : b.y0 - h / 2 - 3;
    else cy = b.y1 + h / 2 + 4;
    ctx.fillStyle = "rgba(20, 21, 23, 0.85)";
    ctx.strokeStyle = "#f2a33a";
    ctx.lineWidth = 1.5;
    ctx.beginPath();
    if (ctx.roundRect) ctx.roundRect(cx - w / 2, cy - h / 2, w, h, 5);
    else ctx.rect(cx - w / 2, cy - h / 2, w, h);
    ctx.fill();
    ctx.stroke();
    ctx.fillStyle = "#ffc56e";
    lines.forEach((l, k) => ctx.fillText(l, cx, cy + 1 + (k - (lines.length - 1) / 2) * (size + 2)));
  }
  ctx.restore();
}

// ---------------------------------------------------------------- the page

export function install(win, doc) {
  const state = { games: null, shown: null, running: null, snapped: null, keys: null, boxes: null, board: -1, onRun: null };
  const keys = () => (state.keys ??= win.fmzxInput ? createKeys(win.fmzxInput) : null);
  win.fmzxKey = (e, down) => keys()?.handle(e, down) ?? false;
  win.addEventListener?.("blur", () => state.keys?.releaseAll());

  // each frame: [view, game] from the firmware (null: the package has no address)
  win.fmzxFrame = (m) => {
    const r = m.fmzx;
    const games = state.games;
    if (!games || !Array.isArray(r) || r[0] > VIEW.MESSAGE || r[1] >= games.length) return;
    const [view, game] = r;
    if (state.running && state.running.view === view && state.running.game === game) return;
    state.running = { view, game };
    keys()?.setRunning(view, games[game]);
    // the key map snaps to a game when it starts (not in the picker: none runs there)
    if (gameView(view) && state.snapped !== game) {
      state.snapped = game;
      state.onRun?.();
    } else if (view === VIEW.PICKER) state.snapped = null;
  };

  win.fmzxDraw = (ctx, W, H) => {
    const input = win.fmzxInput;
    if (!input || !state.games) return;
    const board = input.board();
    if (board !== 0) return; // the FM-1's drawing only
    if (!state.boxes || state.board !== board) {
      state.boxes = scanPanel(input.hit, W, H);
      state.board = board;
    }
    const r = state.running;
    const list = r ? labels(state.games[r.game], r.view) : labels(state.shown, null);
    drawLabels(ctx, state.boxes, list);
  };

  // under the synth, the deck's width: the computer keys and the key map, two cards
  const deck = doc.querySelector(".deck");
  if (!deck) return;
  const wrap = doc.createElement("section");
  wrap.className = "fmzx";
  wrap.setAttribute("aria-label", "Computer keys and key map");
  const el = (tag, text, parent = box) => {
    const n = doc.createElement(tag);
    if (text !== undefined) n.textContent = text;
    parent.appendChild(n);
    return n;
  };
  const box = el("div", undefined, wrap);
  box.className = "box fmzx-keys";
  const mapBox = el("div", undefined, wrap);
  mapBox.className = "box fmzx-map";
  el("h3", "Computer keys");
  const dl = el("dl");
  const FIXED = [
    ["Esc", "HOME"],
    ["Tab", "PLAY (pause, OK)"],
    ["Enter", "PLAY in the picker, the menu and the on-screen keyboard"],
    ["Arrows", "move in the picker and the menu; Kempston in a game"],
    ["A–Z 0–9", "the same Spectrum key (Space, Enter too)"],
    ["L Shift", "CAPS SHIFT"],
    ["Alt", "SYMBOL SHIFT"],
    ["R Shift", "Kempston fire"],
  ];
  for (const [k, v] of FIXED) { el("dt", k, dl); el("dd", v, dl); }
  const msg = el("p");
  deck.insertBefore(wrap, deck.querySelector(".hint"));

  // the demo video, under the two cards: a picture of ours until it is clicked, so nothing is
  // asked of the video's host before that
  const video = videoLinks(VIDEO);
  if (video) {
    const card = doc.createElement("section");
    card.className = "box fmzx-video";
    card.setAttribute("aria-label", "Demo video");
    el("h3", "Demo video", card);
    const frame = el("div", undefined, card);
    frame.className = "fmzx-video-frame";
    const play = el("button", undefined, frame);
    play.type = "button";
    play.setAttribute("aria-label", "Play the demo video");
    const img = el("img", undefined, play);
    img.src = "demo-video.jpg";
    img.alt = "";
    play.addEventListener("click", () => {
      const player = doc.createElement("iframe");
      player.src = video.embed;
      player.title = "FM-1 ZX demo video";
      player.allow = "autoplay; encrypted-media; picture-in-picture; fullscreen";
      player.allowFullscreen = true;
      frame.replaceChildren(player);
    });
    const link = el("a", "Watch on YouTube", el("p", undefined, card));
    link.href = video.watch;
    link.target = "_blank";
    link.rel = "noopener";
    wrap.after(card);
  }

  fetch("keymaps.json", { cache: "no-cache" }) // asked each time: the games of the package published
    .then((r) => { if (!r.ok) throw new Error(r.status); return r.json(); })
    .then((doc_) => {
      const games = doc_.games;
      state.games = games;
      if (Number.isInteger(doc_.fmzx_stats)) win.fmzxStats = doc_.fmzx_stats;
      msg.textContent = win.fmzxStats
        ? "A key presses the FM-1 input the running game maps to that Spectrum key; one the game does not map does nothing."
        : "This package does not tell the page which game runs: the keys are fm1-emu's piano map plus the arrows, Space, Enter, Esc and Tab, and the labels show the game picked here.";
      let saved = null;
      try { saved = win.localStorage.getItem("fmzx.game"); } catch { /* storage blocked */ }
      const head = el("div", undefined, mapBox);
      head.className = "fmzx-head";
      el("h3", "Key map", head);
      const select = el("select", undefined, head);
      select.setAttribute("aria-label", "Game");
      for (const g of games) {
        const o = doc.createElement("option");
        o.value = g.id;
        o.textContent = g.name;
        select.appendChild(o);
      }
      if (games.some((g) => g.id === saved)) select.value = saved;
      // the rows in groups (buttons, notes, knobs), flowing into as many columns as fit
      const list = el("div", undefined, mapBox);
      list.className = "fmzx-rows";
      const GROUPS = { button: "Buttons", note: "Note keys", enc: "Knobs" };
      const fill = () => {
        const game = games.find((g) => g.id === select.value);
        state.shown = game;
        list.replaceChildren();
        const rows = panelRows(game, !!win.fmzxStats);
        for (const [kind, title] of Object.entries(GROUPS)) {
          const mine = rows.filter((_, i) => game.keys[i].input.startsWith(`${kind}.`));
          if (!mine.length) continue;
          const group = el("div", undefined, list);
          group.className = "fmzx-group";
          el("h4", title, group);
          const line = (cells, cls) => {
            const r = el("div", undefined, group);
            r.className = cls;
            for (const c of cells) el("span", c, r);
          };
          line(["Computer", "FM-1", "Spectrum"], "fmzx-row fmzx-th");
          for (const r of mine) line([r.computer, r.fm1, r.spectrum], "fmzx-row");
        }
      };
      select.addEventListener("change", () => {
        try { win.localStorage.setItem("fmzx.game", select.value); } catch { /* storage blocked */ }
        fill();
        select.blur(); // back to playing from the keyboard (upstream's keydown skips selects)
      });
      // the list follows the running game (it snaps back when the running game changes)
      state.onRun = () => {
        const g = games[state.running.game];
        if (g && select.value !== g.id) { select.value = g.id; fill(); }
      };
      fill();
    })
    .catch(() => { msg.textContent = "The game key maps could not be loaded."; });
}

if (typeof window !== "undefined") install(window, document);
