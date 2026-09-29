#!/usr/bin/env python3
"""PolyKara command-line orchestration entry point."""
from __future__ import annotations

import argparse
import json
import shutil
import subprocess
import sys
from pathlib import Path

from polykara.alignment import align
from polykara.ass import ass
from polykara.charts import REGIONS, add_selected
from polykara.config import ALIGN, ASS, AUDIO, EXTERNAL_LYRICS, METADATA, OUTPUT, RAW, SUBTITLES, enabled_regions, run, require_current_ytdlp
from polykara.manifest import check, rows, source
from polykara.providers import external_lyrics_path, external_webpage_path, fetch_external_lyrics
from polykara.subtitle import apply_word_timing, downloaded_subtitle, lrc, plain_lyrics_entries, timed_subtitle, word_json


def download(dry_run: bool, trending: bool = False, regions: str = "", pick: bool = False) -> None:
    if trending:
        selected_regions = [item.strip().lower() for item in regions.split(",") if item.strip()] if regions else enabled_regions()
        add_selected(selected_regions, pick)
        return
    require_current_ytdlp()
    for directory in (RAW, SUBTITLES, METADATA, EXTERNAL_LYRICS):
        directory.mkdir(parents=True, exist_ok=True)
    for row in rows():
        langs = row.get("subtitle_langs", "all").strip() or "all"
        run(["yt-dlp", "--no-playlist", "--restrict-filenames", "--format", "bv*+ba/b", "--merge-output-format", "mp4", "--download-archive", str(RAW.parent / "download-archive.txt"), "--write-info-json", "--write-subs", "--write-auto-subs", "--sub-langs", langs, "--sub-format", "vtt", "-P", f"subtitle:{SUBTITLES}", "-o", "subtitle:%(id)s.%(language)s.%(ext)s", "--no-overwrites", "-o", str(RAW / f"{row['id']}.%(ext)s"), row["url"].strip()], dry_run)
        if not dry_run:
            (METADATA / f"{row['id']}.json").write_text(json.dumps(row, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
            fetch_external_lyrics(row)


def normalize(dry_run: bool) -> None:
    AUDIO.mkdir(parents=True, exist_ok=True)
    for row in rows():
        run(["ffmpeg", "-hide_banner", "-y", "-i", str(source(row["id"])), "-vn", "-ac", "2", "-ar", "48000", "-c:a", "flac", str(AUDIO / f"{row['id']}.flac")], dry_run)


def lyrics(dry_run: bool) -> None:
    ASS.mkdir(parents=True, exist_ok=True)
    for row in rows():
        path = downloaded_subtitle(row)
        source_label = "downloaded subtitle"
        if path is not None:
            entries = timed_subtitle(path)
        elif external_lyrics_path(row).exists():
            path = external_lyrics_path(row)
            entries, source_label = lrc(path), "validated external lyrics"
        elif external_webpage_path(row).exists() and word_json(row) is not None:
            path = external_webpage_path(row)
            entries, source_label = plain_lyrics_entries(path, word_json(row)), "extracted webpage lyrics"
        else:
            raw = row.get("lyrics_file", "").strip()
            if not raw:
                print(f"SKIP {row['id']}: no downloaded subtitle, external lyrics, or lyrics_file")
                continue
            path = Path(raw) if Path(raw).is_absolute() else Path(__file__).resolve().parent / raw
            if not path.exists():
                raise SystemExit(f"Lyrics file not found: {path}")
            entries = lrc(path) if path.suffix.lower() == ".lrc" else timed_subtitle(path) if path.suffix.lower() in {".srt", ".vtt"} else []
            if not entries:
                raise SystemExit(f"Expected .lrc, .srt, or .vtt: {path}")
            source_label = "manifest lyrics_file"
        if not entries:
            raise SystemExit(f"No timed lines found in {path}")
        timing_path = word_json(row)
        if timing_path is None and not dry_run:
            print(f"{row['id']}: word timing missing; starting automatic alignment")
            try:
                from polykara.alignment import align_row
                align_row(row, False)
            except (RuntimeError, subprocess.CalledProcessError) as exc:
                print(f"SKIP {row['id']}: word timings unavailable ({exc})")
                continue
            timing_path = word_json(row)
        if timing_path is None:
            print(f"SKIP {row['id']}: no word-level timing found")
            continue
        try:
            entries = apply_word_timing(entries, timing_path)
        except (OSError, ValueError, KeyError, SystemExit) as exc:
            print(f"SKIP {row['id']}: invalid word timing data ({exc})")
            continue
        if not dry_run:
            (ASS / f"{row['id']}.auto.ass").write_text(ass(row, entries), encoding="utf-8")
        word_lines = sum(1 for _, _, _, words in entries if words)
        print(f"{row['id']}: {len(entries)} lines from {source_label} + word alignment, {word_lines} with word timing -> {row['id']}.auto.ass")


def edited_path(sid: str) -> Path:
    return ASS / f"{sid}.edited.ass"


def render_subtitle(row: dict[str, str]) -> Path:
    edited, auto = edited_path(row["id"]), ASS / f"{row['id']}.auto.ass"
    if edited.exists() and "{\\k" in edited.read_text(encoding="utf-8"):
        return edited
    if edited.exists() and auto.exists() and "{\\k" in auto.read_text(encoding="utf-8"):
        print(f"{row['id']}: edited ASS has no word timing; using automatic ASS")
        return auto
    return edited if edited.exists() else auto


def edit(row: dict[str, str]) -> None:
    auto, edited = ASS / f"{row['id']}.auto.ass", edited_path(row["id"])
    if not auto.exists():
        raise SystemExit(f"Missing {auto}; run lyrics first")
    if not edited.exists():
        shutil.copy2(auto, edited)
        print(f"Created editable copy: {edited}")
    gui = shutil.which("aegisub") or shutil.which("Aegisub") or shutil.which("subtitleedit")
    if gui:
        subprocess.Popen([gui, str(edited)])
    else:
        print(f"Open this file in Aegisub or Subtitle Edit: {edited}")


def render(dry_run: bool, force: bool) -> None:
    OUTPUT.mkdir(parents=True, exist_ok=True)
    song_rows = rows()
    if not dry_run and any((ASS / f"{row['id']}.auto.ass").exists() and "{\\k" not in (ASS / f"{row['id']}.auto.ass").read_text(encoding="utf-8") for row in song_rows):
        print("Some songs have no word timing; starting automatic alignment where possible")
        lyrics(False)
    for row in song_rows:
        output = OUTPUT / f"{row['id']}.mp4"
        if output.exists() and not force:
            answer = input(f"{row['id']} is already processed. Process again? [y/N] ").strip().lower() if sys.stdin.isatty() else ""
            if answer not in {"y", "yes"}:
                print(f"SKIP {row['id']}: output already exists")
                continue
        subtitle = render_subtitle(row)
        if not subtitle.exists():
            raise SystemExit(f"Missing ASS for {row['id']}; run lyrics first")
        if "{\\k" not in subtitle.read_text(encoding="utf-8"):
            print(f"SKIP {row['id']}: no word-level timing; no output rendered")
            continue
        subtitle_filter = str(subtitle).replace("\\", "/").replace(":", "\\:")
        ass_text = subtitle.read_text(encoding="utf-8")
        padding = 0
        for line in ass_text.splitlines():
            if line.startswith("; PolyKaraIntroPaddingMs:"):
                padding = int(line.rsplit(":", 1)[1].strip())
                break
        if padding:
            filter_graph = f"[0:v]tpad=start_mode=clone:start_duration={padding / 1000:g},ass={subtitle_filter}[v];[0:a]adelay={padding}:all=1[a]"
            run(["ffmpeg", "-hide_banner", "-y", "-i", str(source(row["id"])), "-filter_complex", filter_graph, "-map", "[v]", "-map", "[a]", "-c:v", "libx264", "-preset", "medium", "-crf", "18", "-c:a", "aac", "-b:a", "192k", "-movflags", "+faststart", str(output)], dry_run)
        else:
            run(["ffmpeg", "-hide_banner", "-y", "-i", str(source(row["id"])), "-vf", f"ass={subtitle_filter}", "-c:v", "libx264", "-preset", "medium", "-crf", "18", "-c:a", "aac", "-b:a", "192k", "-movflags", "+faststart", str(output)], dry_run)


def cleanup(dry_run: bool, drop_source: bool) -> None:
    targets = list(AUDIO.glob("*"))
    if drop_source:
        for row in rows():
            if (OUTPUT / f"{row['id']}.mp4").exists() and edited_path(row["id"]).exists():
                targets.extend(p for p in RAW.glob(f"{row['id']}.*") if p.suffix.lower() != ".json")
    for path in targets:
        print(f"REMOVE {path}")
        if not dry_run and path.exists():
            path.unlink()


def qa() -> None:
    for row in rows():
        selected = edited_path(row["id"]) if edited_path(row["id"]).exists() else ASS / f"{row['id']}.auto.ass"
        output = OUTPUT / f"{row['id']}.mp4"
        print(f"{row['id']}: source={'edited' if selected.name.endswith('.edited.ass') else 'automatic'}, ass={'OK' if selected.exists() else 'MISSING'}, mp4={'OK' if output.exists() else 'WAITING'}")


def process(dry_run: bool, force: bool) -> None:
    for name, action in (("check", check), ("download", lambda: download(dry_run)), ("normalize", lambda: normalize(dry_run)), ("align", lambda: align(rows(), dry_run)), ("lyrics", lambda: lyrics(dry_run)), ("render", lambda: render(dry_run, force))):
        print(f"== {name} ==")
        action()


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("command", choices=("check", "download", "normalize", "align", "lyrics", "edit", "render", "process", "cleanup", "qa"))
    parser.add_argument("--dry-run", action="store_true")
    parser.add_argument("--reprocess", action="store_true", help="render songs again even when the final MP4 already exists")
    parser.add_argument("--drop-source", action="store_true")
    parser.add_argument("--trending", action="store_true", help="show regional YouTube Top Songs and add selected entries to songs.csv")
    parser.add_argument("--pick", action="store_true", help="interactively choose chart songs; trending mode otherwise adds all candidates")
    parser.add_argument("--regions", default="", help="comma-separated chart region codes: my,id,au,ca,sg,tw,hk,jp,kr,in,cn,us,gb")
    args = parser.parse_args()
    if args.command == "check": check()
    elif args.command == "download": download(args.dry_run, args.trending, args.regions, args.pick)
    elif args.command == "normalize": normalize(args.dry_run)
    elif args.command == "align": align(rows(), args.dry_run)
    elif args.command == "lyrics": lyrics(args.dry_run)
    elif args.command == "edit":
        for row in rows(): edit(row)
    elif args.command == "render": render(args.dry_run, args.reprocess)
    elif args.command == "process": process(args.dry_run, args.reprocess)
    elif args.command == "cleanup": cleanup(args.dry_run, args.drop_source)
    else: qa()
    return 0


if __name__ == "__main__":
    sys.exit(main())
