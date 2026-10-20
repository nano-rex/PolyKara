// PolyKara Player: local music + karaoke lyrics player (KuGou-style features, clean-room).
"use strict";

const $ = (id) => document.getElementById(id);
const media = $("media");
const EQ_BANDS = [31, 62, 125, 250, 500, 1000, 2000, 4000, 8000, 16000];
const EQ_PRESETS = {
  Flat: [0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
  Pop: [-1, 1, 3, 4, 3, 0, -1, -1, 1, 2],
  Rock: [5, 4, 2, -1, -2, 0, 2, 4, 5, 5],
  Jazz: [3, 2, 1, 2, -1, -1, 0, 1, 2, 3],
  Classical: [4, 3, 2, 1, -1, -1, 0, 2, 3, 4],
  Dance: [6, 5, 2, 0, 0, -2, -2, -2, 0, 0],
  Vocal: [-2, -3, -2, 1, 4, 4, 3, 1, 0, -2],
  "Bass boost": [7, 6, 4, 2, 0, 0, 0, 0, 0, 0],
  Treble: [0, 0, 0, 0, 0, 1, 3, 5, 6, 7],
  Soft: [2, 1, 0, -1, -1, 0, 1, 2, 3, 3],
};
const MODES = [["list", "➡ List"], ["loop", "🔁 Loop"], ["one", "🔂 One"], ["shuffle", "🔀 Shuffle"]];

const store = {
  get(key, fallback) { try { const v = localStorage.getItem(key); return v === null ? fallback : JSON.parse(v); } catch (e) { return fallback; } },
  set(key, value) { try { localStorage.setItem(key, JSON.stringify(value)); } catch (e) { /* storage unavailable */ } },
};
const state = {
  tracks: [], byId: {}, view: [], lib: "all", playlist: null,
  playlists: store.get("pk.playlists", {}), queue: [], qi: -1, mode: store.get("pk.mode", "list"),
  current: null, lyrics: { lines: [] }, lineIndex: -1, offset: 0, config: {},
};
const audio = { ctx: null, worklet: null, eq: [], bass: null, dry: null, wet: null, convolver: null, reverbKind: "", analyser: null };
const channel = "BroadcastChannel" in window ? new BroadcastChannel("polykara-lyrics") : null;
let renderer;

// ---------- helpers ----------
async function api(path, options = {}) {
  const response = await fetch(path, options);
  const data = await response.json().catch(() => ({}));
  if (!response.ok) throw new Error(data.error || response.statusText);
  return data;
}
function esc(value) { return String(value ?? "").replace(/[&<>"]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" }[c])); }
function clock(seconds) {
  seconds = Math.max(0, Math.floor(seconds || 0));
  return `${Math.floor(seconds / 60)}:${String(seconds % 60).padStart(2, "0")}`;
}
function status(text) { $("status").textContent = text; }
function lyricTime() { return media.currentTime * 1000 - state.offset; }

// ---------- library ----------
async function loadLibrary(refresh = false) {
  status(refresh ? "Scanning…" : "Loading…");
  state.tracks = await api(`/api/library${refresh ? "?refresh=1" : ""}`);
  state.tracks.sort((a, b) => (a.artist || "").localeCompare(b.artist || "") || a.title.localeCompare(b.title));
  state.byId = Object.fromEntries(state.tracks.map((t) => [t.id, t]));
  status(`${state.tracks.length} tracks`);
  renderTracks();
}
function viewTracks() {
  let list;
  if (state.playlist) list = (state.playlists[state.playlist] || []).map((id) => state.byId[id]).filter(Boolean);
  else if (state.lib === "queue") list = state.queue.map((id) => state.byId[id]).filter(Boolean);
  else list = state.tracks.filter((t) => state.lib === "all" || (state.lib === "lyrics" && t.lyrics) || (state.lib === "video" && t.video));
  const q = $("search").value.trim().toLowerCase();
  return q ? list.filter((t) => `${t.title} ${t.artist} ${t.album}`.toLowerCase().includes(q)) : list;
}
function renderTracks() {
  state.view = viewTracks();
  $("tracks").innerHTML = state.view.map((t, i) => `<tr data-id="${t.id}" class="${state.current && state.current.id === t.id ? "playing" : ""}">
    <td class="muted">${i + 1}</td><td title="${esc(t.path)}">${t.video ? "🎬 " : ""}${esc(t.title)}</td><td>${esc(t.artist)}</td><td>${esc(t.album)}</td>
    <td class="muted">${clock(t.duration)}</td><td>${t.lyrics ? "✓" : ""}</td>
    <td class="act"><button data-act="add" title="Add to playlist">＋</button>${state.playlist ? '<button data-act="remove" title="Remove from playlist">−</button>' : ""}</td></tr>`).join("")
    || `<tr><td colspan="7" class="muted">No tracks. Add folders to [player] library in polykara.toml, then Rescan.</td></tr>`;
  $("playlists").innerHTML = Object.keys(state.playlists).map((name) => `<li data-pl="${esc(name)}" class="${state.playlist === name ? "on" : ""}"><span>${esc(name)} <span class="muted">${state.playlists[name].length}</span></span><span class="x" data-del="${esc(name)}">✕</span></li>`).join("");
  document.querySelectorAll("#libs li").forEach((li) => li.classList.toggle("on", !state.playlist && li.dataset.lib === state.lib));
}
function savePlaylists() { store.set("pk.playlists", state.playlists); renderTracks(); }

