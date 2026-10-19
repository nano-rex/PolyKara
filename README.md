# PolyKara

Multilingual karaoke subtitle automation with an editable ASS workflow.

Documentation: [简体中文](README.zh-CN.md)

Presentation and chart defaults are configured in [`polykara.toml`](polykara.toml). It controls fonts, sizes, colors, title/credit placement, watermarking, video resolution, and enabled chart regions.

PolyKara separates repeatable CLI processing from final GUI adjustments. It produces editable ASS subtitles, supports word-level karaoke highlighting, and renders final MP4 files with FFmpeg.

## Quick start

Edit `songs.csv`, then run the complete pipeline:

```bash
python3 pipeline.py check
python3 pipeline.py process
```

The end-to-end command downloads media and subtitles, retrieves external lyrics, normalizes audio, creates word-level timing, generates ASS, and renders MP4. Existing outputs are skipped in non-interactive runs; use `--reprocess` to render them again.

```bash
python3 pipeline.py process --reprocess
```

## Regional YouTube music charts

Add the top 10 weekly YouTube songs from every configured region:

```bash
python3 pipeline.py download --trending
```

Supported regions include Malaysia, Indonesia, Australia, Canada, Singapore, Taiwan, Hong Kong, Japan, South Korea, India, China, USA, and UK. Restrict the regions with codes:

```bash
python3 pipeline.py download --trending --regions my,id,au,ca,sg,jp,kr,cn
```

The default adds all chart candidates to `songs.csv`, deduplicating URLs and IDs. Use `--pick` for interactive selection:

```bash
python3 pipeline.py download --trending --pick
```

Adding chart entries does not download media. Run `python3 pipeline.py download` afterward, or run `process`.

## Alignment and lyrics

Install the alignment dependencies:

```bash
pip install -r requirements-align.txt
```

WhisperX is preferred. Faster-Whisper is used as a lighter CPU/ARM fallback. The automatic model selector reserves a fixed 2 GiB by default: an 8 GiB system receives a 6 GiB model budget. Override the thresholds or `align_model` in `polykara.toml`/`songs.csv` when needed.

Lyrics priority is:

1. Downloaded YouTube subtitles
2. Validated synchronized lyrics from configured external providers
3. `lyrics_file` in `songs.csv`

External sources include LRCLIB, Lyrics.ovh, and configured public lyric webpages. Webpage extraction reads known lyric containers and does not bypass login, paywalls, CAPTCHAs, or anti-bot controls. Conflicting lyrics are rejected for manual review.

Configure provider order centrally:

```toml
[lyrics]
sources = ["lrclib", "lyrics.ovh", "webpage"]
```

An individual `lyric_sources` value in `songs.csv` overrides the global list for that song. `lyric_pages` remains per-song because webpage URLs identify the specific track.

Enhanced LRC supports word or syllable marks:

```text
[00:12.00]<00:12.00>ありがとう<00:12.80>ございます
```

Songs without word-level timing are skipped rather than rendered with false karaoke highlighting.

### Word-timed karaoke formats

`lyrics_file` also accepts KTV and music-player lyric formats that already carry word timing:

| Extension | Format | Example line |
|---|---|---|
| `.ksc` | KTV karaoke script | `karaoke.add('00:12.000', '00:13.000', '你好[Hi]', '300,400,300');` |
| `.krc` | Word-offset lyrics | `[12000,1000]<0,300,0>你<300,400,0>好` |
| `.qrc` | Word-timestamp lyrics, plain or in XML `LyricContent` | `[12000,1000]你(12000,300)好(12300,400)` |

Only plain-text files are read; binary or encrypted downloads are rejected with a message. UTF-8, UTF-16, and GB18030 encodings are detected. When every line of a lyric file has word timing (these formats, or a fully marked enhanced LRC), PolyKara uses it directly and skips WhisperX alignment.

### Romanized sing-along text

