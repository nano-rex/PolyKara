"""Word timing made or corrected by hand in the timing editor.

The editor saves an enhanced LRC file, work/lyrics/<id>.timed.lrc, with a
time mark before every word (or CJK character). When that file exists it is
the song's lyric source ahead of everything else, and because it already
carries word timing, no automatic alignment is needed for it.

This module has no window code, so it can be tested without a display.
"""
from __future__ import annotations

import array
import copy
import re
import subprocess
import threading
import time
from dataclasses import dataclass, field
from difflib import SequenceMatcher
from pathlib import Path

from .config import AUDIO, EXTERNAL_LYRICS, SPEAKER_TAG
from .manifest import find_source
from .subtitle import fill_gaps, load_units, parse_lrc, plain_entries, split_speaker, time_entries, tokenise, word_json

NO_WINDOW = getattr(subprocess, "CREATE_NO_WINDOW", 0)
# Untimed lines placed after the last timed one are spaced this far apart.
ESTIMATED_LINE_MS = 3000
MIN_TOKEN_MS = 30


@dataclass
class Token:
    text: str
    start: int | None = None
    end: int | None = None

    @property
    def timed(self) -> bool:
        return self.start is not None and self.end is not None


@dataclass
class Line:
    tokens: list[Token]
    speaker: str = ""
    # Line times for lines whose words are not timed yet (from subtitles or synced lyrics).
    start: int | None = None
    end: int | None = None

    @property
    def text(self) -> str:
        return "".join(token.text for token in self.tokens)

    @property
    def timed(self) -> bool:
        return bool(self.tokens) and all(token.timed for token in self.tokens)

    def span(self) -> tuple[int, int] | None:
        """(start, end) from the timed words, else from the line times."""
        times = [(token.start, token.end) for token in self.tokens if token.timed]
        if times:
            return min(start for start, _ in times), max(end for _, end in times)
        if self.start is not None:
            return self.start, self.end if self.end is not None else self.start + 2000
        return None


def edited_path(sid: str) -> Path:
    return EXTERNAL_LYRICS / f"{sid}.timed.lrc"


def media_for(sid: str) -> Path | None:
    """The file to play: the normalized audio when it is still there, else the downloaded media."""
    audio = AUDIO / f"{sid}.flac"
    return audio if audio.exists() else find_source(sid)


def lines_from_entries(entries) -> list[Line]:
    lines = []
    for start, end, text, words in entries:
        speaker, lyric = split_speaker(text)
        if words:
            tokens = [Token(word, left, right) for left, right, word in words]
            # Keep a speaker tag that was timed as part of the first word out of the lyric (and only the tag:
            # the space after an English first word separates it from the next).
            tag = SPEAKER_TAG.match(tokens[0].text)
            if tag and tokens[0].text[tag.end():]:
                tokens[0].text = tokens[0].text[tag.end():]
        else:
            tokens = [Token(piece) for piece in tokenise(lyric)]
        if tokens:
            lines.append(Line(tokens, speaker, start, end))
    return lines


def lines_from_text(text: str) -> list[Line]:
    """Untimed lines from lyric text, one per non-empty line; a [singer: x] tag at the start is kept."""
    lines = []
    for raw in text.splitlines():
        speaker, lyric = split_speaker(raw.strip())
        tokens = [Token(piece) for piece in tokenise(lyric)]
        if tokens:
            lines.append(Line(tokens, speaker))
    return lines


def lines_to_text(lines: list[Line]) -> str:
    return "\n".join((f"[singer: {line.speaker}] " if line.speaker else "") + line.text for line in lines)