// ---------- playback ----------
async function ensureAudio() {
  if (audio.ctx) { if (audio.ctx.state === "suspended") await audio.ctx.resume(); return; }
  const ctx = new (window.AudioContext || window.webkitAudioContext)();
  audio.ctx = ctx;
  const source = ctx.createMediaElementSource(media);
  let head = source;
  try {
    await ctx.audioWorklet.addModule("/static/pitch-worklet.js");
    audio.worklet = new AudioWorkletNode(ctx, "karaoke-processor", { outputChannelCount: [2] });
    source.connect(audio.worklet);
    head = audio.worklet;
  } catch (error) {
    status("Key shift and vocal removal unavailable in this browser");
  }
  audio.eq = EQ_BANDS.map((frequency, i) => {
    const filter = ctx.createBiquadFilter();
    filter.type = i === 0 ? "lowshelf" : i === EQ_BANDS.length - 1 ? "highshelf" : "peaking";
    filter.frequency.value = frequency;
    filter.Q.value = 1.1;
    head.connect(filter);
    head = filter;
    return filter;
  });
  audio.bass = ctx.createBiquadFilter();
  audio.bass.type = "lowshelf"; audio.bass.frequency.value = 90;
  head.connect(audio.bass);
  audio.dry = ctx.createGain(); audio.wet = ctx.createGain(); audio.convolver = ctx.createConvolver();
  audio.analyser = ctx.createAnalyser(); audio.analyser.fftSize = 512;
  audio.bass.connect(audio.dry); audio.bass.connect(audio.convolver); audio.convolver.connect(audio.wet);
  audio.dry.connect(audio.analyser); audio.wet.connect(audio.analyser); audio.analyser.connect(ctx.destination);
  applySound();
}
function impulse(kind) {
  const ctx = audio.ctx, rate = ctx.sampleRate;
  const seconds = { room: 0.7, hall: 2.6, stadium: 4.5, ktv: 1.6 }[kind] || 1;
  const length = Math.round(rate * seconds), buffer = ctx.createBuffer(2, length, rate);
  for (let c = 0; c < 2; c++) {
    const data = buffer.getChannelData(c);
    for (let i = 0; i < length; i++) {
      const t = i / length;
      data[i] = kind === "ktv" ? 0 : (Math.random() * 2 - 1) * Math.pow(1 - t, kind === "room" ? 4 : 2.5);
    }
    if (kind === "ktv") {
      // discrete slap-back echoes, the classic KTV microphone sound
      for (let k = 1; k <= 8; k++) {
        const at = Math.round(rate * (0.11 * k + (c ? 0.007 : 0)));
        if (at < length) for (let j = 0; j < 300; j++) data[at + j] = (Math.random() * 2 - 1) * Math.pow(0.55, k) * (1 - j / 300);
      }
    }
  }
  return buffer;
}
function applySound() {
  if (!audio.ctx) return;
  const settings = sound();
  audio.eq.forEach((filter, i) => { filter.gain.value = settings.eq[i]; });
  audio.bass.gain.value = settings.bass;
  const wet = settings.reverb === "off" ? 0 : settings.wet / 100;
  if (settings.reverb !== "off" && audio.reverbKind !== settings.reverb) {
    audio.convolver.buffer = impulse(settings.reverb);
    audio.reverbKind = settings.reverb;
  }
  audio.wet.gain.value = wet;
  audio.dry.gain.value = 1 - wet * 0.4;
  if (audio.worklet) {
    audio.worklet.parameters.get("ratio").value = Math.pow(2, settings.key / 12);
    audio.worklet.parameters.get("vocalCut").value = settings.vocalCut ? 1 : 0;
  }
  media.preservesPitch = true;
  media.playbackRate = settings.tempo / 100;
}
function sound() {
  return {
    eq: EQ_BANDS.map((_, i) => Number($(`eq-${i}`).value)), bass: Number($("bass").value), reverb: $("reverb").value,
    wet: Number($("wet").value), key: Number($("key").textContent), vocalCut: $("vocal-cut").checked, tempo: Number($("tempo").value),
  };
}
async function playTrack(id, queue = null) {
  if (queue) { state.queue = queue; state.qi = queue.indexOf(id); }
  else if (!state.queue.includes(id)) { state.queue.push(id); state.qi = state.queue.length - 1; }
  else state.qi = state.queue.indexOf(id);
  const track = state.byId[id];
  if (!track) return;
  state.current = track;
  await ensureAudio();
  media.src = `/media/track/${id}`;
  media.classList.toggle("audio-only", !track.video);
  $("ktv-overlay").checked = !track.burned;
  applySound();
  media.play().catch((error) => status(`Cannot play: ${error.message}`));
  $("now-title").textContent = track.title;
  $("now-artist").textContent = [track.artist, track.album].filter(Boolean).join(" · ");
  document.title = `${track.title} — PolyKara Player`;
  state.offset = store.get(`pk.offset.${id}`, 0);
  showOffset();
  if ("mediaSession" in navigator) {
    navigator.mediaSession.metadata = new MediaMetadata({ title: track.title, artist: track.artist, album: track.album });
  }
  renderTracks();
  loadLyrics(track, true);
}
function nextIndex(direction) {
  const n = state.queue.length;
  if (!n) return -1;
  if (state.mode === "shuffle" && n > 1) {
    let pick;
    do { pick = Math.floor(Math.random() * n); } while (pick === state.qi);
    return pick;
  }
  const i = state.qi + direction;
  if (i >= 0 && i < n) return i;
  return state.mode === "loop" || direction < 0 ? (i + n) % n : -1;
}
function skip(direction) {
  const i = nextIndex(direction);
  if (i >= 0) playTrack(state.queue[i]);
}
function togglePlay() {
  if (!media.src) { if (state.view.length) playTrack(state.view[0].id, state.view.map((t) => t.id)); return; }
  if (media.paused) { ensureAudio().then(() => media.play()); } else media.pause();
}

