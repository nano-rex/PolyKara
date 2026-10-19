// AudioWorklet: key shift (two-tap crossfaded delay-line pitch shifter) and
// karaoke vocal removal (centre-channel cancellation that keeps the bass).
"use strict";

class KaraokeProcessor extends AudioWorkletProcessor {
  static get parameterDescriptors() {
    return [
      { name: "ratio", defaultValue: 1, minValue: 0.5, maxValue: 2, automationRate: "k-rate" },
      { name: "vocalCut", defaultValue: 0, minValue: 0, maxValue: 1, automationRate: "k-rate" },
    ];
  }

  constructor() {
    super();
    this.window = Math.round(sampleRate * 0.06);
    this.size = this.window * 2 + 8;
    this.buffers = [new Float32Array(this.size), new Float32Array(this.size)];
    this.write = 0;
    this.phase = 0;
    this.low = [0, 0];
    this.lowCoeff = 1 - Math.exp((-2 * Math.PI * 160) / sampleRate);
  }

  read(buffer, delay) {
    let position = this.write - delay;
    while (position < 0) position += this.size;
    const i = Math.floor(position), frac = position - i;
    return buffer[i % this.size] * (1 - frac) + buffer[(i + 1) % this.size] * frac;
  }

  process(inputs, outputs, parameters) {
    const input = inputs[0], output = outputs[0];
    if (!input || !input.length) return true;
    const ratio = parameters.ratio[0], cut = parameters.vocalCut[0] > 0.5;
    const left = input[0], right = input[1] || input[0];
    const outL = output[0], outR = output[1] || output[0];
    const step = (1 - ratio) / this.window;
    for (let n = 0; n < left.length; n++) {
      let l = left[n], r = right[n];
      if (cut) {
        const mid = (l + r) * 0.5, side = (l - r) * 0.5;
        // two one-pole stages: keep the bass and kick, drop the centred voice
        this.low[0] += this.lowCoeff * (mid - this.low[0]);
        this.low[1] += this.lowCoeff * (this.low[0] - this.low[1]);
        l = side + this.low[1];
        r = -side + this.low[1];
      }
      if (Math.abs(ratio - 1) < 1e-4) {
        outL[n] = l; outR[n] = r;
        continue;
      }
      this.buffers[0][this.write] = l;
      this.buffers[1][this.write] = r;
      this.phase = (this.phase + step) % 1;
      if (this.phase < 0) this.phase += 1;
      const phase2 = (this.phase + 0.5) % 1;
      const g1 = Math.sin(Math.PI * this.phase) ** 2, g2 = Math.sin(Math.PI * phase2) ** 2;
      const d1 = 2 + this.phase * this.window, d2 = 2 + phase2 * this.window;
      outL[n] = this.read(this.buffers[0], d1) * g1 + this.read(this.buffers[0], d2) * g2;
      outR[n] = this.read(this.buffers[1], d1) * g1 + this.read(this.buffers[1], d2) * g2;
      this.write = (this.write + 1) % this.size;
    }
    return true;
  }
}

registerProcessor("karaoke-processor", KaraokeProcessor);
