"""The engine behind the desktop app: lyrics, readings, settings and the song list, over JSON lines.

The desktop app (desktop/, written in Rust) starts ``polykara-cli engine``
once and keeps it running. Each request is one JSON line on stdin,
{"id": 1, "cmd": "load", "args": {...}}, and each answer one JSON line on
stdout, {"id": 1, "ok": true, "result": ...} or {"id": 1, "ok": false,
"error": "..."}. Anything else the pipeline prints goes to stderr, so it can
never break an answer. Everything about lyrics, word timing files and
readings stays in Python, where the pipeline uses the same code.
"""
from __future__ import annotations

import csv
import functools
import json
import os
import shutil
import subprocess
import sys
import tomllib
from pathlib import Path

from . import __version__, readings, settingsfile, timing
from .config import ASS, AUDIO, CONFIG_FILE, EXTERNAL_LYRICS, HOME, MANIFEST, MEDIA_SUFFIXES, OUTPUT, RAW, load_config
from .localmedia import add_files, describe, is_local, local_path, new_id, probe, relative_to_home
from .manifest import REQUIRED_COLUMNS
from .romanize import base_language, ruby_groups
from .timing import Line, Token

COLUMNS = ["id", "url", "title", "artist", "lyricist", "composer", "producer", "template", "language", "subtitle_langs", "lyrics_file", "word_timing_file", "logo_file", "prompt_ms", "align_model", "lyric_sources", "lyric_pages", "voice"]
LIBRARY_NAMES = {"zh": "pinyin", "yue": "Jyutping", "ja": "romaji"}
LIBRARY_HINTS = {"zh": "one syllable per character: 重来 = chóng lái (or chong2 lai2)", "yue": "one syllable per character: 銀行 = ngan4 hong4", "ja": "the whole word in kana or romaji: 君 = きみ"}
NO_WINDOW = getattr(subprocess, "CREATE_NO_WINDOW", 0)


# Song list
def read_songs() -> tuple[list[str], list[dict[str, str]]]:
    if not MANIFEST.exists():
        return list(COLUMNS), []
    with MANIFEST.open(newline="", encoding="utf-8-sig") as handle:
        reader = csv.DictReader(handle)
        fields = list(reader.fieldnames or [])
        songs = [{key: (value or "") for key, value in row.items() if isinstance(key, str)} for row in reader]
    fields += [name for name in COLUMNS if name not in fields]
    return fields, songs


def write_songs(fields: list[str], songs: list[dict[str, str]]) -> None:
    fields = list(fields) + sorted(REQUIRED_COLUMNS - set(fields))
    MANIFEST.parent.mkdir(parents=True, exist_ok=True)
    partial = MANIFEST.with_name(MANIFEST.name + ".tmp")
    with partial.open("w", newline="", encoding="utf-8") as handle:
        writer = csv.DictWriter(handle, fieldnames=fields, extrasaction="ignore")
        writer.writeheader()
        for song in songs:
            writer.writerow({name: song.get(name, "") for name in fields})
    os.replace(partial, MANIFEST)


def find_row(sid: str) -> dict[str, str]:
    row = next((song for song in read_songs()[1] if song.get("id") == sid), None)
    if row is None:
        raise ValueError(f"{sid} is not in songs.csv")
    return row


# Lines travel as {"speaker", "start", "end", "tokens": [{"text", "start", "end"}]}.
def line_out(line: Line) -> dict:
    return {"speaker": line.speaker, "start": line.start, "end": line.end, "tokens": [{"text": token.text, "start": token.start, "end": token.end} for token in line.tokens]}


def line_in(data: dict) -> Line:
    return Line([Token(token["text"], token.get("start"), token.get("end")) for token in data["tokens"]], data.get("speaker", ""), data.get("start"), data.get("end"))


@functools.lru_cache(maxsize=4096)
def token_readings(tokens: tuple[str, ...], language: str, stamp: tuple) -> list[str | None]:
    """The reading above each token: a string over the first token of a group, "" over the rest, None for no reading."""
    settings = dict(load_config()["romanization"])
    try:
        groups = ruby_groups([(0, 0, token) for token in tokens], language, settings) or []
    except Exception:
        groups = []
    shown: list[str | None] = []
    for group in groups:
        shown.append(group["roman"])
        shown.extend([""] * (len(group["tokens"]) - 1))
    # Tokens that were only spaces or punctuation have no group.
    return shown if len(shown) == len(tokens) else [None] * len(tokens)


