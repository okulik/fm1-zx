// SPDX-FileCopyrightText: 2026 FM-1 ZX contributors
// SPDX-License-Identifier: GPL-3.0-or-later
//
// Headless smoke test of the built browser demo (tools/web-demo --check): boots DIR/fm1_web.wasm
// with DIR/fmzx.fwsc as the page does (JIT on, 240 MHz) and checks that it boots, draws the
// game picker, and plays sound from the border-and-tone test. One line per check; exit 0 when
// all pass, 1 otherwise. The preview package (RGB565) boots in a second instance as the colour
// reference for the release package's RGB444.
//   node web/check.mjs build/web build/fm1/fmzx-preview.fwsc
import { readFileSync } from "node:fs";

const RATE = 44100;
const QUANTUM = 128;
const MAX_INSNS = 50_000_000;
// key ids (docs/controls.md)
const OCT_UP = 1;
const PLAY = 12;
// the picker is text on black: about 2300 lit pixels; a blank or
// barely started screen has none (upstream's smoke test uses the same bound)
const PICKER_LIT = 1000;

const arg = process.argv[2];
const previewPath = process.argv[3];
if (!arg || !previewPath) {
  console.error("usage: node web/check.mjs DIR PREVIEW.fwsc");
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
const releasePicker = lcd.slice();
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

// colours: the release package sends RGB444 (COLMOD 0x03), the preview RGB565 (0x55); the same
// picker through both must come out alike, within the widening of 4-bit channels (0xD7 >> 3 vs
// the widened 0xD). The wasm keeps one global machine, so the preview boots in a second instance.
{
  let y;
  const second = await WebAssembly.instantiate(readFileSync(new URL("fm1_web.wasm", dir)), {
    env: jitImports(() => y),
  });
  y = second.instance.exports;
  y.fm1_jit(1, 16);
  y.fm1_ahead(1024);
  const prev = new Uint8Array(readFileSync(previewPath));
  const q = y.fm1_alloc(prev.length);
  new Uint8Array(y.memory.buffer, q, prev.length).set(prev);
  const rc = y.fm1_boot_fwsc(q, prev.length, 0, 0, 240);
  if (rc !== 0) {
    report("colours", false, `preview fm1_boot_fwsc returned ${rc}`);
  } else {
    for (let i = 0; i < 3 * RATE; i += QUANTUM) y.fm1_render(QUANTUM, MAX_INSNS);
    // (the release picker was copied at 3 s, before the sound check left it for the tone test)
    const a = releasePicker;
    const b = new Uint16Array(y.memory.buffer, y.fm1_lcd(), 240 * 240);
    let zeroMismatch = 0;
    let worst = [0, 0, 0];
    const seen = new Set();
    for (let i = 0; i < a.length; i++) {
      if (!a[i] !== !b[i]) zeroMismatch++;
      const d = [11, 5, 0].map((sh, k) => {
        const m = k === 1 ? 63 : 31;
        return Math.abs(((a[i] >> sh) & m) - ((b[i] >> sh) & m));
      });
      for (let k = 0; k < 3; k++) worst[k] = Math.max(worst[k], d[k]);
      if (a[i]) seen.add(a[i]);
    }
    const ok =
      zeroMismatch === 0 && worst[0] <= 1 && worst[1] <= 2 && worst[2] <= 1 && seen.size >= 2;
    report(
      "colours",
      ok,
      `${zeroMismatch} pixels lit in only one package, worst |dR5| ${worst[0]} |dG6| ${worst[1]} |dB5| ${worst[2]}, ${seen.size} distinct colours`,
    );
  }
}

process.exit(failed ? 1 : 0);
