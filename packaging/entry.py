"""Entry point of the Windows build.

The same program is installed twice: PolyKara.exe opens the window, and
polykara-cli.exe (a console program) runs the command-line pipeline. The
window starts the console program for each run, so ffmpeg and yt-dlp never
open console windows of their own.
"""
import sys
from pathlib import Path

if __name__ == "__main__":
    if Path(sys.executable).stem.casefold() == "polykara-cli":
        import pipeline

        sys.exit(pipeline.main())
    from polykara.gui import main

    sys.exit(main())
