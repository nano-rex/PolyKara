// PolyKara Studio: tap-timing karaoke editor (SayaSub-style workflow, clean-room).
"use strict";

const $ = (id) => document.getElementById(id);
const media = $("media"), timeline = $("timeline"), tctx = timeline.getContext("2d");
const state = {
  song: null, lines: [], selected: 0, unit: -1, dirty: false,
  undo: [], redo: [], peaks: [], peakRate: 100, config: {},
  viewStart: 0, pxPerSec: 120, follow: true,
  tap: { on: false, index: 0, draft: null, heldKey: null },
  drag: null,
};
let renderer;

// ---------- model ----------
function cleanLine(line) {
  return { start: Math.round(line.start), end: Math.round(line.end), text: line.text.trim(), words: line.words.map(([a, b, w]) => [Math.round(a), Math.round(b), w]) };
}
function snapshot() { return JSON.stringify(state.lines); }
function commit(before) {
  state.undo.push(before);
  if (state.undo.length > 200) state.undo.shift();
  state.redo = [];
  setDirty(true);
}
function edit(fn) {
  const before = snapshot();
  fn();
  if (snapshot() !== before) commit(before);
  refresh();
}
function setDirty(value) {
  state.dirty = value;
  $("save").textContent = value ? "Save •" : "Save";
}
function line() { return state.lines[state.selected]; }
function unitsOf(item) { return splitUnits(item.text); }
function now() { return media.currentTime * 1000; }
function sortLines() {
  const current = line();
  state.lines.sort((a, b) => a.start - b.start);
  state.selected = Math.max(0, state.lines.indexOf(current));
}
function fixLine(item) {
  if (item.words.length) {
    item.words.sort((a, b) => a[0] - b[0]);
    item.start = Math.min(item.start, item.words[0][0]);
    item.end = Math.max(item.end, item.words[item.words.length - 1][1]);
  }
  item.end = Math.max(item.end, item.start + 100);
}
function parseTime(text) {
  const match = /^(?:(\d+):)?(\d+)(?:[.,](\d{1,3}))?$/.exec(text.trim());
  if (!match) return null;
  return (Number(match[1] || 0) * 60 + Number(match[2])) * 1000 + Number(((match[3] || "0") + "00").slice(0, 3));
}

// ---------- loading / saving ----------
async function api(path, options = {}) {
  const response = await fetch(path, options);
  const data = await response.json().catch(() => ({}));
  if (!response.ok) throw new Error(data.error || response.statusText);
  return data;
}
async function loadSongs() {
  const songs = await api("/api/songs");
  const select = $("song");
  select.innerHTML = songs.map((s) => `<option value="${s.id}">${escapeHtml(s.title || s.id)} — ${escapeHtml(s.artist)}${s.media ? "" : " (no media)"}</option>`).join("");
  const wanted = new URLSearchParams(location.search).get("song") || localStorage.getItem("pk.song");
  if (wanted && songs.some((s) => s.id === wanted)) select.value = wanted;
  if (songs.length) await loadSong(select.value);
  else $("source").textContent = "songs.csv has no songs yet";
}
async function loadSong(id) {
  if (state.dirty && !window.confirm("Discard unsaved changes?")) { $("song").value = state.song.row.id; return; }
  const data = await api(`/api/song/${id}`);
  state.song = data;
  state.lines = data.lines.map((l) => ({ ...l, words: l.words.map((w) => [...w]) }));
  state.selected = 0; state.undo = []; state.redo = []; state.viewStart = 0;
  setDirty(false);
  try { localStorage.setItem("pk.song", id); } catch (e) { /* storage unavailable */ }
  $("source").textContent = `${data.lines.length} lines from ${data.source}`;
  media.src = data.media || "";
  media.classList.toggle("audio-only", !data.video);
  state.peaks = [];
  if (data.media) api(`/api/song/${id}/peaks`).then((p) => { state.peaks = p.peaks; state.peakRate = p.rate; }).catch(() => {});
  if (data.job && data.job.state === "running") pollJob(id);
  refresh();
}
async function save() {
  if (!state.song) return;
  const body = JSON.stringify({ lines: state.lines.map(cleanLine) });
  const result = await api(`/api/song/${state.song.row.id}`, { method: "PUT", headers: { "Content-Type": "application/json" }, body });
  setDirty(false);
  $("source").textContent = `saved ${result.lines} lines → ${result.saved}`;
}

