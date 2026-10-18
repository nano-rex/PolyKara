"""KTV-style layout: two staggered lyric rows, an opening credits card and countdown dots.

This follows the look of the hosted karaoke videos: the upper row is aligned
left and the lower row right, lines are shown in pairs ahead of time, sung text
turns to the singer's colour with a white outline while unsung text stays white
with a dark outline, and romanized readings sit above their characters and fill
with them.

The generated ASS keeps plain \\kf karaoke so it stays editable in Aegisub.
The white sung outline cannot be expressed with \\kf, which only sweeps the
fill, so ``decorate`` adds it when rendering: every karaoke event gets a copy
in the sung colours whose \\clip follows the karaoke timing from left to right.
"""
from __future__ import annotations

import re
import unicodedata

from .layout import measured_font, text_width
from .romanize import romanize_words, ruby_groups
from .subtitle import split_speaker

# Credit labels (singer, composer, lyricist, subtitles) by song language; others use English.
LABELS = {
    "en": ("Singer", "Composer", "Lyricist", "Subtitle Production"),
    "zh": ("歌手", "作曲", "作词", "字幕制作"),
    "ms": ("Penyanyi", "Komposer", "Lirik", "Pembuatan Lirik"),
}
LABEL_LANGUAGES = {"yue": "zh", "id": "ms"}
FEMALE = {"女", "女声", "f", "female", "woman", "girl"}
MALE = {"男", "男声", "m", "male", "man", "boy"}
BOTH = {"合", "合唱", "both", "duet", "all", "together", "chorus"}
NAME_SEPARATORS = ("、", "，", ",", "/")
KARAOKE = re.compile(r"\\(kf|ko|k|K)(\d+)")
LYRIC_STYLES = {"Lyric", "Romanization"}


def voice_of(name: str) -> str | None:
    key = name.strip().casefold()
    return "female" if key in FEMALE else "male" if key in MALE else "both" if key in BOTH else None


def song_voice(row: dict[str, str]) -> str | None:
    return voice_of(row.get("voice") or "")


def label_language(language: str) -> str:
    base = language.casefold().replace("_", "-").split("-", 1)[0]
    base = LABEL_LANGUAGES.get(base, base)
    return base if base in LABELS else "en"


def wide_text(text: str) -> bool:
    return any(unicodedata.east_asian_width(char) in {"W", "F"} for char in text)


def width_of(text: str, values: dict, scale: float = 1.0) -> float:
    return text_width(text, str(values["font"]), float(values["size"]), bool(values.get("bold", False))) * scale


def countdown_ms(config: dict) -> int:
    settings = config.get("countdown", {})
    if not settings.get("enabled", True):
        return 0
    return max(0, int(settings.get("dots", 4))) * max(1, int(config.get("timing", {}).get("dot_interval_ms", 1000)))


def intro_padding_ms(entries, config: dict) -> int:
    """Delay the video when the first line is sung too soon for its countdown."""
    if not config["video"].get("extend_intro", True) or not entries:
        return 0
    need = countdown_ms(config)
    first = min(start for start, _, _, _ in entries)
    return max(0, need + 500 - first) if need else 0


