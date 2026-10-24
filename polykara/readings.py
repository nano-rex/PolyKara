"""Word library: readings that replace the romanization engines' guesses.

Engines such as pypinyin and pykakasi read most words right but not all of
them (重来 is chóng lái, not zhòng lái; 君 is sung kimi, not kun). The
library lists such words with the reading to use. PolyKara ships one per
language in polykara/readings/<language>.txt, and the user's own file,
<PolyKara folder>/readings/<language>.txt, adds words or replaces ones it
ships.

Format, one word per line, "#" starts a comment:
  Mandarin (zh)   重来 chóng lái     one syllable per character; tone marks or numbers
  Cantonese (yue) 銀行 ngan4 hong4   one Jyutping syllable per character
  Japanese (ja)   君 きみ            the reading of the whole word, kana or romaji
"""
from __future__ import annotations

import functools
import re
import unicodedata
from pathlib import Path

from .config import HOME, ROOT

BUILTIN = ROOT / "readings"
LANGUAGES = ("zh", "yue", "ja")
HAN = re.compile(r"[㐀-䶿一-鿿豈-﫿]")
MARKS = {"̄": "1", "́": "2", "̌": "3", "̀": "4"}
TONE_MARKS = {"1": "̄", "2": "́", "3": "̌", "4": "̀"}


def user_file(language: str) -> Path:
    return HOME / "readings" / f"{language}.txt"


def numbered(syllable: str) -> str:
    """Pinyin in tone-number form: chóng → chong2, lǜ → lv4, de → de5."""
    value = syllable.strip().casefold().replace("u:", "ü")
    tone = value[-1] if value[-1:].isdigit() else "5"
    letters = ""
    for char in unicodedata.normalize("NFD", value.rstrip("0123456789")):
        if char in MARKS:
            tone = tone if value[-1:].isdigit() else MARKS[char]
        elif char == "\u0308":  # the dots of ü
            letters = letters[:-1] + "v" if letters.endswith("u") else letters
        else:
            letters += char
    return letters.replace("ü", "v") + tone


def marked(syllable: str) -> str:
    """chong2 → chóng: the mark goes on a or e, on the o of ou, else on the last vowel."""
    letters, tone = syllable[:-1].replace("v", "ü"), syllable[-1]
    if tone not in TONE_MARKS:
        return letters
    index = next((letters.index(vowel) for vowel in "ae" if vowel in letters), None)
    if index is None:
        index = letters.index("o") if "ou" in letters else max((position for position, char in enumerate(letters) if char in "iouü"), default=None)
    if index is None:
        return letters
    return unicodedata.normalize("NFC", letters[:index + 1] + TONE_MARKS[tone] + letters[index + 1:])


def styled(syllable: str, language: str, tone: str) -> str:
    """A library syllable in the tone style of [romanization] tone ("marks", "numbers" or "none")."""
    if language == "zh":
        value = numbered(syllable)
        if tone in {"numbers", "number", "tone3"}:
            return value
        return re.sub(r"\d", "", value).replace("v", "ü") if tone in {"none", "plain", "normal"} else marked(value)
    return re.sub(r"\d", "", syllable) if language == "yue" and tone in {"none", "plain", "normal"} else syllable


def parse(text: str, language: str) -> tuple[dict[str, list[str]], list[str]]:
    """({word: readings}, problems). Chinese words must have one syllable per character."""
    words: dict[str, list[str]] = {}
    problems = []
    for number, raw in enumerate(text.splitlines(), 1):
        line = raw.split("#", 1)[0].strip()
        if not line:
            continue
        word, _, reading = line.partition(" ")
        word, parts = word.strip(), reading.split()
        if not parts:
            problems.append(f"line {number}: {word} has no reading")
            continue
        if language in {"zh", "yue"}:
            if len(parts) != len(word) or not all(HAN.match(char) for char in word):
                problems.append(f"line {number}: {word} needs one syllable for each of its {len(word)} characters")
                continue
        else:
            parts = ["".join(parts)]
        words[word] = parts
    return words, problems


def _stamp(path: Path) -> int:
    try:
        return path.stat().st_mtime_ns
    except OSError:
        return 0


@functools.lru_cache(maxsize=16)
def _library(language: str, builtin_stamp: int, user_stamp: int) -> dict[str, list[str]]:
    words: dict[str, list[str]] = {}
    for path in (BUILTIN / f"{language}.txt", user_file(language)):
        try:
            words.update(parse(path.read_text(encoding="utf-8-sig"), language)[0])
        except (OSError, UnicodeDecodeError):
            pass
    return words


def library(language: str) -> dict[str, list[str]]:
    """The merged library; reread when either file changes."""
    if language not in LANGUAGES:
        return {}
    return _library(language, _stamp(BUILTIN / f"{language}.txt"), _stamp(user_file(language)))


def matches(text: str, words: dict[str, list[str]]) -> list[tuple[int, int, list[str]]]:
    """Library words found in the text, longest first at each position, without overlaps."""
    if not words:
        return []
    longest = max(len(word) for word in words)
    found, index = [], 0
    while index < len(text):
        for size in range(min(longest, len(text) - index), 0, -1):
            reading = words.get(text[index:index + size])
            if reading is not None:
                found.append((index, index + size, reading))
                index += size
                break
        else:
            index += 1
    return found


def apply(text: str, language: str, raw: list[tuple[str, str]], engine, settings: dict) -> list[tuple[str, str]]:
    """The engine's (original, reading) pieces with library readings put in.

    Chinese pieces are single characters, so a library word replaces the
    readings of its characters and the engine keeps its context for the rest.
    Japanese words are cut out of the line and the engine reads what is
    between them.
    """
    tone = str(settings.get("tone", "marks")).casefold()
    found = matches(text, library(language))
    if language == "ja":
        if not found:
            return raw
        result, cursor = [], 0
        for start, end, (reading,) in found:
            if start > cursor:
                result += engine(text[cursor:start], settings)
            result.append((text[start:end], kana_to_romaji(reading)))
            cursor = end
        if cursor < len(text):
            result += engine(text[cursor:], settings)
        # The engine read each gap on its own, so particles after a library word are fixed here.
        particles = {"は": "wa", "へ": "e", "を": "o"}
        return [(original, particles[original] if index and original in particles else reading) for index, (original, reading) in enumerate(result)]
    readings: dict[int, str] = {}
    for start, _, parts in found:
        for offset, part in enumerate(parts):
            readings[start + offset] = styled(part, language, tone)
    if language == "zh":
        # A reduplicated word before 地 (慢慢地, 轻轻地) makes it the particle de.
        for match in re.finditer(r"([㐀-䶿一-鿿])\1地", text):
            readings.setdefault(match.end() - 1, styled("de5", language, tone))
    if not readings:
        return raw
    result, offset = [], 0
    for original, reading in raw:
        if len(original) == 1 and offset in readings:
            reading = readings[offset]
        result.append((original, reading))
        offset += len(original)
    return result


def kana_to_romaji(reading: str) -> str:
    if re.fullmatch(r"[A-Za-z' -]+", reading):
        return reading
    from .romanize import _kakasi

    return "".join(item.get("hepburn") or item["orig"] for item in _kakasi().convert(reading))
