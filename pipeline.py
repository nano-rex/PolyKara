#!/usr/bin/env python3
"""PolyKara: editable CLI pipeline for multilingual KTV source preparation and ASS rendering."""
from __future__ import annotations

import argparse
import csv
import json
import re
import shutil
import subprocess
import sys
import os
from pathlib import Path

ROOT = Path(__file__).resolve().parent
MANIFEST = ROOT / "songs.csv"
WORK = ROOT / "work"
RAW, AUDIO, SUBTITLES, ALIGN, ASS, OUTPUT, METADATA = (WORK / n for n in ("raw", "audio", "subtitles", "align", "ass", "output", "metadata"))
DOWNLOAD_ARCHIVE = WORK / "download-archive.txt"
TIMECODE = re.compile(r"\[(\d+):(\d{2})(?:[.:](\d{1,3}))?\]")
WORDCODE = re.compile(r"<(\d+):(\d{2})(?:[.:](\d{1,3}))?>")
Entry = tuple[int, int, str, list[tuple[int, int, str]]]


def run(cmd: list[str], dry_run: bool = False) -> None:
    print("$", " ".join(cmd))
    if not dry_run:
        subprocess.run(cmd, check=True)


def require_current_ytdlp() -> None:
    result = subprocess.run(["yt-dlp", "--version"], capture_output=True, text=True, check=False)
    version = result.stdout.strip()
    match = re.fullmatch(r"(\d{4})\.(\d{2})\.(\d{2})", version)
    if not match or int("".join(match.groups())) < 20250101:
        raise SystemExit(
            f"yt-dlp {version or 'unknown'} is too old for current YouTube. "
            "Update yt-dlp, then rerun this command."
        )


def rows() -> list[dict[str, str]]:
    with MANIFEST.open(newline="", encoding="utf-8-sig") as f:
        data = list(csv.DictReader(f))
    required = {"id", "url", "title", "artist", "lyricist", "composer", "producer", "template", "language", "subtitle_langs", "lyrics_file", "word_timing_file", "logo_file", "prompt_ms"}
    missing = required - set(data[0] if data else ())
    if missing:
        raise SystemExit("songs.csv is missing: " + ", ".join(sorted(missing)))
    result, seen, urls = [], set(), set()
    for row in data:
        sid = row.get("id", "").strip()
        if sid == "example":
            continue
        if not re.fullmatch(r"[A-Za-z0-9_-]+", sid) or sid in seen:
            raise SystemExit(f"Invalid or duplicate id: {sid}")
        if not row.get("url", "").strip():
            raise SystemExit(f"Missing url for {sid}")
        if row["url"].strip() in urls:
            raise SystemExit(f"Duplicate url for {sid}; use one manifest row per source to keep the download archive safe")
        seen.add(sid)
        urls.add(row["url"].strip())
        result.append(row)
    return result


def source(sid: str) -> Path:
    candidates = [p for p in sorted(RAW.glob(f"{sid}.*")) if p.suffix.lower() not in {".json", ".part"}]
    if not candidates:
        raise SystemExit(f"No downloaded media found for {sid}")
    return candidates[0]


def check() -> None:
    print(f"Songs: {len(rows())}")
    for tool in ("yt-dlp", "ffmpeg", "ffprobe"):
        print(f"{tool}: {shutil.which(tool) or 'MISSING'}")
    if any(shutil.which(tool) is None for tool in ("yt-dlp", "ffmpeg", "ffprobe")):
        raise SystemExit(1)
    require_current_ytdlp()