// ---------- line list ----------
function escapeHtml(value) {
  return String(value).replace(/[&<>"]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" }[c]));
}
function renderList() {
  const list = $("lines");
  list.innerHTML = state.lines.map((item, i) => {
    const units = unitsOf(item);
    const timed = item.words.length === units.length && units.length > 0;
    const body = units.map((u, k) => `<span class="u${item.words[k] ? " timed" : ""}${i === state.selected && k === state.unit ? " cur" : ""}">${escapeHtml(u)}</span>`).join(" ");
    return `<li data-i="${i}" class="${i === state.selected ? "sel" : ""}${timed ? "" : " untimed"}"><span class="t">${fmtTime(item.start)}</span><span class="txt">${body || "&nbsp;"}</span></li>`;
  }).join("");
  const selected = list.children[state.selected];
  if (selected) selected.scrollIntoView({ block: "nearest" });
}
function fillForm() {
  const item = line();
  $("line-text").value = item ? item.text : "";
  $("line-start").value = item ? fmtTime(item.start) : "";
  $("line-end").value = item ? fmtTime(item.end) : "";
}
function refresh() {
  renderList();
  if (document.activeElement && !$("line-form").contains(document.activeElement)) fillForm();
  renderer.setLines(previewLines());
  $("tap-status").textContent = state.tap.on ? tapStatus() : "";
}
function select(i, seek = true) {
  if (!state.lines.length) return;
  state.selected = Math.max(0, Math.min(state.lines.length - 1, i));
  state.unit = -1;
  if (seek && media.paused) media.currentTime = Math.max(0, line().start - 500) / 1000;
  if (state.tap.on) resetTap();
  refresh();
  fillForm();
}

// ---------- tap timing ----------
function resetTap() { state.tap.index = 0; state.tap.draft = null; state.tap.heldKey = null; }
function tapStatus() {
  const item = line();
  if (!item) return "";
  const units = unitsOf(item);
  return `TAP line ${state.selected + 1}: ${units[state.tap.index] ? `next “${units[state.tap.index]}” (${state.tap.index + 1}/${units.length})` : "done"}`;
}
function tapDown(key) {
  const item = line();
  if (!item) return;
  const units = unitsOf(item);
  if (!units.length) return;
  const t = now();
  if (!state.tap.draft) state.tap.draft = units.map((u) => [null, null, u]);
  if (state.tap.heldKey !== null) tapEnd(t);
  if (state.tap.index >= units.length) return;
  state.tap.draft[state.tap.index][0] = t;
  state.tap.heldKey = key;
  state.unit = state.tap.index;
  refresh();
}
function tapUp(key) {
  if (state.tap.heldKey === key) tapEnd(now());
}
function tapEnd(t) {
  const draft = state.tap.draft;
  const unit = draft && draft[state.tap.index];
  state.tap.heldKey = null;
  if (!unit || unit[0] === null) return;
  unit[1] = Math.max(unit[0] + 30, t);
  state.tap.index += 1;
  if (state.tap.index >= draft.length) finishTapLine();
  refresh();
}
function finishTapLine() {
  const draft = state.tap.draft;
  const index = state.selected;
  edit(() => {
    const item = state.lines[index];
    item.words = draft.map(([a, b, w]) => [Math.round(a), Math.round(b), w]);
    const last = item.words[item.words.length - 1][1];
    const next = state.lines[index + 1];
    item.start = item.words[0][0];
    item.end = next && next.start > last && next.words.length ? Math.min(last + 500, next.start) : last + 500;
  });
  resetTap();
  if (index + 1 < state.lines.length) { state.selected = index + 1; state.unit = -1; }
  refresh();
}