def retext(lines: list[Line], text: str) -> list[Line]:
    """Lines for edited lyric text, keeping the timing of every line whose words did not change.

    A changed line keeps its line times, and the timing of its words when it
    still has the same number of them (a corrected typo).
    """
    fresh = lines_from_text(text)
    matcher = SequenceMatcher(None, [line.text for line in lines], [line.text for line in fresh], autojunk=False)
    for tag, i1, i2, j1, j2 in matcher.get_opcodes():
        if tag == "equal":
            for offset in range(i2 - i1):
                old, new = lines[i1 + offset], fresh[j1 + offset]
                new.tokens, new.start, new.end = copy.deepcopy(old.tokens), old.start, old.end
        elif tag == "replace":
            # Pair each changed line with the most similar old line, in order.
            cursor = i1
            for new in fresh[j1:j2]:
                scores = [(SequenceMatcher(None, lines[index].text, new.text, autojunk=False).ratio(), index) for index in range(cursor, i2)]
                score, index = max(scores, default=(0.0, -1))
                if score < 0.5:
                    continue
                old, cursor = lines[index], index + 1
                new.start, new.end = old.start, old.end
                if len(old.tokens) == len(new.tokens):
                    for before, after in zip(old.tokens, new.tokens):
                        after.start, after.end = before.start, before.end
    return fresh


def load(row: dict[str, str]) -> tuple[list[Line], str]:
    """The song's lines with the best timing already known, and where they came from."""
    from .verify import check_lyrics

    sid = row["id"]
    edited = edited_path(sid)
    if edited.exists():
        return lines_from_entries(parse_lrc(edited.read_text(encoding="utf-8-sig"))), f"your saved timing ({edited.name})"
    primary = check_lyrics(row).primary
    if primary is None:
        return [], ""
    entries = primary.entries
    timing = word_json(row)
    if primary.kind == "plain":
        if timing is not None:
            entries = plain_entries(primary.path.read_text(encoding="utf-8-sig"), load_units(timing))
            if entries:
                return lines_from_entries(entries), f"{primary.label}, timed automatically"
        return lines_from_text(primary.path.read_text(encoding="utf-8-sig")), f"{primary.label} (not timed yet)"
    if timing is not None and any(not words for _, _, _, words in entries):
        return lines_from_entries(time_entries(entries, load_units(timing))), f"{primary.label}, timed automatically"
    return lines_from_entries(entries), primary.label


