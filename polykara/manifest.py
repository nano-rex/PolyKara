from __future__ import annotations

import csv
import re
import shutil
from pathlib import Path
from .config import MANIFEST, RAW, require_current_ytdlp


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
            raise SystemExit(f"Duplicate url for {sid}; use one manifest row per source")
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
