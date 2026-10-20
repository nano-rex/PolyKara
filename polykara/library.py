"""Local music library for the player: scanning, tags, and lyric lookup."""
from __future__ import annotations

import hashlib
import json
import re
import subprocess
from pathlib import Path
from urllib.parse import urlencode

from .config import WORDCODE, WORK, Entry, load_config
from .project import lyric_source_entries
from .providers import fetch_json
from .romanize import romanize

AUDIO_SUFFIXES = {".mp3", ".flac", ".m4a", ".aac", ".ogg", ".opus", ".wav", ".wma", ".ape", ".mp4", ".mkv", ".webm", ".mov"}
LYRIC_SUFFIXES = (".krc", ".qrc", ".ksc", ".lrc", ".ass", ".srt", ".vtt", ".txt")
LIBRARY_CACHE = WORK / "library.json"
FETCHED_LYRICS = WORK / "player-lyrics"


def library_roots(root: Path) -> list[Path]:
    configured = load_config().get("player", {}).get("library", ["~/Music", "work/output"])
    result = []
    for item in configured:
        path = Path(str(item)).expanduser()
        path = path if path.is_absolute() else root / path
        if path.is_dir():
            result.append(path.resolve())
    return result


def probe(path: Path) -> dict:
    try:
        output = subprocess.run(["ffprobe", "-v", "quiet", "-print_format", "json", "-show_format", "-show_streams", str(path)], capture_output=True, text=True, timeout=20, check=False).stdout
        data = json.loads(output or "{}")
    except (OSError, subprocess.TimeoutExpired, json.JSONDecodeError):
        data = {}
    tags = {key.casefold(): value for key, value in data.get("format", {}).get("tags", {}).items()}
    video = any(stream.get("codec_type") == "video" and not stream.get("disposition", {}).get("attached_pic") for stream in data.get("streams", []))
    return {
        "title": tags.get("title") or path.stem,
        "artist": tags.get("artist") or tags.get("album_artist") or "",
        "album": tags.get("album") or "",
        "duration": float(data.get("format", {}).get("duration") or 0),
        "video": video,
    }


def scan(root: Path, refresh: bool = False) -> list[dict]:
    """Tracks under the configured folders; ffprobe tags are cached by path, size and mtime."""
    cache = {}
    if LIBRARY_CACHE.exists() and not refresh:
        try:
            cache = {item["path"]: item for item in json.loads(LIBRARY_CACHE.read_text(encoding="utf-8"))}
        except (OSError, ValueError, KeyError):
            cache = {}
    tracks = []
    for folder in library_roots(root):
        for path in sorted(folder.rglob("*")):
            if path.suffix.lower() not in AUDIO_SUFFIXES or not path.is_file():
                continue
            stat = path.stat()
            key = str(path)
            item = cache.get(key)
            if not item or item.get("size") != stat.st_size or item.get("mtime") != int(stat.st_mtime):
                item = {"path": key, "size": stat.st_size, "mtime": int(stat.st_mtime), **probe(path)}
            item["id"] = track_id(path)
            item["lyrics"] = bool(sidecar(path))
            item["burned"] = (WORK / "output").resolve() in path.parents  # PolyKara renders already show lyrics
            tracks.append(item)
    WORK.mkdir(parents=True, exist_ok=True)
    LIBRARY_CACHE.write_text(json.dumps(tracks, ensure_ascii=False) + "\n", encoding="utf-8")
    return tracks


def sidecar(path: Path) -> Path | None:
    for suffix in LYRIC_SUFFIXES:
        for candidate in (path.with_suffix(suffix), path.with_suffix(suffix.upper())):
            if candidate.exists():
                return candidate
    # PolyKara outputs: work/output/<id>.mp4 pairs with work/ass/<id>.edited.ass or .auto.ass
    for name in (f"{path.stem}.edited.ass", f"{path.stem}.auto.ass"):
        candidate = WORK / "ass" / name
        if candidate.exists():
            return candidate
    return stored_lyrics(track_id(path))


def track_id(path: Path) -> str:
    return hashlib.sha1(str(path).encode("utf-8")).hexdigest()[:16]


def stored_lyrics(tid: str) -> Path | None:
    for suffix in LYRIC_SUFFIXES:
        candidate = FETCHED_LYRICS / f"{tid}{suffix}"
        if candidate.exists():
            return candidate
    return None


def store_lyrics(tid: str, suffix: str, data: bytes) -> Path:
    """Keep user-supplied or fetched lyrics in work/, never next to the user's music files."""
    suffix = suffix.lower() if suffix.lower() in LYRIC_SUFFIXES else ".lrc"
    FETCHED_LYRICS.mkdir(parents=True, exist_ok=True)
    for old in FETCHED_LYRICS.glob(f"{tid}.*"):
        old.unlink()
    output = FETCHED_LYRICS / f"{tid}{suffix}"
    output.write_bytes(data)
    return output


def fetch_lrclib(track: dict) -> Path | None:
    query = {"artist_name": track.get("artist", ""), "track_name": track.get("title", "")}
    if track.get("duration"):
        query["duration"] = round(track["duration"])
    try:
        data = fetch_json(f"https://lrclib.net/api/get?{urlencode(query)}")
    except Exception as exc:
        print(f"player: LRCLIB lookup failed for {track.get('title')} ({exc})")
        return None
    synced = str(data.get("syncedLyrics") or "").strip()
    if not synced:
        return None
    return store_lyrics(track["id"], ".lrc", (synced + "\n").encode("utf-8"))


def guess_language(text: str) -> str:
    if re.search(r"[぀-ヿ]", text):
        return "ja"
    if re.search(r"[가-힯]", text):
        return "ko"
    if re.search(r"[一-鿿]", text):
        return "zh"
    return ""


def merge_translations(entries: list[Entry]) -> list[dict]:
    """Two LRC lines with the same timestamp are an original plus its translation."""
    lines: list[dict] = []
    merged = set()
    for start, end, text, words in entries:
        text = WORDCODE.sub("", text).strip()
        if lines and lines[-1]["start"] == start and not words:
            lines[-1]["translation"] = text
            merged.add(len(lines) - 1)
            continue
        lines.append({"start": start, "end": end, "text": text, "words": [list(word) for word in words], "translation": ""})
    for index in merged:
        # LRC end times come from the next timestamp, which for a pair is the same one
        following = lines[index + 1]["start"] if index + 1 < len(lines) else lines[index]["start"] + 4000
        lines[index]["end"] = max(lines[index]["end"], following)
    return lines


def track_lyrics(track: dict, online: bool = True) -> dict:
    path = Path(track["path"])
    source = sidecar(path)
    origin = "sidecar"
    if source is None and online:
        source, origin = fetch_lrclib(track), "lrclib"
    if source is None:
        return {"source": "none", "lines": []}
    try:
        entries = lyric_source_entries(source)
    except SystemExit as exc:
        return {"source": "error", "error": str(exc), "lines": []}
    lines = merge_translations(entries)
    language = guess_language("".join(line["text"] for line in lines[:20]))
    settings = {**load_config().get("romanization", {}), "enabled": True}
    if language:
        for line in lines:
            line["roman"] = romanize(re.sub(r"^\[[^\]]+\]\s*", "", line["text"]), language, settings) or ""
    return {"source": origin, "file": source.name, "language": language, "lines": lines}