// ---------- line operations ----------
function evenTiming(item) {
  const units = unitsOf(item);
  const span = (item.end - item.start) / Math.max(1, units.length);
  item.words = units.map((u, k) => [Math.round(item.start + k * span), Math.round(item.start + (k + 1) * span), u]);
}
function applyForm(event) {
  event.preventDefault();
  const item = line();
  if (!item) return;
  edit(() => {
    const text = $("line-text").value.trim();
    const start = parseTime($("line-start").value), end = parseTime($("line-end").value);
    if (start !== null && start !== item.start) {
      const delta = start - item.start;
      item.start = start; item.end += delta;
      item.words.forEach((w) => { w[0] += delta; w[1] += delta; });
    }
    if (end !== null) item.end = Math.max(item.start + 100, end);
    if (text !== item.text) {
      item.text = text;
      const units = unitsOf(item);
      // keep timing when only the spelling changed; otherwise the line needs re-tapping
      item.words = item.words.length === units.length ? item.words.map((w, k) => [w[0], w[1], units[k]]) : [];
    }
    fixLine(item);
    sortLines();
  });
  fillForm();
}
function insertLine() {
  edit(() => {
    const item = line();
    const start = item ? item.end : now();
    state.lines.splice(state.selected + 1, 0, { start, end: start + 3000, text: "new line", words: [] });
    state.selected += state.lines.length > 1 ? 1 : 0;
  });
  fillForm();
  $("line-text").select();
}
function deleteLine() {
  if (!line()) return;
  edit(() => { state.lines.splice(state.selected, 1); state.selected = Math.min(state.selected, state.lines.length - 1); });
  fillForm();
}
function mergeLine() {
  const item = line(), next = state.lines[state.selected + 1];
  if (!item || !next) return;
  edit(() => {
    const timed = item.words.length && next.words.length;
    item.text = joinWords([item.text, next.text]);
    item.words = timed ? item.words.concat(next.words) : [];
    item.end = Math.max(item.end, next.end);
    state.lines.splice(state.selected + 1, 1);
  });
}
function splitLine() {
  const item = line();
  const units = item ? unitsOf(item) : [];
  const at = state.unit > 0 ? state.unit : Math.floor(units.length / 2);
  if (!item || at <= 0 || at >= units.length) return;
  edit(() => {
    const timed = item.words.length === units.length;
    const second = {
      start: timed ? item.words[at][0] : Math.round((item.start + item.end) / 2), end: item.end,
      text: joinWords(units.slice(at)), words: timed ? item.words.slice(at) : [],
    };
    item.text = joinWords(units.slice(0, at));
    item.words = timed ? item.words.slice(0, at) : [];
    item.end = timed ? item.words[item.words.length - 1][1] : second.start;
    state.lines.splice(state.selected + 1, 0, second);
  });
}
function nudge(delta, all = false) {
  edit(() => {
    for (const item of all ? state.lines : [line()].filter(Boolean)) {
      item.start += delta; item.end += delta;
      item.words.forEach((w) => { w[0] += delta; w[1] += delta; });
      if (item.start < 0) { const fix = -item.start; item.start = 0; item.end += fix; item.words.forEach((w) => { w[0] += fix; w[1] += fix; }); }
    }
  });
}
function importText(text) {
  const rows = text.split(/\r?\n/).map((r) => r.trim()).filter(Boolean);
  if (!rows.length) return;
  edit(() => {
    state.lines = rows.map((r, i) => ({ start: i * 4000, end: i * 4000 + 3500, text: r, words: [] }));
    state.selected = 0;
  });
  fillForm();
}

