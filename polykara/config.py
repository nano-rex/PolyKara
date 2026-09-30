from __future__ import annotations

import os
import re
import subprocess
import tomllib
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
MANIFEST = ROOT / "songs.csv"
CONFIG_FILE = ROOT / "polykara.toml"
WORK = ROOT / "work"
RAW, AUDIO, SUBTITLES, ALIGN, ASS, OUTPUT, METADATA, EXTERNAL_LYRICS = (WORK / n for n in ("raw", "audio", "subtitles", "align", "ass", "output", "metadata", "lyrics"))
DOWNLOAD_ARCHIVE = WORK / "download-archive.txt"
TIMECODE = re.compile(r"\[(\d+):(\d{2})(?:[.:](\d{1,3}))?\]")
WORDCODE = re.compile(r"<(\d+):(\d{2})(?:[.:](\d{1,3}))?>")
Entry = tuple[int, int, str, list[tuple[int, int, str]]]
LONG_PAUSE_MS = 30_000
DOT_INTERVAL_MS = 1_000
MAX_SINGERS = 16
SPEAKER_PALETTE = (
    "&H00B4771F", "&H000E7FFF", "&H002CA02C", "&H002827D6",
    "&H00BD6794", "&H004B568C", "&H00C277E3", "&H007F7F7F",
    "&H0022BDBC", "&H00CFBE17", "&H00808000", "&H0020A5DA",
    "&H00616FFF", "&H0002DEA4", "&H0082004B", "&H00C000C0",
)
SPEAKER_TAG = re.compile(r"^\[(?:singer|speaker|vocal|role)\s*:\s*([^\]]+)\]\s*", re.IGNORECASE)


DEFAULT_CONFIG = {
    "video": {"play_res_x": 1920, "play_res_y": 1080, "extend_intro": True},
    "title": {"enabled": True, "text": "{title}  |  {artist}", "font": "Arial", "size": 42, "color": "&H00FFFFFF", "horizontal": "center", "vertical": "top", "margin_l": 40, "margin_r": 40, "margin_v": 40, "start_ms": 0, "duration_ms": 6000, "fade_in_ms": 300, "fade_out_ms": 300, "bold": True, "italic": False, "outline": 2, "shadow": 1},
    "credit": {"enabled": True, "text": "作词：{lyricist}    作曲：{composer}    字幕制作：{producer}", "font": "Arial", "size": 28, "color": "&H00FFFFFF", "horizontal": "center", "vertical": "bottom", "margin_l": 40, "margin_r": 40, "margin_v": 70, "start_ms": 0, "duration_ms": 6000, "fade_in_ms": 300, "fade_out_ms": 300, "bold": False, "italic": False, "outline": 2, "shadow": 1},
    "lyric": {"font": "Arial", "size": 58, "color": "&H00FF0000", "secondary_color": "&H00FFFFFF", "outline_color": "&H80000000", "back_color": "&H50000000", "alignment": 2, "margin_l": 80, "margin_r": 80, "margin_v": 150, "bold": True, "italic": False, "outline": 3, "shadow": 1},
    "karaoke": {"active_color": "&H00FF0000", "inactive_color": "&H00FFFFFF", "tag": "kf"},
    "romanization": {"enabled": True, "languages": ["zh", "yue", "ja", "ko", "hi", "ta", "bn", "gu", "kn", "ml", "mr", "ne", "pa", "sa", "te", "or"], "font": "Arial", "size": 30, "color": "&H00FFFFFF", "outline_color": "&H80000000", "back_color": "&H50000000", "horizontal": "center", "vertical": "bottom", "margin_l": 80, "margin_r": 80, "margin_v": 225, "bold": False, "italic": False, "outline": 2, "shadow": 1, "tone": "numbers"},
    "lyrics": {"sources": ["lrclib", "lyrics.ovh", "webpage"]},
    "alignment": {"model": "auto", "device": "auto", "compute_type": "int8", "reserve_memory_gib": 2, "base_min_budget_gib": 2, "small_min_budget_gib": 4, "medium_min_budget_gib": 8, "large_min_budget_gib": 12, "cpu_threads": 0, "num_workers": 1},
    "timing": {"long_pause_ms": 30000, "dot_interval_ms": 1000},
    "singers": {"max": 16},
    "prompt": {"font": "Arial", "size": 44, "color": "&H00FFFFFF", "alignment": 1, "margin_l": 80, "margin_r": 80, "margin_v": 220, "bold": True, "italic": False, "outline": 2, "shadow": 1},
    "watermark": {"enabled": False, "text": "PolyKara", "font": "Arial", "size": 24, "color": "&H80FFFFFF", "alignment": 9, "margin_l": 40, "margin_r": 40, "margin_v": 40, "bold": False, "italic": False, "outline": 1, "shadow": 0},
    "charts": {"enabled_regions": ["my", "id", "au", "ca", "sg", "tw", "hk", "jp", "kr", "in", "cn", "us", "gb"]},
}


def _merge(base: dict, override: dict) -> dict:
    for key, value in override.items():
        if isinstance(value, dict) and isinstance(base.get(key), dict):
            _merge(base[key], value)
        else:
            base[key] = value
    return base


def load_config() -> dict:
    config = {key: (value.copy() if isinstance(value, dict) else list(value) if isinstance(value, list) else value) for key, value in DEFAULT_CONFIG.items()}
    if CONFIG_FILE.exists():
        with CONFIG_FILE.open("rb") as handle:
            _merge(config, tomllib.load(handle))
    return config


def enabled_regions() -> list[str]:
    return [str(region).lower() for region in load_config().get("charts", {}).get("enabled_regions", [])]


def run(cmd: list[str], dry_run: bool = False) -> None:
    print("$", " ".join(cmd))
    if not dry_run:
        subprocess.run(cmd, check=True)


def require_current_ytdlp() -> None:
    result = subprocess.run(["yt-dlp", "--version"], capture_output=True, text=True, check=False)
    version = result.stdout.strip()
    match = re.fullmatch(r"(\d{4})\.(\d{2})\.(\d{2})", version)
    if not match or int("".join(match.groups())) < 20250101:
        raise SystemExit(f"yt-dlp {version or 'unknown'} is too old for current YouTube. Update yt-dlp, then rerun this command.")


def total_memory_bytes() -> int:
    try:
        for line in Path("/proc/meminfo").read_text(encoding="utf-8").splitlines():
            if line.startswith("MemTotal:"):
                return int(line.split()[1]) * 1024
    except (OSError, ValueError, IndexError):
        pass
    return os.sysconf("SC_PAGE_SIZE") * os.sysconf("SC_PHYS_PAGES")