For non-Latin lyrics, PolyKara can add a romanized line above the original lyric. Install the optional engines with:

```bash
pip install -r requirements-romanization.txt
```

The default configuration supports Mandarin pinyin, Cantonese Jyutping, Japanese Hepburn romaji, Korean romanization, and Indic scripts including Tamil. Set `romanization.enabled = false`, remove a language from `romanization.languages`, or change `romanization.tone` to `"marks"` for tone marks such as `nǐ hǎo` instead of `ni3 hao3`.

Romanization is generated from the lyric text and displayed above the timed original line. The original line remains the authoritative karaoke timing; if an optional engine is not installed, PolyKara keeps the original lyric rather than failing the render.

## Karaoke presentation

Edit `polykara.toml` to change the lyric font, size, colors, title card, credits, or watermark. For example:

```toml
[lyric]
font = "Noto Sans CJK SC"
size = 60
color = "&H00FF0000"

[alignment]
model = "auto"
device = "auto"
compute_type = "int8"
reserve_memory_gib = 2
small_min_budget_gib = 4
cpu_threads = 0

[title]
text = "{title} — {artist}"
horizontal = "center"
vertical = "middle"
duration_ms = 5000
fade_in_ms = 500
fade_out_ms = 500

[credit]
text = "Edited by {producer}"
horizontal = "center"
vertical = "bottom"

[watermark]
enabled = true
text = "My Karaoke Studio"
alignment = 9

[charts]
enabled_regions = ["my", "sg", "jp", "us"]

[video]
extend_intro = true
```

When `extend_intro` is enabled, the renderer adds a frozen opening frame and matching silence if the title/credit cards would overlap the first lyric. Lyric timing is shifted by the same amount, so the title card can fade into the actual video without subtitle collision.

Alignment resource policy, pause thresholds, and the singer limit are also configurable in this file. `reserve_memory_gib = 2` reserves a fixed 2 GiB for the system, so an 8 GiB machine receives a 6 GiB model-selection budget. This is a selection policy, not a hard operating-system memory limit.

For vocal pauses of at least 30 seconds, the final three seconds display `.`, `..`, and `...`, one state per second, above and left-aligned with the upcoming lyric. Normal gaps do not display dots.

Unmarked lyrics are blue by default. Up to 16 singers can be labeled:

```text
[00:12.00][singer:A]First singer line
[00:18.00][singer:B]Second singer line
[00:24.00][singer:duet]Together line
```

Singer colors are assigned consistently by first appearance while preserving word-level highlighting.

### KTV two-line layout and effects

Set `layout.mode = "alternate"` for the classic KTV look: lines alternate between a top-left and a bottom-right row, and each line appears `lead_in_ms` before it is sung (never before its row is free).

`[effects]` adds per-syllable animation on top of the `\kf` highlight. Built-in presets are `fade`, `pop`, and `glow`; tune them with `[effects.params]`, or write your own ASS tags:

```toml
[effects]
preset = "pop"
[effects.params]
scale = 130
```

```toml
[effects]
syllable = '\fscx100\fscy100\t({start},{mid},\fscx{scale}\fscy{scale})\t({mid},{end},\fscx100\fscy100)'
[effects.params]
scale = 140
```

`{start}`, `{mid}`, `{end}`, `{dur}`, and `{index}` are milliseconds relative to the line event, and every `params` key is available as a placeholder.

## Live output

Play the karaoke in real time instead of rendering an MP4:

```bash
sudo modprobe v4l2loopback video_nr=10 card_label=PolyKara exclusive_caps=1
python3 pipeline.py live --song amazing-grace-vocal
python3 pipeline.py live --target udp://127.0.0.1:5000
python3 pipeline.py live --target rtmp://live.example.com/app/STREAM_KEY
```

A `/dev/videoN` target appears as a webcam in OBS, browsers, and video-call apps (video only; set `live.audio_output = "pulse"` or `"alsa"` to play the audio locally at the same time). Stream URLs carry H.264 video and AAC audio.

