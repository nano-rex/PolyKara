"""polykara.toml as form fields: which settings exist, how each is edited, and in-place changes.

Shared by the settings window and the engine that serves the desktop app, so
it has no window code. Saving rewrites only the values that changed, in
place, so the file keeps its comments and layout.
"""
from __future__ import annotations

import copy
import re
import tomllib

from .config import DEFAULT_CONFIG, _merge

# (tab, sections) in the order shown.
TABS = [
    ("Lyrics", ["lyric", "karaoke", "singers"]),
    ("Readings", ["romanization"]),
    ("Layout", ["layout", "effects", "timing", "prompt"]),
    ("Title and credits", ["title", "credit"]),
    ("Video", ["video", "render", "logo", "watermark"]),
    ("Lyrics search", ["lyrics", "alignment", "charts"]),
    ("Player and live", ["player", "live"]),
]
SECTION_NAMES = {
    "lyric": "Lyric text", "karaoke": "Sung colours", "singers": "Singers and voice colours", "romanization": "Pinyin / romaji above the characters",
    "layout": "Lines on screen", "effects": "Karaoke effects", "player": "Music player", "live": "Live output", "countdown": "Countdown dots", "timing": "Pauses and timing", "prompt": "Pause prompt",
    "card": "Opening card (KTV layouts)", "title": "Title", "credit": "Credits",
    "video": "Frame size", "render": "Video encoding", "logo": "Logo", "watermark": "Watermark text",
    "lyrics": "Lyric sources and accuracy check", "alignment": "Automatic word timing (Whisper)", "charts": "YouTube charts",
}
CHOICES = {
    ("layout", "mode"): ["single", "alternate"],
    ("effects", "preset"): ["none", "fade", "pop", "glow"],
    ("live", "audio_output"): ["none", "pulse", "alsa"],
    ("render", "fit"): ["blur", "pad", "none"],
    ("render", "preset"): ["ultrafast", "superfast", "veryfast", "faster", "fast", "medium", "slow", "slower", "veryslow", ""],
    ("render", "video_codec"): ["libx264", "libx265", "h264_nvenc", "hevc_nvenc", "h264_qsv", "h264_amf"],
    ("render", "audio_codec"): ["aac", "libmp3lame", "copy"],
    ("logo", "position"): ["top-left", "top-right", "bottom-left", "bottom-right", "center"],
    ("romanization", "layout"): ["ruby", "rows"],
    ("romanization", "tone"): ["marks", "numbers", "none"],
    ("karaoke", "tag"): ["kf", "k", "ko"],
    ("alignment", "model"): ["auto", "tiny", "base", "small", "medium", "large-v3"],
    ("alignment", "device"): ["auto", "cpu", "cuda"],
    ("alignment", "compute_type"): ["int8", "int8_float16", "float16", "float32"],
}
POSITIONS = {"horizontal": ["left", "center", "right"], "vertical": ["top", "middle", "bottom"]}
# Colour settings that also accept a word instead of a colour.
COLOUR_WORDS = {("card", "title_outline_color"): ["voice"]}
ASS_COLOUR = re.compile(r"^&H([0-9A-Fa-f]{2})([0-9A-Fa-f]{2})([0-9A-Fa-f]{2})([0-9A-Fa-f]{2})$")
LABELS = {"cjk_size": "Size for Chinese / Japanese / Korean lines", "margin_v": "Vertical margin", "margin_l": "Left margin", "margin_r": "Right margin",
          "top_margin_v": "Upper row height from top", "bottom_margin_v": "Lower row height from bottom", "lead_ms": "Show line before it is sung (ms)",
          "hold_ms": "Keep line after it is sung (ms)", "fade_ms": "Fade (ms)", "page_break_ms": "New page after a pause of (ms)", "play_res_x": "Width",
          "play_res_y": "Height", "extend_intro": "Add time before an early first line", "crf": "Quality (CRF, lower is better)", "blur_sigma": "Blur strength",
          "fit": "Videos of another shape", "file": "Image file", "tone": "Pinyin tones", "ruby_rise": "Reading height above characters",
          "ruby_spacing": "Space between characters", "min_sources": "Lyric files needed for 'verified'", "agreement_threshold": "Agreement needed (0-1)",
          "require_verified": "Only make videos with verified lyrics", "auto_captions": "Use YouTube automatic captions", "show_label": "Show 女/男/合 labels",
          "long_pause_ms": "Show countdown after a pause of (ms)", "tag": "Karaoke effect", "active_color": "Sung colour", "inactive_color": "Unsung colour",
          "sung_outline_color": "Sung outline colour", "secondary_color": "Unsung colour", "enabled_regions": "Chart regions"}


def label_for(key: str) -> str:
    if key in LABELS:
        return LABELS[key]
    text = key.replace("_ms", " (ms)").replace("color", "colour").replace("_", " ")
    return text[:1].upper() + text[1:]


def current_values(text: str) -> dict:
    config = copy.deepcopy(DEFAULT_CONFIG)
    _merge(config, tomllib.loads(text))
    return config


