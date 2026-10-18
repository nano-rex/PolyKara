"""Songs made from media files on this computer instead of a video URL.

A local file is listed in songs.csv with its path in the ``url`` column. The
download step copies it into work/raw instead of running yt-dlp, and the title
and singer used for the lyrics search come from the file's own tags or, when
it has none, from its name ("Artist - Title.mp4", "01. Title.mp3", "歌手-歌名.mkv").
"""
from __future__ import annotations

import csv
import json
import os
import re
import shutil
import subprocess
from pathlib import Path

from .config import HOME, MANIFEST, MEDIA_SUFFIXES
from .manifest import REQUIRED_COLUMNS, SongError

COLUMNS = ["id", "url", "title", "artist", "lyricist", "composer", "producer", "template", "language", "subtitle_langs", "lyrics_file", "word_timing_file", "logo_file", "prompt_ms", "align_model", "lyric_sources", "lyric_pages", "voice"]
# Words around a song name in file names that are not part of it.
NOISE = re.compile(r"[\(\[【（][^\)\]】）]*(official|video|audio|lyric|mv|m/v|karaoke|ktv|hd|4k|1080p|720p|live|官方|完整版|高清|伴奏|歌词)[^\)\]】）]*[\)\]】）]", re.IGNORECASE)
SEPARATOR = re.compile(r"\s+[-–—|]\s+|\s*[－—]\s*")
# "01. ", "02 - ", "07 " at the start; a longer number such as 1979 is left as part of the title.
TRACK_NUMBER = re.compile(r"^\s*(?:\d{1,3}\s*[.\-_)\]]\s*|\d{1,2}\s+(?=\D))")
HAN, KANA, HANGUL = re.compile(r"[一-鿿㐀-䶿]"), re.compile(r"[぀-ヿ]"), re.compile(r"[가-힣]")


def is_local(url: str) -> bool:
    """True when the url column names a file rather than a web address."""
    value = url.strip()
    return bool(value) and (value.startswith("file://") or "://" not in value)


def local_path(url: str) -> Path:
    value = url.strip()
    if value.startswith("file://"):
        from urllib.parse import unquote, urlparse

        parsed = urlparse(value)
        value = unquote(parsed.path)
        if re.match(r"^/[A-Za-z]:", value):  # file:///C:/Music/song.mp3
            value = value[1:]
    path = Path(value).expanduser()
    return path if path.is_absolute() else HOME / path


def probe(path: Path) -> dict:
    """Duration, tags, and whether the file has a real video track (not just cover art)."""
    try:
        result = subprocess.run(["ffprobe", "-v", "error", "-show_entries", "stream=codec_type:stream_disposition=attached_pic:format=duration:format_tags", "-of", "json", str(path)], capture_output=True, text=True, encoding="utf-8", errors="replace", check=False, timeout=60)
        data = json.loads(result.stdout or "{}")
    except (OSError, subprocess.TimeoutExpired, ValueError):
        data = {}
    streams = data.get("streams") or []
    tags = {str(key).casefold(): str(value) for key, value in ((data.get("format") or {}).get("tags") or {}).items()}
    try:
        duration = float((data.get("format") or {}).get("duration") or 0)
    except (TypeError, ValueError):
        duration = 0.0
    video = any(stream.get("codec_type") == "video" and not (stream.get("disposition") or {}).get("attached_pic") for stream in streams)
    cover = any(stream.get("codec_type") == "video" and (stream.get("disposition") or {}).get("attached_pic") for stream in streams)
    audio = any(stream.get("codec_type") == "audio" for stream in streams)
    return {"duration": duration, "tags": tags, "video": video, "cover": cover, "audio": audio}


def guess_from_name(stem: str) -> tuple[str, str]:
    """(artist, title) read from a file name; artist is empty when the name does not say."""
    name = TRACK_NUMBER.sub("", stem.replace("_", " "))
    name = re.sub(r"\s+", " ", NOISE.sub("", name)).strip(" -")
    parts = [part.strip() for part in SEPARATOR.split(name) if part.strip()]
    if len(parts) < 2 and HAN.search(name) and name.count("-") == 1:
        # Chinese file names often use a bare hyphen: 歌手-歌名.
        parts = [part.strip() for part in name.split("-") if part.strip()]
    if len(parts) >= 2:
        return parts[0], " - ".join(parts[1:])
    return "", name or stem