## Karaoke Studio (timing editor)

A browser-based word-timing editor, served locally (no extra dependencies):

```bash
python3 pipeline.py studio            # opens http://127.0.0.1:8765/
```

Pick a song from `songs.csv`. The studio loads its saved project, the edited/automatic ASS, or `lyrics_file`, in that order. Then:

- **Tap timing** (`T`): play the song and hold `Space`, `F`, or `J` for each unit while it is sung. A unit is one CJK character or one Latin word. Alternate `F`/`J` for legato syllables, and use `Backspace` to step back.
- **Timeline**: shows the waveform and word blocks. Drag word edges or bodies to adjust them, `Alt`+drag moves a whole line, `Ctrl`+wheel zooms.
- **Line tools**: import plain text, add/delete/merge/split lines, spread timing evenly, shift everything by an offset, undo/redo.
- **Preview**: live karaoke overlay on the video, in single or KTV two-line layout, with the effect presets.
- **Save** (`Ctrl+S`): writes `work/project/<id>.json` and `work/ass/<id>.edited.ass`, which `render` and `live` use. **Export** downloads ASS, enhanced LRC, KSC, KRC, or SRT; **Render MP4** runs `render --song <id>`.

## Music player

A local music and karaoke player with KTV features:

```bash
python3 pipeline.py player            # opens http://127.0.0.1:8765/player
```

- Scans the folders in `[player] library` (default `~/Music` and `work/output`) for audio and video.
- Lyrics come from a sidecar file with the same name (`.krc`, `.qrc`, `.ksc`, `.lrc`, `.ass`, `.srt`, `.vtt`), a file you load in the Lyrics tab, or LRCLIB.
  - Two LRC lines with the same timestamp are shown as original plus translation.
  - Romanization appears when the optional engines are installed.
  - Lyric files you load or fetch are stored in `work/player-lyrics/`; your music folders are never written to.
- **Views**: scrolling lyrics with word-by-word highlight and a per-track offset; a KTV two-line overlay on the video; and a desktop lyric pop-up window.
- **Sound**: 10-band equalizer with presets, bass boost, key shift (±12 semitones, tempo unchanged), tempo (pitch unchanged), reverb (room, hall, stadium, KTV echo), and a karaoke mode that removes centre-panned vocals.
- **Library**: playlists, play queue, list/loop/one/shuffle modes, search, and media keys.

Both pages listen on `127.0.0.1` only. Use `--port` to change the port and `--no-browser` to skip opening a browser.

## GUI editing and maintenance

Create editable copies in Aegisub or Subtitle Edit:

```bash
python3 pipeline.py edit
```

Check processing state:

```bash
python3 pipeline.py qa
```

Clean reproducible FLAC intermediates:

```bash
python3 pipeline.py cleanup --dry-run
python3 pipeline.py cleanup
```

Use `cleanup --drop-source` only after confirming that final MP4 files and edited ASS files exist.

## Project structure

```text
pipeline.py             CLI compatibility entry point
polykara/config.py      paths, constants, command helpers
polykara/manifest.py    manifest and tool validation
polykara/providers.py   external lyrics and webpage providers
polykara/charts.py      regional YouTube chart picker
polykara/alignment.py   WhisperX/Faster-Whisper alignment
polykara/subtitle.py    LRC/SRT/VTT parsing and timing application
polykara/ass.py         ASS styles, karaoke tags, and pause cues
polykara/lyricformats.py KSC/KRC/QRC word-timed lyric readers
polykara/project.py     studio projects, ASS read-back, LRC/KSC/KRC/SRT export
polykara/library.py     player library scan and lyric lookup
polykara/server.py      local web server for the studio and player
polykara/web/           studio, player, and desktop-lyric pages
tests/                  python3 -m unittest discover -s tests -t .  and  node --test tests/js/test_web.js
```

Respect the terms of service and copyright requirements of every media and lyrics source.
