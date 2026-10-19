"""Readers for word-timed karaoke lyric formats (plain-text forms only).

KSC  karaoke.add('mm:ss.xxx', 'mm:ss.xxx', 'text', 'd1,d2,...');
KRC  [line_start,line_dur]<offset,dur,0>word<offset,dur,0>word...
QRC  [line_start,line_dur]word(start,dur)word(start,dur)...  (optionally inside XML LyricContent="...")
"""
from __future__ import annotations

import html
import re
from pathlib import Path

from .config import Entry

CJK = re.compile(r"[　-鿿぀-ヿ가-힯豈-﫿]")
KSC_ADD = re.compile(r"karaoke\.add\(\s*'([^']*)'\s*,\s*'([^']*)'\s*,\s*'((?:[^'\\]|\\.|'')*)'\s*,\s*'([^']*)'\s*\)", re.IGNORECASE)
KSC_TIME = re.compile(r"(?:(\d+):)?(\d+):(\d+)(?:\.(\d{1,3}))?")
KRC_LINE = re.compile(r"^\[(\d+),(\d+)\](.*)$")
KRC_WORD = re.compile(r"<(\d+),(\d+),\d+>([^<]*)")
QRC_WORD = re.compile(r"([^()]*)\((\d+),(\d+)\)")
QRC_CONTENT = re.compile(r'LyricContent="([^"]*)"', re.DOTALL)


def read_text(path: Path) -> str:
    data = path.read_bytes()
    if data[:2] in (b"\xff\xfe", b"\xfe\xff"):
        return data.decode("utf-16")
    if b"\0" in data[:4096]:
        raise SystemExit(f"{path} is a binary or encrypted lyric file; export it as plain text first")
    for encoding in ("utf-8-sig", "gb18030"):
        try:
            return data.decode(encoding)
        except UnicodeDecodeError:
            continue
    raise SystemExit(f"{path}: unrecognised text encoding")


def join_words(words: list[str]) -> str:
    return "".join(words) if any(CJK.search(word) for word in words) else " ".join(word.strip() for word in words)


def entry(words: list[tuple[int, int, str]], start: int | None = None, end: int | None = None) -> Entry | None:
    words = [(left, max(left + 10, right), text.strip()) for left, right, text in words if text.strip()]
    if not words:
        return None
    start = words[0][0] if start is None else start
    end = words[-1][1] if end is None else end
    return (start, max(start + 300, end), join_words([text for _, _, text in words]), words)


def ksc_time(value: str) -> int:
    match = KSC_TIME.fullmatch(value.strip())
    if not match:
        raise ValueError(f"bad KSC time {value!r}")
    hours, minutes, seconds, fraction = match.groups()
    return ((int(hours or 0) * 60 + int(minutes)) * 60 + int(seconds)) * 1000 + int(((fraction or "0") + "00")[:3])


def ksc_units(text: str, count: int) -> list[str]:
    """Split KSC text into timing units: [bracketed] groups, else one CJK char or one Latin word."""
    units, buffer = [], ""
    for token in re.findall(r"\[[^\]]*\]|\s+|.", text):
        if token.startswith("[") and token.endswith("]") and len(token) > 1:
            units.append(token[1:-1])
        elif token.isspace():
            if buffer:
                units.append(buffer)
                buffer = ""
        elif CJK.match(token):
            if buffer:
                units.append(buffer)
                buffer = ""
            units.append(token)
        else:
            buffer += token
    if buffer:
        units.append(buffer)
    if len(units) != count:
        chars = [char for char in text.replace("[", "").replace("]", "") if not char.isspace()]
        if len(chars) == count:
            return chars
    return units


def ksc(path: Path) -> list[Entry]:
    result = []
    for match in KSC_ADD.finditer(read_text(path)):
        start, end = ksc_time(match.group(1)), ksc_time(match.group(2))
        text = match.group(3).replace("''", "'").replace("\\'", "'")
        durations = [int(item) for item in re.findall(r"-?\d+", match.group(4))]
        units = ksc_units(text, len(durations))
        if len(units) != len(durations):
            print(f"WARN {path.name}: {len(units)} units vs {len(durations)} durations at {match.group(1)}; spreading evenly")
            durations = [max(10, (end - start) // max(1, len(units)))] * len(units)
        cursor, words = start, []
        for unit, duration in zip(units, durations):
            words.append((cursor, cursor + max(0, duration), unit))
            cursor += max(0, duration)
        item = entry(words, start, end)
        if item:
            result.append(item)
    return result


def krc(path: Path) -> list[Entry]:
    result = []
    for line in read_text(path).splitlines():
        match = KRC_LINE.match(line.strip())
        if not match:
            continue
        start, duration = int(match.group(1)), int(match.group(2))
        words = [(start + int(offset), start + int(offset) + int(length), text) for offset, length, text in KRC_WORD.findall(match.group(3))]
        item = entry(words, start, start + duration)
        if item:
            result.append(item)
    return result


def qrc(path: Path) -> list[Entry]:
    text = read_text(path)
    content = QRC_CONTENT.search(text)
    if content:
        text = html.unescape(content.group(1))
    result = []
    for line in text.splitlines():
        match = KRC_LINE.match(line.strip())
        if not match:
            continue
        start, duration = int(match.group(1)), int(match.group(2))
        words = [(int(left), int(left) + int(length), word) for word, left, length in QRC_WORD.findall(match.group(3))]
        item = entry(words, start, start + duration)
        if item:
            result.append(item)
    return result


READERS = {".ksc": ksc, ".krc": krc, ".qrc": qrc}
