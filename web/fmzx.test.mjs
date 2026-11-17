// SPDX-FileCopyrightText: 2026 FM-1 ZX contributors
// SPDX-License-Identifier: GPL-3.0-or-later
import test from "node:test";
import assert from "node:assert/strict";
import {
  VIEW, HOME, PLAY, LEGACY, inputId, target, tokenOf, buildTable, resolve, createKeys, panelRows, labels,
  scanPanel, shortName,
} from "./fmzx.js";

const spy = () => {
  const calls = [];
  return { calls, setKey: (...a) => calls.push(["setKey", ...a]), turn: (...a) => calls.push(["turn", ...a]) };
};
const ev = (code, extra = {}) => ({ code, key: extra.key ?? "", repeat: false, preventDefault() { this.prevented = true; }, ...extra });

// a key map as keymaps.json gives it (buttons, notes, encoders in that order)
const GAME = {
  id: "test",
  name: "Test",
  keys: [
    { input: "button.OCT-", actions: ["key.1"] },
    { input: "button.FX", actions: ["key.CAPS", "key.5"] },
    { input: "button.SEQ", actions: ["key.CAPS"] },
    { input: "button.REC", actions: ["key.SYM"] },
    { input: "note.F3", actions: ["kempston.left"] },
    { input: "note.F#3", actions: ["kempston.up"] },
    { input: "note.G#3", actions: ["kempston.up"] },
    { input: "note.C4", actions: ["key.ENTER"] },
    { input: "note.D4", actions: ["key.SPACE"] },
    { input: "note.E4", actions: ["key.Q"] },
    { input: "note.F4", actions: ["key.5"] },
    { input: "note.C5", actions: ["kempston.fire"] },
    { input: "note.D5", actions: ["kempston.fire"] },
    { input: "note.G5", actions: ["none"] },
    { input: "enc.K1.cw", actions: ["key.P"] },
    { input: "enc.K1.ccw", actions: ["key.O"] },
    { input: "enc.SELECT.cw", actions: ["key.Q"] },
  ],
};

test("input ids", () => {
  assert.equal(inputId("OCT-"), 0);
  assert.equal(inputId("REC"), 13);
  assert.equal(inputId("F3"), 14);
  assert.equal(inputId("C4"), 21);
  assert.equal(inputId("G5"), 40);
  assert.deepEqual(target("enc.K1.cw"), { enc: 3, step: 1 });
  assert.deepEqual(target("enc.ALGORITHM.ccw"), { enc: 2, step: -1 });
});

test("physical keys give Spectrum keys", () => {
  assert.equal(tokenOf("KeyQ"), "key.Q");
  assert.equal(tokenOf("Digit0"), "key.0");
  assert.equal(tokenOf("Space"), "key.SPACE");
  assert.equal(tokenOf("Enter"), "key.ENTER");
  assert.equal(tokenOf("ShiftLeft"), "key.CAPS");
  assert.equal(tokenOf("AltLeft"), "key.SYM");
  assert.equal(tokenOf("AltRight"), "key.SYM");
  assert.equal(tokenOf("ShiftRight"), "kempston.fire");
  assert.equal(tokenOf("ArrowUp"), "kempston.up");
  assert.equal(tokenOf("Semicolon"), null);
});

test("the table: singles, the first of duplicates, chords, keys over encoders, none skipped", () => {
  const t = buildTable(GAME);
  assert.deepEqual(t.single.get("key.1"), { key: 0 });
  assert.deepEqual(t.single.get("kempston.up"), { key: 15 }); // F#3 before G#3
  assert.deepEqual(t.single.get("kempston.fire"), { key: 33 }); // C5 before D5
  assert.deepEqual(t.single.get("key.Q"), { key: 25 }); // E4, not the SELECT turn after it
  assert.deepEqual(t.single.get("key.P"), { enc: 3, step: 1 });
  assert.equal(t.single.has("none"), false);
  assert.equal(t.single.has("key.Z"), false);
  assert.deepEqual(t.chords, [{ tokens: ["key.CAPS", "key.5"], target: { key: 2 } }]);
  assert.deepEqual([...resolve(new Set(["key.CAPS", "key.5"]), t)], [2]);
  assert.deepEqual([...resolve(new Set(["key.CAPS"]), t)], [11]);
  assert.deepEqual([...resolve(new Set(["key.5", "key.1"]), t)].sort((a, b) => a - b), [0, 26]);
});