// ---------- lyrics ----------
async function loadLyrics(track, online) {
  $("ly-source").textContent = "loading lyrics…";
  try {
    state.lyrics = await api(`/api/track/${track.id}/lyrics?online=${online ? 1 : 0}`);
  } catch (error) {
    state.lyrics = { lines: [], source: "error", error: error.message };
  }
  if (state.current !== track) return;
  if (state.lyrics.lines.length && !track.lyrics) { track.lyrics = true; renderTracks(); }
  const label = { sidecar: `from ${state.lyrics.file}`, lrclib: "from LRCLIB", none: "no lyrics found", error: state.lyrics.error || "error" }[state.lyrics.source] || "";
  $("ly-source").textContent = label;
  renderLyrics();
}
function renderLyrics() {
  const lines = state.lyrics.lines || [];
  const showSub = $("ly-sub").checked;
  $("lyric-view").innerHTML = lines.length ? lines.map((line, i) => {
    const body = line.words && line.words.length
      ? line.words.map((w, k) => `${k ? separator(line.words[k - 1][2], w[2]) : ""}<span class="w">${esc(w[2])}</span>`).join("")
      : esc(line.text.replace(/^\[[^\]]+\]\s*/, ""));
    const sub = showSub ? [line.translation, line.roman].filter(Boolean).map((s) => `<span class="sub">${esc(s)}</span>`).join("") : "";
    return `<p class="ly" data-i="${i}">${body}${sub}</p>`;
  }).join("") : `<p class="muted">No lyrics. Load a lyric file or search online.</p>`;
  state.lineIndex = -1;
  renderer.setLines(lines.map((l) => ({ ...l, text: l.text.replace(/^\[[^\]]+\]\s*/, ""), roman: showSub ? l.translation || l.roman : "" })));
  broadcastLines();
}
function updateLyrics() {
  const lines = state.lyrics.lines || [];
  if (!lines.length) return;
  const t = lyricTime();
  const index = activeLine(lines, t);
  const view = $("lyric-view");
  if (index !== state.lineIndex) {
    view.querySelectorAll(".ly.cur").forEach((p) => p.classList.remove("cur"));
    const p = view.children[index];
    if (p) {
      p.classList.add("cur");
      const box = $("tab-lyrics");
      box.scrollTo({ top: p.offsetTop - box.clientHeight / 2 + p.clientHeight / 2, behavior: "smooth" });
    }
    state.lineIndex = index;
  }
  const line = lines[index], p = view.children[index];
  if (line && p && line.words && line.words.length) {
    p.querySelectorAll(".w").forEach((span, k) => {
      span.style.setProperty("--p", `${(wordProgress(line.words[k], t) * 100).toFixed(1)}%`);
    });
  }
}
function showOffset() { $("ly-offset").textContent = `offset ${(state.offset / 1000).toFixed(1)}s`; }
function shiftOffset(delta) {
  state.offset += delta;
  if (state.current) store.set(`pk.offset.${state.current.id}`, state.offset);
  showOffset();
  broadcastTime();
}

