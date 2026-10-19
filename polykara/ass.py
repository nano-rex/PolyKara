from __future__ import annotations

import re

from .config import DOT_INTERVAL_MS, LONG_PAUSE_MS, MAX_SINGERS, SPEAKER_PALETTE, load_config
from .romanize import romanize
from .subtitle import split_speaker

CJK = re.compile(r"[　-鿿぀-ヿ가-힯豈-﫿]")

# Per-syllable effect presets. Templates are ASS override tags formatted with
# {start} {mid} {end} {dur} {index} (ms relative to the event start) plus params.
# Each syllable template resets its own state first so effects do not leak forward.
EFFECT_PRESETS = {
    "none": {"line": "", "syllable": "", "params": {}},
    "fade": {"line": "\\fad({fade_in},{fade_out})", "syllable": "", "params": {"fade_in": 200, "fade_out": 200}},
    "pop": {"line": "", "syllable": "\\fscx100\\fscy100\\t({start},{mid},\\fscx{scale}\\fscy{scale})\\t({mid},{end},\\fscx100\\fscy100)", "params": {"scale": 125}},
    "glow": {"line": "", "syllable": "\\blur0\\t({start},{mid},\\blur{blur})\\t({mid},{end},\\blur0)", "params": {"blur": 6}},
}


def effect_templates(config: dict) -> tuple[str, str, dict]:
    values = config.get("effects", {})
    name = str(values.get("preset", "none")).lower()
    if name not in EFFECT_PRESETS:
        raise ValueError(f"effects.preset must be one of {', '.join(EFFECT_PRESETS)}")
    preset = EFFECT_PRESETS[name]
    params = {**preset["params"], **values.get("params", {})}
    return str(values.get("line", preset["line"])), str(values.get("syllable", preset["syllable"])), params


def apply_template(template: str, params: dict, **timing: int) -> str:
    if not template:
        return ""
    try:
        return template.format(**params, **timing)
    except (KeyError, IndexError) as exc:
        raise ValueError(f"Unknown effect template field: {exc}") from exc


