from __future__ import annotations

import json
import re
from pathlib import Path
from .config import ALIGN, SUBTITLES, TIMECODE, WORDCODE, Entry, SPEAKER_TAG


def word_json(row: dict[str, str]) -> Path | None:
    configured = row.get("word_timing_file", "").strip()
    if configured:
        path = Path(configured)
        path = path if path.is_absolute() else Path(__file__).resolve().parent.parent / path
        return path if path.exists() else None
    exact = ALIGN / f"{row['id']}.json"
    if exact.exists():
        return exact
    candidates = sorted(path for path in ALIGN.rglob("*.json") if path.name == f"{row['id']}.json" or path.name.startswith(f"{row['id']}.") or path.parent.name == row["id"])
    return candidates[0] if candidates else None


def tokenise(text: str) -> list[str]:
    if re.search(r"[\u3000-\u9fff\u3040-\u30ff\uac00-\ud7af]", text):
        return [char for char in text if not char.isspace() and not re.match(r"[，。！？、,.!?]", char)]
    return text.split()


def split_speaker(text: str) -> tuple[str, str]:
    match = SPEAKER_TAG.match(text)
    return (match.group(1).strip(), text[match.end():].strip()) if match else ("", text.strip())


def apply_word_timing(entries: list[Entry], path: Path) -> list[Entry]:
    data = json.loads(path.read_text(encoding="utf-8"))
    segments = data.get("segments", []) or ([{"words": data["word_segments"]}] if data.get("word_segments") else [])
    timed_words = [(float(word["start"]) * 1000, float(word["end"]) * 1000, str(word.get("word", "")).strip()) for segment in segments for word in segment.get("words", segment.get("word_segments", [])) if word.get("start") is not None and word.get("end") is not None]
    if not timed_words:
        raise SystemExit(f"No word timings found in {path}")
    result, cursor = [], 0
    for start, end, text, _ in entries:
        _, lyric_text = split_speaker(text)
        selected = [item for item in timed_words if item[1] >= start and item[0] <= end] or timed_words[cursor:cursor + max(1, len(tokenise(lyric_text)))]
        cursor = max(cursor, timed_words.index(selected[-1]) + 1) if selected else cursor
        tokens = tokenise(lyric_text)
        if not selected or not tokens:
            result.append((start, end, text, []))
            continue
        boundaries = []
        for index, token in enumerate(tokens):
            left, right = selected[min(index, len(selected) - 1)][0:2]
            if index >= len(selected):
                left = start + (end - start) * index / len(tokens)
                right = start + (end - start) * (index + 1) / len(tokens)
            boundaries.append((max(start, int(left)), min(end, int(right)), token))
        result.append((start, end, text, boundaries))
    return result


def plain_lyrics_entries(path: Path, timing_path: Path) -> list[Entry]:
    data = json.loads(timing_path.read_text(encoding="utf-8"))
    timed = [(int(float(word["start"]) * 1000), int(float(word["end"]) * 1000)) for segment in data.get("segments", []) for word in segment.get("words", segment.get("word_segments", [])) if word.get("start") is not None and word.get("end") is not None]
    result, cursor = [], 0
    for line in [line.strip() for line in path.read_text(encoding="utf-8").splitlines() if line.strip()]:
        selected = timed[cursor:cursor + max(1, len(tokenise(line)))]
        if not selected:
            break
        result.append((selected[0][0], max(selected[0][0] + 300, selected[-1][1]), line, []))
        cursor += len(selected)
    return result


def ms(match: re.Match[str]) -> int:
    fraction = int(((match.group(3) or "0") + "00")[:2]) * 10
    return (int(match.group(1)) * 60 + int(match.group(2))) * 1000 + fraction


def ms_inline(match: re.Match[str]) -> int:
    fraction = int(((match.group(3) or "0") + "00")[:2]) * 10
    return (int(match.group(1)) * 60 + int(match.group(2))) * 1000 + fraction


def inline_time(text: str, index: int, fallback: int) -> int:
    marks = list(WORDCODE.finditer(text))
    return ms_inline(marks[index]) if index < len(marks) else fallback


def lrc(path: Path) -> list[Entry]:
    items = []
    for line in path.read_text(encoding="utf-8-sig").splitlines():
        marks = list(TIMECODE.finditer(line))
        text = line[marks[-1].end():].strip() if marks else ""
        speaker, lyric_text = split_speaker(text)
        if lyric_text:
            marks_words = list(WORDCODE.finditer(lyric_text))
            words = [(item.start(), item.end(), lyric_text[item.end():marks_words[i + 1].start() if i + 1 < len(marks_words) else len(lyric_text)].strip()) for i, item in enumerate(marks_words)]
            tagged = f"[speaker:{speaker}] {lyric_text}" if speaker else lyric_text
            items.extend((ms(mark), tagged, words) for mark in marks)
    items.sort()
    result = []
    for i, (start, text, words) in enumerate(items):
        end = items[i + 1][0] if i + 1 < len(items) else start + max(2000, len(text) * 180)
        word_timings = []
        _, lyric_text = split_speaker(text)
        for index, (_, _, word) in enumerate(words):
            word_start = inline_time(lyric_text, index, start)
            word_end = inline_time(lyric_text, index + 1, end)
            if word:
                word_timings.append((max(start, word_start), max(word_start + 10, word_end), word))
        result.append((start, max(start + 300, end), text, word_timings))
    return result


def subtitle_time(value: str) -> int:
    parts = value.replace(",", ".").split(":")
    hours, minutes, seconds = (0, *parts) if len(parts) == 2 else parts
    whole, _, fraction = seconds.partition(".")
    return (int(hours) * 3600 + int(minutes) * 60 + int(whole)) * 1000 + int((fraction + "000")[:3])


def timed_subtitle(path: Path) -> list[Entry]:
    lines, result, index = path.read_text(encoding="utf-8-sig").splitlines(), [], 0
    while index < len(lines):
        match = re.search(r"(\d{1,2}:)?\d{1,2}:\d{2}[,.]\d{3}\s*-->\s*(\d{1,2}:)?\d{1,2}:\d{2}[,.]\d{3}", lines[index])
        if not match:
            index += 1
            continue
        start_text, end_text = [part.strip().split()[0] for part in lines[index].split("-->", 1)]
        start, end, index = subtitle_time(start_text), subtitle_time(end_text), index + 1
        text_lines = []
        while index < len(lines) and lines[index].strip():
            text_lines.append(re.sub(r"<[^>]+>|\{[^}]+\}", "", lines[index]).strip())
            index += 1
        text = " ".join(part for part in text_lines if part)
        speaker, lyric_text = split_speaker(text)
        if lyric_text:
            tagged = f"[speaker:{speaker}] {lyric_text}" if speaker else lyric_text
            result.append((start, max(start + 300, end), tagged, []))
        index += 1
    return result


def downloaded_subtitle(row: dict[str, str]) -> Path | None:
    candidates = [path for path in sorted(SUBTITLES.glob(f"{row['id']}.*")) if path.suffix.lower() in {".vtt", ".srt"}]
    if not candidates:
        return None
    preferred = [item.strip().lower() for item in row.get("subtitle_langs", "").split(",") if item.strip() and item.strip().lower() != "all"]
    return sorted(candidates, key=lambda path: (1 if ".auto." in path.name.lower() else 0, 0 if any(item in path.name.lower() for item in preferred) else 1, path.name))[0]
