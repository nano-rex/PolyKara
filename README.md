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

The end-to-end command downloads media and subtitles, retrieves external lyrics, normalizes audio, creates word-level timing, generates ASS, and renders MP4.

Finished work is reused: a song whose MP4 already exists is skipped, downloaded media and existing word timing are not fetched or computed again, and a song is rendered again automatically when its `.edited.ass` is newer than its MP4. Use `--reprocess` to rebuild the ASS and MP4 anyway, and add `--realign` to repeat word alignment as well.

```bash
python3 pipeline.py process --reprocess
python3 pipeline.py process --reprocess --realign
```

The download step checks media and lyrics separately and only fetches what is missing. A video that is already in `work/raw` and playable (checked locally with `ffprobe`) is not downloaded again, and a file that is not playable or has no audio track, such as an interrupted download or a failed merge, is removed and downloaded again. Lyrics are collected until the song has at least three usable lyric files (see the accuracy check below); a song that already has them makes no subtitle or lyric-provider request. When every source has been tried, that is remembered so the same lookups are not repeated on every run; `python3 pipeline.py download --reprocess` tries them again.

### One song failing never stops the batch

Every step runs song by song. When a step fails for one song (a download error, a missing lyrics file, an alignment or FFmpeg error), PolyKara prints `SKIP <id>: <step> failed (<reason>)`, leaves that song out of the remaining steps, and continues with the next song. A summary at the end lists each skipped song with its reason, and the command exits with status 1 so scripts can notice. Fix the cause and rerun the same command; only the unfinished songs are processed.

A song for which no lyrics can be found is set aside the same way, before its video is downloaded: lyrics are looked up first, and without any lyric file the song is skipped and listed at the end as something to complete later. The later steps behave the same when run on their own: `normalize`, `align`, `lyrics`, and `render` skip any song that has a video but no lyric file and continue with the next one. This is not counted as an error. Add a `lyrics_file` or `lyric_pages` for it and rerun.

Only problems that affect every song stop the run: a missing `yt-dlp`/`ffmpeg`, or an invalid `songs.csv` or `polykara.toml`.

### Working on selected songs

Every command accepts song ids, either after the command or with `--only`:

```bash
python3 pipeline.py process my-song another-song
python3 pipeline.py render --reprocess --only my-song
python3 pipeline.py edit my-song
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

### Lyric files and the accuracy check

Every source is saved to its own file so the files can be compared with each other:

| Trust order | Source | File |
| --- | --- | --- |
| 1 | `lyrics_file` in `songs.csv` (`.lrc`, `.srt`, `.vtt`, or plain `.txt`) | the file you name |
| 2 | Creator-made YouTube subtitles | `work/subtitles/<id>.<lang>.vtt` |
| 3 | LRCLIB synced lyrics | `work/lyrics/<id>.lrclib.lrc` |
| 4 | NetEase Cloud Music synced lyrics | `work/lyrics/<id>.netease.lrc` |
| 5 | YouTube automatic captions in the song's `language` | `work/subtitles/<id>.auto.<lang>.vtt` |
| 6 | Plain text from LRCLIB, Lyrics.ovh, and each `lyric_pages` URL | `work/lyrics/<id>.<source>.txt` |

A song needs at least three usable lyric files for a proper accuracy check. The download step keeps looking, lyric providers first and YouTube subtitles only if files are still missing, until three exist or every source has been tried. If fewer are available, add a `lyrics_file` or `lyric_pages` (several URLs separated by `|`) in `songs.csv`.

The check runs at the end of `download`, at the start of `lyrics`, and on its own with `python3 pipeline.py verify`; it only reads the lyric files, so it does not need alignment. It compares the files word by word. Lyrics are **verified** when at least three files, including the one used for rendering, agree. Without a `lyrics_file`, the most trusted timed file that the others confirm is used, so one wrong source is outvoted; a `lyrics_file` is always used, and is flagged when the other files contradict it. It prints each file's agreement and saves the result to `work/lyrics/<id>.check.json`. `qa` shows the verdict per song. After alignment, the `lyrics` step also reports how much of the lyrics was recognised in the audio.

Songs that are not verified are still produced, with a warning and an entry in the end-of-run list. Change this in `polykara.toml`:

```toml
[lyrics]
min_sources = 3
agreement_threshold = 0.6
require_verified = true   # skip unverified songs instead of warning
```

Install `zhconv` (included in `requirements-romanization.txt`) so that Traditional and Simplified Chinese versions of the same lyrics count as agreeing. When `subtitle_langs` is empty, only subtitles in the song's `language` are requested.

LRCLIB is searched with the video title cleaned of decorations such as `(Official Video)`, and the record closest to the video's length is chosen. A warning is printed when the lengths differ by more than five seconds, because the line timing is then likely offset.

External sources include LRCLIB, NetEase Cloud Music, Lyrics.ovh, and configured public lyric webpages. Webpage extraction reads known lyric containers and does not bypass login, paywalls, CAPTCHAs, or anti-bot controls.

Configure provider order centrally:

```toml
[lyrics]
sources = ["lrclib", "netease", "lyrics.ovh", "webpage"]
```

An individual `lyric_sources` value in `songs.csv` overrides the global list for that song. `lyric_pages` remains per-song because webpage URLs identify the specific track.

Enhanced LRC supports word or syllable marks:

```text
[00:12.00]<00:12.00>ありがとう<00:12.80>ございます
```

Inline marks are kept as the word timing for that line, so a fully marked file needs no alignment. `[offset:±ms]` and empty timestamp lines (`[01:23.00]` with no text, marking where the previous line ends) are honoured.

Songs without word-level timing are skipped rather than rendered with false karaoke highlighting.

### How word timing is applied

Recognised words are matched to the lyric text, not just counted, so an extra or missing recognised word does not shift the rest of the line. Words the recogniser missed are placed between their matched neighbours. Chinese, Japanese, and Korean lyrics are highlighted per character. Lines where nothing was recognised use evenly spaced timing and are listed after the `lyrics` step, so you know which lines to check in the editor.

The highlight starts when each word is actually sung: the wait before the first word and pauses between words are written into the ASS as empty `\k` syllables. `[timing]` in `polykara.toml` also controls how early a line appears (`lead_in_ms`) and when a line followed by a long instrumental leaves the screen (`max_tail_ms`, `tail_hold_ms`).

### Two-row lyrics for non-Latin scripts

Lyrics written in a non-Latin script get their reading directly above the characters it belongs to: pinyin above each Chinese character, romaji above each Japanese word, romanization above each Korean syllable, and so on. The reading and its characters are highlighted together. Characters are spaced so that a long reading such as `zhuang1` never runs into its neighbour. Lines written only in Latin letters keep a single row.

| Script | Romanization | Engine |
| --- | --- | --- |
| Chinese (Mandarin) | Pinyin, `ni3 hao3` or `nǐ hǎo` (`romanization.tone`) | `pypinyin` |
| Chinese (Cantonese, `language` = `yue` or `zh-hk`) | Jyutping | `ToJyutping` |
| Japanese | Hepburn romaji | `pykakasi` |
| Korean | Revised Romanization | built in |
| Indic scripts (Hindi, Tamil, ...) | ITRANS | `indic-transliteration` |
| Cyrillic, Greek, Thai, Arabic, Hebrew, others | ASCII transliteration | `anyascii` |

The engine is chosen from the script of each line, and the song's `language` decides how Chinese characters are read. Install the engines with:

```bash
pip install -r requirements-romanization.txt
```

If an engine is missing, the `lyrics` step prints which package to install and that line keeps a single row. Japanese readings from `pykakasi` are dictionary-based and can be wrong for names and some kanji (君 may come out as `kun` instead of `kimi`); correct them in the edited ASS.

Each line is laid out from the font widths libass will use, measured with Pillow and fontconfig when they are installed (`pip install pillow`); without them a safe estimate is used and characters are spaced a little wider. A line too long for the screen is scaled down to fit. `romanization.ruby_spacing` sets the space between characters, and `gap` the space between the reading and its characters. `layout = "rows"` shows the reading as one centred line above the original instead, which is easier to edit in Aegisub because the ruby layout writes one event per character. In that mode `margin_v = "auto"` places the row above the lyric line, or set a number yourself. Set `karaoke = false` for a non-highlighted row, `languages = ["zh", "ja"]` to limit it to some song languages, or `enabled = false` to turn it off. Use a font with CJK glyphs for these songs, for example `font = "Noto Sans CJK SC"` in `[lyric]` and `[romanization]`.

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

Add a logo to every video with `[logo]`, or per song with the `logo_file` column in `songs.csv`:

```toml
[logo]
file = "assets/logo.png"
width = 220
position = "top-right"   # top/bottom/middle + left/right/center
opacity = 0.9
```

Encoder settings live in `[render]` (`video_codec`, `preset`, `crf`, `pixel_format`, `audio_codec`, `audio_bitrate`, `extra_args`).

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

After saving changes to `work/ass/<id>.edited.ass`, run `python3 pipeline.py render <id>`; a song whose edited ASS is newer than its MP4 is rendered again without `--reprocess`. The edited file is never overwritten.

Check processing state (media, lyric source, word timing, ASS coverage, and whether the MP4 is stale):

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
polykara/verify.py      lyric file collection and accuracy check
polykara/ass.py         ASS styles, karaoke tags, and pause cues
polykara/romanize.py    optional romanization engines
tests/                  unit tests for parsing, timing, and batch handling
```

## Tests

The parsing, timing, and ASS logic has unit tests that need no media, network, or extra packages:

```bash
python3 -m unittest discover -s tests -t .
```

Respect the terms of service and copyright requirements of every media and lyrics source.
