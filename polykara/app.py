"""Start-up for the installed application, which carries its own tools.

The Windows build bundles ffmpeg, ffprobe and yt-dlp in a bin folder next to
the program, and a copy of the default songs.csv, polykara.toml and example
lyrics. ``prepare`` puts that bin folder first on PATH and fills the user's
PolyKara folder with the defaults the first time it is used.
"""
from __future__ import annotations

import os
import shutil
import sys

from .config import HOME, ROOT

BIN = ROOT / "bin"
DEFAULTS = ROOT / "defaults"


def prepare() -> None:
    for stream in (sys.stdout, sys.stderr):
        # Song titles and lyrics are printed as they are; a Windows pipe defaults to a code page.
        if stream is not None and hasattr(stream, "reconfigure"):
            stream.reconfigure(encoding="utf-8", errors="replace")
    if BIN.is_dir() and str(BIN) not in os.environ.get("PATH", "").split(os.pathsep):
        os.environ["PATH"] = str(BIN) + os.pathsep + os.environ.get("PATH", "")
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