def download(dry_run: bool) -> None:
    require_current_ytdlp()
    RAW.mkdir(parents=True, exist_ok=True)
    SUBTITLES.mkdir(parents=True, exist_ok=True)
    METADATA.mkdir(parents=True, exist_ok=True)
    for row in rows():
        subtitle_langs = row.get("subtitle_langs", "all").strip() or "all"
        run(["yt-dlp", "--no-playlist", "--restrict-filenames", "--format", "bv*+ba/b", "--merge-output-format", "mp4", "--download-archive", str(DOWNLOAD_ARCHIVE), "--write-info-json", "--write-subs", "--write-auto-subs", "--sub-langs", subtitle_langs, "--sub-format", "vtt", "-P", f"subtitle:{SUBTITLES}", "-o", "subtitle:%(id)s.%(language)s.%(ext)s", "--no-overwrites", "-o", str(RAW / f"{row['id']}.%(ext)s"), row["url"].strip()], dry_run)
        if not dry_run:
            (METADATA / f"{row['id']}.json").write_text(json.dumps(row, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")


def normalize(dry_run: bool) -> None:
    AUDIO.mkdir(parents=True, exist_ok=True)
    for row in rows():
        run(["ffmpeg", "-hide_banner", "-y", "-i", str(source(row["id"])), "-vn", "-ac", "2", "-ar", "48000", "-c:a", "flac", str(AUDIO / f"{row['id']}.flac")], dry_run)


def align_row(row: dict[str, str], dry_run: bool) -> None:
    """Run WhisperX for one song and keep its word-level JSON intermediate."""
    if shutil.which("whisperx") is None:
        raise RuntimeError("WhisperX is not installed")
    ALIGN.mkdir(parents=True, exist_ok=True)
    language = row.get("language", "").strip()
    audio = AUDIO / f"{row['id']}.flac"
    if not language:
        raise RuntimeError("language is missing")
    if not audio.exists():
        raise RuntimeError(f"normalized audio is missing: {audio}")
    model = alignment_model(row)
    print(f"{row['id']}: using WhisperX model {model}")
    run(["whisperx", str(audio), "--model", model, "--language", language, "--device", row.get("device", "cpu") or "cpu", "--compute_type", row.get("compute_type", "int8") or "int8", "--output_format", "json", "--output_dir", str(ALIGN), "--return_char_alignments"], dry_run)


def total_memory_bytes() -> int:
    """Return host/WSL memory without requiring an optional Python package."""
    try:
        for line in Path("/proc/meminfo").read_text(encoding="utf-8").splitlines():
            if line.startswith("MemTotal:"):
                return int(line.split()[1]) * 1024
    except (OSError, ValueError, IndexError):
        pass
    return os.sysconf("SC_PAGE_SIZE") * os.sysconf("SC_PHYS_PAGES")


def alignment_model(row: dict[str, str]) -> str:
    """Choose a practical WhisperX model while reserving half of system RAM."""
    configured = row.get("align_model", "auto").strip().lower() or "auto"
    if configured != "auto":
        return configured
    total_gib = total_memory_bytes() / (1024 ** 3)
    budget_gib = total_gib / 2
    if budget_gib >= 12:
        model = "large-v3"
    elif budget_gib >= 8:
        model = "medium"
    elif budget_gib >= 4:
        model = "small"
    elif budget_gib >= 2:
        model = "base"
    else:
        model = "tiny"
    print(f"{row['id']}: detected {total_gib:.1f} GiB RAM; reserving {budget_gib:.1f} GiB for alignment -> {model}")
    return model


def align(dry_run: bool) -> None:
    """Run WhisperX for every eligible song; skip failures individually."""
    skipped = []
    for row in rows():
        try:
            align_row(row, dry_run)
        except (RuntimeError, subprocess.CalledProcessError) as exc:
            print(f"SKIP {row['id']}: alignment unavailable ({exc})")
            skipped.append(row["id"])
    if skipped:
        print("Skipped alignment: " + ", ".join(skipped))


def cleanup(dry_run: bool, drop_source: bool) -> None:
    """Remove reproducible intermediates, retaining editable project files."""
    targets = list(AUDIO.glob("*"))
    if drop_source:
        for row in rows():
            output = OUTPUT / f"{row['id']}.mp4"
            edited = ASS / f"{row['id']}.edited.ass"
            if output.exists() and edited.exists():
                targets.extend(p for p in RAW.glob(f"{row['id']}.*") if p.suffix.lower() not in {".json"})
    for path in targets:
        print(f"REMOVE {path}")
        if not dry_run and path.exists():
            path.unlink()


def word_json(row: dict[str, str]) -> Path | None:
    configured = row.get("word_timing_file", "").strip()
    path = (ROOT / configured) if configured and not Path(configured).is_absolute() else Path(configured) if configured else ALIGN / f"{row['id']}.json"
    return path if path.exists() else None


def tokenise(text: str) -> list[str]:
    if re.search(r"[\u3000-\u9fff\u3040-\u30ff\uac00-\ud7af]", text):
        return [char for char in text if not char.isspace() and not re.match(r"[，。！？、,.!?]", char)]
    return text.split()


def apply_word_timing(entries: list[Entry], path: Path) -> list[Entry]:
    data = json.loads(path.read_text(encoding="utf-8"))
    timed_words = []
    for segment in data.get("segments", []):
        for word in segment.get("words", segment.get("word_segments", [])):
            if word.get("start") is not None and word.get("end") is not None:
                timed_words.append((float(word["start"]) * 1000, float(word["end"]) * 1000, str(word.get("word", "")).strip()))
    if not timed_words:
        raise SystemExit(f"No word timings found in {path}")
    result: list[Entry] = []
    cursor = 0
    for start, end, text, _ in entries:
        selected = [item for item in timed_words if item[1] >= start and item[0] <= end]
        if not selected:
            selected = timed_words[cursor:cursor + max(1, len(tokenise(text)))]
        cursor = max(cursor, timed_words.index(selected[-1]) + 1) if selected else cursor
        tokens = tokenise(text)
        if not selected or not tokens:
            result.append((start, end, text, []))
            continue
        # Preserve the official lyric text while borrowing measured boundaries.
        boundaries = []
        for index, token in enumerate(tokens):
            left = selected[min(index, len(selected) - 1)][0]
            right = selected[min(index, len(selected) - 1)][1]
            if index >= len(selected):
                left = start + (end - start) * index / len(tokens)
                right = start + (end - start) * (index + 1) / len(tokens)
            boundaries.append((max(start, int(left)), min(end, int(right)), token))
        result.append((start, end, text, boundaries))
    return result


def ms(match: re.Match[str]) -> int:
    fraction = (match.group(3) or "0")
    fraction = int((fraction + "00")[:2]) * 10
    return (int(match.group(1)) * 60 + int(match.group(2))) * 1000 + fraction


def lrc(path: Path) -> list[Entry]:
    items = []
    for line in path.read_text(encoding="utf-8-sig").splitlines():
        marks = list(TIMECODE.finditer(line))
        text = line[marks[-1].end():].strip() if marks else ""
        if text:
            word_marks = list(WORDCODE.finditer(text))
            words = [(ms_word.start(), ms_word.end(), text[ms_word.end():word_marks[i + 1].start() if i + 1 < len(word_marks) else len(text)].strip()) for i, ms_word in enumerate(word_marks)]
            items.extend((ms(mark), text, words) for mark in marks)
    items.sort()
    result = []
    for i, (start, text, words) in enumerate(items):
        end = items[i + 1][0] if i + 1 < len(items) else start + max(2000, len(text) * 180)
        word_timings = []
        for index, (_, _, word) in enumerate(words):
            word_start = ms_wordcode(text, index, words, start)
            word_end = ms_wordcode(text, index + 1, words, end)
            if word:
                word_timings.append((max(start, word_start), max(word_start + 10, word_end), word))
        result.append((start, max(start + 300, end), text, word_timings))
    return result


def ms_wordcode(text: str, index: int, words: list[tuple[int, int, str]], fallback: int) -> int:
    marks = list(WORDCODE.finditer(text))
    return ms_inline(marks[index]) if index < len(marks) else fallback


def ms_inline(match: re.Match[str]) -> int:
    fraction = int(((match.group(3) or "0") + "00")[:2]) * 10
    return (int(match.group(1)) * 60 + int(match.group(2))) * 1000 + fraction


def subtitle_time(value: str) -> int:
    value = value.replace(",", ".")
    parts = value.split(":")
    if len(parts) == 2:
        minutes, seconds = parts
        hours = 0
    else:
        hours, minutes, seconds = parts
    whole, _, fraction = seconds.partition(".")
    return (int(hours) * 3600 + int(minutes) * 60 + int(whole)) * 1000 + int((fraction + "000")[:3])


def timed_subtitle(path: Path) -> list[Entry]:
    """Parse common SRT/VTT cues into the internal line-timing format."""
    lines = path.read_text(encoding="utf-8-sig").splitlines()
    result: list[Entry] = []
    index = 0
    while index < len(lines):
        match = re.search(r"(\d{1,2}:)?\d{1,2}:\d{2}[,.]\d{3}\s*-->\s*(\d{1,2}:)?\d{1,2}:\d{2}[,.]\d{3}", lines[index])
        if not match:
            index += 1
            continue
        start_text, end_text = [part.strip().split()[0] for part in lines[index].split("-->", 1)]
        start = subtitle_time(start_text)
        end = subtitle_time(end_text)
        index += 1
        text_lines = []
        while index < len(lines) and lines[index].strip():
            text_lines.append(re.sub(r"<[^>]+>|\{[^}]+\}", "", lines[index]).strip())
            index += 1
        text = " ".join(part for part in text_lines if part)
        if text:
            result.append((start, max(start + 300, end), text, []))
        index += 1
    return result


def downloaded_subtitle(row: dict[str, str]) -> Path | None:
    candidates = sorted(SUBTITLES.glob(f"{row['id']}.*"))
    candidates = [path for path in candidates if path.suffix.lower() in {".vtt", ".srt"}]
    if not candidates:
        return None
    preferred = [item.strip().lower() for item in row.get("subtitle_langs", "").split(",") if item.strip() and item.strip().lower() != "all"]
    def rank(path: Path) -> tuple[int, int, str]:
        name = path.name.lower()
        automatic = 1 if ".auto." in name else 0
        language = next((item for item in preferred if item in name), "")
        return (automatic, 0 if language else 1, name)
    return sorted(candidates, key=rank)[0]


def at(value: int) -> str:
    cs = max(0, value) // 10
    return f"{cs // 360000}:{(cs // 6000) % 60:02d}:{(cs // 100) % 60:02d}.{cs % 100:02d}"


def esc(value: str) -> str:
    return value.replace("\\", "\\N").replace("{", "\\{").replace("}", "\\}").replace("\n", "\\N")


def ass(row: dict[str, str], entries: list[Entry]) -> str:
    title = esc(f"{row['title']}  |  {row['artist']}")
    credit = esc(f"作词：{row['lyricist']}    作曲：{row['composer']}    字幕制作：{row['producer']}")
    out = [
        "[Script Info]", "ScriptType: v4.00+", "PlayResX: 1920", "PlayResY: 1080", "WrapStyle: 2", "ScaledBorderAndShadow: yes", "",
        "[V4+ Styles]", "Format: Name, Fontname, Fontsize, PrimaryColour, SecondaryColour, OutlineColour, BackColour, Bold, Italic, Underline, StrikeOut, ScaleX, ScaleY, Spacing, Angle, BorderStyle, Outline, Shadow, Alignment, MarginL, MarginR, MarginV, Encoding",
        "Style: Header,Arial,42,&H00FFFFFF,&H00FFFFFF,&H80000000,&H50000000,1,0,0,0,100,100,0,0,1,2,1,8,40,40,40,1",
        "Style: Credit,Arial,28,&H00FFFFFF,&H00FFFFFF,&H80000000,&H50000000,0,0,0,0,100,100,0,0,1,2,1,2,40,40,70,1",
        "Style: Lyric,Arial,58,&H0000FFFF,&H00FFFFFF,&H80000000,&H50000000,1,0,0,0,100,100,0,0,1,3,1,2,80,80,150,1", "",
        "Style: Prompt,Arial,44,&H00FFFFFF,&H00FFFFFF,&H80000000,&H50000000,1,0,0,0,100,100,0,0,1,2,1,2,80,80,150,1", "",
        "[Events]", "Format: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text",
        f"Dialogue: 0,0:00:00.00,0:00:06.00,Header,,0,0,40,,{title}",
        f"Dialogue: 0,0:00:00.00,0:00:06.00,Credit,,0,0,70,,{credit}",
    ]
    for start, end, text, words in entries:
        prompt_ms = max(0, int(row.get("prompt_ms", "1000") or "1000"))
        prompt_start = max(0, start - prompt_ms)
        if prompt_start < start:
            out.append(f"Dialogue: 2,{at(prompt_start)},{at(start)},Prompt,,0,0,150,,...")
        if words:
            karaoke = []
            for word_start, word_end, word in words:
                duration = max(1, round((word_end - word_start) / 10))
                karaoke.append(f"{{\\k{duration}}}{esc(word)}")
            visible = " ".join(karaoke)
        else:
            visible = esc(text)
        out.append(f"Dialogue: 0,{at(start)},{at(end)},Lyric,,0,0,150,,{visible}")
    return "\n".join(out) + "\n"


def lyrics(dry_run: bool) -> None:
    ASS.mkdir(parents=True, exist_ok=True)
    for row in rows():
        path = downloaded_subtitle(row)
        source_label = "downloaded subtitle"
        if path is not None:
            entries = timed_subtitle(path)
        else:
            raw = row.get("lyrics_file", "").strip()
            if not raw:
                print(f"SKIP {row['id']}: no downloaded subtitle and lyrics_file is empty")
                continue
            path = Path(raw)
            if not path.is_absolute():
                path = ROOT / path
            if not path.exists():
                raise SystemExit(f"Lyrics file not found: {path}")
            if path.suffix.lower() == ".lrc":
                entries = lrc(path)
            elif path.suffix.lower() in {".srt", ".vtt"}:
                entries = timed_subtitle(path)
            else:
                raise SystemExit(f"Expected .lrc, .srt, or .vtt: {path}")
            source_label = "manifest lyrics_file"
        if not entries:
            raise SystemExit(f"No timed lines found in {path}")
        timing_path = word_json(row)
        if timing_path is None and not dry_run:
            print(f"{row['id']}: word timing missing; starting automatic alignment")
            try:
                align_row(row, False)
            except (RuntimeError, subprocess.CalledProcessError) as exc:
                print(f"SKIP {row['id']}: word timings unavailable ({exc})")
                continue
            timing_path = word_json(row)
        if timing_path is None:
            print(f"SKIP {row['id']}: no word-level timing found")
            continue
        try:
            entries = apply_word_timing(entries, timing_path)
        except (OSError, ValueError, KeyError, SystemExit) as exc:
            print(f"SKIP {row['id']}: invalid word timing data ({exc})")
            continue
        source_label += " + word alignment"
        if not dry_run:
            (ASS / f"{row['id']}.auto.ass").write_text(ass(row, entries), encoding="utf-8")
        word_lines = sum(1 for _, _, _, words in entries if words)
        timing_note = f", {word_lines} with word timing" if word_lines else ", line timing only"
        print(f"{row['id']}: {len(entries)} lines from {source_label}{timing_note} -> {row['id']}.auto.ass")


def edited_path(sid: str) -> Path:
    return ASS / f"{sid}.edited.ass"


def edit(row: dict[str, str]) -> None:
    auto = ASS / f"{row['id']}.auto.ass"
    edited = edited_path(row["id"])
    if not auto.exists():
        raise SystemExit(f"Missing {auto}; run lyrics first")
    if not edited.exists():
        shutil.copy2(auto, edited)
        print(f"Created editable copy: {edited}")
    gui = shutil.which("aegisub") or shutil.which("Aegisub") or shutil.which("subtitleedit")
    if gui:
        subprocess.Popen([gui, str(edited)])
    else:
        print(f"Open this file in Aegisub or Subtitle Edit: {edited}")


def confirm_reprocess(sid: str, output: Path, force: bool, dry_run: bool) -> bool:
    """Decide whether an existing final output should be rendered again."""
    if not output.exists():
        return True
    if force:
        print(f"REPROCESS {sid}: --reprocess was supplied")
        return True
    if not sys.stdin.isatty():
        print(f"SKIP {sid}: output already exists (non-interactive run)")
        return False
    try:
        answer = input(f"{sid} is already processed. Process again? [y/N] ").strip().lower()
    except EOFError:
        answer = ""
    if answer not in {"y", "yes"}:
        print(f"SKIP {sid}: output already exists")
        return False
    return True


def render(dry_run: bool, force: bool) -> None:
    OUTPUT.mkdir(parents=True, exist_ok=True)
    song_rows = rows()
    if not dry_run:
        needs_alignment = any(
            not edited_path(row["id"]).exists()
            and (ASS / f"{row['id']}.auto.ass").exists()
            and "{\\k" not in (ASS / f"{row['id']}.auto.ass").read_text(encoding="utf-8")
            for row in song_rows
        )
        if needs_alignment:
            print("Some songs have no word timing; starting automatic alignment where possible")
            lyrics(False)
    for row in song_rows:
        output = OUTPUT / f"{row['id']}.mp4"
        if not confirm_reprocess(row["id"], output, force, dry_run):
            continue
        subtitle = edited_path(row["id"]) if edited_path(row["id"]).exists() else ASS / f"{row['id']}.auto.ass"
        if not subtitle.exists():
            raise SystemExit(f"Missing ASS for {row['id']}; run lyrics first")
        ass_text = subtitle.read_text(encoding="utf-8")
        if "{\\k" not in ass_text:
            print(f"SKIP {row['id']}: no word-level timing; no output rendered")
            continue
        subtitle_filter_path = str(subtitle).replace("\\", "/").replace(":", "\\:")
        vf = f"ass={subtitle_filter_path}"
        run(["ffmpeg", "-hide_banner", "-y", "-i", str(source(row["id"])), "-vf", vf, "-c:v", "libx264", "-preset", "medium", "-crf", "18", "-c:a", "aac", "-b:a", "192k", "-movflags", "+faststart", str(output)], dry_run)


def qa() -> None:
    for row in rows():
        selected = edited_path(row["id"]) if edited_path(row["id"]).exists() else ASS / f"{row['id']}.auto.ass"
        output = OUTPUT / f"{row['id']}.mp4"
        print(f"{row['id']}: source={'edited' if selected.name.endswith('.edited.ass') else 'automatic'}, ass={'OK' if selected.exists() else 'MISSING'}, mp4={'OK' if output.exists() else 'WAITING'}")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("command", choices=("check", "download", "normalize", "align", "lyrics", "edit", "render", "cleanup", "qa"))
    parser.add_argument("--dry-run", action="store_true")
    parser.add_argument("--reprocess", action="store_true", help="render songs again even when the final MP4 already exists")
    parser.add_argument("--drop-source", action="store_true", help="also remove raw source media after rendered output and edited ASS exist")
    args = parser.parse_args()
    if args.command == "check": check()
    elif args.command == "download": download(args.dry_run)
    elif args.command == "normalize": normalize(args.dry_run)
    elif args.command == "align": align(args.dry_run)
    elif args.command == "lyrics": lyrics(args.dry_run)
    elif args.command == "edit":
        for row in rows(): edit(row)
    elif args.command == "render": render(args.dry_run, args.reprocess)
    elif args.command == "cleanup": cleanup(args.dry_run, args.drop_source)
    else: qa()
    return 0


if __name__ == "__main__":
    sys.exit(main())
