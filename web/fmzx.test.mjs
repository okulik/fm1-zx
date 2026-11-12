// SPDX-FileCopyrightText: 2026 FM-1 ZX contributors
// SPDX-License-Identifier: GPL-3.0-or-later
import test from "node:test";
import assert from "node:assert/strict";
import { KEYMAP, UPSTREAM, handle, computerKeys, panelRows } from "./fmzx.js";

const spy = () => {
  const calls = [];
  return { calls, setKey: (...a) => calls.push(["setKey", ...a]), turn: (...a) => calls.push(["turn", ...a]) };
};

test("claims exactly the table", () => {
  const want = ["ArrowLeft", "ArrowRight", "ArrowUp", "ArrowDown", " ", "Enter", "Escape", "c", "v", "b", "[", "]"];
  assert.deepEqual(Object.keys(KEYMAP).sort(), want.sort());
});

test("space is fire not PLAY", () => {
  const s = spy();
  assert.equal(handle({ key: " ", repeat: false }, true, s), true);
  assert.deepEqual(s.calls, [["setKey", 33, true]]);
});

test("arrows press joystick and turn encoder", () => {
  let s = spy();
  handle({ key: "ArrowUp", repeat: false }, true, s);
  assert.deepEqual(s.calls, [["setKey", 15, true], ["turn", 2, -1]]);
  s = spy();
  handle({ key: "ArrowUp", repeat: false }, false, s);
  assert.deepEqual(s.calls, [["setKey", 15, false]]);
  s = spy();
  handle({ key: "ArrowUp", repeat: true }, true, s);
  assert.deepEqual(s.calls, [["turn", 2, -1]]);
});

test("unclaimed keys pass through", () => {
  const s = spy();
  assert.equal(handle({ key: "a" }, true, s), false);
  assert.deepEqual(s.calls, []);
});

test("letters are matched lower-cased", () => {
  const s = spy();
  assert.equal(handle({ key: "C", repeat: false }, true, s), true);
  assert.deepEqual(s.calls, [["setKey", 17, true]]);
});

test("every FM-1 key has a computer key", () => {
  for (let id = 0; id <= 40; id++) assert.ok(computerKeys(id).length > 0, `key ${id}`);
});

test("no key collides", () => {
  for (const k of Object.keys(KEYMAP)) assert.ok(!(k in UPSTREAM), k);
});

test("castaway rows", () => {
  const game = {
    id: "castaway",
    name: "Castaway",
    keys: [
      { input: "note.F3", actions: ["key.O"] },
      { input: "note.C5", actions: ["key.SPACE"] },
      { input: "enc.K1.cw", actions: ["kempston.right", "key.CAPS"] },
    ],
  };
  const rows = panelRows(game);
  assert.deepEqual(rows[0], { computer: "←", fm1: "F3", spectrum: "O" });
  assert.deepEqual(rows[1], { computer: "Space / K", fm1: "C5", spectrum: "SPACE" });
  assert.deepEqual(rows[2], { computer: "—", fm1: "K1 cw", spectrum: "Kempston right + CAPS SHIFT" });
});

test("claimed keys prevent the browser default, unclaimed do not", () => {
  let n = 0;
  const e = (key) => ({ key, repeat: false, preventDefault: () => n++ });
  handle(e("ArrowDown"), true, spy());
  handle(e(" "), false, spy());
  assert.equal(n, 2);
  handle(e("a"), true, spy());
  assert.equal(n, 2);
});
