// Shared karaoke helpers for the studio and the player: timing maths and a canvas renderer
// that mirrors the ASS output (\kf wipe, single/alternate KTV rows, effect presets).
"use strict";

const CJK = /[　-鿿぀-ヿ가-힯豈-﫿]/;
const UNIT = /[　-鿿぀-ヿ가-힯豈-﫿]|[^\s|　-鿿぀-ヿ가-힯豈-﫿]+/g;

function splitUnits(text) {
  return text.match(UNIT) || [];
}

function separator(previous, word) {
  if (!previous) return "";
  return CJK.test(previous.slice(-1)) && CJK.test(word.slice(0, 1)) ? "" : " ";
}

function joinWords(words) {
  let out = "", previous = "";
  for (const word of words) { out += separator(previous, word) + word; previous = word; }
  return out;
}

function fmtTime(ms, withHours = false) {
  ms = Math.max(0, Math.round(ms));
  const m = Math.floor(ms / 60000), s = Math.floor(ms / 1000) % 60, cs = Math.floor(ms / 10) % 100;
  const head = withHours ? `${Math.floor(m / 60)}:${String(m % 60).padStart(2, "0")}` : String(m).padStart(2, "0");
  return `${head}:${String(s).padStart(2, "0")}.${String(cs).padStart(2, "0")}`;
}

// ASS colours are &HAABBGGRR with inverted alpha.
function assColor(value, fallback = "#ffffff") {
  const match = /&H([0-9a-f]{2})?([0-9a-f]{2})([0-9a-f]{2})([0-9a-f]{2})/i.exec(value || "");
  if (!match) return fallback;
  const alpha = 1 - parseInt(match[1] || "00", 16) / 255;
  return `rgba(${parseInt(match[4], 16)},${parseInt(match[3], 16)},${parseInt(match[2], 16)},${alpha.toFixed(3)})`;
}

// Index of the line being sung (or the last one started) at time t.
function activeLine(lines, t) {
  let lo = 0, hi = lines.length - 1, found = -1;
  while (lo <= hi) {
    const mid = (lo + hi) >> 1;
    if (lines[mid].start <= t) { found = mid; lo = mid + 1; } else hi = mid - 1;
  }
  return found;
}

// Same rule as polykara/ass.py: alternate rows show a line lead_in early, never before its row is free.
function layoutShown(lines, mode, leadIn) {
  const rowFree = [0, 0];
  return lines.map((line, index) => {
    if (mode !== "alternate") return { shown: line.start, slot: 0 };
    const slot = index % 2;
    const shown = Math.min(line.start, Math.max(rowFree[slot], line.start - leadIn));
    rowFree[slot] = line.end;
    return { shown, slot };
  });
}

// Fraction (0..1) of a word already sung, as \kf wipes it.
function wordProgress(word, t) {
  const [left, right] = word;
  if (t <= left) return 0;
  if (t >= right) return 1;
  return (t - left) / Math.max(1, right - left);
}

function styleFromConfig(config) {
  const lyric = config.lyric || {}, karaoke = config.karaoke || {}, video = config.video || {};
  const layout = config.layout || {}, effects = config.effects || {}, roman = config.romanization || {};
  return {
    width: video.play_res_x || 1920, height: video.play_res_y || 1080,
    font: lyric.font || "Arial", size: lyric.size || 58, bold: lyric.bold !== false,
    active: assColor(karaoke.active_color || lyric.color, "#3060ff"),
    inactive: assColor(karaoke.inactive_color || lyric.secondary_color, "#ffffff"),
    outlineColor: assColor(lyric.outline_color, "rgba(0,0,0,0.5)"), outline: lyric.outline ?? 3,
    marginV: lyric.margin_v ?? 150, marginL: lyric.margin_l ?? 80, marginR: lyric.margin_r ?? 80,
    mode: String(layout.mode || "single").toLowerCase(), leadIn: layout.lead_in_ms ?? 1500,
    rowGap: (layout.top_margin_v ?? (lyric.margin_v ?? 150) + 130) - (lyric.margin_v ?? 150),
    effect: String(effects.preset || "none").toLowerCase(), params: effects.params || {},
    romanSize: roman.size || 30,
  };
}

class KaraokeRenderer {
  constructor(canvas, style) {
    this.canvas = canvas;
    this.ctx = canvas.getContext("2d");
    this.setStyle(style);
    this.lines = [];
    this.layout = [];
  }

  setStyle(style) {
    this.style = { ...style };
    this.canvas.width = this.style.width;
    this.canvas.height = this.style.height;
    if (this.lines) this.setLines(this.lines);
  }

  setLines(lines) {
    this.lines = lines;
    this.layout = layoutShown(lines, this.style.mode, this.style.leadIn);
  }

