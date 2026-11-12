// SPDX-FileCopyrightText: 2026 FM-1 ZX contributors
// SPDX-License-Identifier: GPL-3.0-or-later
//
// Headless smoke test of the built browser demo (tools/web-demo --check): boots DIR/fm1_web.wasm
// with DIR/fmzx.fwsc as the page does (JIT on, 240 MHz) and checks that it boots, draws the
// game picker, and plays sound from the border-and-tone test. One line per check; exit 0 when
// all pass, 1 otherwise.
//   node web/check.mjs build/web
import { readFileSync } from "node:fs";

const RATE = 44100;
const QUANTUM = 128;
const MAX_INSNS = 50_000_000;
// key ids (docs/controls.md)
const OCT_UP = 1;
const PLAY = 12;
// the picker is text on black: about 2000 lit pixels (2041 release, 2309 preview); a blank or
// barely started screen has none (upstream's smoke test uses the same bound)
const PICKER_LIT = 1000;

const arg = process.argv[2];
if (!arg) {
  console.error("usage: node web/check.mjs DIR");
  process.exit(1);
}
const dir = new URL(`${arg}/`, `file://${process.cwd()}/`);
let failed = false;
const report = (name, ok, detail) => {
  console.log(`${name}: ${ok ? "OK" : "FAIL"} (${detail})`);
  if (!ok) failed = true;
};

const { jitImports } = await import(new URL("jit.js", dir));
let x;
const { instance } = await WebAssembly.instantiate(readFileSync(new URL("fm1_web.wasm", dir)), {
  env: jitImports(() => x),
});
x = instance.exports;
x.fm1_jit(1, 16); // as the page runs it: hot code compiled, and the even pace
x.fm1_ahead(1024);

const start = performance.now();
const pkg = new Uint8Array(readFileSync(new URL("fmzx.fwsc", dir)));
const p = x.fm1_alloc(pkg.length);
new Uint8Array(x.memory.buffer, p, pkg.length).set(pkg);
const booted = x.fm1_boot_fwsc(p, pkg.length, 0, 0, 240);
report("boots", booted === 0, `fm1_boot_fwsc returned ${booted}, ${pkg.length} bytes of package`);
if (booted !== 0) process.exit(1);

// runs `s` emulated seconds; returns the RMS of the left output over them
const run = (s) => {
  let sum = 0;
  let n = 0;
  for (let i = 0; i < s * RATE; i += QUANTUM) {
    x.fm1_render(QUANTUM, MAX_INSNS);
    for (const v of new Float32Array(x.memory.buffer, x.fm1_out_l(), QUANTUM)) {
      sum += v * v;
      n++;
    }
  }
  return Math.sqrt(sum / n);
};
// a button down for 100 ms, then up
const press = (id) => {
  x.fm1_key(id, 1);
  run(0.1);
  x.fm1_key(id, 0);
};

run(3);
const lcd = new Uint16Array(x.memory.buffer, x.fm1_lcd(), 240 * 240);
const lit = lcd.reduce((n, v) => n + (v ? 1 : 0), 0);
const halted = x.fm1_halted();
report("picker", halted === 0 && lit > PICKER_LIT, `halted ${halted}, ${lit} lit pixels after 3 s`);

// OCT+ moves the picker to the second entry, "Border and tone test"; PLAY starts it
press(OCT_UP);
const quiet = run(0.5);
press(PLAY);
run(1.0 - 0.1);
const loud = run(1.0);
report(
  "sound",
  loud > 0.001 && loud > 10 * quiet,
  `RMS ${quiet.toFixed(4)} before PLAY, ${loud.toFixed(4)} in the second after it`,
);

const emulated = x.fm1_ms() / 1000;
const wall = (performance.now() - start) / 1000;
const speed = (emulated / wall).toFixed(2);
const span = `${emulated.toFixed(1)} s in ${wall.toFixed(1)} s`;
console.log(`speed: ${speed} emulated s / wall s (${span})`);
process.exit(failed ? 1 : 0);