// ---------- timeline ----------
const LANE_Y = [62, 112], LANE_H = 40;
function timeToX(ms) { return (ms / 1000 - state.viewStart) * state.pxPerSec; }
function xToTime(x) { return (x / state.pxPerSec + state.viewStart) * 1000; }
function drawTimeline() {
  const width = timeline.clientWidth;
  if (timeline.width !== width) timeline.width = width;
  const h = timeline.height;
  tctx.fillStyle = "#15171c"; tctx.fillRect(0, 0, width, h);
  const t = now();
  if (state.follow && !media.paused) {
    const x = timeToX(t);
    if (x > width * 0.8 || x < 0) state.viewStart = t / 1000 - (width * 0.2) / state.pxPerSec;
  }
  state.viewStart = Math.max(0, state.viewStart);
  // waveform
  if (state.peaks.length) {
    tctx.fillStyle = "#2c4a6e";
    const per = state.peakRate / state.pxPerSec;
    for (let x = 0; x < width; x++) {
      const from = Math.floor((state.viewStart + x / state.pxPerSec) * state.peakRate);
      let peak = 0;
      for (let k = from; k < from + Math.max(1, per); k++) peak = Math.max(peak, state.peaks[k] || 0);
      const ph = (peak / 255) * (h - 30);
      tctx.fillRect(x, 20 + (h - 30 - ph) / 2, 1, Math.max(1, ph));
    }
  }
  // ruler
  tctx.fillStyle = "#8a93a6"; tctx.font = "11px sans-serif";
  const step = state.pxPerSec > 300 ? 0.5 : state.pxPerSec > 60 ? 1 : 5;
  for (let s = Math.ceil(state.viewStart / step) * step; timeToX(s * 1000) < width; s += step) {
    const x = timeToX(s * 1000);
    tctx.fillRect(x, 0, 1, Number.isInteger(s) ? 10 : 5);
    if (Number.isInteger(s)) tctx.fillText(fmtTime(s * 1000).slice(0, 5), x + 3, 12);
  }
  // lines and words
  const viewEnd = xToTime(width);
  state.lines.forEach((item, i) => {
    if (item.end < state.viewStart * 1000 || item.start > viewEnd) return;
    const y = LANE_Y[i % 2];
    const x0 = timeToX(item.start), x1 = timeToX(item.end);
    tctx.fillStyle = i === state.selected ? "rgba(255,200,0,0.18)" : "rgba(255,255,255,0.06)";
    tctx.fillRect(x0, y - 14, x1 - x0, LANE_H + 14);
    tctx.fillStyle = i === state.selected ? "#ffd166" : "#9aa3b5";
    tctx.font = "11px sans-serif";
    tctx.fillText(`${i + 1}`, x0 + 2, y - 3);
    item.words.forEach(([a, b, w], k) => {
      const wx = timeToX(a), ww = Math.max(2, timeToX(b) - wx);
      tctx.fillStyle = i === state.selected && k === state.unit ? "#ff9f1c" : "#3a86ff";
      tctx.fillRect(wx, y, ww, LANE_H - 8);
      tctx.strokeStyle = "#0b0d12"; tctx.strokeRect(wx + 0.5, y + 0.5, ww - 1, LANE_H - 9);
      tctx.save(); tctx.beginPath(); tctx.rect(wx, y, ww, LANE_H); tctx.clip();
      tctx.fillStyle = "#fff"; tctx.font = "14px sans-serif"; tctx.fillText(w, wx + 3, y + 21);
      tctx.restore();
    });
    if (!item.words.length) { tctx.fillStyle = "#aaa"; tctx.font = "13px sans-serif"; tctx.fillText(item.text, x0 + 4, y + 21); }
  });
  // tap draft
  if (state.tap.on && state.tap.draft) {
    const y = LANE_Y[state.selected % 2];
    state.tap.draft.forEach(([a, b]) => {
      if (a === null) return;
      const wx = timeToX(a), ww = timeToX(b === null ? t : b) - wx;
      tctx.fillStyle = "rgba(255,80,80,0.7)"; tctx.fillRect(wx, y, Math.max(2, ww), LANE_H - 8);
    });
  }
  // playhead
  tctx.fillStyle = "#ff4d4d"; tctx.fillRect(timeToX(t), 0, 2, h);
}
function hitTest(x, y) {
  const t = xToTime(x);
  for (let i = 0; i < state.lines.length; i++) {
    const ly = LANE_Y[i % 2];
    if (y < ly - 14 || y > ly + LANE_H) continue;
    const item = state.lines[i];
    for (let k = 0; k < item.words.length; k++) {
      const [a, b] = item.words[k];
      const xa = timeToX(a), xb = timeToX(b);
      if (x >= xa - 5 && x <= xb + 5) {
        const edge = Math.abs(x - xa) <= 5 ? "start" : Math.abs(x - xb) <= 5 ? "end" : "move";
        return { line: i, word: k, edge };
      }
    }
    if (t >= item.start && t <= item.end) return { line: i, word: -1, edge: "line" };
  }
  return null;
}
timeline.addEventListener("mousedown", (event) => {
  const x = event.offsetX, y = event.offsetY;
  const hit = hitTest(x, y);
  if (!hit) { media.currentTime = Math.max(0, xToTime(x)) / 1000; return; }
  state.selected = hit.line; state.unit = hit.word;
  const whole = event.altKey || hit.edge === "line";
  state.drag = { ...hit, whole, x0: x, before: snapshot(), orig: JSON.parse(JSON.stringify(state.lines[hit.line])) };
  refresh(); fillForm();
});
window.addEventListener("mousemove", (event) => {
  if (!state.drag) {
    const hit = event.target === timeline ? hitTest(event.offsetX, event.offsetY) : null;
    timeline.style.cursor = hit && hit.edge !== "move" && hit.edge !== "line" ? "ew-resize" : hit ? "grab" : "default";
    return;
  }
  const rect = timeline.getBoundingClientRect();
  const delta = Math.round(((event.clientX - rect.left - state.drag.x0) / state.pxPerSec) * 1000);
  const item = state.lines[state.drag.line], orig = state.drag.orig;
  if (state.drag.whole) {
    item.start = Math.max(0, orig.start + delta); item.end = orig.end + (item.start - orig.start);
    item.words = orig.words.map(([a, b, w]) => [a + (item.start - orig.start), b + (item.start - orig.start), w]);
  } else {
    const k = state.drag.word, [a, b, w] = orig.words[k];
    const prevEnd = k > 0 ? item.words[k - 1][1] : -Infinity, nextStart = k + 1 < item.words.length ? item.words[k + 1][0] : Infinity;
    if (state.drag.edge === "start") item.words[k] = [Math.min(b - 20, Math.max(prevEnd, a + delta)), b, w];
    else if (state.drag.edge === "end") item.words[k] = [a, Math.max(a + 20, Math.min(nextStart, b + delta)), w];
    else { const d = Math.max(prevEnd - a, Math.min(nextStart - b, delta)); item.words[k] = [a + d, b + d, w]; }
    item.start = Math.min(orig.start, item.words[0][0]);
    item.end = Math.max(orig.end, item.words[item.words.length - 1][1]);
    if (k === 0) item.start = item.words[0][0];
  }
  renderer.setLines(previewLines());
});
window.addEventListener("mouseup", () => {
  if (!state.drag) return;
  const before = state.drag.before;
  state.drag = null;
  sortLines();
  if (snapshot() !== before) commit(before);
  refresh(); fillForm();
});
timeline.addEventListener("wheel", (event) => {
  event.preventDefault();
  if (event.ctrlKey) {
    const anchor = xToTime(event.offsetX) / 1000;
    state.pxPerSec = Math.max(20, Math.min(600, state.pxPerSec * (event.deltaY < 0 ? 1.2 : 1 / 1.2)));
    state.viewStart = anchor - event.offsetX / state.pxPerSec;
    $("zoom").value = state.pxPerSec;
  } else {
    state.viewStart += ((event.deltaY || event.deltaX) / state.pxPerSec) * 2;
    state.follow = media.paused ? state.follow : false;
  }
}, { passive: false });