  font(size) {
    return `${this.style.bold ? "bold " : ""}${size}px "${this.style.font}", "Noto Sans CJK SC", sans-serif`;
  }

  // Lines visible at time t with their row slot.
  visible(t) {
    const out = [];
    const last = activeLine(this.lines, t + (this.style.mode === "alternate" ? this.style.leadIn : 0));
    for (let i = Math.max(0, last - 3); i <= last + 1 && i < this.lines.length; i++) {
      if (i < 0) continue;
      const { shown, slot } = this.layout[i];
      if (t >= shown && t < this.lines[i].end) out.push({ index: i, slot });
    }
    return out;
  }

  draw(t, extra = {}) {
    const { ctx, style } = this;
    ctx.clearRect(0, 0, style.width, style.height);
    if (extra.background) { ctx.fillStyle = extra.background; ctx.fillRect(0, 0, style.width, style.height); }
    for (const { index, slot } of this.visible(t)) this.drawLine(this.lines[index], t, slot, index === extra.selected);
  }

  drawLine(line, t, slot, selected) {
    const { ctx, style } = this;
    const words = line.words && line.words.length ? line.words : null;
    const texts = words ? words.map((w) => w[2]) : [line.text];
    ctx.font = this.font(style.size);
    const pieces = [];
    let x = 0, previous = "";
    for (const text of texts) {
      const sep = separator(previous, text);
      x += sep ? ctx.measureText(sep).width : 0;
      const width = ctx.measureText(text).width;
      pieces.push({ text, x, width });
      x += width;
      previous = text;
    }
    const total = x;
    let left, y;
    if (style.mode === "alternate") {
      left = slot === 0 ? style.marginL : style.width - style.marginR - total;
      y = style.height - (slot === 0 ? style.marginV + style.rowGap : style.marginV);
    } else {
      left = (style.width - total) / 2;
      y = style.height - style.marginV;
    }
    left = Math.max(10, Math.min(left, style.width - total - 10));
    let alpha = 1;
    if (style.effect === "fade") {
      const fadeIn = style.params.fade_in ?? 200, fadeOut = style.params.fade_out ?? 200;
      const shown = this.layout[this.lines.indexOf(line)]?.shown ?? line.start;
      alpha = Math.min(1, (t - shown) / Math.max(1, fadeIn), (line.end - t) / Math.max(1, fadeOut));
    }
    ctx.save();
    ctx.globalAlpha = Math.max(0, alpha);
    ctx.textBaseline = "alphabetic";
    ctx.lineJoin = "round";
    pieces.forEach((piece, i) => {
      const word = words ? words[i] : [line.start, line.end, line.text];
      const progress = words ? wordProgress(word, t) : 0;
      const singing = progress > 0 && progress < 1;
      ctx.save();
      const cx = left + piece.x + piece.width / 2;
      if (singing && style.effect === "pop") {
        const scale = (style.params.scale ?? 125) / 100;
        const k = 1 + (scale - 1) * Math.sin(Math.PI * progress);
        ctx.translate(cx, y); ctx.scale(k, k); ctx.translate(-cx, -y);
      }
      if (singing && style.effect === "glow") {
        ctx.shadowColor = style.active;
        ctx.shadowBlur = (style.params.blur ?? 6) * 3 * Math.sin(Math.PI * progress);
      }
      const px = left + piece.x;
      ctx.lineWidth = style.outline * 2;
      ctx.strokeStyle = style.outlineColor;
      ctx.strokeText(piece.text, px, y);
      ctx.fillStyle = style.inactive;
      ctx.fillText(piece.text, px, y);
      if (progress > 0) {
        ctx.beginPath();
        ctx.rect(px - style.outline, y - style.size * 1.3, piece.width * progress + style.outline, style.size * 1.8);
        ctx.clip();
        ctx.fillStyle = style.active;
        ctx.fillText(piece.text, px, y);
      }
      ctx.restore();
    });
    if (selected) {
      ctx.strokeStyle = "rgba(255,200,0,0.8)";
      ctx.lineWidth = 3;
      ctx.strokeRect(left - 12, y - style.size * 1.05, total + 24, style.size * 1.35);
    }
    if (line.roman) {
      ctx.font = this.font(style.romanSize);
      ctx.fillStyle = style.inactive;
      ctx.lineWidth = 4;
      ctx.strokeText(line.roman, left, y - style.size * 1.15);
      ctx.fillText(line.roman, left, y - style.size * 1.15);
    }
    ctx.restore();
  }
}

if (typeof module !== "undefined") {
  module.exports = { splitUnits, joinWords, separator, fmtTime, assColor, activeLine, layoutShown, wordProgress, styleFromConfig, KaraokeRenderer };
}