// ---------- desktop lyric window ----------
function broadcastLines() {
  if (!channel) return;
  const showSub = $("ly-sub").checked;
  channel.postMessage({ type: "lines", title: state.current ? state.current.title : "", lines: (state.lyrics.lines || []).map((l) => ({ start: l.start, end: l.end, text: l.text.replace(/^\[[^\]]+\]\s*/, ""), words: l.words, roman: showSub ? l.translation || l.roman : "" })), config: state.config });
}
function broadcastTime() {
  if (channel) channel.postMessage({ type: "time", t: lyricTime(), rate: media.playbackRate, playing: !media.paused, sent: Date.now() });
}
if (channel) channel.onmessage = (event) => { if (event.data && event.data.type === "hello") { broadcastLines(); broadcastTime(); } };

// ---------- visualizer + frame loop ----------
const viz = $("viz"), vctx = viz.getContext("2d");
let bins = null;
function drawViz() {
  if (!audio.analyser || $("tab-ktv").hidden) return;
  const w = (viz.width = viz.clientWidth), h = (viz.height = viz.clientHeight);
  bins = bins || new Uint8Array(audio.analyser.frequencyBinCount);
  audio.analyser.getByteFrequencyData(bins);
  const count = 64, bw = w / count;
  for (let i = 0; i < count; i++) {
    const v = bins[Math.floor((i / count) ** 1.6 * bins.length)] / 255;
    vctx.fillStyle = `hsl(${200 + i * 2}, 90%, ${45 + v * 20}%)`;
    vctx.fillRect(i * bw + 1, h - v * h, bw - 2, v * h);
  }
}
let lastBroadcast = 0;
function frame(stamp) {
  if (media.duration) {
    if (!state.seeking) $("seek").value = Math.round((media.currentTime / media.duration) * 1000);
    $("t-now").textContent = clock(media.currentTime);
    $("t-total").textContent = clock(media.duration);
  }
  if (!$("tab-lyrics").hidden) updateLyrics();
  if (!$("tab-ktv").hidden) {
    if ($("ktv-overlay").checked) renderer.draw(lyricTime(), { background: media.classList.contains("audio-only") ? "#07080c" : null });
    else renderer.draw(-1e9, { background: media.classList.contains("audio-only") ? "#07080c" : null });
  }
  drawViz();
  if (stamp - lastBroadcast > 250) { broadcastTime(); lastBroadcast = stamp; }
  requestAnimationFrame(frame);
}

