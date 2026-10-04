from __future__ import annotations

from .config import DOT_INTERVAL_MS, LONG_PAUSE_MS, MAX_SINGERS, SPEAKER_PALETTE, load_config
from .layout import text_width
from .romanize import romanize, romanize_words, ruby_groups
from .subtitle import split_speaker


def at(value: int) -> str:
    cs = max(0, value) // 10
    return f"{cs // 360000}:{(cs // 6000) % 60:02d}:{(cs // 100) % 60:02d}.{cs % 100:02d}"


def esc(value: str) -> str:
    # A literal backslash would start an ASS override sequence, so it is shown as a slash.
    return value.replace("\\", "/").replace("{", "\\{").replace("}", "\\}").replace("\n", "\\N")


def karaoke_text(words, line_start: int, tag: str) -> str:
    """Build karaoke tags whose durations add up from the line start.

    ASS karaoke durations are cumulative, so the wait before the first word and
    every pause between words is emitted as an empty \\k syllable.
    """
    parts, cursor = [], max(0, line_start) // 10
    for left, right, word in words:
        begin = max(cursor, left // 10)
        finish = max(begin + 1, right // 10)
        if begin > cursor:
            parts.append(f"{{\\k{begin - cursor}}}")
        parts.append(f"{{\\{tag}{finish - begin}}}{esc(word)}")
        cursor = finish
    return "".join(parts)


def alignment(values: dict) -> int:
    if "alignment" in values:
        return int(values["alignment"])
    horizontal = {"left": 0, "center": 1, "right": 2}.get(str(values.get("horizontal", "center")).lower(), 1)
    vertical = {"bottom": 0, "middle": 3, "center": 3, "top": 6}.get(str(values.get("vertical", "bottom")).lower(), 0)
    return vertical + horizontal + 1


def card_end_ms(config: dict) -> int:
    card_end = 0
    for name in ("title", "credit"):
        values = config[name]
        if values.get("enabled", True):
            card_end = max(card_end, int(values.get("start_ms", 0)) + max(0, int(values.get("duration_ms", 6000))))
    return card_end


def intro_padding_ms(entries) -> int:
    config = load_config()
    if not config["video"].get("extend_intro", True) or not entries:
        return 0
    first_lyric = min(start for start, _, _, _ in entries)
    return max(0, card_end_ms(config) - first_lyric)


def ruby_events(groups: list[dict], shown: int, end: int, colour: str, tag: str, karaoke: bool, video: dict, lyric: dict, roman: dict) -> list[str] | None:
    """Place each romanized piece directly above the characters it reads.

    Every piece and its characters become a pair of events centred on the same
    x position. Returns None when the line cannot be laid out this way, so the
    caller falls back to two plain rows.
    """
    width, height = float(video["play_res_x"]), float(video["play_res_y"])
    lyric_size, roman_size = float(lyric["size"]), float(roman["size"])
    lyric_bold, roman_bold = bool(lyric.get("bold", True)), bool(roman.get("bold", False))
    spacing = lyric_size * float(roman.get("ruby_spacing", 0.15))
    space = text_width(" ", lyric["font"], lyric_size, lyric_bold)
    columns = []
    for group in groups:
        base = "".join(word for _, _, word in group["tokens"])
        column = max(text_width(base, lyric["font"], lyric_size, lyric_bold), text_width(group["roman"] or "", roman["font"], roman_size, roman_bold)) + spacing
        columns.append((space if group["space_before"] else 0.0, column))
    total = sum(before + column for before, column in columns)
    left_margin, right_margin = float(lyric["margin_l"]), float(lyric["margin_r"])
    available = width - left_margin - right_margin
    scale = min(1.0, available / total) if total else 1.0
    position = alignment(lyric)
    horizontal, vertical = (position - 1) % 3, (position - 1) // 3
    # A line too long to fit at half size, or vertically centred lyrics, keep the two-row layout.
    if scale < 0.5 or vertical == 1:
        return None
    x = left_margin if horizontal == 0 else width - right_margin - total * scale if horizontal == 2 else left_margin + (available - total * scale) / 2
    gap = float(roman.get("gap", 6))
    if vertical == 0:
        anchor, base_y = 2, height - float(lyric["margin_v"])
        roman_y = base_y - lyric_size * scale - gap
    else:
        anchor, roman_y = 8, float(lyric["margin_v"])
        base_y = roman_y + roman_size * scale + gap
    size_tags = f"\\fscx{scale * 100:.0f}\\fscy{scale * 100:.0f}" if scale < 1 else ""
    events = []
    for group, (before, column) in zip(groups, columns):
        centre = x + (before + column / 2) * scale
        x += (before + column) * scale
        events.append(f"Dialogue: 0,{at(shown)},{at(end)},Lyric,,0,0,0,,{{\\an{anchor}\\pos({centre:.0f},{base_y:.0f}){size_tags}}}{colour}{karaoke_text(group['tokens'], shown, tag)}")
        if group["roman"]:
            reading = karaoke_text([(group["tokens"][0][0], group["tokens"][-1][1], group["roman"])], shown, tag) if karaoke else esc(group["roman"])
            events.append(f"Dialogue: 1,{at(shown)},{at(end)},Romanization,,0,0,0,,{{\\an{anchor}\\pos({centre:.0f},{roman_y:.0f}){size_tags}}}{colour}{reading}")
    return events


def ass(row: dict[str, str], entries) -> str:
    config = load_config()
    video, title_style = config["video"], config["title"]
    credit_style, lyric_style = config["credit"], dict(config["lyric"])
    karaoke_style = config.get("karaoke", {})
    lyric_style["color"] = karaoke_style.get("active_color", lyric_style.get("color", "&H00FF0000"))
    lyric_style["secondary_color"] = karaoke_style.get("inactive_color", lyric_style.get("secondary_color", "&H00FFFFFF"))
    prompt_style, watermark_style = config["prompt"], config["watermark"]
    romanization_style = dict(config.get("romanization", {}))
    romanize_karaoke = bool(romanization_style.get("karaoke", True))
    if romanize_karaoke:
        # The romanized row fills with the same colours as the original row.
        romanization_style["color"], romanization_style["secondary_color"] = lyric_style["color"], lyric_style["secondary_color"]
    # MarginV for lyric lines that have a romanized row; 0 keeps the style's margin.
    lyric_margin = 0
    if str(romanization_style.get("margin_v", "auto")).lower() == "auto":
        # Two rows: the romanized reading on top, the original characters directly below it.
        lyric_alignment, gap = alignment(lyric_style), int(romanization_style.get("gap", 6))
        romanization_style.update(alignment=lyric_alignment, margin_l=lyric_style["margin_l"], margin_r=lyric_style["margin_r"])
        if lyric_alignment >= 7:
            romanization_style["margin_v"] = lyric_style["margin_v"]
            lyric_margin = int(lyric_style["margin_v"]) + int(romanization_style["size"]) + gap
        else:
            romanization_style["margin_v"] = int(lyric_style["margin_v"]) + int(lyric_style["size"]) + gap
    def text_template(values: dict, fallback: str) -> str:
        try:
            return str(values.get("text", fallback)).format(**row)
        except KeyError as exc:
            raise ValueError(f"Unknown title/credit template field: {exc.args[0]}") from exc
        except (IndexError, ValueError) as exc:
            raise ValueError(f"Invalid title/credit template ({exc}); write literal braces as {{{{ and }}}}") from exc
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
    slots = {speaker.casefold(): SPEAKER_PALETTE[index % len(SPEAKER_PALETTE)] for index, speaker in enumerate(speakers)}
    timing = config.get("timing", {})
    long_pause_ms = int(timing.get("long_pause_ms", LONG_PAUSE_MS))
    dot_interval_ms = int(timing.get("dot_interval_ms", DOT_INTERVAL_MS))
    karaoke_tag = str(karaoke_style.get("tag", "kf")).lower()
    if karaoke_tag not in {"k", "kf"}:
        raise ValueError("karaoke.tag must be 'k' or 'kf'")
    lead_in_ms = max(0, int(timing.get("lead_in_ms", 0)))
    # Lines never appear early enough to collide with the title cards or the previous line.
    previous_end, shown_until = 0, min(card_end_ms(config), first_lyric + intro_padding)
    for start, end, text, words in entries:
        start += intro_padding
        end += intro_padding
        speaker, lyric_text = split_speaker(text)
        if start - previous_end >= long_pause_ms:
            for count in range(1, 4):
                dot_start = start - (4 - count) * dot_interval_ms
                out.append(f"Dialogue: 2,{at(dot_start)},{at(dot_start + dot_interval_ms)},Prompt,,0,0,0,,{'.' * count}")
        shown = min(start, max(shown_until, start - lead_in_ms))
        if words:
            visible = karaoke_text([(left + intro_padding, right + intro_padding, word) for left, right, word in words], shown, karaoke_tag)
        else:
            visible = esc(lyric_text)
        colour = ""
        if speaker:
            semantic = {"duet": "&H00FFFFFF", "both": "&H00FFFFFF", "shared": "&H00FFFFFF", "backing": "&H0000FF00"}
            colour = f"{{\\c{semantic.get(speaker.casefold(), slots[speaker.casefold()])}}}"
            visible = colour + visible
        language = row.get("language", "")
        if words and str(romanization_style.get("layout", "ruby")).lower() == "ruby":
            groups = ruby_groups(words, language, romanization_style)
            shifted = [{**group, "tokens": [(left + intro_padding, right + intro_padding, word) for left, right, word in group["tokens"]]} for group in groups or []]
            events = ruby_events(shifted, shown, end, colour, karaoke_tag, romanize_karaoke, video, lyric_style, romanization_style) if groups else None
            if events:
                out.extend(events)
                previous_end = max(previous_end, end)
                shown_until = max(shown_until, end)
                continue
        timed_roman = romanize_words(words, language, romanization_style) if words else None
        if timed_roman and romanize_karaoke:
            # Each romanized piece is timed from the characters it reads, so both rows fill together.
            romanized = colour + karaoke_text([(left + intro_padding, right + intro_padding, word) for left, right, word in timed_roman], shown, karaoke_tag)
        else:
            plain = "".join(word for _, _, word in timed_roman).strip() if timed_roman else romanize(lyric_text, language, romanization_style)
            romanized = esc(plain) if plain else ""
        if romanized:
            out.append(f"Dialogue: 1,{at(shown)},{at(end)},Romanization,,0,0,0,,{romanized}")
        out.append(f"Dialogue: 0,{at(shown)},{at(end)},Lyric,,0,0,{lyric_margin if romanized else 0},,{visible}")
        previous_end = max(previous_end, end)
        shown_until = max(shown_until, end)
    return "\n".join(out) + "\n"