def library_stamp(language: str) -> tuple:
    return tuple(readings._stamp(path) for path in (readings.BUILTIN / f"{language}.txt", readings.user_file(language)))


def ass_colour(value: str) -> str | None:
    return settingsfile.ass_to_hex(str(value))


# Commands
def hello(_args: dict) -> dict:
    tools = {name: shutil.which(name) for name in ("ffmpeg", "ffprobe", "ffplay", "yt-dlp")}
    return {
        "version": __version__, "home": str(HOME), "manifest": str(MANIFEST), "config": str(CONFIG_FILE),
        "folders": {"raw": str(RAW), "audio": str(AUDIO), "ass": str(ASS), "output": str(OUTPUT), "lyrics": str(EXTERNAL_LYRICS)},
        "media_suffixes": list(MEDIA_SUFFIXES), "tools": tools, "path": os.environ.get("PATH", ""),
    }


def songs(_args: dict) -> dict:
    fields, rows = read_songs()
    return {"fields": fields, "songs": rows}


def save_songs(args: dict) -> dict:
    write_songs(args["fields"], args["songs"])
    return songs({})


def describe_file(args: dict) -> dict:
    return describe(Path(args["path"]), set(args.get("taken", [])))


def add_media(args: dict) -> dict:
    added = add_files([Path(item) for item in args["paths"]])
    return {"added": [row["id"] for row in added], "skipped": len(args["paths"]) - len(added), **songs({})}


def make_id(args: dict) -> dict:
    return {"id": new_id(args.get("title", ""), set(args.get("taken", [])))}


def relative(args: dict) -> dict:
    return {"path": relative_to_home(Path(args["path"]))}


def load(args: dict) -> dict:
    row = find_row(args["id"])
    sid = row["id"]
    try:
        lines, label = timing.load(row)
        problem = ""
    except Exception as exc:  # a broken lyric file must not stop the editor
        lines, label, problem = [], "", f"The lyrics of {sid} could not be read ({exc}). Paste them with Lyrics text…."
    media = timing.media_for(sid)
    if media is None and is_local(row.get("url", "")) and local_path(row["url"]).is_file():
        media = local_path(row["url"])
    karaoke = load_config().get("karaoke", {})
    return {
        "row": row, "lines": [line_out(line) for line in lines], "label": label, "problem": problem,
        "media": str(media) if media else None, "language": base_language(row.get("language", "") or ""),
        "sung_colour": ass_colour(karaoke.get("active_color", "")) or "#ff0000",
        "edited_ass": str(ASS / f"{sid}.edited.ass"), "library": base_language(row.get("language", "") or "") in readings.LANGUAGES,
    }


def prepare_audio(args: dict) -> dict:
    """Extract the audio once (exact seeking in video files), then read its duration."""
    sid, media = args["id"], Path(args["media"])
    audio = AUDIO / f"{sid}.flac"
    if media != audio and probe(media)["video"]:
        AUDIO.mkdir(parents=True, exist_ok=True)
        partial = AUDIO / f"{sid}.partial.flac"
        subprocess.run(["ffmpeg", "-hide_banner", "-loglevel", "error", "-y", "-i", str(media), "-vn", "-ac", "2", "-ar", "48000", "-c:a", "flac", str(partial)], check=False, timeout=900, stdin=subprocess.DEVNULL, creationflags=NO_WINDOW)
        if partial.exists() and partial.stat().st_size:
            os.replace(partial, audio)
            media = audio
        partial.unlink(missing_ok=True)
    return {"media": str(media), "duration": int(probe(media)["duration"] * 1000)}


def save(args: dict) -> dict:
    path, estimated = timing.save(find_row(args["id"]), [line_in(item) for item in args["lines"]])
    return {"path": str(path), "name": path.name, "estimated": estimated}


def retext(args: dict) -> dict:
    return {"lines": [line_out(line) for line in timing.retext([line_in(item) for item in args["lines"]], args["text"])]}


def lines_text(args: dict) -> dict:
    return {"text": timing.lines_to_text([line_in(item) for item in args["lines"]])}


def line_readings(args: dict) -> dict:
    language = args["language"]
    stamp = library_stamp(language)
    return {"readings": [token_readings(tuple(tokens), language, stamp) for tokens in args["lines"]], "stamp": list(stamp)}