const playing = (view = VIEW.GAME) => {
  const s = spy();
  const k = createKeys(s);
  k.setRunning(view, GAME);
  return { s, k };
};

test("in a game a key presses the mapped FM-1 input, by physical key", () => {
  const { s, k } = playing();
  assert.equal(k.handle(ev("KeyQ", { key: "й" }), true), true);
  assert.equal(k.handle(ev("KeyQ"), false), true);
  assert.deepEqual(s.calls, [["setKey", 25, true], ["setKey", 25, false]]);
});

test("an unmapped Spectrum key does nothing but is claimed (no piano note)", () => {
  const { s, k } = playing();
  const e = ev("KeyZ");
  assert.equal(k.handle(e, true), true);
  assert.equal(e.prevented, true);
  assert.deepEqual(s.calls, []);
  assert.equal(k.handle(ev("Semicolon"), true), true);
  assert.deepEqual(s.calls, []);
});

test("a chord presses its input; one key alone its own", () => {
  const { s, k } = playing();
  k.handle(ev("ShiftLeft"), true);
  assert.deepEqual(s.calls, [["setKey", 11, true]]);
  s.calls.length = 0;
  k.handle(ev("Digit5"), true);
  assert.deepEqual(s.calls, [["setKey", 11, false], ["setKey", 2, true]]);
  s.calls.length = 0;
  k.handle(ev("ShiftLeft"), false);
  assert.deepEqual(s.calls, [["setKey", 2, false], ["setKey", 26, true]]);
});

test("arrows, Space, Enter, Alt and right Shift in a game", () => {
  const { s, k } = playing();
  for (const c of ["ArrowLeft", "ArrowUp", "Space", "Enter", "AltLeft", "ShiftRight"]) {
    k.handle(ev(c), true);
    k.handle(ev(c), false);
  }
  const pressed = s.calls.filter((c) => c[2]).map((c) => c[1]);
  assert.deepEqual(pressed, [14, 15, 23, 21, 13, 33]);
  assert.equal(s.calls.some((c) => c[0] === "turn"), false); // the knobs stay still in a game
});

test("an encoder target turns on each press, repeats included", () => {
  const { s, k } = playing();
  k.handle(ev("KeyP"), true);
  k.handle(ev("KeyP", { repeat: true }), true);
  k.handle(ev("KeyO"), true);
  assert.deepEqual(s.calls, [["turn", 3, 1], ["turn", 3, 1], ["turn", 3, -1]]);
});

test("repeats do not press again", () => {
  const { s, k } = playing();
  k.handle(ev("KeyQ"), true);
  k.handle(ev("KeyQ", { repeat: true }), true);
  assert.deepEqual(s.calls, [["setKey", 25, true]]);
});

test("Esc is HOME and Tab is PLAY in every view, and without the running game", () => {
  for (const view of [...Object.values(VIEW), null]) {
    const s = spy();
    const k = createKeys(s);
    if (view !== null) k.setRunning(view, GAME);
    const e = ev("Tab");
    assert.equal(k.handle(e, true), true);
    assert.equal(e.prevented, true);
    k.handle(ev("Tab"), false);
    k.handle(ev("Escape"), true);
    k.handle(ev("Escape"), false);
    assert.deepEqual(s.calls, [["setKey", PLAY, true], ["setKey", PLAY, false], ["setKey", HOME, true], ["setKey", HOME, false]], `view ${view}`);
  }
});