def guess_language(text: str) -> str:
    if KANA.search(text):
        return "ja"
    if HANGUL.search(text):
        return "ko"
    if HAN.search(text):
        return "zh"
    return "en"


def ascii_name(text: str) -> str:
    """The text in Latin letters, romanized when it is written in another script."""
    if re.search(r"[A-Za-z0-9]", text) and not (HAN.search(text) or KANA.search(text) or HANGUL.search(text)):
        return text
    from .romanize import romanize

    try:
        return romanize(text, guess_language(text), {"enabled": True, "languages": "auto", "tone": "none"}) or text
    except Exception:
        return text


def new_id(title: str, taken: set[str]) -> str:
    base = re.sub(r"[^a-z0-9]+", "-", ascii_name(title).casefold()).strip("-")[:40] or "song"
    candidate, number = base, 2
    while candidate in taken:
        candidate, number = f"{base}-{number}", number + 1
    return candidate


def describe(path: Path, taken: set[str]) -> dict[str, str]:
    """A songs.csv row for a media file."""
    info = probe(path)
    artist, title = guess_from_name(path.stem)
    title = info["tags"].get("title", "").strip() or title
    artist = info["tags"].get("artist", "").strip() or info["tags"].get("album_artist", "").strip() or artist
    try:
        url = str(path.relative_to(HOME))
    except ValueError:
        url = str(path)
    row = {name: "" for name in COLUMNS}
    row.update(id=new_id(f"{artist} {title}" if artist else title, taken), url=url, title=title, artist=artist, language=guess_language(title + artist), template="solo", prompt_ms="1000", align_model="auto")
    return row


def add_files(paths: list[Path]) -> list[dict[str, str]]:
    """Add media files to songs.csv; returns the rows that were added (files already listed are skipped)."""
    fields, songs = [], []
    if MANIFEST.exists():
        with MANIFEST.open(newline="", encoding="utf-8-sig") as handle:
            reader = csv.DictReader(handle)
            fields = list(reader.fieldnames or [])
            songs = [{key: (value or "") for key, value in row.items() if isinstance(key, str)} for row in reader]
    fields += [name for name in COLUMNS if name not in fields]
    fields += sorted(REQUIRED_COLUMNS - set(fields))
    listed = {local_path(song.get("url", "")).resolve() for song in songs if is_local(song.get("url", ""))}
    taken = {song.get("id", "") for song in songs}
    added = []
    for path in paths:
        path = Path(path).expanduser().resolve()
        if not path.is_file() or path in listed:
            continue
        row = describe(path, taken)
        taken.add(row["id"])
        listed.add(path)
        songs.append(row)
        added.append(row)
    if added:
        MANIFEST.parent.mkdir(parents=True, exist_ok=True)
        partial = MANIFEST.with_name(MANIFEST.name + ".tmp")
        with partial.open("w", newline="", encoding="utf-8") as handle:
            writer = csv.DictWriter(handle, fieldnames=fields, extrasaction="ignore")
            writer.writeheader()
            for song in songs:
                writer.writerow({name: song.get(name, "") for name in fields})
        os.replace(partial, MANIFEST)
    return added


def bring_in(row: dict[str, str], raw: Path) -> Path:
    """Place the song's local file in work/raw as <id><suffix>, linked when possible, copied otherwise."""
    source = local_path(row["url"])
    if not source.is_file():
        raise SongError(f"media file not found: {source}")
    suffix = source.suffix.lower()
    if suffix not in MEDIA_SUFFIXES:
        raise SongError(f"{source.name} is not a supported media file ({', '.join(MEDIA_SUFFIXES)})")
    raw.mkdir(parents=True, exist_ok=True)
    target = raw / f"{row['id']}{suffix}"
    if target.exists():
        target.unlink()
    try:
        os.link(source, target)
    except OSError:
        shutil.copy2(source, target)
    info = probe(target)
    # The lyrics search compares record lengths with this, as it does with yt-dlp's info JSON.
    (raw / f"{row['id']}.info.json").write_text(json.dumps({"duration": info["duration"], "title": row.get("title", ""), "local_file": str(source)}, ensure_ascii=False) + "\n", encoding="utf-8")
    return target
