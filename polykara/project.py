"""Editable karaoke projects: JSON storage, ASS read-back, and lyric exporters."""
from __future__ import annotations

import json
import re
from pathlib import Path

from .ass import CJK, ass
from .config import ASS, WORK, Entry
from .lyricformats import READERS, join_words
from .subtitle import lrc, timed_subtitle

PROJECTS = WORK / "project"
DIALOGUE = re.compile(r"^Dialogue:\s*\d+,([^,]+),([^,]+),([^,]*),(?:[^,]*,){4}(.*)$")
KARAOKE_TAG = re.compile(r"\\k[fo]?(\d+)", re.IGNORECASE)
UNIT = re.compile(r"[\u3000-\u9fff\u3040-\u30ff\uac00-\ud7af\uf900-\ufaff]|[^\s|\u3000-\u9fff\u3040-\u30ff\uac00-\ud7af\uf900-\ufaff]+")


def ass_time(value: str) -> int:
    hours, minutes, seconds = value.strip().split(":")
    whole, _, fraction = seconds.partition(".")
    return ((int(hours) * 60 + int(minutes)) * 60 + int(whole)) * 1000 + int((fraction + "00")[:2]) * 10


def unescape(value: str) -> str:
    return value.replace("\\N", " ").replace("\\n", " ").replace("\\h", " ").replace("\\{", "{").replace("\\}", "}")


def parse_ass(text: str) -> list[Entry]:
    """Read the Lyric events of a PolyKara (or Aegisub \\k) ASS file back into entries."""
    padding = 0
    match = re.search(r"^; PolyKaraIntroPaddingMs:\s*(\d+)", text, re.MULTILINE)
    if match:
        padding = int(match.group(1))
    result = []
    for line in text.splitlines():
        event = DIALOGUE.match(line)
        if not event or event.group(3).strip().casefold() not in {"lyric", "default"}:
            continue
        shown, end, body = ass_time(event.group(1)) - padding, ass_time(event.group(2)) - padding, event.group(4)
        words, cursor, pending = [], shown, None
        for tags, chunk in re.findall(r"\{([^}]*)\}([^{]*)", body if body.startswith("{") else "{}" + body):
            durations = KARAOKE_TAG.findall(tags)
            if durations:
                if pending:
                    words.append(pending)
                    pending = None
                duration = int(durations[-1]) * 10
                if chunk.strip():
                    pending = (cursor, cursor + duration, unescape(chunk).strip())
                cursor += duration
            elif pending and chunk.strip():
                pending = (pending[0], pending[1], pending[2] + unescape(chunk).strip())
        if pending:
            words.append(pending)
        plain = re.sub(r"\{[^}]*\}", "", body)
        lyric = join_words([word for _, _, word in words]) if words else unescape(plain).strip()
        if lyric:
            start = words[0][0] if words else shown
            result.append((start, end, lyric, words))
    return result


def to_json(entries: list[Entry]) -> dict:
    return {"version": 1, "lines": [{"start": start, "end": end, "text": text, "words": [list(word) for word in words]} for start, end, text, words in entries]}


def from_json(data: dict) -> list[Entry]:
    result = []
    for line in data.get("lines", []):
        words = [(int(left), int(right), str(text)) for left, right, text in line.get("words", []) if str(text).strip()]
        start = int(line.get("start", words[0][0] if words else 0))
        end = max(int(line.get("end", words[-1][1] if words else start + 2000)), start + 10)
        text = str(line.get("text", "")).strip() or join_words([word for _, _, word in words])
        if text:
            result.append((start, end, text, sorted(words)))
    return sorted(result, key=lambda item: item[0])


def project_path(sid: str) -> Path:
    return PROJECTS / f"{sid}.json"


def lyric_source_entries(path: Path) -> list[Entry]:
    suffix = path.suffix.lower()
    if suffix == ".lrc":
        return lrc(path)
    if suffix in {".srt", ".vtt"}:
        return timed_subtitle(path)
    if suffix == ".ass":
        return parse_ass(path.read_text(encoding="utf-8-sig"))
    if suffix in READERS:
        return READERS[suffix](path)
    lines = [line.strip() for line in path.read_text(encoding="utf-8-sig").splitlines() if line.strip()]
    return [(index * 4000, index * 4000 + 3500, line, []) for index, line in enumerate(lines)]