test("Enter is PLAY in the picker, the menus and the on-screen keyboard, ENTER in the game", () => {
  for (const view of [VIEW.PICKER, VIEW.MENU, VIEW.KEYS, VIEW.MESSAGE, VIEW.OSK]) {
    const { s, k } = playing(view);
    k.handle(ev("Enter"), true);
    assert.deepEqual(s.calls, [["setKey", PLAY, true]], `view ${view}`);
  }
  for (const view of [VIEW.GAME, VIEW.PAUSED]) {
    const { s, k } = playing(view);
    k.handle(ev("Enter"), true);
    assert.deepEqual(s.calls, [["setKey", 21, true]], `view ${view}`);
  }
});

test("Enter pressed in the picker lets PLAY go though the game started meanwhile", () => {
  const { s, k } = playing(VIEW.PICKER);
  k.handle(ev("Enter"), true);
  k.setRunning(VIEW.GAME, GAME);
  k.handle(ev("Enter"), false);
  assert.deepEqual(s.calls, [["setKey", PLAY, true], ["setKey", PLAY, false]]);
});

test("in the picker and the menus the arrows turn SELECT and ALGORITHM, letters do nothing", () => {
  for (const view of [VIEW.PICKER, VIEW.MENU, VIEW.KEYS]) {
    const { s, k } = playing(view);
    k.handle(ev("ArrowRight"), true);
    k.handle(ev("ArrowRight"), false);
    k.handle(ev("ArrowUp", { repeat: true }), true);
    assert.equal(k.handle(ev("KeyQ"), true), true);
    assert.deepEqual(s.calls, [["turn", 0, 1], ["turn", 2, -1]], `view ${view}`);
  }
});

test("the on-screen keyboard: arrows move its cursor, letters still reach the game", () => {
  const { s, k } = playing(VIEW.OSK);
  k.handle(ev("ArrowDown"), true);
  k.handle(ev("KeyQ"), true);
  assert.deepEqual(s.calls, [["turn", 2, 1], ["setKey", 25, true]]);
});

test("leaving the game releases the keys held for it", () => {
  const { s, k } = playing();
  k.handle(ev("KeyQ"), true);
  k.setRunning(VIEW.MENU, GAME);
  assert.deepEqual(s.calls, [["setKey", 25, true], ["setKey", 25, false]]);
  k.releaseAll();
  s.calls.length = 0;
  k.setRunning(VIEW.GAME, GAME);
  assert.deepEqual(s.calls, []);
});

test("a key held in the game and let go in the menu is not pressed again on return", () => {
  const { s, k } = playing();
  k.handle(ev("ArrowLeft"), true);
  k.setRunning(VIEW.MENU, GAME);
  k.handle(ev("ArrowLeft"), false);
  k.setRunning(VIEW.GAME, GAME);
  assert.deepEqual(s.calls, [["setKey", 14, true], ["setKey", 14, false]]);
});

test("without the running game: the legacy table by event.key, the rest to fm1-emu", () => {
  const s = spy();
  const k = createKeys(s);
  assert.equal(k.handle(ev("Space", { key: " " }), true), true);
  assert.equal(k.handle(ev("ArrowUp", { key: "ArrowUp" }), true), true);
  assert.equal(k.handle(ev("Enter", { key: "Enter" }), true), true);
  assert.equal(k.handle(ev("KeyA", { key: "a" }), true), false);
  assert.deepEqual(s.calls, [["setKey", 33, true], ["setKey", 15, true], ["turn", 2, -1], ["setKey", PLAY, true]]);
  assert.ok(Object.keys(LEGACY).includes("Tab"));
});

test("panel rows: the computer key that reaches each input", () => {
  const rows = panelRows(GAME);
  const by = Object.fromEntries(rows.map((r) => [r.fm1, r]));
  assert.deepEqual(by["FX"], { computer: "Left Shift + 5", fm1: "FX", spectrum: "CAPS SHIFT + 5" });
  assert.equal(by["F#3"].computer, "↑");
  assert.equal(by["G#3"].computer, "—"); // the second input for Kempston up
  assert.equal(by["C5"].computer, "Right Shift");
  assert.equal(by["D4"].computer, "Space");
  assert.equal(by["REC"].computer, "Alt");
  assert.equal(by["K1 cw"].computer, "P");
  assert.equal(by["SELECT cw"].computer, "—");
  assert.equal(by["G5"].computer, "—");
});

