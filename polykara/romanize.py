"""Romanized sing-along lines for lyrics written in non-Latin scripts.

A lyric line is split into pieces (a Chinese character, a Japanese word, a
Korean syllable, ...). Each piece keeps its position in the original text, so
the romanized row can be highlighted in step with the original row.
"""
from __future__ import annotations

import functools
import re
import unicodedata

HAN = "㐀-䶿一-鿿豈-﫿"
KANA = re.compile(r"[぀-ヿㇰ-ㇿ]")
HANGUL = re.compile(r"[가-힣]")
HAN_CHAR = re.compile(rf"[{HAN}]")
INDIC = {"ta", "hi", "bn", "gu", "kn", "ml", "mr", "ne", "pa", "sa", "te", "or"}
# Optional engines that were needed but are not installed, as {module: language}.
MISSING: dict[str, str] = {}
Piece = tuple[int, int, str]  # start and end offset in the line, romanized text


def base_language(language: str) -> str:
    tag = language.casefold().replace("_", "-").strip()
    # Hong Kong Chinese is sung in Cantonese; other zh-* tags are Mandarin.
    return "yue" if tag in {"zh-hk", "zh-yue"} else tag.split("-", 1)[0]


def non_latin(text: str) -> bool:
    return any(unicodedata.category(char).startswith("L") and not unicodedata.name(char, "").startswith("LATIN") for char in text)


def engine_for(text: str, language: str) -> str | None:
    """Pick the romanizer from the script of the line, using the song language to read Chinese characters."""
    if not non_latin(text):
        return None
    if KANA.search(text):
        return "ja"
    if HANGUL.search(text):
        return "ko"
    if HAN_CHAR.search(text):
        return language if language in {"yue", "ja"} else "zh"
    return language if language in INDIC else "other"


def enabled(settings: dict, language: str, engine: str) -> bool:
    if not settings.get("enabled", True):
        return False
    languages = settings.get("languages", "auto")
    if isinstance(languages, str):
        return languages.casefold() == "auto"
    wanted = {str(item).casefold() for item in languages}
    return language in wanted or engine in wanted or "auto" in wanted


def pieces(text: str, language: str, settings: dict) -> tuple[str, list[Piece]] | None:
    """(engine, romanized pieces covering the line), or None when the line needs no romanization."""
    language = base_language(language)
    engine = engine_for(text, language)
    if engine is None or not enabled(settings, language, engine):
        return None
    try:
        raw = _indic(text, engine) if engine in INDIC else ENGINES[engine](text, settings)
    except ImportError as exc:
        MISSING[exc.name or str(exc)] = engine
        return None
    items = _locate(text, raw)
    return (engine, items) if items else None


def _locate(text: str, raw: list[tuple[str, str]]) -> list[Piece]:
    """Turn (original, romanized) pairs into offsets, merging punctuation into the piece before it."""
    result: list[list] = []
    cursor = 0
    for original, roman in raw:
        start = text.find(original, cursor) if original else -1
        if start < 0:
            # The engine changed the text; show the line without per-piece timing.
            return [(0, len(text), " ".join(item[1].strip() for item in raw if item[1].strip()))]
        cursor = start + len(original)
        # Text the engine passed through unchanged (Latin words, spaces) is split into words.
        parts = [(start + match.start(), start + match.end(), match.group()) for match in re.finditer(r"\S+", original)] if roman == original else [(start, cursor, roman.strip())]
        for begin, end, value in parts:
            if not value:
                continue
            if result and not re.search(r"\w", value):
                # Full-width marks such as ，and ！ read better as their ASCII forms here.
                result[-1][1], result[-1][2] = end, result[-1][2] + unicodedata.normalize("NFKC", value)
            else:
                result.append([begin, end, value])
    return [(begin, end, value) for begin, end, value in result]


def _spaced(text: str, items: list[Piece], engine: str) -> list[str]:
    """Romanized text of each piece with the space that should follow it."""
    out = []
    for index, (_, end, roman) in enumerate(items):
        following = items[index + 1][0] if index + 1 < len(items) else None
        # Korean syllables of one word are written together; other scripts get a space per piece.
        space = following is not None and (engine != "ko" or text[end:following].isspace())
        out.append(roman + (" " if space else ""))
    return out


def romanize(text: str, language: str, settings: dict) -> str | None:
    """The whole line romanized, for lines without word timing."""
    found = pieces(text, language, settings)
    if not found:
        return None
    engine, items = found
    return "".join(_spaced(text, items, engine)).strip()


def romanize_words(words: list[tuple[int, int, str]], language: str, settings: dict) -> list[tuple[int, int, str]] | None:
    """Romanized pieces timed from the original words they cover, so both rows highlight together."""
    text = "".join(word for _, _, word in words)
    found = pieces(text, language, settings)
    if not found:
        return None
    engine, items = found
    owner = []
    for index, (_, _, word) in enumerate(words):
        owner.extend([index] * len(word))
    timed = []
    for (start, end, _), label in zip(items, _spaced(text, items, engine)):
        covered = [owner[offset] for offset in range(start, end) if not text[offset].isspace()]
        if covered:
            timed.append((words[covered[0]][0], words[covered[-1]][1], label))
    if timed:
        timed[-1] = (*timed[-1][:2], timed[-1][2].rstrip())
    return timed or None