// ---------- preview ----------
function previewLines() {
  return state.lines.map((item) => ({ start: item.start, end: item.end, text: item.text, words: item.words.length === unitsOf(item).length ? item.words : [] }));
}
function previewStyle() {
  const style = styleFromConfig(state.config);
  style.mode = $("pv-mode").value;
  style.effect = $("pv-effect").value;
  return style;
}
function frame() {
  const t = now();
  renderer.draw(t, { selected: state.selected, background: media.classList.contains("audio-only") ? "#101318" : null });
  $("clock").textContent = `${fmtTime(t)} / ${fmtTime((media.duration || 0) * 1000)}`;
  drawTimeline();
  requestAnimationFrame(frame);
}

// ---------- render job ----------
async function pollJob(id) {
  const box = $("job");
  box.hidden = false;
  for (;;) {
    const job = await api(`/api/jobs/${id}`).catch(() => ({ state: "failed", log: ["lost connection"] }));
    box.textContent = `[${job.state}] ` + (job.log || []).slice(-6).join("\n");
    if (job.state !== "running") break;
    await new Promise((r) => setTimeout(r, 1000));
  }
  $("render").disabled = false;
}

// ---------- keyboard ----------
const TAP_KEYS = new Set([" ", "f", "j"]);
function typing(event) { return ["INPUT", "TEXTAREA", "SELECT"].includes(event.target.tagName) && event.target.type !== "range"; }
document.addEventListener("keydown", (event) => {
  if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === "s") { event.preventDefault(); save().catch(alertError); return; }
  if (typing(event)) return;
  const key = event.key.length === 1 ? event.key.toLowerCase() : event.key;
  if ((event.ctrlKey || event.metaKey) && key === "z") { event.preventDefault(); event.shiftKey ? redo() : undo(); return; }
  if ((event.ctrlKey || event.metaKey) && key === "y") { event.preventDefault(); redo(); return; }
  if (state.tap.on && TAP_KEYS.has(key)) { event.preventDefault(); if (!event.repeat) tapDown(key); return; }
  switch (key) {
    case " ": case "p": event.preventDefault(); togglePlay(); break;
    case "t": toggleTap(); break;
    case "Escape": if (state.tap.on) toggleTap(); break;
    case "Backspace":
      if (state.tap.on && state.tap.index > 0) { event.preventDefault(); state.tap.index -= 1; state.tap.heldKey = null; state.tap.draft[state.tap.index] = [null, null, state.tap.draft[state.tap.index][2]]; refresh(); }
      break;
    case "ArrowUp": event.preventDefault(); select(state.selected - 1); break;
    case "ArrowDown": event.preventDefault(); select(state.selected + 1); break;
    case "ArrowLeft": event.preventDefault(); media.currentTime = Math.max(0, media.currentTime - (event.shiftKey ? 0.2 : 2)); break;
    case "ArrowRight": event.preventDefault(); media.currentTime += event.shiftKey ? 0.2 : 2; break;
    case "r": if (line()) { media.currentTime = Math.max(0, line().start - 1000) / 1000; media.play(); } break;
    case "[": nudge(event.shiftKey ? -10 : -50); break;
    case "]": nudge(event.shiftKey ? 10 : 50); break;
    case "Delete": deleteLine(); break;
    default: return;
  }
});
document.addEventListener("keyup", (event) => {
  const key = event.key.length === 1 ? event.key.toLowerCase() : event.key;
  if (state.tap.on && TAP_KEYS.has(key)) { event.preventDefault(); tapUp(key); }
});
function togglePlay() { media.paused ? media.play().catch(alertError) : media.pause(); }
function toggleTap() {
  state.tap.on = !state.tap.on;
  resetTap();
  $("tap").classList.toggle("on", state.tap.on);
  if (state.tap.on) { state.follow = true; if (line()) media.currentTime = Math.max(0, line().start - 2000) / 1000; }
  refresh();
}
function undo() {
  if (!state.undo.length) return;
  state.redo.push(snapshot());
  state.lines = JSON.parse(state.undo.pop());
  state.selected = Math.min(state.selected, state.lines.length - 1);
  setDirty(true); refresh(); fillForm();
}
function redo() {
  if (!state.redo.length) return;
  state.undo.push(snapshot());
  state.lines = JSON.parse(state.redo.pop());
  setDirty(true); refresh(); fillForm();
}
function alertError(error) { $("source").textContent = `⚠ ${error.message || error}`; }

