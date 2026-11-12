// SPDX-FileCopyrightText: 2026 FM-1 ZX contributors
// SPDX-License-Identifier: GPL-3.0-or-later
//
// The browser page's computer-keyboard map and controls panel. The keys claimed here (the
// joystick arrows, Space as fire, Enter, Esc and a few notes below C4) win over upstream's map;
// everything else is left to upstream's own handler.

export const KEYMAP = {
  ArrowLeft: [{ key: 14 }, { enc: 0, step: -1 }],
  ArrowRight: [{ key: 18 }, { enc: 0, step: 1 }],
  ArrowUp: [{ key: 15 }, { enc: 2, step: -1 }],
  ArrowDown: [{ key: 16 }, { enc: 2, step: 1 }],
  " ": [{ key: 33 }],
  Enter: [{ key: 12 }],
  Escape: [{ key: 8 }],
  c: [{ key: 17 }],
  v: [{ key: 19 }],
  b: [{ key: 20 }],
  "[": [{ key: 39 }],
  "]": [{ key: 40 }],
};

// Upstream's map (for the panel only): C4..F5 piano keys, OCT-/OCT+, REC, and the top buttons.
export const UPSTREAM = (() => {
  const m = { z: 0, x: 1, r: 13 };
  [..."awsedftgyhujkolp;'"].forEach((k, i) => { m[k] = 21 + i; });
  [2, 3, 4, 5, 6, 7, 8, 9, 10, 11].forEach((id, i) => { m[String((i + 1) % 10)] = id; });
  return m;
})();

// Upstream's , . turn the ALGORITHM encoder.
const UPSTREAM_ENC = { ",": { enc: 2, step: -1 }, ".": { enc: 2, step: 1 } };

export function handle(e, down, input) {
  const actions = KEYMAP[e.key.length === 1 ? e.key.toLowerCase() : e.key];
  if (!actions) return false;
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

const SHOWN = { ArrowLeft: "←", ArrowRight: "→", ArrowUp: "↑", ArrowDown: "↓", " ": "Space", Enter: "Enter", Escape: "Esc" };
const show = (k) => SHOWN[k] ?? k.toUpperCase();

export function computerKeys(id) {
  const ours = [];
  for (const [k, acts] of Object.entries(KEYMAP)) if (acts.some((a) => a.key === id)) ours.push(show(k));
  const theirs = Object.entries(UPSTREAM).filter(([, v]) => v === id).map(([k]) => show(k));
  return [...ours, ...theirs];
}

const ENCODERS = { SELECT: 0, ALGORITHM: 2 };

function encoderKeys(name, step) {
  const enc = ENCODERS[name];
  if (enc === undefined) return [];
  const ours = Object.entries(KEYMAP).filter(([, acts]) => acts.some((a) => a.enc === enc && a.step === step));
  const theirs = Object.entries(UPSTREAM_ENC).filter(([, a]) => a.enc === enc && a.step === step);
  return [...ours, ...theirs].map(([k]) => show(k));
}

const SHIFTS = { CAPS: "CAPS SHIFT", SYM: "SYMBOL SHIFT" };
function actionName(a) {
  const [kind, name] = a.split(".");
  if (kind === "kempston") return `Kempston ${name}`;
  return SHIFTS[name] ?? name;
}

const BUTTONS = ["OCT-", "OCT+", "FX", "SCL", "ENV", "LFO", "EDIT", "GLO", "HOME", "SAVE", "ARP", "SEQ", "PLAY", "REC"];
const NOTES = ["F", "F#", "G", "G#", "A", "A#", "B", "C", "C#", "D", "D#", "E"];
function inputId(name) {
  const b = BUTTONS.indexOf(name);
  if (b >= 0) return b;
  const m = /^([A-G]#?)(\d)$/.exec(name);
  if (!m) return -1;
  // semitones above F3: the scale runs F..E, so C..E belong to the octave number's previous span
  const pc = NOTES.indexOf(m[1]);
  const oct = Number(m[2]) - (pc >= 7 ? 1 : 0);
  return 14 + (oct - 3) * 12 + pc;
}

export function panelRows(game) {
  return game.keys.map(({ input, actions }) => {
    const [kind, ...rest] = input.split(".");
    let computer = "—";
    let fm1 = rest.join(".");
    if (kind === "enc") {
      const [name, dir] = rest;
      fm1 = `${name} ${dir}`;
      const keys = encoderKeys(name, dir === "cw" ? 1 : -1);
      if (keys.length) computer = keys.join(" / ");
    } else {
      const keys = computerKeys(inputId(rest[0]));
      if (keys.length) computer = keys.join(" / ");
    }
    return { computer, fm1, spectrum: actions.map(actionName).join(" + ") };
  });
}

const FIXED = [
  ["← → / ↑ ↓", "move in the picker"],
  ["Enter", "PLAY"],
  ["Esc", "HOME"],
  ["Z / X", "OCT- / OCT+"],
];

export function install(win, doc) {
  win.fmzxKey = (e, down) => (win.fmzxInput ? handle(e, down, win.fmzxInput) : false);
  const side = doc.querySelector(".side");
  if (!side) return;
  const box = doc.createElement("aside");
  box.className = "box fmzx";
  const el = (tag, text, parent = box) => {
    const n = doc.createElement(tag);
    if (text !== undefined) n.textContent = text;
    parent.appendChild(n);
    return n;
  };
  el("h3", "Picker and menu");
  const dl = el("dl");
  for (const [k, v] of FIXED) { el("dt", k, dl); el("dd", v, dl); }
  const msg = el("p");
  if (!win.fmzxInput) msg.textContent = "Computer keyboard input is not available on this page.";
  side.insertBefore(box, side.firstChild);

  fetch("keymaps.json")
    .then((r) => { if (!r.ok) throw new Error(r.status); return r.json(); })
    .then(({ games }) => {
      let saved = null;
      try { saved = win.localStorage.getItem("fmzx.game"); } catch { /* storage blocked */ }
      const select = el("select");
      select.setAttribute("aria-label", "Game");
      for (const g of games) {
        const o = doc.createElement("option");
        o.value = g.id;
        o.textContent = g.name;
        select.appendChild(o);
      }
      if (games.some((g) => g.id === saved)) select.value = saved;
      const table = el("table");
      const fill = () => {
        const game = games.find((g) => g.id === select.value);
        table.replaceChildren();
        const head = el("tr", undefined, table);
        for (const h of ["Computer", "FM-1", "Spectrum"]) el("th", h, head);
        for (const r of panelRows(game)) {
          const tr = el("tr", undefined, table);
          el("td", r.computer, tr); el("td", r.fm1, tr); el("td", r.spectrum, tr);
        }
      };
      select.addEventListener("change", () => {
        try { win.localStorage.setItem("fmzx.game", select.value); } catch { /* storage blocked */ }
        fill();
      });
      fill();
    })
    .catch(() => { msg.textContent = "The game key maps could not be loaded."; });
}

if (typeof window !== "undefined") install(window, document);