def placements(entries, config: dict) -> list[dict]:
    """Decide the row and the time on screen of every line.

    ``pairs`` shows two lines at a time: both appear before the first is sung
    and leave after the second. ``alternate`` gives each row its next line as
    soon as the line there has been sung. A gap of page_break_ms or more
    starts again in the upper row, and a line after a long pause (or the first
    line) appears early enough for its countdown.
    """
    layout, timing = config["layout"], config.get("timing", {})
    mode = str(layout.get("mode", "pairs")).lower()
    lead, hold = max(0, int(layout.get("lead_ms", 2000))), max(0, int(layout.get("hold_ms", 800)))
    page_break = max(0, int(layout.get("page_break_ms", 6000)))
    long_pause = int(timing.get("long_pause_ms", 10000))
    countdown_lead = countdown_ms(config) + (500 if countdown_ms(config) else 0)
    placed = [{"start": start, "end": end, "slot": 0, "show": start, "hide": end, "countdown": False} for start, end, _, _ in entries]
    stanzas: list[list[int]] = []
    previous_end = None
    for index, item in enumerate(placed):
        if previous_end is None or item["start"] - previous_end >= page_break:
            stanzas.append([])
        stanzas[-1].append(index)
        item["countdown"] = countdown_lead > 0 and (previous_end is None or item["start"] - previous_end >= long_pause)
        previous_end = item["end"] if previous_end is None else max(previous_end, item["end"])
    floor = 0
    for stanza in stanzas:
        first = placed[stanza[0]]
        first_lead = max(lead, countdown_lead) if first["countdown"] else lead
        if mode == "alternate":
            for position, index in enumerate(stanza):
                item = placed[index]
                item["slot"] = position % 2
                if position < 2:
                    # The first two lines of a stanza appear together.
                    item["show"] = min(item["start"], max(floor, first["start"] - first_lead))
                else:
                    item["show"] = min(item["start"], max(placed[stanza[position - 2]]["hide"], item["start"] - lead))
                after = stanza[position + 2] if position + 2 < len(stanza) else None
                limit = placed[after]["start"] - lead if after is not None else item["end"] + hold
                item["hide"] = max(item["end"], min(item["end"] + hold, limit))
        else:
            pages = [stanza[offset:offset + 2] for offset in range(0, len(stanza), 2)]
            for number, page in enumerate(pages):
                start = placed[page[0]]["start"]
                end = max(placed[index]["end"] for index in page)
                show = min(start, max(floor, start - (first_lead if number == 0 else lead)))
                following = pages[number + 1] if number + 1 < len(pages) else None
                limit = placed[following[0]]["start"] - lead if following else end + hold
                hide = max(end, min(end + hold, limit))
                for slot, index in enumerate(page):
                    placed[index].update(slot=slot, show=show, hide=hide)
                floor = hide
        floor = max([floor] + [placed[index]["hide"] for index in stanza])
    return placed


def wrap_credit(head: str, value: str, style: dict, max_width: float) -> list[tuple[str, bool]]:
    """Rows of one credit as (text, indented); a long list of names continues under the first name."""
    names, current = [], ""
    for char in value:
        current += char
        if char in NAME_SEPARATORS or current.endswith(" & "):
            names.append(current)
            current = ""
    if current:
        names.append(current)
    indent = width_of(head, style)
    rows, line = [], ""
    for name in names:
        candidate = line + name if line else name.lstrip()
        if line and indent + width_of(candidate.rstrip(), style) > max_width:
            rows.append(line.rstrip())
            line = name.lstrip()
        else:
            line = candidate
    rows.append(line.rstrip())
    return [((head if number == 0 else "") + text, number > 0) for number, text in enumerate(rows) if text]


def style_line(name: str, values: dict, primary: str, secondary: str, outline: str, back: str, alignment: int = 1) -> str:
    return (
        f"Style: {name},{values['font']},{values['size']},{primary},{secondary},{outline},{back},"
        f"{1 if values.get('bold', False) else 0},{1 if values.get('italic', False) else 0},0,0,100,100,0,0,1,"
        f"{values.get('outline', 3)},{values.get('shadow', 0)},{alignment},0,0,0,1"
    )