// ---------- wiring ----------
$("song").addEventListener("change", (e) => loadSong(e.target.value).catch(alertError));
$("save").addEventListener("click", () => save().catch(alertError));
$("export").addEventListener("change", async (e) => {
  const fmt = e.target.value;
  e.target.value = "";
  if (!fmt || !state.song) return;
  if (state.dirty) await save().catch(alertError);
  location.href = `/api/song/${state.song.row.id}/export/${fmt}`;
});
$("render").addEventListener("click", async () => {
  if (!state.song) return;
  try {
    await save();
    $("render").disabled = true;
    await api(`/api/song/${state.song.row.id}/render`, { method: "POST" });
    pollJob(state.song.row.id);
  } catch (error) { alertError(error); $("render").disabled = false; }
});
$("lines").addEventListener("click", (e) => {
  const li = e.target.closest("li");
  if (!li) return;
  const i = Number(li.dataset.i);
  const unit = e.target.classList.contains("u") ? [...li.querySelectorAll(".u")].indexOf(e.target) : -1;
  select(i);
  state.unit = unit;
  const item = line();
  if (unit >= 0 && item.words[unit] && media.paused) media.currentTime = item.words[unit][0] / 1000;
  refresh();
});
$("line-form").addEventListener("submit", applyForm);
$("import").addEventListener("click", () => { $("import-text").value = state.lines.map((l) => l.text).join("\n"); $("import-dialog").showModal(); });
$("import-dialog").addEventListener("close", () => { if ($("import-dialog").returnValue === "ok") importText($("import-text").value); });
$("add-line").addEventListener("click", insertLine);
$("del-line").addEventListener("click", deleteLine);
$("merge-line").addEventListener("click", mergeLine);
$("split-line").addEventListener("click", splitLine);
$("even").addEventListener("click", () => line() && edit(() => evenTiming(line())));
$("clear").addEventListener("click", () => line() && edit(() => { line().words = []; }));
$("play").addEventListener("click", togglePlay);
$("tap").addEventListener("click", (e) => { toggleTap(); e.target.blur(); });
$("undo").addEventListener("click", undo);
$("redo").addEventListener("click", redo);
$("apply-offset").addEventListener("click", () => { const d = parseInt($("offset").value, 10); if (d) nudge(d, true); $("offset").value = "0"; });
$("zoom").addEventListener("input", (e) => { state.pxPerSec = Number(e.target.value); });
$("rate").addEventListener("change", (e) => { media.playbackRate = Number(e.target.value); });
for (const id of ["pv-mode", "pv-effect"]) $(id).addEventListener("change", () => { renderer.setStyle(previewStyle()); renderer.setLines(previewLines()); });
media.addEventListener("play", () => { $("play").textContent = "❚❚"; state.follow = true; });
media.addEventListener("pause", () => { $("play").textContent = "▶"; });
// libass maps PlayRes onto the video frame, so the preview takes the video's shape too
media.addEventListener("loadedmetadata", () => {
  $("stage").style.aspectRatio = media.videoWidth && media.videoHeight ? `${media.videoWidth} / ${media.videoHeight}` : "16 / 9";
});
window.addEventListener("beforeunload", (e) => { if (state.dirty) { e.preventDefault(); e.returnValue = ""; } });

(async function init() {
  state.config = await api("/api/config").catch(() => ({}));
  const style = styleFromConfig(state.config);
  $("pv-mode").value = style.mode === "alternate" ? "alternate" : "single";
  $("pv-effect").value = ["none", "fade", "pop", "glow"].includes(style.effect) ? style.effect : "none";
  renderer = new KaraokeRenderer($("preview"), previewStyle());
  await loadSongs().catch(alertError);
  requestAnimationFrame(frame);
})();