def ruby_groups(words: list[tuple[int, int, str]], language: str, settings: dict) -> list[dict] | None:
    """Group the timed words of a line under the romanized piece that reads them.

    Each group is {"tokens": [(start, end, text)], "roman": str | None,
    "space_before": bool}; roman is None for text such as English words that
    needs no reading above it.
    """
    text = "".join(word for _, _, word in words)
    found = pieces(text, language, settings)
    if not found:
        return None
    _, items = found
    groups = [{"start": start, "roman": None if text[start:end] == roman else roman, "tokens": [], "space_before": index > 0 and text[items[index - 1][1]:start].strip() == "" and start > items[index - 1][1]} for index, (start, end, roman) in enumerate(items)]
    offset = 0
    for left, right, word in words:
        first = offset + len(word) - len(word.lstrip())
        offset += len(word)
        owner = max((group for group in groups if group["start"] <= first), key=lambda group: group["start"], default=groups[0])
        owner["tokens"].append((left, right, word.strip()))
    return [group for group in groups if group["tokens"]] or None


def _mandarin(text: str, settings: dict) -> list[tuple[str, str]]:
    from pypinyin import Style, pinyin

    tone = str(settings.get("tone", "marks")).casefold()
    style = Style.TONE3 if tone in {"numbers", "number", "tone3"} else Style.TONE
    values = [item[0] for item in pinyin(text, style=style, neutral_tone_with_five=True, errors="default")]
    # pypinyin returns one item per Chinese character and one per run of anything else.
    runs = re.findall(rf"[{HAN}]|[^{HAN}]+", text)
    if len(runs) != len(values):
        return [(text, " ".join(values))]
    return list(zip(runs, values))


def _cantonese(text: str, settings: dict) -> list[tuple[str, str]]:
    from ToJyutping import get_jyutping_list

    result: list[tuple[str, str]] = []
    for char, reading in get_jyutping_list(text):
        if reading:
            result.append((char, reading))
        elif result and result[-1][0] == result[-1][1]:
            result[-1] = (result[-1][0] + char, result[-1][1] + char)
        else:
            result.append((char, char))
    return result


@functools.lru_cache(maxsize=1)
def _kakasi():
    import pykakasi

    return pykakasi.kakasi()


def _japanese(text: str, settings: dict) -> list[tuple[str, str]]:
    result = []
    for item in _kakasi().convert(text):
        roman = item.get("hepburn") or item["orig"]
        # Particles are written は/へ/を but sung wa/e/o.
        if result and item["orig"] in {"は", "へ", "を"}:
            roman = {"は": "wa", "へ": "e", "を": "o"}[item["orig"]]
        result.append((item["orig"], roman))
    return result


# Revised Romanization of Korean, per syllable.
INITIALS = ["g", "kk", "n", "d", "tt", "r", "m", "b", "pp", "s", "ss", "", "j", "jj", "ch", "k", "t", "p", "h"]
VOWELS = ["a", "ae", "ya", "yae", "eo", "e", "yeo", "ye", "o", "wa", "wae", "oe", "yo", "u", "wo", "we", "wi", "yu", "eu", "ui", "i"]
FINALS = ["", "k", "k", "k", "n", "n", "n", "t", "l", "k", "m", "l", "l", "l", "p", "l", "m", "p", "p", "t", "t", "ng", "t", "t", "k", "t", "p", "t"]
# A final consonant followed by a silent ㅇ is sung at the start of the next syllable.
CARRIED = {1: "g", 2: "kk", 4: "n", 7: "d", 8: "r", 16: "m", 17: "b", 19: "s", 20: "ss", 22: "j", 23: "ch", 24: "k", 25: "t", 26: "p", 27: ""}


def _korean(text: str, settings: dict) -> list[tuple[str, str]]:
    result: list[tuple[str, str]] = []
    carried = None
    for index, char in enumerate(text):
        if not HANGUL.match(char):
            carried = None
            if result and result[-1][0] == result[-1][1] and not HANGUL.match(result[-1][0][-1]):
                result[-1] = (result[-1][0] + char, result[-1][1] + char)
            else:
                result.append((char, char))
            continue
        code = ord(char) - 0xAC00
        initial, vowel, final = code // 588, code % 588 // 28, code % 28
        start = carried if carried is not None else INITIALS[initial]
        following = text[index + 1] if index + 1 < len(text) else ""
        silent_next = bool(HANGUL.match(following)) and (ord(following) - 0xAC00) // 588 == 11
        if final in CARRIED and silent_next:
            # 사랑이 → sa-rang-i but 먹어 → meo-geo: the final moves to the next syllable.
            ending, carried = "", CARRIED[final]
        else:
            ending, carried = FINALS[final], None
        result.append((char, start + VOWELS[vowel] + ending))
    return result


def _indic(text: str, language: str) -> list[tuple[str, str]]:
    from indic_transliteration import sanscript
    from indic_transliteration.sanscript import transliterate

    schemes = {
        "ta": sanscript.TAMIL, "hi": sanscript.DEVANAGARI, "bn": sanscript.BENGALI,
        "gu": sanscript.GUJARATI, "kn": sanscript.KANNADA, "ml": sanscript.MALAYALAM,
        "mr": sanscript.DEVANAGARI, "ne": sanscript.DEVANAGARI, "pa": sanscript.GURMUKHI,
        "sa": sanscript.DEVANAGARI, "te": sanscript.TELUGU, "or": sanscript.ORIYA,
    }
    return [(word, transliterate(word, schemes[language], sanscript.ITRANS)) for word in text.split()]


def _other(text: str, settings: dict) -> list[tuple[str, str]]:
    """Cyrillic, Greek, Thai, Arabic, Hebrew and other scripts, word by word."""
    from anyascii import anyascii

    return [(word, anyascii(word)) for word in text.split()]


ENGINES = {"zh": _mandarin, "yue": _cantonese, "ja": _japanese, "ko": _korean, "other": _other}