def karaoke_text(words, event_start: int, tag: str, syllable_template: str = "", params: dict | None = None) -> str:
    params = params or {}
    parts, cursor, previous = [], event_start, ""
    for index, (left, right, word) in enumerate(words):
        gap = left - cursor
        separator = "" if not previous or (CJK.search(previous[-1]) and CJK.search(word[:1])) else " "
        if gap >= 10:
            parts.append(f"{{\\k{round(gap / 10)}}}")
        duration = max(1, round((right - left) / 10))
        rel_start, rel_end = max(0, left - event_start), max(0, right - event_start)
        effect = apply_template(syllable_template, params, start=rel_start, mid=(rel_start + rel_end) // 2, end=rel_end, dur=rel_end - rel_start, index=index)
        parts.append(f"{separator}{{\\{tag}{duration}{effect}}}{esc(word)}")
        cursor, previous = max(cursor, left) + duration * 10, word
    return "".join(parts)


def at(value: int) -> str:
    cs = max(0, value) // 10
    return f"{cs // 360000}:{(cs // 6000) % 60:02d}:{(cs // 100) % 60:02d}.{cs % 100:02d}"


def esc(value: str) -> str:
    return value.replace("\\", "\\N").replace("{", "\\{").replace("}", "\\}").replace("\n", "\\N")


def alignment(values: dict) -> int:
    if "alignment" in values:
        return int(values["alignment"])
    horizontal = {"left": 0, "center": 1, "right": 2}.get(str(values.get("horizontal", "center")).lower(), 1)
    vertical = {"bottom": 0, "middle": 3, "center": 3, "top": 6}.get(str(values.get("vertical", "bottom")).lower(), 0)
    return vertical + horizontal + 1


def intro_padding_ms(entries) -> int:
    config = load_config()
    if not config["video"].get("extend_intro", True) or not entries:
        return 0
    first_lyric = min(start for start, _, _, _ in entries)
    card_end = 0
    for name in ("title", "credit"):
        values = config[name]
        if values.get("enabled", True):
            card_end = max(card_end, int(values.get("start_ms", 0)) + max(0, int(values.get("duration_ms", 6000))))
    return max(0, card_end - first_lyric)


def ass(row: dict[str, str], entries) -> str:
    config = load_config()
    if str(config.get("layout", {}).get("mode", "single")).lower() == "pairs":
        from .ktv import document

        return document(row, entries, config, at, esc, karaoke_text, SPEAKER_PALETTE)
    video, title_style = config["video"], config["title"]
    credit_style, lyric_style = config["credit"], dict(config["lyric"])
    karaoke_style = config.get("karaoke", {})
    lyric_style["color"] = karaoke_style.get("active_color", lyric_style.get("color", "&H00FF0000"))
    lyric_style["secondary_color"] = karaoke_style.get("inactive_color", lyric_style.get("secondary_color", "&H00FFFFFF"))
    prompt_style, watermark_style = config["prompt"], config["watermark"]
    romanization_style = config.get("romanization", {})
    def text_template(values: dict, fallback: str) -> str:
        try:
            return str(values.get("text", fallback)).format(**row)
        except KeyError as exc:
            raise ValueError(f"Unknown title/credit template field: {exc.args[0]}") from exc
    title = esc(text_template(title_style, "{title}  |  {artist}"))
    credit = esc(text_template(credit_style, "作词：{lyricist}    作曲：{composer}    字幕制作：{producer}"))
    def style(name: str, values: dict, primary: str, secondary: str = "&H00FFFFFF", outline: str = "&H80000000", back: str = "&H50000000") -> str:
        return f"Style: {name},{values['font']},{values['size']},{values.get('color', primary)},{values.get('secondary_color', secondary)},{values.get('outline_color', outline)},{values.get('back_color', back)},{1 if values.get('bold', True) else 0},{1 if values.get('italic', False) else 0},{1 if values.get('underline', False) else 0},{1 if values.get('strikeout', False) else 0},100,100,0,0,1,{values.get('outline', 2)},{values.get('shadow', 1)},{alignment(values)},{values['margin_l']},{values['margin_r']},{values['margin_v']},1"
    out = [
        "[Script Info]", "ScriptType: v4.00+", f"PlayResX: {video['play_res_x']}", f"PlayResY: {video['play_res_y']}", "WrapStyle: 2", "ScaledBorderAndShadow: yes", "",
        "[V4+ Styles]", "Format: Name, Fontname, Fontsize, PrimaryColour, SecondaryColour, OutlineColour, BackColour, Bold, Italic, Underline, StrikeOut, ScaleX, ScaleY, Spacing, Angle, BorderStyle, Outline, Shadow, Alignment, MarginL, MarginR, MarginV, Encoding",
        style("Header", title_style, "&H00FFFFFF"), style("Credit", credit_style, "&H00FFFFFF"), style("Lyric", lyric_style, "&H00FF0000"), style("Romanization", romanization_style, "&H00FFFFFF"), style("Prompt", prompt_style, "&H00FFFFFF"), style("Watermark", watermark_style, "&H80FFFFFF"), "",
        "[Events]", "Format: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text",
    ]
    first_lyric = min((start for start, _, _, _ in entries), default=0)
    def card_event(style_name: str, values: dict, text: str) -> str | None:
        start = max(0, int(values.get("start_ms", 0)))
        requested_end = start + max(0, int(values.get("duration_ms", 6000)))
        end = requested_end
        if end <= start:
            return None
        fade_in = min(max(0, int(values.get("fade_in_ms", 0))), end - start)
        fade_out = min(max(0, int(values.get("fade_out_ms", 0))), end - start - fade_in)
        animation = f"{{\\fad({fade_in},{fade_out})}}" if fade_in or fade_out else ""
        return f"Dialogue: 0,{at(start)},{at(end)},{style_name},,0,0,0,,{animation}{text}"
    if title_style.get("enabled", True):
        event = card_event("Header", title_style, title)
        if event:
            out.append(event)
    if credit_style.get("enabled", True):
        event = card_event("Credit", credit_style, credit)
        if event:
            out.append(event)
    if watermark_style.get("enabled", False) and watermark_style.get("text", ""):
        out.append(f"Dialogue: 1,0:00:00.00,9:59:59.99,Watermark,,0,0,0,,{esc(str(watermark_style['text']))}")
    intro_padding = intro_padding_ms(entries)
    out.insert(1, f"; PolyKaraIntroPaddingMs: {intro_padding}")
    speakers = []
    for _, _, text, _ in entries:
        speaker, _ = split_speaker(text)
        if speaker and speaker.casefold() not in {item.casefold() for item in speakers}:
            speakers.append(speaker)
    max_singers = int(config.get("singers", {}).get("max", MAX_SINGERS))
    if len(speakers) > max_singers:
        raise ValueError(f"{row['id']} has {len(speakers)} singers; maximum is {max_singers}")
    slots = {speaker.casefold(): SPEAKER_PALETTE[index] for index, speaker in enumerate(speakers)}
    timing = config.get("timing", {})
    long_pause_ms = int(timing.get("long_pause_ms", LONG_PAUSE_MS))
    dot_interval_ms = int(timing.get("dot_interval_ms", DOT_INTERVAL_MS))
    karaoke_tag = str(karaoke_style.get("tag", "kf")).lower()
    if karaoke_tag not in {"k", "kf"}:
        raise ValueError("karaoke.tag must be 'k' or 'kf'")
    line_template, syllable_template, effect_params = effect_templates(config)
    layout = config.get("layout", {})
    alternate = str(layout.get("mode", "single")).lower() == "alternate"
    if str(layout.get("mode", "single")).lower() not in {"single", "alternate"}:
        raise ValueError("layout.mode must be 'single' or 'alternate'")
    lead_in_ms = max(0, int(layout.get("lead_in_ms", 1500))) if alternate else 0
    lyric_margin_v = int(lyric_style.get("margin_v", 150))
    roman_margin = romanization_style.get("margin_v", 225)
    romanization_gap = int(roman_margin) - lyric_margin_v if str(roman_margin).lower() != "auto" else 75
    rows_layout = (
        (int(layout.get("top_alignment", 1)), int(layout.get("top_margin_v", lyric_margin_v + 130))),
        (int(layout.get("bottom_alignment", 3)), int(layout.get("bottom_margin_v", lyric_margin_v))),
    )
    row_free = [0, 0]
    previous_end = 0
    for index, (start, end, text, words) in enumerate(entries):
        start += intro_padding
        end += intro_padding
        words = [(left + intro_padding, right + intro_padding, word) for left, right, word in words]
        speaker, lyric_text = split_speaker(text)
        if start - previous_end >= long_pause_ms:
            for count in range(1, 4):
                dot_start = start - (4 - count) * dot_interval_ms
                out.append(f"Dialogue: 2,{at(dot_start)},{at(dot_start + dot_interval_ms)},Prompt,,0,0,220,,{'.' * count}")
        slot = index % 2 if alternate else 0
        # Alternate mode shows a line early, but never before its row is free.
        shown = min(start, max(row_free[slot], start - lead_in_ms)) if alternate else start
        row_free[slot] = end
        if words:
            visible = karaoke_text(words, shown, karaoke_tag, syllable_template, effect_params)
        else:
            visible = esc(lyric_text)
        if speaker:
            semantic = {"duet": "&H00FFFFFF", "both": "&H00FFFFFF", "shared": "&H00FFFFFF", "backing": "&H0000FF00"}
            visible = f"{{\\c{semantic.get(speaker.casefold(), slots[speaker.casefold()])}}}{visible}"
        line_effect = apply_template(line_template, effect_params, start=0, mid=(end - shown) // 2, end=end - shown, dur=end - shown, index=index)
        if alternate:
            align_tag, margin_v = rows_layout[slot]
            prefix = f"{{\\an{align_tag}{line_effect}}}"
        else:
            align_tag, margin_v = 0, lyric_margin_v
            prefix = f"{{{line_effect}}}" if line_effect else ""
        romanized = romanize(lyric_text, row.get("language", ""), romanization_style)
        if romanized:
            roman_prefix = f"{{\\an{align_tag}}}" if alternate else ""
            roman_margin = margin_v + romanization_gap if alternate else 0
            out.append(f"Dialogue: 1,{at(shown)},{at(end)},Romanization,,0,0,{roman_margin},,{roman_prefix}{esc(romanized)}")
        out.append(f"Dialogue: 0,{at(shown)},{at(end)},Lyric,,0,0,{margin_v},,{prefix}{visible}")
        previous_end = max(previous_end, end)
    return "\n".join(out) + "\n"
