// Run with: node --test tests/js/
"use strict";
const test = require("node:test");
const assert = require("node:assert");
const path = require("node:path");
const fs = require("node:fs");
const vm = require("node:vm");
const web = path.join(__dirname, "..", "..", "polykara", "web");
const k = require(path.join(web, "karaoke.js"));

test("units split CJK per character and Latin per word", () => {
  assert.deepStrictEqual(k.splitUnits("我爱 you|so"), ["我", "爱", "you", "so"]);
  assert.strictEqual(k.joinWords(["我", "爱", "you", "so"]), "我爱 you so");
});

test("ASS colours are BGR with inverted alpha", () => {
  assert.strictEqual(k.assColor("&H00FF0000"), "rgba(0,0,255,1.000)");
  assert.strictEqual(k.assColor("&H800000FF"), "rgba(255,0,0,0.498)");
});

test("alternate layout matches polykara/ass.py lead-in rule", () => {
  const lines = [{ start: 1000, end: 2000 }, { start: 2500, end: 3500 }, { start: 4000, end: 5000 }];
  assert.deepStrictEqual(k.layoutShown(lines, "alternate", 1500).map((x) => x.shown), [0, 1000, 2500]);
  assert.deepStrictEqual(k.layoutShown(lines, "single", 1500).map((x) => x.shown), [1000, 2500, 4000]);
});

test("active line and word progress", () => {
  const lines = [{ start: 0 }, { start: 1000 }, { start: 2000 }];
  assert.strictEqual(k.activeLine(lines, 1500), 1);
  assert.strictEqual(k.activeLine(lines, -1), -1);
  assert.strictEqual(k.wordProgress([1000, 2000, "x"], 1250), 0.25);
});

function loadWorklet() {
  let Processor;
  const context = { sampleRate: 48000, AudioWorkletProcessor: class {}, registerProcessor: (name, cls) => { Processor = cls; }, Math };
  vm.runInNewContext(fs.readFileSync(path.join(web, "pitch-worklet.js"), "utf8"), context);
  return new Processor();
}
function crossings(signal) {
  let count = 0;
  for (let i = 1; i < signal.length; i++) if (signal[i - 1] < 0 && signal[i] >= 0) count++;
  return count;
}
function run(processor, ratio, vocalCut, makeLeft, makeRight) {
  const seconds = 1, rate = 48000, block = 128, out = [];
  for (let start = 0; start < rate * seconds; start += block) {
    const l = new Float32Array(block), r = new Float32Array(block), ol = new Float32Array(block), or = new Float32Array(block);
    for (let n = 0; n < block; n++) { l[n] = makeLeft((start + n) / rate); r[n] = makeRight((start + n) / rate); }
    processor.process([[l, r]], [[ol, or]], { ratio: [ratio], vocalCut: [vocalCut] });
    out.push(...ol);
  }
  return out.slice(rate * 0.2);
}

test("worklet shifts a 440 Hz tone up two semitones", () => {
  const tone = (t) => Math.sin(2 * Math.PI * 440 * t);
  const out = run(loadWorklet(), Math.pow(2, 2 / 12), 0, tone, tone);
  const hz = crossings(out) / 0.8;
  assert.ok(Math.abs(hz - 493.9) < 12, `got ${hz} Hz`);
});

test("worklet vocal cut removes a centred tone but keeps a side tone", () => {
  const centre = (t) => Math.sin(2 * Math.PI * 1000 * t);
  const rms = (xs) => Math.sqrt(xs.reduce((a, x) => a + x * x, 0) / xs.length);
  assert.ok(rms(run(loadWorklet(), 1, 1, centre, centre)) < 0.03);
  assert.ok(rms(run(loadWorklet(), 1, 1, centre, (t) => -centre(t))) > 0.5);
});