def document(row: dict[str, str], entries, config: dict, at, esc, karaoke_text, speaker_palette) -> str:
    video, layout = config["video"], config["layout"]
    width, height = int(video["play_res_x"]), int(video["play_res_y"])
    lyric, roman, karaoke = dict(config["lyric"]), dict(config["romanization"]), config.get("karaoke", {})
    singers, card, countdown = config.get("singers", {}), config.get("card", {}), config.get("countdown", {})
    tag = str(karaoke.get("tag", "kf")).lower()
    if tag not in {"k", "kf"}:
        raise ValueError("karaoke.tag must be 'k' or 'kf'")
    unsung = str(karaoke.get("inactive_color", "&H00FFFFFF"))
    default_colour = str(karaoke.get("active_color", "&H000000FC"))
    sung_outline = str(karaoke.get("sung_outline_color", "&H00FFFFFF"))
    lyric_outline, lyric_back = str(lyric.get("outline_color", "&H00000000")), str(lyric.get("back_color", "&H80000000"))
    margin_l, margin_r = float(layout.get("margin_l", 105)), float(layout.get("margin_r", 105))
    row_y = (height - float(layout.get("top_margin_v", 250)), height - float(layout.get("bottom_margin_v", 70)))
    lyric_size, roman_size = float(lyric["size"]), float(roman["size"])
    gap = float(roman.get("gap", 8))
    roman_karaoke = bool(roman.get("karaoke", True))
    fade = max(0, int(layout.get("fade_ms", 250)))

    def colour_of(voice: str | None) -> str:
        return str(singers.get(f"{voice}_color", default_colour)) if voice else default_colour

    voice = song_voice(row)
    song_colour = colour_of(voice)
    countdown_style = {"font": countdown.get("font", lyric["font"]), "size": countdown.get("size", 40), "bold": False, "outline": countdown.get("outline", 2), "shadow": 0}
    title_style = {"font": card.get("font", lyric["font"]), "size": card.get("title_size", 96), "bold": card.get("title_bold", True), "outline": card.get("title_outline", 4), "shadow": card.get("title_shadow", 2)}
    credit_style = {"font": card.get("font", lyric["font"]), "size": card.get("credit_size", 46), "bold": card.get("credit_bold", True), "outline": card.get("credit_outline", 2), "shadow": card.get("credit_shadow", 2)}
    title_outline = str(card.get("title_outline_color", "voice"))
    title_outline = song_colour if title_outline.lower() == "voice" else title_outline
    out = [
        "[Script Info]", "ScriptType: v4.00+", f"PlayResX: {width}", f"PlayResY: {height}", "WrapStyle: 2", "ScaledBorderAndShadow: yes", "; PolyKaraLayout: ktv", "",
        "[V4+ Styles]", "Format: Name, Fontname, Fontsize, PrimaryColour, SecondaryColour, OutlineColour, BackColour, Bold, Italic, Underline, StrikeOut, ScaleX, ScaleY, Spacing, Angle, BorderStyle, Outline, Shadow, Alignment, MarginL, MarginR, MarginV, Encoding",
        style_line("Lyric", lyric, song_colour, unsung, lyric_outline, lyric_back),
        style_line("Romanization", roman, song_colour, unsung, str(roman.get("outline_color", lyric_outline)), str(roman.get("back_color", lyric_back)), 2),
        style_line("Label", lyric, song_colour, unsung, sung_outline, lyric_back),
        style_line("Prompt", countdown_style, song_colour, song_colour, sung_outline, "&H00000000"),
        style_line("CardTitle", title_style, str(card.get("title_color", "&H00FFFFFF")), "&H00FFFFFF", title_outline, "&H80000000", 5),
        style_line("CardCredit", credit_style, str(card.get("credit_color", "&H00FFFFFF")), "&H00FFFFFF", str(card.get("credit_outline_color", "&H00000000")), "&H80000000", 7),
    ]
    watermark = config.get("watermark", {})
    if watermark.get("enabled", False) and watermark.get("text", ""):
        out.append(style_line("Watermark", watermark, str(watermark.get("color", "&H80FFFFFF")), "&H00FFFFFF", "&H80000000", "&H00000000", int(watermark.get("alignment", 9))))
    out += ["", "[Events]", "Format: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text"]

    padding = intro_padding_ms(entries, config)
    out.insert(1, f"; PolyKaraIntroPaddingMs: {padding}")
    shifted = [(start + padding, end + padding, text, [(left + padding, right + padding, word) for left, right, word in words]) for start, end, text, words in entries]
    placed = placements(shifted, config)

    # Opening card: the title centred, the credits as a left-aligned block below it.
    if card.get("enabled", True):
        card_start = max(0, int(card.get("start_ms", 0)))
        card_end = card_start + max(0, int(card.get("duration_ms", 5000)))
        card_fade = f"\\fad({int(card.get('fade_in_ms', 300))},{int(card.get('fade_out_ms', 500))})"
        if card_end > card_start:
            out.append(f"Dialogue: 0,{at(card_start)},{at(card_end)},CardTitle,,0,0,0,,{{\\an5\\pos({width / 2:.0f},{float(card.get('title_y', 190)):.0f})\\blur{card.get('title_blur', 3)}{card_fade}}}{esc(row.get('title', ''))}")
            language = label_language(row.get("language", ""))
            labels = card.get("labels", {}).get(language) or LABELS[language]
            values = (row.get("artist", ""), row.get("composer", ""), row.get("lyricist", ""), row.get("producer", ""))
            max_width = float(card.get("credit_max_width", 1100))
            rows: list[tuple[str, float]] = []
            for label, value in zip(labels, values):
                if value.strip():
                    head = f"{label} : "
                    rows += [(text, width_of(head, credit_style) if indented else 0.0) for text, indented in wrap_credit(head, value.strip(), credit_style, max_width)]
            if rows:
                line_height = float(credit_style["size"]) * float(card.get("credit_line_spacing", 1.25))
                top = float(card.get("credit_y", 300))
                # Keep the credits above the upper lyric row, shrinking them when there are many.
                limit = row_y[0] - lyric_size - roman_size - gap - 40
                scale = max(0.6, min(1.0, (limit - top) / (line_height * len(rows))))
                block = max(offset + width_of(text, credit_style) for text, offset in rows) * scale
                left = (width - block) / 2
                size = f"\\fscx{scale * 100:.0f}\\fscy{scale * 100:.0f}" if scale < 1 else ""
                for number, (text, offset) in enumerate(rows):
                    out.append(f"Dialogue: 0,{at(card_start)},{at(card_end)},CardCredit,,0,0,0,,{{\\an7\\pos({left + offset * scale:.0f},{top + number * line_height * scale:.0f}){size}{card_fade}}}{esc(text)}")

    if watermark.get("enabled", False) and watermark.get("text", ""):
        out.append(f"Dialogue: 1,0:00:00.00,9:59:59.99,Watermark,,0,0,0,,{esc(str(watermark['text']))}")

    named = []
    for _, _, text, _ in shifted:
        speaker, _ = split_speaker(text)
        if speaker and voice_of(speaker) is None and speaker.casefold() not in {item.casefold() for item in named}:
            named.append(speaker)
    max_singers = int(singers.get("max", 16))
    if len(named) > max_singers:
        raise ValueError(f"{row['id']} has {len(named)} singers; maximum is {max_singers}")
    slots = {speaker.casefold(): speaker_palette[index % len(speaker_palette)] for index, speaker in enumerate(named)}
    language = row.get("language", "")
    ruby_spacing = float(roman.get("ruby_spacing", 0.04))
    # Lines in Chinese, Japanese or Korean script use cjk_size: those fonts draw their
    # characters smaller than Latin letters at the same font size.
    cjk_size = float(lyric.get("cjk_size", lyric_size))
    # Readings sit this fraction of the font size above the bottom of the row; CJK fonts
    # leave room above their characters, so the reading is lowered to sit close to them.
    ruby_rise = float(roman.get("ruby_rise", 0.78))
    show_label = bool(singers.get("show_label", True))
    interval = max(1, int(config.get("timing", {}).get("dot_interval_ms", 1000)))

    for number, ((start, end, text, words), place) in enumerate(zip(shifted, placed), 1):
        # The Name field numbers the lines, since both lines of a pair share their times.
        name = f"line{number}"
        speaker, body = split_speaker(text)
        line_voice = voice_of(speaker) if speaker else None
        colour = colour_of(line_voice) if line_voice else slots[speaker.casefold()] if speaker else song_colour
        colour_tag = f"{{\\1c{colour}}}" if colour != song_colour else ""
        label = ""
        if speaker and show_label:
            label = f"{speaker}：" if wide_text(speaker) else f"{speaker}: "
        # "rows" puts the whole romanized line above the lyric instead of a reading over each piece.
        rows_layout = str(roman.get("layout", "ruby")).lower() == "rows"
        groups = ruby_groups(words, language, roman) if words and not rows_layout else None
        line_size = cjk_size if wide_text(body) else lyric_size
        line_style = dict(lyric, size=line_size)
        space, spacing = width_of(" ", line_style), line_size * ruby_spacing
        label_width = width_of(label, line_style) if label else 0.0
        if groups:
            columns = []
            for group in groups:
                base = "".join(word for _, _, word in group["tokens"])
                column = max(width_of(base, line_style), width_of(group["roman"] or "", roman)) + spacing
                columns.append((space if group["space_before"] else 0.0, column))
            total = label_width + sum(before + column for before, column in columns)
        else:
            content = "".join(word for _, _, word in words) if words else body
            total = label_width + width_of(content, line_style)
        available = width - margin_l - margin_r
        scale = min(1.0, available / total) if total else 1.0
        slot, show, hide = place["slot"], place["show"], place["hide"]
        y = row_y[slot]
        left = margin_l if slot == 0 else width - margin_r - total * scale
        font_size = f"\\fs{line_size:g}" if line_size != lyric_size else ""
        size = f"\\fscx{scale * 100:.0f}\\fscy{scale * 100:.0f}" if scale < 1 else ""
        fades = f"\\fad({fade},{fade})" if fade and hide - show > 4 * fade else ""
        if label:
            out.append(f"Dialogue: 2,{at(show)},{at(hide)},Label,{name},0,0,0,,{{\\an1\\pos({left:.0f},{y:.0f}){font_size}{size}{fades}\\1c{colour}}}{esc(label)}")
        x = left + label_width * scale
        has_roman = False
        if groups:
            roman_y = y - line_size * scale * ruby_rise - gap
            for group, (before, column) in zip(groups, columns):
                centre = x + (before + column / 2) * scale
                x += (before + column) * scale
                out.append(f"Dialogue: 0,{at(show)},{at(hide)},Lyric,{name},0,0,0,,{{\\an2\\pos({centre:.0f},{y:.0f}){font_size}{size}{fades}}}{colour_tag}{karaoke_text(group['tokens'], show, tag)}")
                if group["roman"]:
                    has_roman = True
                    reading = karaoke_text([(group["tokens"][0][0], group["tokens"][-1][1], group["roman"])], show, tag) if roman_karaoke else f"{{\\1c{unsung}}}{esc(group['roman'])}"
                    out.append(f"Dialogue: 1,{at(show)},{at(hide)},Romanization,{name},0,0,0,,{{\\an2\\pos({centre:.0f},{roman_y:.0f}){size}{fades}}}{colour_tag}{reading}")
        else:
            visible = karaoke_text(words, show, tag) if words else f"{{\\1c{unsung}}}{esc(body)}"
            out.append(f"Dialogue: 0,{at(show)},{at(hide)},Lyric,{name},0,0,0,,{{\\an1\\pos({x:.0f},{y:.0f}){font_size}{size}{fades}}}{colour_tag}{visible}")
            timed_roman = romanize_words(words, language, roman) if rows_layout and words else None
            if timed_roman:
                has_roman = True
                roman_y = y - line_size * scale * ruby_rise - gap
                anchor, roman_x = (1, x) if slot == 0 else (3, width - margin_r)
                reading = karaoke_text(timed_roman, show, tag) if roman_karaoke else f"{{\\1c{unsung}}}{esc(''.join(word for _, _, word in timed_roman).strip())}"
                out.append(f"Dialogue: 1,{at(show)},{at(hide)},Romanization,{name},0,0,0,,{{\\an{anchor}\\pos({roman_x:.0f},{roman_y:.0f}){size}{fades}}}{colour_tag}{reading}")
        if place["countdown"]:
            dots = min(int(countdown.get("dots", 4)), max(0, start - show) // interval)
            dot_y = y - line_size * scale * (ruby_rise if has_roman else 1) - ((roman_size * scale + gap) if has_roman else 0) - float(countdown.get("gap", 6))
            dot_colour = f"\\1c{colour}\\2c{colour}" if colour != song_colour else ""
            for number in range(dots):
                begin = start - (dots - number) * interval
                out.append(f"Dialogue: 2,{at(begin)},{at(begin + interval)},Prompt,,0,0,0,,{{\\an1\\pos({left:.0f},{dot_y:.0f}){dot_colour}}}{str(countdown.get('char', '●')) * (dots - number)}")
    return "\n".join(out) + "\n"


def _styles(text: str) -> dict[str, dict]:
    styles, fields = {}, []
    for line in text.splitlines():
        if line.startswith("Format:") and not fields and "Fontname" in line:
            fields = [item.strip() for item in line[len("Format:"):].split(",")]
        elif line.startswith("Style:") and fields:
            values = dict(zip(fields, (item.strip() for item in line[len("Style:"):].split(","))))
            styles[values["Name"]] = {"font": values["Fontname"], "size": float(values["Fontsize"]), "bold": values.get("Bold", "0") not in {"0", ""}, "outline": float(values.get("Outline", 0)), "shadow": float(values.get("Shadow", 0))}
    return styles


def _segments(text: str) -> tuple[str, list[tuple[str, int, str]], dict] | None:
    """Split event text into the text before the first syllable and (kind, centiseconds, text) syllables."""
    before, syllables, tags, index = "", [], "", 0
    while index < len(text):
        if text[index] == "{":
            close = text.find("}", index)
            if close < 0:
                return None
            block = text[index + 1:close]
            tags += block
            for kind, duration in KARAOKE.findall(block):
                syllables.append((kind, int(duration), ""))
            index = close + 1
            continue
        plain = ""
        while index < len(text) and text[index] != "{":
            step = 2 if text[index] == "\\" and index + 1 < len(text) else 1
            plain += text[index:index + step]
            index += step
        plain = plain.replace("\\h", " ").replace("\\N", "").replace("\\{", "{").replace("\\}", "}")
        if syllables:
            kind, duration, existing = syllables[-1]
            syllables[-1] = (kind, duration, existing + plain)
        else:
            before += plain
    position = re.search(r"\\pos\(\s*([-\d.]+)\s*,\s*([-\d.]+)\s*\)", tags)
    if not position or not syllables:
        return None
    anchor = re.search(r"\\an(\d)", tags)
    scale = re.search(r"\\fscx([\d.]+)", tags)
    size = re.search(r"\\fs([\d.]+)", tags)
    return before, syllables, {"x": float(position.group(1)), "an": int(anchor.group(1)) if anchor else 1, "scale": float(scale.group(1)) / 100 if scale else 1.0, "size": float(size.group(1)) if size else None}


def decorate(text: str, config: dict) -> tuple[str, bool]:
    """Add the sung-colour overlay to every karaoke event; returns (text, decorated).

    The overlay is placed by measuring the text, so it is only added when the
    fonts can be measured (Pillow and fontconfig); otherwise the plain \\kf
    karaoke is rendered as it is.
    """
    styles = _styles(text)
    for name in LYRIC_STYLES:
        values = styles.get(name)
        if values is None or measured_font(values["font"], values["bold"]) is None:
            return text, False
    height = int(config["video"]["play_res_y"])
    sung_outline = str(config.get("karaoke", {}).get("sung_outline_color", "&H00FFFFFF"))
    out = []
    for line in text.splitlines():
        out.append(line)
        if not line.startswith("Dialogue:"):
            continue
        fields = line[len("Dialogue:"):].split(",", 9)
        if len(fields) < 10 or fields[3].strip() not in LYRIC_STYLES or "\\k" not in fields[9]:
            continue
        parsed = _segments(fields[9])
        if parsed is None:
            continue
        before, syllables, place = parsed
        style = styles[fields[3].strip()]
        values = {"font": style["font"], "size": place["size"] or style["size"], "bold": style["bold"]}
        widths = [width_of(syllable, values, place["scale"]) for _, _, syllable in syllables]
        lead = width_of(before, values, place["scale"]) if before else 0.0
        total = lead + sum(widths)
        column = (place["an"] - 1) % 3
        left = place["x"] - (0 if column == 0 else total / 2 if column == 1 else total)
        pad = (style["outline"] + style["shadow"]) * place["scale"] + 4
        start = left - pad
        clips = [f"\\clip({start:.0f},0,{left + lead if before else start:.0f},{height})"]
        cursor, elapsed = left + lead, 0
        for number, ((kind, duration, syllable), width) in enumerate(zip(syllables, widths)):
            if syllable.strip() or width > 0:
                cursor += width
                edge = cursor + (pad if number == len(syllables) - 1 else 0)
                begin, finish = elapsed * 10, (elapsed + duration) * 10 if kind in {"kf", "K"} else elapsed * 10 + 1
                clips.append(f"\\t({begin},{max(finish, begin + 1)},\\clip({start:.0f},0,{edge:.0f},{height}))")
            elapsed += duration
        body = KARAOKE.sub("", fields[9]).replace("{}", "")
        close = body.find("}")
        overlay = f"{{\\3c{sung_outline}{''.join(clips)}}}"
        body = body[:close + 1] + overlay + body[close + 1:] if body.startswith("{") and close > 0 else overlay + body
        layer = int(fields[0].strip() or 0) + 5
        out.append("Dialogue: " + ",".join([str(layer)] + fields[1:9] + [body]))
    return "\n".join(out) + ("\n" if text.endswith("\n") else ""), True
