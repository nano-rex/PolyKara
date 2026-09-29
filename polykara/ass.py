from __future__ import annotations

from .config import DOT_INTERVAL_MS, LONG_PAUSE_MS, MAX_SINGERS, SPEAKER_PALETTE
from .subtitle import split_speaker


def at(value: int) -> str:
    cs = max(0, value) // 10
    return f"{cs // 360000}:{(cs // 6000) % 60:02d}:{(cs // 100) % 60:02d}.{cs % 100:02d}"


def esc(value: str) -> str:
    return value.replace("\\", "\\N").replace("{", "\\{").replace("}", "\\}").replace("\n", "\\N")


def ass(row: dict[str, str], entries) -> str:
    title = esc(f"{row['title']}  |  {row['artist']}")
    credit = esc(f"作词：{row['lyricist']}    作曲：{row['composer']}    字幕制作：{row['producer']}")
    out = [
        "[Script Info]", "ScriptType: v4.00+", "PlayResX: 1920", "PlayResY: 1080", "WrapStyle: 2", "ScaledBorderAndShadow: yes", "",
        "[V4+ Styles]", "Format: Name, Fontname, Fontsize, PrimaryColour, SecondaryColour, OutlineColour, BackColour, Bold, Italic, Underline, StrikeOut, ScaleX, ScaleY, Spacing, Angle, BorderStyle, Outline, Shadow, Alignment, MarginL, MarginR, MarginV, Encoding",
        "Style: Header,Arial,42,&H00FFFFFF,&H00FFFFFF,&H80000000,&H50000000,1,0,0,0,100,100,0,0,1,2,1,8,40,40,40,1",
        "Style: Credit,Arial,28,&H00FFFFFF,&H00FFFFFF,&H80000000,&H50000000,0,0,0,0,100,100,0,0,1,2,1,2,40,40,70,1",
        "Style: Lyric,Arial,58,&H00FF0000,&H00FFFFFF,&H80000000,&H50000000,1,0,0,0,100,100,0,0,1,3,1,2,80,80,150,1", "",
        "Style: Prompt,Arial,44,&H00FFFFFF,&H00FFFFFF,&H80000000,&H50000000,1,0,0,0,100,100,0,0,1,2,1,1,80,80,220,1", "",
        "[Events]", "Format: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text",
        f"Dialogue: 0,0:00:00.00,0:00:06.00,Header,,0,0,40,,{title}",
        f"Dialogue: 0,0:00:00.00,0:00:06.00,Credit,,0,0,70,,{credit}",
    ]
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
