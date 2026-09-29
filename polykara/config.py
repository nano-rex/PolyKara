from __future__ import annotations

import os
import re
import subprocess
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
MANIFEST = ROOT / "songs.csv"
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