def finished(lines: list[Line]) -> tuple[list, int]:
    """Entries ready to save, and how many lines had no timing and were placed by estimate.

    Words left untimed inside a timed line are spread between their timed
    neighbours; a line with no times at all is placed in the gap between the
    lines around it.
    """
    spans = [line.span() for line in lines]
    entries, estimated = [], 0
    index = 0
    while index < len(lines):
        if spans[index] is not None:
            index += 1
            continue
        stop = index
        while stop < len(lines) and spans[stop] is None:
            stop += 1
        count = stop - index
        low = spans[index - 1][1] if index else 0
        high = spans[stop][0] if stop < len(lines) else low + ESTIMATED_LINE_MS * count
        step = max(300, (high - low) // count)
        for offset in range(count):
            begin = low + step * offset
            spans[index + offset] = (begin, begin + step)
        estimated += count
        index = stop
    for line, (start, end) in zip(lines, spans):
        times = fill_gaps([(token.start, token.end) if token.timed else None for token in line.tokens], start, max(end, start + MIN_TOKEN_MS * len(line.tokens)))
        words = [(left, max(right, left + 10), token.text) for (left, right), token in zip(times, line.tokens)]
        text = (f"[speaker:{line.speaker}] " if line.speaker else "") + line.text.strip()
        entries.append((words[0][0], max(words[-1][1], words[0][0] + 300), text, words))
    return entries, estimated


def stamp(value: int) -> str:
    value = max(0, int(value))
    return f"{value // 60000:02d}:{value // 1000 % 60:02d}.{value % 1000:03d}"


def to_lrc(entries, title: str = "", artist: str = "") -> str:
    """Enhanced LRC: [line start] <word start>word ... <last word end>, plus an empty [mark] where a line ends early."""
    out = ["[re:PolyKara timing editor]"]
    if title:
        out.append(f"[ti:{title}]")
    if artist:
        out.append(f"[ar:{artist}]")
    for index, (start, end, text, words) in enumerate(entries):
        speaker, _ = split_speaker(text)
        body = ""
        for number, (left, right, word) in enumerate(words):
            body += f"<{stamp(left)}>{word}"
            # An empty mark ends a word before the next one starts (a pause in the singing).
            if number + 1 == len(words) or words[number + 1][0] > right:
                body += f"<{stamp(right)}>"
        out.append(f"[{stamp(start)}]" + (f"[speaker:{speaker}] " if speaker else "") + body)
        following = entries[index + 1][0] if index + 1 < len(entries) else None
        if following is None or following > end:
            out.append(f"[{stamp(end)}]")
    return "\n".join(out) + "\n"


def save(row: dict[str, str], lines: list[Line]) -> tuple[Path, int]:
    entries, estimated = finished(lines)
    target = edited_path(row["id"])
    target.parent.mkdir(parents=True, exist_ok=True)
    partial = target.with_name(target.name + ".tmp")
    partial.write_text(to_lrc(entries, row.get("title", ""), row.get("artist", "")), encoding="utf-8")
    partial.replace(target)
    return target, estimated


def waveform(path: Path, bin_ms: int = 10, rate: int = 8000) -> list[int]:
    """Peak level (0-32767) of every bin_ms of the audio, decoded with ffmpeg."""
    result = subprocess.run(["ffmpeg", "-hide_banner", "-loglevel", "error", "-i", str(path), "-vn", "-ac", "1", "-ar", str(rate), "-f", "s16le", "-"], capture_output=True, check=False, timeout=600, creationflags=NO_WINDOW)
    samples = array.array("h")
    samples.frombytes(result.stdout[: len(result.stdout) // 2 * 2])
    size = max(1, rate * bin_ms // 1000)
    return [max(max(chunk), -min(chunk)) if chunk else 0 for chunk in (samples[offset:offset + size] for offset in range(0, len(samples), size))]


STATUS = re.compile(r"^\s*(\d+\.\d+)\s+M-")


class Player:
    """Plays audio with ffplay and follows the playback clock it reports.

    ffplay prints its audio clock about every 30 ms; the position between two
    reports is extrapolated, so it is accurate to a few milliseconds even
    though ffplay starts at the nearest seek point rather than exactly where
    asked. With a slower speed, ffplay's clock runs in played time and is
    converted back to song time.
    """

    def __init__(self):
        self.process: subprocess.Popen | None = None
        self.lock = threading.Lock()
        self.requested = 0.0
        self.speed = 1.0
        self.first: float | None = None
        self.clock: tuple[float, float] | None = None

    @property
    def playing(self) -> bool:
        return self.process is not None and self.process.poll() is None

    def play(self, path: Path, at_ms: int, speed: float = 1.0) -> None:
        self.stop()
        self.requested, self.speed, self.first, self.clock = at_ms / 1000, speed, None, None
        command = ["ffplay", "-nodisp", "-autoexit", "-vn", "-sn", "-hide_banner", "-loglevel", "error", "-stats", "-ss", f"{at_ms / 1000:.3f}"]
        if abs(speed - 1.0) > 0.001:
            command += ["-af", f"atempo={speed:g}"]
        self.process = subprocess.Popen(command + [str(path)], stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.PIPE, creationflags=NO_WINDOW)
        threading.Thread(target=self._follow, args=(self.process,), daemon=True).start()

    def _follow(self, process: subprocess.Popen) -> None:
        assert process.stderr is not None
        buffer = b""
        while True:
            chunk = process.stderr.read1(256) if hasattr(process.stderr, "read1") else process.stderr.read(64)
            if not chunk:
                break
            buffer += chunk
            *parts, buffer = re.split(rb"[\r\n]", buffer)
            for part in reversed(parts):
                match = STATUS.match(part.decode("ascii", "replace"))
                if match:
                    value = float(match.group(1))
                    with self.lock:
                        if process is self.process:
                            if self.first is None:
                                self.first = value
                            self.clock = (value, time.monotonic())
                    break

    def position(self) -> int:
        """Song position in milliseconds."""
        with self.lock:
            if self.clock is None or self.first is None:
                return int(self.requested * 1000)
            value, seen = self.clock
            first = self.first
        # A stalled clock (paused output, end of file) must not run away.
        elapsed = min(time.monotonic() - seen, 0.25)
        return round((first + (value - first + elapsed) * self.speed) * 1000)

    def stop(self) -> None:
        process, self.process = self.process, None
        if process is not None and process.poll() is None:
            process.kill()
            try:
                process.wait(timeout=2)
            except subprocess.TimeoutExpired:
                pass
