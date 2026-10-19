from __future__ import annotations

import json
import re
from difflib import SequenceMatcher
from pathlib import Path
from .config import ALIGN, SUBTITLES, TIMECODE, WORDCODE, Entry, SPEAKER_TAG
from .manifest import resolve

# Scripts written without spaces: kana, CJK ideographs, and Hangul are timed per character.
CJK = "぀-ヿ㐀-䶿一-鿿豈-﫿가-힯"
TOKEN = re.compile(rf"[{CJK}]|[^\s{CJK}]+")
OFFSET_TAG = re.compile(r"^\s*\[offset\s*:\s*([+-]?\d+)\s*\]", re.IGNORECASE)
CUE_TIME = re.compile(r"((?:\d{1,2}:)?\d{1,2}:\d{2}[,.]\d{1,3})\s*-->\s*((?:\d{1,2}:)?\d{1,2}:\d{2}[,.]\d{1,3})")
# Estimated duration of a token nobody recognised, and the longest one may be stretched.
TYPICAL_TOKEN_MS = 400
MAX_TOKEN_MS = 1200
MIN_SPREAD_MS = 80
Unit = tuple[int, int, str]


def word_json(row: dict[str, str]) -> Path | None:
    configured = row.get("word_timing_file", "").strip()
    if configured:
        path = resolve(configured)
        if path.exists():
            return path
        print(f"WARN {row['id']}: word_timing_file not found ({path}); using automatic alignment")
    exact = ALIGN / f"{row['id']}.json"
    if exact.exists():
        return exact
    candidates = sorted(path for path in ALIGN.rglob("*.json") if path.name.startswith(f"{row['id']}.") or path.parent.name == row["id"])
    return candidates[0] if candidates else None


def token_key(token: str) -> str:
    """Comparison form of a token: case-folded with punctuation removed."""
    return re.sub(r"[\W_]+", "", token.casefold())


def tokenise(text: str) -> list[str]:
    """Split a lyric line into karaoke syllables.

    Tokens keep their punctuation and one trailing space where the source had
    whitespace, so joining them with "" reproduces the line.
    """
    tokens: list[str] = []
    prefix = ""
    for match in TOKEN.finditer(text):
        piece = match.group() + (" " if text[match.end():match.end() + 1].isspace() else "")
        if token_key(piece):
            tokens.append(prefix + piece)
            prefix = ""
        elif tokens:
            tokens[-1] += piece
        else:
            prefix += piece
    if prefix:
        tokens.append(prefix)
    if tokens:
        tokens[-1] = tokens[-1].rstrip()
    return tokens


def split_speaker(text: str) -> tuple[str, str]:
    match = SPEAKER_TAG.match(text)
    return (match.group(1).strip(), text[match.end():].strip()) if match else ("", text.strip())


