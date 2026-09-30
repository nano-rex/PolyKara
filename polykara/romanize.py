"""Optional romanization helpers for sing-along subtitles."""
from __future__ import annotations


def romanize(text: str, language: str, settings: dict) -> str | None:
    if not settings.get("enabled", True):
        return None
    language = language.casefold().replace("_", "-").split("-", 1)[0]
    if language not in {str(item).casefold() for item in settings.get("languages", [])}:
        return None
    try:
        if language in {"zh", "cmn"}:
            return _mandarin(text, settings)
        if language in {"yue", "zh-hk", "zh-tw"}:
            return _cantonese(text)
        if language == "ja":
            return _japanese(text)
        if language == "ko":
            return _korean(text)
        if language in {"ta", "hi", "bn", "gu", "kn", "ml", "mr", "ne", "pa", "sa", "te", "or"}:
            return _indic(text, language)
    except ImportError:
        return None
    return None


def _mandarin(text: str, settings: dict) -> str:
    from pypinyin import Style, pinyin

    tone = str(settings.get("tone", "numbers")).casefold()
    style = Style.TONE3 if tone in {"numbers", "number", "tone3"} else Style.TONE
    values = pinyin(text, style=style, neutral_tone_with_five=True, errors="default")
    return " ".join(item[0] for item in values).strip()


def _cantonese(text: str) -> str:
    from tojyutping import get_jyutping

    return get_jyutping(text).strip()


def _japanese(text: str) -> str:
    import pykakasi

    converter = pykakasi.kakasi()
    return " ".join(item["hepburn"] for item in converter.convert(text) if item.get("hepburn")).strip()


def _korean(text: str) -> str:
    from hangul_romanize import Transliter

    return Transliter().translit(text).strip()


def _indic(text: str, language: str) -> str:
    from indic_transliteration import sanscript
    from indic_transliteration.sanscript import transliterate

    schemes = {
        "ta": sanscript.TAMIL, "hi": sanscript.DEVANAGARI, "bn": sanscript.BENGALI,
        "gu": sanscript.GUJARATI, "kn": sanscript.KANNADA, "ml": sanscript.MALAYALAM,
        "mr": sanscript.DEVANAGARI, "ne": sanscript.DEVANAGARI, "pa": sanscript.GURMUKHI,
        "sa": sanscript.DEVANAGARI, "te": sanscript.TELUGU, "or": sanscript.ORIYA,
    }
    return transliterate(text, schemes[language], sanscript.ITRANS).strip()
