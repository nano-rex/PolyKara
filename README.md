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

Enhanced LRC supports word or syllable marks:

```text
[00:12.00]<00:12.00>ありがとう<00:12.80>ございます
```

Songs without word-level timing are skipped rather than rendered with false karaoke highlighting.

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
```

Respect the terms of service and copyright requirements of every media and lyrics source.
