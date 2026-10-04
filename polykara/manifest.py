from __future__ import annotations

import csv
import importlib.util
import re
import shutil
from pathlib import Path
from .config import MANIFEST, MEDIA_SUFFIXES, RAW, ROOT, WORK, require_current_ytdlp

REQUIRED_COLUMNS = {"id", "url", "title", "artist", "lyricist", "composer", "producer", "template", "language", "subtitle_langs", "lyrics_file", "word_timing_file", "logo_file", "prompt_ms"}
class SongError(Exception):
    """A problem that stops one song but must not stop the batch."""


# Songs selected with --only / positional ids; empty means every manifest row.
SELECTED: set[str] = set()


def select(ids: list[str]) -> None:
    SELECTED.clear()
    SELECTED.update(item.strip() for item in ids if item.strip())


def rows() -> list[dict[str, str]]:
    if not MANIFEST.exists():
        raise SystemExit(f"Manifest not found: {MANIFEST}")
    with MANIFEST.open(newline="", encoding="utf-8-sig") as f:
        reader = csv.DictReader(f)
        fieldnames = reader.fieldnames or []
        # Short rows yield None values and long rows a None key; normalise both away.
        data = [{key: (value or "").strip() for key, value in row.items() if isinstance(key, str)} for row in reader]
    missing = REQUIRED_COLUMNS - set(fieldnames)
    if missing:
        raise SystemExit("songs.csv is missing: " + ", ".join(sorted(missing)))
    result, seen, urls = [], set(), set()
    for row in data:
        sid = row["id"]
        if sid == "example" or not any(row.values()):
            continue
        if not re.fullmatch(r"[A-Za-z0-9_-]+", sid) or sid in seen:
            raise SystemExit(f"Invalid or duplicate id: {sid!r}")
        if not row["url"]:
            raise SystemExit(f"Missing url for {sid}")
        if row["url"] in urls:
            raise SystemExit(f"Duplicate url for {sid}; use one manifest row per source")
        seen.add(sid)
        urls.add(row["url"])
        result.append(row)
    unknown = SELECTED - seen
    if unknown:
        raise SystemExit("Not in songs.csv: " + ", ".join(sorted(unknown)))
    return [row for row in result if row["id"] in SELECTED] if SELECTED else result


def resolve(value: str) -> Path:
    """Resolve a manifest path relative to the project root."""
    path = Path(value).expanduser()
    return path if path.is_absolute() else ROOT / path


def find_source(sid: str) -> Path | None:
    exact = RAW / f"{sid}.mp4"
    if exact.exists():
        return exact
    # Ignore metadata and yt-dlp leftovers such as id.f137.mp4 or id.mp4.part.
    candidates = [p for p in sorted(RAW.glob(f"{sid}.*")) if p.suffix.lower() in MEDIA_SUFFIXES and p.stem == sid]
    return candidates[0] if candidates else None


def source(sid: str) -> Path:
    path = find_source(sid)
    if path is None:
        raise SystemExit(f"No downloaded media found for {sid}")
    return path


def check(need_ytdlp: bool = True) -> None:
    print(f"Songs: {len(rows())}")
    tools = ("yt-dlp", "ffmpeg", "ffprobe")
    for tool in tools:
        print(f"{tool}: {shutil.which(tool) or 'MISSING'}")
    aligner = "whisperx" if shutil.which("whisperx") else "faster-whisper" if importlib.util.find_spec("faster_whisper") else None
    print(f"aligner: {aligner or 'MISSING (pip install -r requirements-align.txt)'}")
    engines = {"pypinyin": "zh", "ToJyutping": "yue", "pykakasi": "ja", "indic_transliteration": "indic", "anyascii": "other scripts"}
    available = ["ko"] + [label for module, label in engines.items() if importlib.util.find_spec(module)]
    missing = [label for module, label in engines.items() if not importlib.util.find_spec(module)]
    print(f"romanization engines: {', '.join(available)}" + (f"; missing {', '.join(missing)} (pip install -r requirements-romanization.txt)" if missing else ""))
    base = WORK if WORK.exists() else ROOT
    print(f"free disk space: {shutil.disk_usage(base).free / 1024 ** 3:.1f} GiB in {base}")
    required = tools if need_ytdlp else tools[1:]
    if any(shutil.which(tool) is None for tool in required):
        raise SystemExit("Required tools are missing: " + ", ".join(tool for tool in required if shutil.which(tool) is None))
    if need_ytdlp:
        require_current_ytdlp()