test("panel rows without the running game: the legacy keys", () => {
  const game = {
    keys: [
      { input: "note.F3", actions: ["key.O"] },
      { input: "note.C5", actions: ["key.SPACE"] },
      { input: "enc.K1.cw", actions: ["kempston.right", "key.CAPS"] },
    ],
  };
  const rows = panelRows(game, false);
  assert.deepEqual(rows[0], { computer: "↓", fm1: "F3", spectrum: "O" });
  assert.deepEqual(rows[1], { computer: "Space / K", fm1: "C5", spectrum: "SPACE" });
  assert.deepEqual(rows[2], { computer: "—", fm1: "K1 cw", spectrum: "Kempston right + CAPS SHIFT" });
});

test("labels: the game's map in a game view, the menu controls elsewhere", () => {
  const inGame = labels(GAME, VIEW.GAME);
  const text = (l, t) => l.find((x) => JSON.stringify(x.target) === JSON.stringify(t))?.text;
  assert.equal(text(inGame, { key: HOME }), "HOME (Esc)");
  assert.equal(text(inGame, { key: PLAY }), "PLAY (Tab)");
  assert.equal(text(inGame, { key: 2 }), "CS+5");
  assert.equal(text(inGame, { key: 21 }), "ENT");
  assert.equal(text(inGame, { key: 23 }), "SPC");
  assert.equal(text(inGame, { key: 33 }), "FIRE");
  assert.equal(text(inGame, { key: 14 }), "←");
  assert.equal(text(inGame, { key: 40 }), undefined); // none
  assert.equal(text(inGame, { key: 30 }), undefined); // unmapped
  assert.equal(text(inGame, { enc: 3, step: 0 }), "O P");
  const menu = labels(GAME, VIEW.MENU);
  assert.deepEqual(menu.map((l) => l.text), ["HOME (Esc)", "PLAY (Tab)", "← →", "↑ ↓"]);
  assert.ok(labels(GAME, null).some((l) => l.text === "CS+5")); // no running game: the side list's
});

test("labels in the on-screen keyboard: its own buttons, the game's map for the rest", () => {
  const osk = labels(GAME, VIEW.OSK);
  const text = (l, t) => l.find((x) => JSON.stringify(x.target) === JSON.stringify(t))?.text;
  assert.equal(text(osk, { key: 0 }), "KEYS ↑"); // OCT- (the game's 1 does not reach it here)
  assert.equal(text(osk, { key: 1 }), "KEYS ↓"); // OCT+
  assert.equal(text(osk, { key: 9 }), "CS"); // SAVE: CAPS SHIFT for the next key
  assert.equal(text(osk, { key: 13 }), "SS"); // REC: SYMBOL SHIFT
  assert.equal(osk.filter((l) => l.target.key === 0).length, 1);
  assert.equal(text(osk, { key: 2 }), "CS+5"); // FX: still the game's
  assert.equal(text(osk, { enc: 0, step: 0 }), "← →");
});

test("short names", () => {
  assert.equal(shortName("key.SYM"), "SS");
  assert.equal(shortName("kempston.down"), "↓");
  assert.equal(shortName("none"), "");
});

test("the panel scan boxes each control", () => {
  const hit = (x, y) => (x < 10 && y < 4 ? 114 : x >= 20 && x < 30 ? 0 : -1);
  const b = scanPanel(hit, 40, 8, 2);
  assert.deepEqual(b.get(114), { x0: 1, y0: 1, x1: 9, y1: 3 });
  assert.deepEqual(b.get(0), { x0: 21, y0: 1, x1: 29, y1: 7 });
});
