"""Start-up for the installed application, which carries its own tools.

The Windows build bundles ffmpeg, ffprobe and yt-dlp in a bin folder next to
the program, and a copy of the default songs.csv and polykara.toml. ``prepare`` puts that bin folder first on PATH and fills the user's
PolyKara folder with the defaults the first time it is used.
"""
from __future__ import annotations

import os
import shutil
import sys

from .config import HOME, ROOT

BIN = ROOT / "bin"
# Demo songs that earlier versions put in a new songs.csv; removed from it once.
OLD_DEMO_SONGS = {"amazing-grace-vocal": "https://www.youtube.com/watch?v=24cWZeB4ldE", "silent-night-vocal": "https://www.youtube.com/watch?v=25NY8yTolMY"}
DEFAULTS = ROOT / "defaults"


def prepare() -> None:
    for stream in (sys.stdin, sys.stdout, sys.stderr):
        # Song titles and lyrics are printed (and, by the engine, read) as they are; a Windows pipe defaults to a code page.
        if stream is not None and hasattr(stream, "reconfigure"):
            stream.reconfigure(encoding="utf-8", errors="replace")
    if BIN.is_dir() and str(BIN) not in os.environ.get("PATH", "").split(os.pathsep):
        os.environ["PATH"] = str(BIN) + os.pathsep + os.environ.get("PATH", "")
    drop_demo_songs()
    if not DEFAULTS.is_dir():
        return
    HOME.mkdir(parents=True, exist_ok=True)
    for item in DEFAULTS.iterdir():
        target = HOME / item.name
        if target.exists():
            continue
        if item.is_dir():
            shutil.copytree(item, target)
        else:
            shutil.copy2(item, target)


def drop_demo_songs() -> None:
    """Remove the two demo songs of earlier versions from songs.csv, when they are unchanged."""
    manifest = HOME / "songs.csv"
    if not manifest.is_file():
        return
    try:
        lines = manifest.read_text(encoding="utf-8-sig").splitlines(keepends=True)
    except (OSError, UnicodeDecodeError):
        return
    kept = [line for line in lines if not any(line.startswith(f"{sid},{url},") for sid, url in OLD_DEMO_SONGS.items())]
    if len(kept) != len(lines):
        partial = manifest.with_name(manifest.name + ".tmp")
        partial.write_text("".join(kept), encoding="utf-8")
        os.replace(partial, manifest)