def library(args: dict) -> dict:
    language = args["language"]
    if language not in readings.LANGUAGES:
        raise ValueError("The word library is for songs in Mandarin (zh), Cantonese (yue) and Japanese (ja).")
    path, builtin = readings.user_file(language), readings.BUILTIN / f"{language}.txt"
    name = LIBRARY_NAMES[language]
    return {
        "name": name, "hint": LIBRARY_HINTS[language], "user_file": str(path),
        "text": path.read_text(encoding="utf-8-sig") if path.exists() else f"# Your {name} readings: one word per line, the word then its reading.\n",
        "builtin_count": len(readings.parse(builtin.read_text(encoding="utf-8"), language)[0]) if builtin.exists() else 0,
    }


def save_library(args: dict) -> dict:
    language, content = args["language"], args["text"]
    problems = readings.parse(content, language)[1]
    path = readings.user_file(language)
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(content.rstrip("\n") + "\n", encoding="utf-8")
    return {"problems": problems}


def settings_form(args: dict) -> dict:
    text = args["text"] if "text" in args else (CONFIG_FILE.read_text(encoding="utf-8") if CONFIG_FILE.exists() else "")
    try:
        return {"text": text, "tabs": settingsfile.form(text), "error": ""}
    except tomllib.TOMLDecodeError as exc:
        return {"text": text, "tabs": [], "error": f"polykara.toml has a mistake, so only the file can be edited until it is fixed: {exc}"}


def settings_apply(args: dict) -> dict:
    return {"text": settingsfile.apply_changes(args["text"], args.get("changes", []))}


def settings_save(args: dict) -> dict:
    text = settingsfile.apply_changes(args["text"], args.get("changes", []))
    tomllib.loads(text)
    CONFIG_FILE.parent.mkdir(parents=True, exist_ok=True)
    partial = CONFIG_FILE.with_name(CONFIG_FILE.name + ".tmp")
    partial.write_text(text, encoding="utf-8")
    os.replace(partial, CONFIG_FILE)
    return settings_form({"text": text})


def fonts(_args: dict) -> dict:
    """Font family names installed on this computer."""
    names: set[str] = set()
    if os.name == "nt":
        import winreg

        for root, key in ((winreg.HKEY_LOCAL_MACHINE, r"SOFTWARE\Microsoft\Windows NT\CurrentVersion\Fonts"), (winreg.HKEY_CURRENT_USER, r"SOFTWARE\Microsoft\Windows NT\CurrentVersion\Fonts")):
            try:
                with winreg.OpenKey(root, key) as handle:
                    index = 0
                    while True:
                        try:
                            name = winreg.EnumValue(handle, index)[0]
                        except OSError:
                            break
                        index += 1
                        name = name.split(" (")[0]
                        for part in name.split(" & "):
                            for style in (" Bold Italic", " Bold", " Italic", " Light", " Semibold", " Black", " Regular"):
                                part = part.removesuffix(style)
                            names.add(part.strip())
            except OSError:
                pass
    elif shutil.which("fc-list"):
        result = subprocess.run(["fc-list", ":", "family"], capture_output=True, text=True, check=False)
        names = {family.strip() for line in result.stdout.splitlines() for family in line.split(",")[:1]}
    return {"fonts": sorted((name for name in names if name and not name.startswith("@")), key=str.casefold)}


COMMANDS = {
    "hello": hello, "songs": songs, "save_songs": save_songs, "describe": describe_file, "add_files": add_media, "new_id": make_id, "relative": relative,
    "load": load, "prepare_audio": prepare_audio, "save": save, "retext": retext, "lines_text": lines_text, "readings": line_readings,
    "library": library, "save_library": save_library, "settings_form": settings_form, "settings_apply": settings_apply, "settings_save": settings_save, "fonts": fonts,
}


def handle(request: dict) -> dict:
    answer = {"id": request.get("id")}
    try:
        command = COMMANDS[request["cmd"]]
        answer.update(ok=True, result=command(request.get("args") or {}))
    except Exception as exc:
        answer.update(ok=False, error=str(exc) or type(exc).__name__)
    return answer


def serve(source=None, sink=None) -> int:
    """Answer requests until stdin closes."""
    source = source or sys.stdin
    sink = sink or sys.stdout
    # Whatever the pipeline prints while answering goes to stderr, never between answers.
    sys.stdout = sys.stderr
    try:
        for raw in source:
            if not raw.strip():
                continue
            try:
                request = json.loads(raw)
            except json.JSONDecodeError as exc:
                answer = {"id": None, "ok": False, "error": f"not JSON: {exc}"}
            else:
                answer = handle(request)
            sink.write(json.dumps(answer, ensure_ascii=False) + "\n")
            sink.flush()
    finally:
        sys.stdout = sink
    return 0
