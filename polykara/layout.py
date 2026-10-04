"""Text width estimates in ASS PlayRes units, for placing romanization above each character.

libass scales a font so that its ascent plus descent equals the style's font
size, so one em is usually smaller than the font size (0.9 for Arial, 0.69 for
Noto Sans CJK). With Pillow and fontconfig installed, widths are measured from
the fonts libass will use. Without them the estimate treats one em as the full
font size, which can only overestimate: characters end up a little further
apart but never overlap.
"""
from __future__ import annotations

import functools
import shutil
import subprocess
import unicodedata

# Advance widths of Arial / Helvetica in 1/1000 em for printable ASCII (32-126).
ARIAL_REGULAR = [278, 278, 355, 556, 556, 889, 667, 191, 333, 333, 389, 584, 278, 333, 278, 278, 556, 556, 556, 556, 556, 556, 556, 556, 556, 556, 278, 278, 584, 584, 584, 556, 1015, 667, 667, 722, 722, 667, 611, 778, 722, 278, 500, 667, 556, 833, 722, 778, 667, 778, 722, 667, 611, 722, 667, 944, 667, 667, 611, 278, 278, 278, 469, 556, 333, 556, 556, 500, 556, 556, 278, 556, 556, 222, 222, 500, 222, 833, 556, 556, 556, 556, 333, 500, 278, 556, 500, 722, 500, 500, 500, 334, 260, 334, 584]
ARIAL_BOLD = [278, 333, 474, 556, 556, 889, 722, 238, 333, 333, 389, 584, 278, 333, 278, 278, 556, 556, 556, 556, 556, 556, 556, 556, 556, 556, 333, 333, 584, 584, 584, 611, 975, 722, 722, 722, 722, 667, 611, 778, 722, 278, 556, 722, 611, 833, 722, 778, 667, 778, 722, 667, 611, 722, 667, 944, 667, 667, 611, 333, 278, 333, 584, 556, 333, 556, 611, 556, 611, 556, 333, 611, 611, 278, 278, 556, 278, 889, 611, 611, 611, 611, 389, 556, 333, 611, 556, 778, 556, 556, 500, 389, 280, 389, 584]
# Characters sampled to find the font libass falls back to for each wide script.
SCRIPT_SAMPLES = {"han": "4e00", "kana": "3042", "hangul": "ac00"}


def wide(char: str) -> bool:
    return unicodedata.east_asian_width(char) in {"W", "F"}


def script_of(char: str) -> str:
    code = ord(char)
    if 0x3040 <= code <= 0x30FF:
        return "kana"
    if 0xAC00 <= code <= 0xD7A3 or 0x1100 <= code <= 0x11FF or 0x3130 <= code <= 0x318F:
        return "hangul"
    return "han"


@functools.lru_cache(maxsize=32)
def font_file(pattern: str) -> str | None:
    if shutil.which("fc-match") is None:
        return None
    try:
        result = subprocess.run(["fc-match", "-f", "%{file}", pattern], capture_output=True, text=True, check=False, timeout=10)
    except (OSError, subprocess.TimeoutExpired):
        return None
    return result.stdout.strip() or None


@functools.lru_cache(maxsize=32)
def pillow_font(path: str):
    """A font loaded at 1000 px, with its libass em factor; None when Pillow cannot load it."""
    try:
        from PIL import ImageFont

        font = ImageFont.truetype(path, 1000)
        ascent, descent = font.getmetrics()
        return font, 1000 / max(1, ascent + descent)
    except Exception:
        return None


def measured_font(family: str, bold: bool, sample: str | None = None):
    pattern = family + (":bold" if bold else "") + (f":charset={sample}" if sample else "")
    path = font_file(pattern)
    return pillow_font(path) if path else None


def narrow_width(text: str, family: str, size: float, bold: bool) -> float:
    loaded = measured_font(family, bold)
    if loaded:
        font, em = loaded
        return font.getlength(text) / 1000 * em * size
    table = ARIAL_BOLD if bold else ARIAL_REGULAR
    total = 0
    for char in text:
        # Accented letters such as ǎ take the width of their base letter.
        base = unicodedata.normalize("NFD", char)[:1] or char
        code = ord(base)
        total += table[code - 32] if 32 <= code <= 126 else 600
    return total / 1000 * size


def wide_width(char: str, family: str, size: float, bold: bool) -> float:
    loaded = measured_font(family, bold, SCRIPT_SAMPLES[script_of(char)])
    if loaded:
        font, em = loaded
        return font.getlength(char) / 1000 * em * size
    return size


def text_width(text: str, family: str, size: float, bold: bool) -> float:
    total, run = 0.0, ""
    for char in text:
        if wide(char):
            if run:
                total, run = total + narrow_width(run, family, size, bold), ""
            total += wide_width(char, family, size, bold)
        else:
            run += char
    return total + (narrow_width(run, family, size, bold) if run else 0.0)