def load_entries(row: dict[str, str], root: Path) -> tuple[list[Entry], str]:
    """Best available timing for a song: saved project, edited ASS, automatic ASS, then lyrics_file."""
    sid = row["id"]
    if project_path(sid).exists():
        return from_json(json.loads(project_path(sid).read_text(encoding="utf-8"))), "project"
    for name, label in ((f"{sid}.edited.ass", "edited ASS"), (f"{sid}.auto.ass", "automatic ASS")):
        if (ASS / name).exists():
            return parse_ass((ASS / name).read_text(encoding="utf-8")), label
    raw = row.get("lyrics_file", "").strip()
    if raw:
        path = Path(raw) if Path(raw).is_absolute() else root / raw
        if path.exists():
            return lyric_source_entries(path), "lyrics_file"
    return [], "empty"


def save_entries(row: dict[str, str], entries: list[Entry]) -> Path:
    """Persist the project and write the edited ASS that render/live prefer."""
    PROJECTS.mkdir(parents=True, exist_ok=True)
    ASS.mkdir(parents=True, exist_ok=True)
    project_path(row["id"]).write_text(json.dumps(to_json(entries), ensure_ascii=False, indent=1) + "\n", encoding="utf-8")
    edited = ASS / f"{row['id']}.edited.ass"
    edited.write_text(ass(row, entries), encoding="utf-8")
    return edited


def split_units(text: str) -> list[str]:
    """Timing units: one CJK character, or one Latin word; '|' splits a word into syllables."""
    return UNIT.findall(text)


def lrc_stamp(value: int, brackets: str = "[]") -> str:
    value = max(0, value)
    return f"{brackets[0]}{value // 60000:02d}:{value // 1000 % 60:02d}.{value % 1000 // 10:02d}{brackets[1]}"


def separator(previous: str, word: str) -> str:
    return "" if not previous or (CJK.search(previous[-1:]) and CJK.search(word[:1])) else " "


def export_lrc(entries: list[Entry]) -> str:
    out = []
    for start, end, text, words in entries:
        if not words:
            out.append(f"{lrc_stamp(start)}{text}")
            continue
        body, previous = "", ""
        for left, _, word in words:
            body += f"{separator(previous, word)}{lrc_stamp(left, '<>')}{word}"
            previous = word
        out.append(f"{lrc_stamp(start)}{body}{lrc_stamp(words[-1][1], '<>')}")
    return "\n".join(out) + "\n"


def export_srt(entries: list[Entry]) -> str:
    def stamp(value: int) -> str:
        value = max(0, value)
        return f"{value // 3600000:02d}:{value // 60000 % 60:02d}:{value // 1000 % 60:02d},{value % 1000:03d}"
    return "\n".join(f"{index}\n{stamp(start)} --> {stamp(end)}\n{text}\n" for index, (start, end, text, _) in enumerate(entries, 1))


def export_krc(entries: list[Entry]) -> str:
    out = []
    for start, end, text, words in entries:
        words = words or [(start, end, text)]
        out.append(f"[{start},{end - start}]" + "".join(f"<{left - start},{right - left},0>{word}" for left, right, word in words))
    return "\n".join(out) + "\n"


def export_ksc(entries: list[Entry]) -> str:
    def stamp(value: int) -> str:
        return f"{value // 60000:02d}:{value // 1000 % 60:02d}.{value % 1000:03d}"
    out = ["karaoke := CreateKaraokeObject;", "karaoke.rows := 2;", "karaoke.clear;", ""]
    for start, end, text, words in entries:
        words = words or [(start, end, text)]
        # KSC durations are back to back, so a gap is folded into the word before it.
        units = [word if CJK.fullmatch(word) else f"[{word}]" for _, _, word in words]
        durations = [(words[i + 1][0] if i + 1 < len(words) else right) - left for i, (left, right, _) in enumerate(words)]
        text = "".join(units).replace("'", "''")
        out.append(f"karaoke.add('{stamp(words[0][0])}', '{stamp(words[-1][1])}', '{text}', '{','.join(str(max(0, d)) for d in durations)}');")
    return "\n".join(out) + "\n"


EXPORTERS = {"lrc": export_lrc, "srt": export_srt, "krc": export_krc, "ksc": export_ksc}
