from __future__ import annotations

from .config import DOT_INTERVAL_MS, LONG_PAUSE_MS, MAX_SINGERS, SPEAKER_PALETTE, load_config
from .subtitle import split_speaker


def at(value: int) -> str:
    cs = max(0, value) // 10
    return f"{cs // 360000}:{(cs // 6000) % 60:02d}:{(cs // 100) % 60:02d}.{cs % 100:02d}"


def esc(value: str) -> str:
    return value.replace("\\", "\\N").replace("{", "\\{").replace("}", "\\}").replace("\n", "\\N")


def ass(row: dict[str, str], entries) -> str:
    config = load_config()
    video, title_style = config["video"], config["title"]
    credit_style, lyric_style = config["credit"], config["lyric"]
    prompt_style, watermark_style = config["prompt"], config["watermark"]
    def text_template(values: dict, fallback: str) -> str:
        try:
            return str(values.get("text", fallback)).format(**row)
        except KeyError as exc:
            raise ValueError(f"Unknown title/credit template field: {exc.args[0]}") from exc
    title = esc(text_template(title_style, "{title}  |  {artist}"))
    credit = esc(text_template(credit_style, "作词：{lyricist}    作曲：{composer}    字幕制作：{producer}"))
    def style(name: str, values: dict, primary: str, secondary: str = "&H00FFFFFF", outline: str = "&H80000000", back: str = "&H50000000") -> str:
        return f"Style: {name},{values['font']},{values['size']},{values.get('color', primary)},{values.get('secondary_color', secondary)},{values.get('outline_color', outline)},{values.get('back_color', back)},1,0,0,0,100,100,0,0,1,2,1,{values['alignment']},{values['margin_l']},{values['margin_r']},{values['margin_v']},1"
    out = [
        "[Script Info]", "ScriptType: v4.00+", f"PlayResX: {video['play_res_x']}", f"PlayResY: {video['play_res_y']}", "WrapStyle: 2", "ScaledBorderAndShadow: yes", "",
        "[V4+ Styles]", "Format: Name, Fontname, Fontsize, PrimaryColour, SecondaryColour, OutlineColour, BackColour, Bold, Italic, Underline, StrikeOut, ScaleX, ScaleY, Spacing, Angle, BorderStyle, Outline, Shadow, Alignment, MarginL, MarginR, MarginV, Encoding",
        style("Header", title_style, "&H00FFFFFF"), style("Credit", credit_style, "&H00FFFFFF"), style("Lyric", lyric_style, "&H00FF0000"), style("Prompt", prompt_style, "&H00FFFFFF"), style("Watermark", watermark_style, "&H80FFFFFF"), "",
        "[Events]", "Format: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text",
    ]
    if title_style.get("enabled", True):
        out.append(f"Dialogue: 0,0:00:00.00,0:00:06.00,Header,,0,0,0,,{title}")
    if credit_style.get("enabled", True):
        out.append(f"Dialogue: 0,0:00:00.00,0:00:06.00,Credit,,0,0,0,,{credit}")
    if watermark_style.get("enabled", False) and watermark_style.get("text", ""):
        out.append(f"Dialogue: 1,0:00:00.00,9:59:59.99,Watermark,,0,0,0,,{esc(str(watermark_style['text']))}")
    speakers = []
    for _, _, text, _ in entries:
        speaker, _ = split_speaker(text)
        if speaker and speaker.casefold() not in {item.casefold() for item in speakers}:
            speakers.append(speaker)
    if len(speakers) > MAX_SINGERS:
        raise ValueError(f"{row['id']} has {len(speakers)} singers; maximum is {MAX_SINGERS}")
    slots = {speaker.casefold(): SPEAKER_PALETTE[index] for index, speaker in enumerate(speakers)}
    previous_end = 0
    for start, end, text, words in entries:
        speaker, lyric_text = split_speaker(text)
        if start - previous_end >= LONG_PAUSE_MS:
            for count in range(1, 4):
                dot_start = start - (4 - count) * DOT_INTERVAL_MS
                out.append(f"Dialogue: 2,{at(dot_start)},{at(dot_start + DOT_INTERVAL_MS)},Prompt,,0,0,220,,{'.' * count}")
        if words:
            visible = " ".join(f"{{\\k{max(1, round((right - left) / 10))}}}{esc(word)}" for left, right, word in words)
        else:
            visible = esc(lyric_text)
        if speaker:
            semantic = {"duet": "&H00FFFFFF", "both": "&H00FFFFFF", "shared": "&H00FFFFFF", "backing": "&H0000FF00"}
            visible = f"{{\\c{semantic.get(speaker.casefold(), slots[speaker.casefold()])}}}{visible}"
        out.append(f"Dialogue: 0,{at(start)},{at(end)},Lyric,,0,0,150,,{visible}")
        previous_end = max(previous_end, end)
    return "\n".join(out) + "\n"
