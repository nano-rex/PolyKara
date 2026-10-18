from __future__ import annotations

import copy
import functools
import os
import re
import subprocess
import tomllib
from pathlib import Path

import sys

# Program files: the repository, or the unpacked application when PolyKara is installed.
ROOT = Path(getattr(sys, "_MEIPASS", Path(__file__).resolve().parent.parent))


def _home() -> Path:
    """Where songs.csv, polykara.toml and work/ live.

    POLYKARA_HOME chooses the folder. The installed application uses
    Documents/PolyKara, because its program folder is not meant for user files;
    a checkout of the repository keeps using the repository itself.
    """
    if os.environ.get("POLYKARA_HOME"):
        return Path(os.environ["POLYKARA_HOME"]).expanduser()
    if getattr(sys, "frozen", False):
        return Path.home() / "Documents" / "PolyKara"
    return ROOT


HOME = _home()
MANIFEST = HOME / "songs.csv"
CONFIG_FILE = HOME / "polykara.toml"
WORK = HOME / "work"
RAW, AUDIO, SUBTITLES, ALIGN, ASS, OUTPUT, METADATA, EXTERNAL_LYRICS = (WORK / n for n in ("raw", "audio", "subtitles", "align", "ass", "output", "metadata", "lyrics"))
MEDIA_SUFFIXES = (".mp4", ".mkv", ".webm", ".mov", ".m4v", ".flv", ".avi", ".ts", ".mpg", ".mpeg", ".vob", ".wmv", ".mp3", ".m4a", ".flac", ".wav", ".ogg", ".opus", ".aac", ".wma")
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
    "render": {"video_codec": "libx264", "preset": "medium", "crf": 18, "pixel_format": "yuv420p", "audio_codec": "aac", "audio_bitrate": "192k", "extra_args": [], "fit": "blur", "blur_sigma": 30, "background_color": "black"},
    "logo": {"file": "", "width": 220, "position": "top-right", "margin_x": 40, "margin_y": 40, "opacity": 1.0},
    "title": {"enabled": True, "text": "{title}  |  {artist}", "font": "Arial", "size": 42, "color": "&H00FFFFFF", "horizontal": "center", "vertical": "top", "margin_l": 40, "margin_r": 40, "margin_v": 40, "start_ms": 0, "duration_ms": 6000, "fade_in_ms": 300, "fade_out_ms": 300, "bold": True, "italic": False, "outline": 2, "shadow": 1},
    "credit": {"enabled": True, "text": "作词：{lyricist}    作曲：{composer}    字幕制作：{producer}", "font": "Arial", "size": 28, "color": "&H00FFFFFF", "horizontal": "center", "vertical": "bottom", "margin_l": 40, "margin_r": 40, "margin_v": 70, "start_ms": 0, "duration_ms": 6000, "fade_in_ms": 300, "fade_out_ms": 300, "bold": False, "italic": False, "outline": 2, "shadow": 1},
    "layout": {"mode": "pairs", "margin_l": 105, "margin_r": 105, "top_margin_v": 230, "bottom_margin_v": 78, "lead_ms": 2000, "hold_ms": 800, "fade_ms": 250, "page_break_ms": 6000},
    "card": {"enabled": True, "font": "Calibri", "start_ms": 0, "duration_ms": 5000, "fade_in_ms": 300, "fade_out_ms": 500, "title_y": 170, "title_size": 130, "title_bold": True, "title_color": "&H00FFFFFF", "title_outline_color": "voice", "title_outline": 3, "title_shadow": 2, "title_blur": 4, "credit_y": 280, "credit_size": 54, "credit_bold": True, "credit_color": "&H00FFFFFF", "credit_outline_color": "&H00000000", "credit_outline": 2, "credit_shadow": 2, "credit_line_spacing": 1.3, "credit_max_width": 1100, "labels": {}},
    "countdown": {"enabled": True, "dots": 4, "char": "●", "font": "Arial", "size": 56, "outline": 2, "gap": 6},
    "lyric": {"font": "Calibri", "size": 120, "cjk_size": 145, "color": "&H000000FC", "secondary_color": "&H00FFFFFF", "outline_color": "&H00000000", "back_color": "&H80000000", "alignment": 2, "margin_l": 80, "margin_r": 80, "margin_v": 150, "bold": False, "italic": False, "outline": 3, "shadow": 1},
    "karaoke": {"active_color": "&H000000FC", "inactive_color": "&H00FFFFFF", "sung_outline_color": "&H00FFFFFF", "tag": "kf"},
    "romanization": {"enabled": True, "languages": "auto", "layout": "ruby", "ruby_spacing": 0.04, "ruby_rise": 0.78, "karaoke": True, "gap": 8, "font": "Arial", "size": 44, "color": "&H00FFFFFF", "outline_color": "&H00000000", "back_color": "&H80000000", "horizontal": "center", "vertical": "bottom", "margin_l": 80, "margin_r": 80, "margin_v": "auto", "bold": False, "italic": False, "outline": 2, "shadow": 1, "tone": "none"},
    "lyrics": {"sources": ["lrclib", "netease", "lyrics.ovh", "webpage"], "auto_captions": True, "min_sources": 3, "agreement_threshold": 0.6, "require_verified": False},
    "alignment": {"model": "auto", "device": "auto", "compute_type": "int8", "reserve_memory_gib": 2, "base_min_budget_gib": 2, "small_min_budget_gib": 4, "medium_min_budget_gib": 8, "large_min_budget_gib": 12, "cpu_threads": 0, "num_workers": 1},
    "timing": {"long_pause_ms": 10000, "dot_interval_ms": 1000, "lead_in_ms": 500, "max_tail_ms": 5000, "tail_hold_ms": 1000},
    "singers": {"max": 16, "show_label": True, "female_color": "&H000000FC", "male_color": "&H00882D14", "both_color": "&H00187E12"},
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


@functools.lru_cache(maxsize=1)
def load_config() -> dict:
    """Return the merged configuration. The result is cached; treat it as read-only."""
    config = copy.deepcopy(DEFAULT_CONFIG)
    if CONFIG_FILE.exists():
        try:
            with CONFIG_FILE.open("rb") as handle:
                _merge(config, tomllib.load(handle))
        except tomllib.TOMLDecodeError as exc:
            raise SystemExit(f"{CONFIG_FILE.name} is not valid TOML: {exc}") from exc
    return config


def enabled_regions() -> list[str]:
    return [str(region).lower() for region in load_config().get("charts", {}).get("enabled_regions", [])]


def run(cmd: list[str], dry_run: bool = False, cwd: Path | None = None) -> None:
    print("$", " ".join(cmd))
    if not dry_run:
        subprocess.run(cmd, check=True, cwd=cwd)


@functools.lru_cache(maxsize=1)
def require_current_ytdlp() -> None:
    try:
        result = subprocess.run(["yt-dlp", "--version"], capture_output=True, text=True, check=False)
    except FileNotFoundError:
        raise SystemExit("yt-dlp is not installed or not on PATH") from None
    version = result.stdout.strip()
    # Nightly builds append a fourth component (2025.01.15.232805).
    match = re.match(r"(\d{4})\.(\d{2})\.(\d{2})(?:\.\d+)?$", version)
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
