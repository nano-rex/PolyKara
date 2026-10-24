"""Entry point of the Windows build.

The same program is installed twice: polykara-cli.exe (a console program)
runs the command-line pipeline and the engine behind the desktop app, and
PolyKara-classic.exe opens the old Tk window. The desktop app, PolyKara.exe,
is a separate Rust program (src/) that starts polykara-cli.exe for each
run, so ffmpeg and yt-dlp never open console windows of their own.
"""
import sys
from pathlib import Path

if __name__ == "__main__":
    if Path(sys.executable).stem.casefold() == "polykara-cli":
        import pipeline

        sys.exit(pipeline.main())
    from polykara.gui import main

    sys.exit(main())