def timed_units(data: dict) -> list[Unit]:
    """Flatten WhisperX/Faster-Whisper JSON into (start_ms, end_ms, key) units, one per lyric token."""
    words = [word for segment in data.get("segments") or [] for word in segment.get("words") or segment.get("word_segments") or []]
    if not words:
        words = data.get("word_segments") or []
    units: list[Unit] = []
    for word in words:
        if word.get("start") is None or word.get("end") is None:
            continue
        start, end = int(float(word["start"]) * 1000), int(float(word["end"]) * 1000)
        keys = [key for key in (token_key(token) for token in tokenise(str(word.get("word", "")))) if key]
        span = max(0, end - start)
        # A recognised CJK "word" covers several lyric characters; share its time between them.
        for index, key in enumerate(keys):
            units.append((start + span * index // len(keys), start + span * (index + 1) // len(keys), key))
    units.sort(key=lambda unit: unit[:2])
    return units


def load_units(path: Path) -> list[Unit]:
    units = timed_units(json.loads(path.read_text(encoding="utf-8")))
    if not units:
        raise ValueError(f"No word timings found in {path}")
    return units


def match_tokens(keys: list[str], units: list[Unit]) -> list[tuple[int, int] | None]:
    """Give each lyric token the time of the recognised word it corresponds to, or None."""
    times: list[tuple[int, int] | None] = [None] * len(keys)
    if not keys or not units:
        return times
    matcher = SequenceMatcher(None, keys, [unit[2] for unit in units], autojunk=False)
    for tag, i1, i2, j1, j2 in matcher.get_opcodes():
        count = i2 - i1
        if tag == "equal" or (tag == "replace" and count == j2 - j1):
            # Equal-length mismatches are misheard words or homophones in the same position.
            for offset in range(count):
                times[i1 + offset] = units[j1 + offset][:2]
        elif tag == "replace":
            low, high = units[j1][0], units[j2 - 1][1]
            if MIN_SPREAD_MS * count <= high - low <= MAX_TOKEN_MS * count:
                for offset in range(count):
                    times[i1 + offset] = (low + (high - low) * offset // count, low + (high - low) * (offset + 1) // count)
    return times


def fill_gaps(times: list[tuple[int, int] | None], low: int, high: int) -> list[tuple[int, int]]:
    """Estimate unmatched tokens from their matched neighbours and keep the sequence monotonic."""
    filled: list[tuple[int, int] | None] = list(times)
    index = 0
    while index < len(filled):
        if filled[index] is not None:
            index += 1
            continue
        stop = index
        while stop < len(filled) and filled[stop] is None:
            stop += 1
        count = stop - index
        left = filled[index - 1][1] if index else None
        right = filled[stop][0] if stop < len(filled) else None
        if left is None and right is None:
            left, right = low, min(high, low + count * MAX_TOKEN_MS)
        elif left is None:
            left = max(low, right - count * TYPICAL_TOKEN_MS)
        elif right is None:
            right = min(high, left + count * TYPICAL_TOKEN_MS)
        else:
            right = min(right, left + count * MAX_TOKEN_MS)
        right = max(left, right)
        for offset in range(count):
            filled[index + offset] = (left + (right - left) * offset // count, left + (right - left) * (offset + 1) // count)
        index = stop
    result, cursor = [], low
    for left, right in filled:
        left = min(max(left, cursor), high)
        right = min(max(right, left), high)
        result.append((left, right))
        cursor = right
    return result


def time_entries(entries: list[Entry], units: list[Unit], max_tail_ms: int = 0, tail_hold_ms: int = 1000, estimated: list[int] | None = None) -> list[Entry]:
    """Attach word timing to each line by matching lyric tokens against recognised words.

    Lines that already carry word timing (enhanced LRC) are kept as they are.
    Start times of lines where nothing was recognised are appended to `estimated`.
    """
    result, used = [], 0
    for start, end, text, words in entries:
        if words:
            result.append((start, end, text, words))
            continue
        tokens = tokenise(split_speaker(text)[1])
        if not tokens:
            result.append((start, end, text, []))
            continue
        first = used
        while first < len(units) and units[first][1] < start:
            first += 1
        last = first
        while last < len(units) and units[last][0] <= end:
            last += 1
        used = max(used, last)
        times = match_tokens([token_key(token) for token in tokens], units[first:last])
        recognised = any(item is not None for item in times)
        if not recognised and estimated is not None:
            estimated.append(start)
        timed = [(left, right, token) for (left, right), token in zip(fill_gaps(times, start, end), tokens)]
        # A line that runs into a long instrumental should leave the screen after its last word.
        if recognised and max_tail_ms > 0 and end - timed[-1][1] > max_tail_ms:
            end = max(start + 300, timed[-1][1] + max(0, tail_hold_ms))
        result.append((start, end, text, timed))
    return result


def apply_word_timing(entries: list[Entry], path: Path, max_tail_ms: int = 0, tail_hold_ms: int = 1000, estimated: list[int] | None = None) -> list[Entry]:
    return time_entries(entries, load_units(path), max_tail_ms, tail_hold_ms, estimated)


def plain_entries(text: str, units: list[Unit]) -> list[Entry]:
    """Time untimed lyric lines by aligning the whole text against the recognised words."""
    lines = [tokens for tokens in (tokenise(line.strip()) for line in text.splitlines()) if tokens]
    keys = [token_key(token) for tokens in lines for token in tokens]
    if not keys or not units:
        return []
    times = fill_gaps(match_tokens(keys, units), units[0][0], units[-1][1])
    result, cursor = [], 0
    for tokens in lines:
        timed = [(left, right, token) for (left, right), token in zip(times[cursor:cursor + len(tokens)], tokens)]
        cursor += len(tokens)
        start = timed[0][0]
        result.append((start, max(start + 300, timed[-1][1]), "".join(tokens), timed))
    return result


def plain_lyrics_entries(path: Path, timing_path: Path) -> list[Entry]:
    return plain_entries(path.read_text(encoding="utf-8"), load_units(timing_path))


def ms(match: re.Match[str]) -> int:
    fraction = int(((match.group(3) or "") + "000")[:3])
    return (int(match.group(1)) * 60 + int(match.group(2))) * 1000 + fraction


def inline_words(lyric: str, start: int, end: int) -> tuple[str, list[tuple[int, int, str]]]:
    """Split an enhanced-LRC line into its plain text and (start, end, word) timings."""
    marks = list(WORDCODE.finditer(lyric))
    if not marks:
        return lyric.strip(), []
    pieces = [(start, lyric[:marks[0].start()])]
    for index, mark in enumerate(marks):
        stop = marks[index + 1].start() if index + 1 < len(marks) else len(lyric)
        pieces.append((ms(mark), lyric[mark.end():stop]))
    words = []
    for index, (left, piece) in enumerate(pieces):
        if not piece.strip():
            continue
        following = pieces[index + 1] if index + 1 < len(pieces) else None
        right = following[0] if following else end
        spaced = piece[-1].isspace() or bool(following and following[1][:1].isspace())
        words.append((max(start, left), max(left + 10, right), piece.strip() + (" " if spaced else "")))
    if words:
        words[-1] = (*words[-1][:2], words[-1][2].rstrip())
    return "".join(word for _, _, word in words), words


def parse_lrc(content: str) -> list[Entry]:
    offset, items = 0, []
    for line in content.splitlines():
        line = line.strip()
        tag = OFFSET_TAG.match(line)
        if tag:
            # A positive LRC offset makes the lyrics appear earlier.
            offset = -int(tag.group(1))
            continue
        marks, position = [], 0
        while True:
            mark = TIMECODE.match(line, position)
            if not mark:
                break
            marks.append(mark)
            position = mark.end()
            position += len(line[position:]) - len(line[position:].lstrip())
        if not marks:
            continue
        # A timestamp without text marks where the previous line ends.
        items.extend((ms(mark), line[position:].strip()) for mark in marks)
    items.sort(key=lambda item: item[0])
    result = []
    for index, (start, raw) in enumerate(items):
        speaker, lyric = split_speaker(raw)
        if not lyric:
            continue
        # Lines sharing a timestamp (original plus translation) end together at the next later mark.
        end = next((time for time, _ in items[index + 1:] if time > start), start + max(2000, len(lyric) * 180))
        end = max(start + 300, end)
        clean, words = inline_words(lyric, start, end)
        if not clean:
            continue
        text = f"[speaker:{speaker}] {clean}" if speaker else clean
        shift = max(-start, offset)
        result.append((start + shift, end + shift, text, [(left + shift, right + shift, word) for left, right, word in words]))
    return result


def lrc(path: Path) -> list[Entry]:
    return parse_lrc(path.read_text(encoding="utf-8-sig"))


def subtitle_time(value: str) -> int:
    parts = value.replace(",", ".").split(":")
    hours, minutes, seconds = (0, *parts) if len(parts) == 2 else parts
    whole, _, fraction = seconds.partition(".")
    return (int(hours) * 3600 + int(minutes) * 60 + int(whole)) * 1000 + int((fraction + "000")[:3])


def parse_timed_subtitle(content: str) -> list[Entry]:
    lines, result, index, previous = content.splitlines(), [], 0, ""
    while index < len(lines):
        match = CUE_TIME.search(lines[index])
        if not match:
            index += 1
            continue
        start, end, index = subtitle_time(match.group(1)), subtitle_time(match.group(2)), index + 1
        text_lines = []
        # YouTube captions contain whitespace-only lines inside a cue, so only a truly
        # empty line, or a blank one followed by the next cue, ends the text block.
        while index < len(lines) and lines[index] != "" and not (not lines[index].strip() and any(CUE_TIME.search(line) for line in lines[index + 1:index + 3])):
            cleaned = re.sub(r"<[^>]+>|\{[^}]+\}|[♪♫♬]", "", lines[index]).strip()
            if cleaned:
                text_lines.append(cleaned)
            index += 1
        # YouTube automatic captions repeat the previous line above each new one and
        # insert near-zero-length cues that only restate it.
        if end - start <= 50:
            continue
        if len(text_lines) > 1 and text_lines[0] == previous:
            text_lines = text_lines[1:]
        if not text_lines:
            continue
        previous = text_lines[-1]
        text = " ".join(text_lines)
        speaker, lyric_text = split_speaker(text)
        # Skip sound descriptions such as [Music] or [Applause].
        if lyric_text and not re.fullmatch(r"\[[^\]]*\]", lyric_text):
            tagged = f"[speaker:{speaker}] {lyric_text}" if speaker else lyric_text
            result.append((start, max(start + 300, end), tagged, []))
    return result


def timed_subtitle(path: Path) -> list[Entry]:
    return parse_timed_subtitle(path.read_text(encoding="utf-8-sig"))


def downloaded_subtitle(row: dict[str, str], auto: bool = False) -> Path | None:
    """Return the best downloaded subtitle: creator-made by default, automatic captions with auto=True."""
    candidates = [path for path in sorted(SUBTITLES.glob(f"{row['id']}.*")) if path.suffix.lower() in {".vtt", ".srt"} and (".auto." in path.name.lower()) == auto]
    if not candidates:
        return None
    language = row.get("language", "").strip().lower()
    preferred = [item.strip().lower().rstrip("*.") for item in row.get("subtitle_langs", "").split(",") if item.strip() and item.strip().lower() != "all"]

    def tag(path: Path) -> str:
        return path.name.lower()[len(row["id"]) + 1:].removeprefix("auto.").rsplit(".", 1)[0]

    return sorted(candidates, key=lambda path: (0 if language and tag(path).startswith(language) else 1, 0 if any(item and tag(path).startswith(item) for item in preferred) else 1, path.name))[0]