def comments(text: str) -> dict[tuple[str, str], str]:
    """The comment lines just above each key (or above its section), as help text."""
    found, section, pending, section_help = {}, "", [], {}
    for line in text.splitlines():
        stripped = line.strip()
        if stripped.startswith("#"):
            pending.append(stripped.lstrip("#").strip())
            continue
        header = re.match(r"^\[([^\]]+)\]", stripped)
        if header:
            section = header.group(1).strip()
            if pending:
                section_help[section] = " ".join(pending)
        else:
            match = re.match(r"^([A-Za-z0-9_]+)\s*=", stripped)
            if match and pending:
                found[(section, match.group(1))] = " ".join(pending)
        if stripped:
            pending = []
    for section, value in section_help.items():
        found.setdefault((section, ""), value)
    return found


def literal(value) -> str:
    """A TOML literal for a setting value."""
    if isinstance(value, bool):
        return "true" if value else "false"
    if isinstance(value, (int, float)):
        return repr(value)
    if isinstance(value, list):
        return "[" + ", ".join(literal(item) for item in value) + "]"
    return '"' + str(value).replace("\\", "\\\\").replace('"', '\\"') + '"'


def set_value(text: str, section: str, key: str, value) -> str:
    """The file text with one setting changed in place, added to its section, or added with a new section."""
    lines = text.splitlines()
    start = next((index for index, line in enumerate(lines) if re.match(rf"^\s*\[{re.escape(section)}\]\s*(#.*)?$", line)), None)
    entry = f"{key} = {literal(value)}"
    if start is None:
        return text.rstrip("\n") + f"\n\n[{section}]\n{entry}\n"
    end = next((index for index in range(start + 1, len(lines)) if re.match(r"^\s*\[", lines[index])), len(lines))
    last_key = start
    for index in range(start + 1, end):
        if re.match(rf"^\s*{re.escape(key)}\s*=", lines[index]):
            lines[index] = entry
            return "\n".join(lines) + "\n"
        if re.match(r"^\s*[A-Za-z0-9_]+\s*=", lines[index]):
            last_key = index
    lines.insert(last_key + 1, entry)
    return "\n".join(lines) + "\n"


def ass_to_hex(value: str) -> str | None:
    match = ASS_COLOUR.match(value.strip())
    if not match:
        return None
    _, blue, green, red = match.groups()
    return f"#{red}{green}{blue}".lower()


def hex_to_ass(colour: str, alpha: str = "00") -> str:
    red, green, blue = colour[1:3], colour[3:5], colour[5:7]
    return f"&H{alpha}{blue}{green}{red}".upper()


def field_kind(section: str, key: str, value) -> tuple[str, list[str]]:
    """How a setting is edited, and the choices offered for it."""
    if isinstance(value, bool):
        return "bool", []
    if isinstance(value, list):
        return "list", []
    if isinstance(value, (int, float)):
        return "number", []
    if (section, key) in CHOICES or key in POSITIONS:
        return "choice", CHOICES.get((section, key)) or POSITIONS[key]
    if key == "font":
        return "font", []
    if key.endswith("color") or ASS_COLOUR.match(str(value)):
        return "colour", COLOUR_WORDS.get((section, key), [])
    if key == "file":
        return "file", []
    return "text", []


def shown(value) -> str | bool:
    """A value as it is shown in its field: lists as comma-separated text."""
    if isinstance(value, bool):
        return value
    if isinstance(value, list):
        return ", ".join(str(item) for item in value)
    if isinstance(value, (int, float)):
        return repr(value)
    return str(value)


def field_value(section: str, key: str, raw, original):
    """A field's text read back as a value of the setting's type."""
    if isinstance(original, bool):
        return bool(raw)
    if isinstance(original, list):
        items = [item.strip() for item in str(raw).split(",") if item.strip()]
        return [int(item) if re.fullmatch(r"-?\d+", item) and all(isinstance(old, int) for old in original) and original else item for item in items]
    if isinstance(original, (int, float)):
        text = str(raw).strip()
        try:
            return int(text) if isinstance(original, int) and re.fullmatch(r"-?\d+", text) else float(text)
        except ValueError:
            raise ValueError(f"[{section}] {label_for(key)} must be a number") from None
    return str(raw)


def form(text: str) -> list[dict]:
    """Every known setting as tabs of sections of fields, with the file's comments as help."""
    values = current_values(text)
    help_text = comments(text)
    tabs, placed = [], set()

    def section(name: str) -> dict:
        fields = []
        for key, value in values[name].items():
            if isinstance(value, dict):
                continue
            kind, choices = field_kind(name, key, value)
            note = help_text.get((name, key)) or help_text.get((name, ""), "")
            if kind == "colour":
                note = (note + " " if note else "") + "Colours are written &H<alpha><blue><green><red> in hex; alpha 00 is solid, FF invisible."
            fields.append({"key": key, "label": label_for(key), "kind": kind, "choices": choices, "value": shown(value), "help": note, "wide": key == "text"})
        return {"id": name, "title": SECTION_NAMES.get(name, name), "fields": fields}

    for name, sections in TABS:
        present = [item for item in sections if isinstance(values.get(item), dict)]
        placed.update(present)
        tabs.append({"name": name, "sections": [section(item) for item in present]})
    others = [name for name, value in values.items() if isinstance(value, dict) and name not in placed]
    if others:
        tabs.append({"name": "Other", "sections": [section(item) for item in others]})
    return tabs


def apply_changes(text: str, changes: list) -> str:
    """The file text with each changed field, [section, key, shown value], written in."""
    values = current_values(text)
    for section, key, raw in changes:
        original = values.get(section, {}).get(key)
        value = field_value(section, key, raw, original)
        if value != original:
            text = set_value(text, section, key, value)
    return text