// ---------- wiring ----------
function setupSoundPanel() {
  $("eq").innerHTML = EQ_BANDS.map((f, i) => `<label><input id="eq-${i}" type="range" min="-12" max="12" step="0.5" value="0">${f >= 1000 ? `${f / 1000}k` : f}</label>`).join("");
  $("eq-preset").innerHTML = Object.keys(EQ_PRESETS).map((name) => `<option>${name}</option>`).join("");
  const saved = store.get("pk.sound", null);
  if (saved) {
    saved.eq.forEach((v, i) => { $(`eq-${i}`).value = v; });
    $("bass").value = saved.bass; $("reverb").value = saved.reverb; $("wet").value = saved.wet;
    $("key").textContent = saved.key; $("vocal-cut").checked = saved.vocalCut; $("tempo").value = saved.tempo;
    $("eq-preset").value = saved.preset || "Flat";
  }
  $("tempo-v").textContent = `${(Number($("tempo").value) / 100).toFixed(2)}×`;
  const changed = () => {
    $("tempo-v").textContent = `${(Number($("tempo").value) / 100).toFixed(2)}×`;
    store.set("pk.sound", { ...sound(), preset: $("eq-preset").value });
    applySound();
  };
  $("fx").addEventListener("input", changed);
  $("eq-preset").addEventListener("change", () => { EQ_PRESETS[$("eq-preset").value].forEach((v, i) => { $(`eq-${i}`).value = v; }); changed(); });
  $("key-up").addEventListener("click", () => { $("key").textContent = Math.min(12, Number($("key").textContent) + 1); changed(); });
  $("key-down").addEventListener("click", () => { $("key").textContent = Math.max(-12, Number($("key").textContent) - 1); changed(); });
  $("fx-btn").addEventListener("click", () => { $("fx").hidden = !$("fx").hidden; });
  $("fx-close").addEventListener("click", () => { $("fx").hidden = true; });
}
function setTab(name) {
  document.querySelectorAll(".tabs button").forEach((b) => b.classList.toggle("on", b.dataset.tab === name));
  for (const tab of ["songs", "lyrics", "ktv"]) $(`tab-${tab}`).hidden = tab !== name;
  state.lineIndex = -1;
  store.set("pk.tab", name);
}
document.querySelectorAll(".tabs button").forEach((b) => b.addEventListener("click", () => setTab(b.dataset.tab)));
$("tracks").addEventListener("dblclick", (e) => {
  const tr = e.target.closest("tr[data-id]");
  if (tr) playTrack(tr.dataset.id, state.view.map((t) => t.id));
});
$("tracks").addEventListener("click", (e) => {
  const button = e.target.closest("button[data-act]"), tr = e.target.closest("tr[data-id]");
  if (!button || !tr) return;
  const id = tr.dataset.id;
  if (button.dataset.act === "remove" && state.playlist) {
    state.playlists[state.playlist] = state.playlists[state.playlist].filter((x) => x !== id);
    return savePlaylists();
  }
  const names = Object.keys(state.playlists);
  const name = (window.prompt(`Add to playlist${names.length ? ` (${names.join(", ")})` : ""}:`, state.playlist || names[0] || "Favourites") || "").trim();
  if (!name) return;
  state.playlists[name] = state.playlists[name] || [];
  if (!state.playlists[name].includes(id)) state.playlists[name].push(id);
  savePlaylists();
});
$("libs").addEventListener("click", (e) => { const li = e.target.closest("li"); if (li) { state.lib = li.dataset.lib; state.playlist = null; renderTracks(); setTab("songs"); } });
$("playlists").addEventListener("click", (e) => {
  const del = e.target.closest("[data-del]");
  if (del) {
    if (window.confirm(`Delete playlist “${del.dataset.del}”?`)) { delete state.playlists[del.dataset.del]; if (state.playlist === del.dataset.del) state.playlist = null; savePlaylists(); }
    return;
  }
  const li = e.target.closest("li");
  if (li) { state.playlist = li.dataset.pl; renderTracks(); setTab("songs"); }
});
$("new-pl").addEventListener("click", () => {
  const name = (window.prompt("Playlist name:") || "").trim();
  if (name && !state.playlists[name]) { state.playlists[name] = []; savePlaylists(); }
});
$("search").addEventListener("input", renderTracks);
$("rescan").addEventListener("click", () => loadLibrary(true).catch((e) => status(e.message)));
$("play").addEventListener("click", togglePlay);
$("prev").addEventListener("click", () => (media.currentTime > 3 ? (media.currentTime = 0) : skip(-1)));
$("next").addEventListener("click", () => skip(1));
$("mode").addEventListener("click", () => {
  const i = (MODES.findIndex(([m]) => m === state.mode) + 1) % MODES.length;
  state.mode = MODES[i][0]; store.set("pk.mode", state.mode); $("mode").textContent = MODES[i][1];
});
$("mode").textContent = (MODES.find(([m]) => m === state.mode) || MODES[0])[1];
$("seek").addEventListener("input", () => { state.seeking = true; });
$("seek").addEventListener("change", () => { if (media.duration) media.currentTime = (Number($("seek").value) / 1000) * media.duration; state.seeking = false; });
$("volume").addEventListener("input", () => { media.volume = Number($("volume").value) / 100; store.set("pk.volume", media.volume); });
media.volume = store.get("pk.volume", 0.8); $("volume").value = Math.round(media.volume * 100);
media.addEventListener("play", () => { $("play").textContent = "❚❚"; broadcastTime(); });
media.addEventListener("pause", () => { $("play").textContent = "▶"; broadcastTime(); });
media.addEventListener("seeked", broadcastTime);
media.addEventListener("ended", () => { if (state.mode === "one") { media.currentTime = 0; media.play(); } else skip(1); });
$("ly-minus").addEventListener("click", () => shiftOffset(-500));
$("ly-plus").addEventListener("click", () => shiftOffset(500));
$("ly-size").addEventListener("input", () => { document.documentElement.style.setProperty("--lyric-size", `${$("ly-size").value}px`); store.set("pk.lysize", $("ly-size").value); });
$("ly-size").value = store.get("pk.lysize", 22); document.documentElement.style.setProperty("--lyric-size", `${$("ly-size").value}px`);
$("ly-sub").addEventListener("change", renderLyrics);
$("ly-search").addEventListener("click", () => state.current && loadLyrics(state.current, true));
$("ly-studio").addEventListener("click", () => {
  if (!state.current) return;
  const stem = state.current.path.split(/[\\/]/).pop().replace(/\.[^.]+$/, "");
  window.open(`/studio?song=${encodeURIComponent(stem)}`, "_blank");
});
$("ly-file").addEventListener("change", async () => {
  const file = $("ly-file").files[0];
  if (!file || !state.current) return;
  const ext = (file.name.match(/\.[^.]+$/) || [".lrc"])[0];
  try {
    state.lyrics = await api(`/api/track/${state.current.id}/lyrics`, { method: "POST", headers: { "X-Lyric-Format": ext }, body: await file.arrayBuffer() });
    $("ly-source").textContent = `from ${file.name}`;
    renderLyrics();
  } catch (error) { $("ly-source").textContent = error.message; }
  $("ly-file").value = "";
});
$("desk").addEventListener("click", () => {
  window.open("/lyrics", "polykara-desktop-lyric", "width=1000,height=190,menubar=no,toolbar=no,location=no,status=no");
  setTimeout(() => { broadcastLines(); broadcastTime(); }, 800);
});
document.addEventListener("keydown", (e) => {
  if (["INPUT", "TEXTAREA", "SELECT"].includes(e.target.tagName) && e.target.type !== "range" && e.target.type !== "checkbox") return;
  if (e.key === " ") { e.preventDefault(); togglePlay(); }
  else if (e.key === "ArrowRight") media.currentTime += 5;
  else if (e.key === "ArrowLeft") media.currentTime = Math.max(0, media.currentTime - 5);
  else if (e.key === "ArrowUp") { e.preventDefault(); media.volume = Math.min(1, media.volume + 0.05); $("volume").value = Math.round(media.volume * 100); }
  else if (e.key === "ArrowDown") { e.preventDefault(); media.volume = Math.max(0, media.volume - 0.05); $("volume").value = Math.round(media.volume * 100); }
  else if (e.key.toLowerCase() === "n") skip(1);
  else if (e.key.toLowerCase() === "b") skip(-1);
});
if ("mediaSession" in navigator) {
  navigator.mediaSession.setActionHandler("play", togglePlay);
  navigator.mediaSession.setActionHandler("pause", togglePlay);
  navigator.mediaSession.setActionHandler("nexttrack", () => skip(1));
  navigator.mediaSession.setActionHandler("previoustrack", () => skip(-1));
}

(async function init() {
  setupSoundPanel();
  state.config = await api("/api/config").catch(() => ({}));
  const style = styleFromConfig(state.config);
  style.mode = "alternate";
  renderer = new KaraokeRenderer($("ktv-canvas"), style);
  setTab(store.get("pk.tab", "songs"));
  await loadLibrary().catch((e) => status(e.message));
  requestAnimationFrame(frame);
})();
