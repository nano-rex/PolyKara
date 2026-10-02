#!/usr/bin/env python3
"""PolyKara command-line orchestration entry point."""
from __future__ import annotations

import argparse
import json
import os
import re
import shutil
import subprocess
import sys
from pathlib import Path
from typing import Callable

from polykara.alignment import align_row, is_aligned
from polykara.ass import ass
from polykara.charts import add_selected
from polykara.config import ASS, AUDIO, EXTERNAL_LYRICS, METADATA, OUTPUT, RAW, SUBTITLES, enabled_regions, load_config, run, require_current_ytdlp
from polykara.manifest import check, find_source, resolve, rows, select, source
from polykara.providers import external_lyrics_path, external_webpage_path, fetch_external_lyrics
from polykara.subtitle import apply_word_timing, downloaded_subtitle, lrc, plain_lyrics_entries, timed_subtitle, word_json

KARAOKE_TAG = "{\\k"
# Songs that failed a step in this run, with the reason. They are left out of later steps.
FAILED: dict[str, str] = {}


class SongError(Exception):
    """A problem that stops one song but must not stop the batch."""


def describe(exc: BaseException) -> str:
    if isinstance(exc, subprocess.CalledProcessError):
        tool = exc.cmd[0] if isinstance(exc.cmd, (list, tuple)) and exc.cmd else exc.cmd
        return f"{tool} exited with status {exc.returncode}"
    if isinstance(exc, SystemExit):
        return str(exc.code) if isinstance(exc.code, str) else f"exit status {exc.code}"
    return str(exc) or type(exc).__name__


def each(step: str, song_rows: list[dict[str, str]], action: Callable[[dict[str, str]], None]) -> None:
    """Run one step for every song. A failure skips that song only; the rest of the batch continues."""
    for row in song_rows:
        if row["id"] in FAILED:
            continue
        try:
            action(row)
        except KeyboardInterrupt:
            raise
        except (Exception, SystemExit) as exc:
            FAILED[row["id"]] = f"{step}: {describe(exc)}"
            print(f"SKIP {row['id']}: {step} failed ({describe(exc)}); continuing with the next song")


def auto_path(sid: str) -> Path:
    return ASS / f"{sid}.auto.ass"


def edited_path(sid: str) -> Path:
    return ASS / f"{sid}.edited.ass"


def output_path(sid: str) -> Path:
    return OUTPUT / f"{sid}.mp4"


def newer(path: Path, than: Path) -> bool:
    return path.exists() and than.exists() and path.stat().st_mtime > than.stat().st_mtime


def has_karaoke(path: Path) -> bool:
    return path.exists() and KARAOKE_TAG in path.read_text(encoding="utf-8")


def download_subtitles(row: dict[str, str], dry_run: bool) -> None:
    """Fetch creator subtitles, falling back to automatic captions in the song language."""
    sid, marker = row["id"], SUBTITLES / f"{row['id']}.checked"
    if marker.exists() or downloaded_subtitle(row) or downloaded_subtitle(row, auto=True):
        return
    language = row.get("language", "").lower().replace("_", "-").split("-", 1)[0]
    # "all" would also request every auto-translated track and trips YouTube rate limits.
    langs = row.get("subtitle_langs", "") or (f"{language}.*" if language else "all")
    base = ["yt-dlp", "--no-playlist", "--restrict-filenames", "--skip-download", "--ignore-errors", "--retries", "3", "--fragment-retries", "3", "--sleep-requests", "1", "--sub-format", "vtt", "--no-overwrites", "-P", f"subtitle:{SUBTITLES}"]
    run(base + ["--write-subs", "--sub-langs", langs, "-o", f"subtitle:{sid}.%(ext)s", row["url"]], dry_run)
    if language and load_config()["lyrics"].get("auto_captions", True) and (dry_run or downloaded_subtitle(row) is None):
        run(base + ["--write-auto-subs", "--sub-langs", f"{language}.*", "-o", f"subtitle:{sid}.auto.%(ext)s", row["url"]], dry_run)
    if not dry_run:
        marker.write_text("", encoding="utf-8")


def playable(path: Path) -> bool:
    """True when ffprobe can open the file and reports a duration (a truncated download fails this)."""
    try:
        result = subprocess.run(["ffprobe", "-v", "error", "-show_entries", "format=duration", "-of", "csv=p=0", str(path)], capture_output=True, text=True, check=False, timeout=60)
    except (FileNotFoundError, subprocess.TimeoutExpired):
        # Without a verdict the file is kept rather than downloaded again.
        return True
    try:
        return result.returncode == 0 and float(result.stdout.strip().splitlines()[0]) > 0
    except (ValueError, IndexError):
        return False


def download_song(row: dict[str, str], dry_run: bool, force: bool = False) -> None:
    sid = row["id"]
    media = find_source(sid)
    if media is not None and not playable(media):
        print(f"{sid}: {media.name} is not playable; removing it and downloading again")
        if not dry_run:
            media.unlink()
        media = None
    if media is not None and not force:
        # Nothing is requested from YouTube or the lyric providers for a song that is already here.
        print(f"{sid}: media found and playable; skipping download")
        return
    require_current_ytdlp()
    if media is None:
        run(["yt-dlp", "--no-playlist", "--restrict-filenames", "--format", "bv*+ba/b", "--merge-output-format", "mp4", "--write-info-json", "--retries", "3", "--fragment-retries", "3", "--sleep-requests", "1", "--no-overwrites", "-o", str(RAW / f"{sid}.%(ext)s"), row["url"]], dry_run)
        if not dry_run and find_source(sid) is None:
            raise SongError("yt-dlp finished without producing a media file")
    # Subtitles and external lyrics are optional inputs: report problems and carry on.
    try:
        if force and not dry_run:
            (SUBTITLES / f"{sid}.checked").unlink(missing_ok=True)
        download_subtitles(row, dry_run)
    except (subprocess.CalledProcessError, OSError) as exc:
        print(f"WARN {sid}: subtitle download failed ({describe(exc)}); continuing with other lyric sources")
    if dry_run:
        return
    (METADATA / f"{sid}.json").write_text(json.dumps(row, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    try:
        fetch_external_lyrics(row)
    except Exception as exc:
        print(f"WARN {sid}: external lyrics lookup failed ({describe(exc)})")


def download(dry_run: bool, trending: bool = False, regions: str = "", pick: bool = False, force: bool = False) -> None:
    if trending:
        selected_regions = [item.strip().lower() for item in regions.split(",") if item.strip()] if regions else enabled_regions()
        add_selected(selected_regions, pick)
        return
    song_rows = rows()
    for directory in (RAW, SUBTITLES, METADATA, EXTERNAL_LYRICS):
        directory.mkdir(parents=True, exist_ok=True)
    each("download", song_rows, lambda row: download_song(row, dry_run, force))


def normalize_song(row: dict[str, str], dry_run: bool, force: bool = False) -> None:
    sid = row["id"]
    media, audio = source(sid), AUDIO / f"{sid}.flac"
    if not force and audio.exists() and audio.stat().st_mtime >= media.stat().st_mtime:
        print(f"{sid}: normalized audio is up to date")
        return
    AUDIO.mkdir(parents=True, exist_ok=True)
    # Encode to a temporary name so an interrupted run never leaves a truncated file behind.
    partial = AUDIO / f"{sid}.partial.flac"
    try:
        run(["ffmpeg", "-hide_banner", "-loglevel", "warning", "-y", "-i", str(media), "-vn", "-ac", "2", "-ar", "48000", "-c:a", "flac", str(partial)], dry_run)
        if not dry_run:
            os.replace(partial, audio)
    finally:
        partial.unlink(missing_ok=True)


def prepare_audio(row: dict[str, str], dry_run: bool, realign: bool) -> None:
    """Normalize audio only when alignment still needs it, so cleaned-up songs are not re-encoded."""
    if not realign and is_aligned(row):
        print(f"{row['id']}: word timing already exists; audio not needed")
        return
    normalize_song(row, dry_run)


def normalize(dry_run: bool, force: bool = False) -> None:
    each("normalize", rows(), lambda row: normalize_song(row, dry_run, force))


def lyric_source(row: dict[str, str]) -> tuple[str, str, Path, list] | None:
    """Pick the best available lyrics as (kind, label, path, entries).

    Order: the manifest's own lyrics_file, creator-made subtitles, validated
    external synced lyrics, automatic captions, then plain webpage lyrics.
    Plain lyrics have no entries yet; they are timed against the alignment.
    """
    candidates: list[tuple[str, str, Path]] = []
    raw = row.get("lyrics_file", "")
    if raw:
        path = resolve(raw)
        if not path.exists():
            raise SongError(f"lyrics_file not found: {path}")
        kind = {".lrc": "lrc", ".srt": "subtitle", ".vtt": "subtitle", ".txt": "plain"}.get(path.suffix.lower())
        if kind is None:
            raise SongError(f"lyrics_file must be .lrc, .srt, .vtt, or .txt: {path}")
        candidates.append((kind, "manifest lyrics_file", path))
    manual = downloaded_subtitle(row)
    if manual is not None:
        candidates.append(("subtitle", "downloaded subtitle", manual))
    if external_lyrics_path(row).exists():
        candidates.append(("lrc", "validated external lyrics", external_lyrics_path(row)))
    automatic = downloaded_subtitle(row, auto=True)
    if automatic is not None and load_config()["lyrics"].get("auto_captions", True):
        candidates.append(("subtitle", "automatic captions", automatic))
    if external_webpage_path(row).exists():
        candidates.append(("plain", "extracted webpage lyrics", external_webpage_path(row)))
    for kind, label, path in candidates:
        if kind == "plain":
            if path.read_text(encoding="utf-8-sig").strip():
                return kind, label, path, []
            continue
        entries = lrc(path) if kind == "lrc" else timed_subtitle(path)
        if entries:
            return kind, label, path, entries
        print(f"{row['id']}: no timed lines in {path.name}; trying the next lyric source")
    return None


def needs_alignment(kind: str, entries: list) -> bool:
    return kind == "plain" or any(not words for _, _, _, words in entries)


def align_song(row: dict[str, str], dry_run: bool, force: bool = False) -> None:
    sid = row["id"]
    if not force and is_aligned(row):
        print(f"{sid}: word timing is up to date; skipping alignment")
        return
    # Alignment is the slowest step, so it only runs for songs that have lyrics to time.
    found = lyric_source(row)
    if found is None:
        raise SongError("no lyrics_file, downloaded subtitle, or external lyrics to time")
    if not needs_alignment(found[0], found[3]):
        print(f"{sid}: lyrics already carry word timing; alignment not needed")
        return
    if not dry_run and not (AUDIO / f"{sid}.flac").exists():
        normalize_song(row, False)
    align_row(row, dry_run)


def align_stage(dry_run: bool, force: bool = False) -> None:
    each("align", rows(), lambda row: align_song(row, dry_run, force))


def timestamp(value: int) -> str:
    return f"{value // 60000:02d}:{value // 1000 % 60:02d}"


def lyrics_song(row: dict[str, str], dry_run: bool) -> None:
    sid = row["id"]
    found = lyric_source(row)
    if found is None:
        raise SongError("no lyrics_file, downloaded subtitle, or external lyrics")
    kind, label, path, entries = found
    estimated: list[int] = []
    if needs_alignment(kind, entries):
        timing_path = word_json(row)
        if timing_path is None:
            if dry_run:
                raise SongError("no word-level timing found (a dry run does not align)")
            print(f"{sid}: word timing missing; starting automatic alignment")
            if not (AUDIO / f"{sid}.flac").exists():
                normalize_song(row, False)
            align_row(row, False)
            timing_path = word_json(row)
        if timing_path is None:
            raise SongError("alignment produced no word-level timing")
        timing = load_config()["timing"]
        if kind == "plain":
            entries = plain_lyrics_entries(path, timing_path)
        else:
            entries = apply_word_timing(entries, timing_path, int(timing.get("max_tail_ms", 0)), int(timing.get("tail_hold_ms", 1000)), estimated)
    if not entries:
        raise SongError(f"no timed lines found in {path}")
    text, target = ass(row, entries), auto_path(sid)
    # Leave an unchanged file alone so its modification time keeps meaning "lyrics changed".
    if not dry_run and not (target.exists() and target.read_text(encoding="utf-8") == text):
        ASS.mkdir(parents=True, exist_ok=True)
        target.write_text(text, encoding="utf-8")
    word_lines = sum(1 for _, _, _, words in entries if words)
    print(f"{sid}: {len(entries)} lines from {label}, {word_lines} with word timing -> {target.name}")
    if estimated:
        shown = ", ".join(timestamp(value) for value in estimated[:10]) + (", ..." if len(estimated) > 10 else "")
        print(f"{sid}: {len(estimated)} line(s) had no recognised words and use evenly spaced timing; check them in the editor: {shown}")
    if edited_path(sid).exists() and not dry_run:
        print(f"{sid}: {edited_path(sid).name} exists and is used for rendering; it is never overwritten")


def lyrics(dry_run: bool) -> None:
    each("lyrics", rows(), lambda row: lyrics_song(row, dry_run))


def render_subtitle(row: dict[str, str]) -> Path:
    edited, auto = edited_path(row["id"]), auto_path(row["id"])
    if has_karaoke(edited):
        return edited
    if edited.exists() and has_karaoke(auto):
        print(f"{row['id']}: edited ASS has no word timing; using automatic ASS")
        return auto
    return edited if edited.exists() else auto


def edit(row: dict[str, str]) -> None:
    auto, edited = auto_path(row["id"]), edited_path(row["id"])
    if not auto.exists() and not edited.exists():
        raise SongError(f"missing {auto.name}; run lyrics first")
    if not edited.exists():
        shutil.copy2(auto, edited)
        print(f"Created editable copy: {edited}")
    gui = shutil.which("aegisub") or shutil.which("Aegisub") or shutil.which("subtitleedit")
    if gui:
        subprocess.Popen([gui, str(edited)])
    else:
        print(f"Open this file in Aegisub or Subtitle Edit: {edited}")


def video_size(path: Path) -> tuple[int, int]:
    result = subprocess.run(["ffprobe", "-v", "error", "-select_streams", "v:0", "-show_entries", "stream=width,height", "-of", "csv=p=0:s=x", str(path)], capture_output=True, text=True, check=True)
    match = re.search(r"(\d+)x(\d+)", result.stdout)
    if not match:
        raise SongError(f"cannot read the video size of {path.name}")
    return int(match.group(1)), int(match.group(2))


def logo_for(row: dict[str, str]) -> Path | None:
    raw = row.get("logo_file", "") or str(load_config()["logo"].get("file", "")).strip()
    if not raw:
        return None
    path = resolve(raw)
    if not path.exists():
        raise SongError(f"logo file not found: {path}")
    return path


def logo_filters(media: Path, video: str) -> list[str]:
    """Scale and place the logo in PlayRes units so it matches the subtitle layout at any resolution."""
    config = load_config()
    settings = config["logo"]
    scale = video_size(media)[0] / max(1, int(config["video"]["play_res_x"]))
    width = max(2, round(float(settings.get("width", 220)) * scale))
    margin_x, margin_y = round(float(settings.get("margin_x", 40)) * scale), round(float(settings.get("margin_y", 40)) * scale)
    position = str(settings.get("position", "top-right")).lower()
    x = str(margin_x) if "left" in position else f"W-w-{margin_x}" if "right" in position else "(W-w)/2"
    y = str(margin_y) if "top" in position else f"H-h-{margin_y}" if "bottom" in position else "(H-h)/2"
    opacity = min(1.0, max(0.0, float(settings.get("opacity", 1.0))))
    return [f"[1:v]scale={width}:-1,format=rgba,colorchannelmixer=aa={opacity:g}[logo]", f"{video}[logo]overlay={x}:{y}[branded]"]


def encoder_args() -> list[str]:
    settings = load_config()["render"]
    args = ["-c:v", str(settings.get("video_codec", "libx264"))]
    # Empty values are left out so hardware encoders with other rate-control options can be used.
    for flag, key in (("-preset", "preset"), ("-crf", "crf"), ("-pix_fmt", "pixel_format")):
        if str(settings.get(key, "")).strip():
            args += [flag, str(settings[key])]
    args += ["-c:a", str(settings.get("audio_codec", "aac"))]
    if str(settings.get("audio_bitrate", "")).strip():
        args += ["-b:a", str(settings["audio_bitrate"])]
    return args + [str(item) for item in settings.get("extra_args", [])] + ["-movflags", "+faststart"]


def render_song(row: dict[str, str], dry_run: bool, force: bool) -> None:
    sid = row["id"]
    output = output_path(sid)
    if not dry_run and not has_karaoke(edited_path(sid)) and not has_karaoke(auto_path(sid)):
        print(f"{sid}: no karaoke subtitle yet; generating lyrics first")
        lyrics_song(row, False)
    subtitle = render_subtitle(row)
    if not subtitle.exists():
        raise SongError("missing ASS subtitle; run lyrics first")
    ass_text = subtitle.read_text(encoding="utf-8")
    if KARAOKE_TAG not in ass_text:
        raise SongError("no word-level timing; no output rendered")
    if output.exists() and not force:
        if newer(subtitle, output) and subtitle == edited_path(sid):
            print(f"{sid}: {subtitle.name} changed after the last render; rendering again")
        else:
            answer = input(f"{sid} is already processed. Process again? [y/N] ").strip().lower() if sys.stdin.isatty() else ""
            if answer not in {"y", "yes"}:
                print(f"{sid}: output already exists; skipping (use --reprocess to render again)")
                return
    media, logo = source(sid), logo_for(row)
    match = re.search(r"^; PolyKaraIntroPaddingMs:\s*(\d+)", ass_text, re.MULTILINE)
    padding = int(match.group(1)) if match else 0
    filters, video, audio = [], "[0:v]", "0:a:0?"
    if padding:
        filters.append(f"{video}tpad=start_mode=clone:start_duration={padding / 1000:g}[padded]")
        filters.append(f"[0:a:0]adelay={padding}:all=1[a]")
        video, audio = "[padded]", "[a]"
    if logo is not None:
        filters += logo_filters(media, video)
        video = "[branded]"
    # ffmpeg runs inside the ASS directory: ids are filter-safe, absolute paths are not.
    filters.append(f"{video}ass={subtitle.name}[v]")
    partial = OUTPUT / f"{sid}.partial.mp4"
    cmd = ["ffmpeg", "-hide_banner", "-y", "-i", str(media)] + (["-i", str(logo)] if logo is not None else [])
    cmd += ["-filter_complex", ";".join(filters), "-map", "[v]", "-map", audio] + encoder_args() + [str(partial)]
    OUTPUT.mkdir(parents=True, exist_ok=True)
    try:
        run(cmd, dry_run, cwd=subtitle.parent)
        if not dry_run:
            # Only a finished render gets the final name, so an interrupted one is never mistaken for done.
            os.replace(partial, output)
            print(f"{sid}: rendered {output}")
    finally:
        partial.unlink(missing_ok=True)


def render(dry_run: bool, force: bool) -> None:
    each("render", rows(), lambda row: render_song(row, dry_run, force))


def cleanup(dry_run: bool, drop_source: bool) -> None:
    targets = list(AUDIO.glob("*")) + list(OUTPUT.glob("*.partial.mp4"))
    if drop_source:
        for row in rows():
            if output_path(row["id"]).exists() and edited_path(row["id"]).exists():
                targets.extend(p for p in RAW.glob(f"{row['id']}.*") if p.suffix.lower() != ".json")
    for path in targets:
        print(f"REMOVE {path}")
        if not dry_run and path.exists():
            path.unlink()


def qa() -> None:
    for row in rows():
        sid = row["id"]
        selected = edited_path(sid) if edited_path(sid).exists() else auto_path(sid)
        try:
            found = lyric_source(row)
            lyric_state = found[1] if found else "MISSING"
        except SongError as exc:
            lyric_state = f"ERROR ({exc})"
        if selected.exists():
            lines = [line for line in selected.read_text(encoding="utf-8").splitlines() if line.startswith("Dialogue:") and ",Lyric," in line]
            timed = sum(1 for line in lines if KARAOKE_TAG in line)
            ass_state = f"{'edited' if selected == edited_path(sid) else 'automatic'} ({timed}/{len(lines)} lines with word timing)"
        else:
            ass_state = "MISSING"
        output = output_path(sid)
        mp4_state = "WAITING" if not output.exists() else "STALE (subtitle edited after render)" if newer(edited_path(sid), output) else "OK"
        print(f"{sid}: media={'OK' if find_source(sid) else 'MISSING'}, lyrics={lyric_state}, timing={'OK' if word_json(row) else 'none'}, ass={ass_state}, mp4={mp4_state}")


def process(dry_run: bool, force: bool, realign: bool = False) -> None:
    song_rows = rows()
    if not force:
        pending = []
        for row in song_rows:
            sid = row["id"]
            if output_path(sid).exists() and not newer(edited_path(sid), output_path(sid)):
                print(f"{sid}: final MP4 is up to date; skipping (use --reprocess to rebuild)")
            else:
                pending.append(row)
        song_rows = pending
    print("== check ==")
    # yt-dlp is only required when some song still has to be downloaded.
    check(need_ytdlp=any(find_source(row["id"]) is None for row in song_rows))
    for directory in (RAW, SUBTITLES, METADATA, EXTERNAL_LYRICS):
        directory.mkdir(parents=True, exist_ok=True)
    steps: tuple[tuple[str, Callable[[dict[str, str]], None]], ...] = (
        ("download", lambda row: download_song(row, dry_run)),
        ("normalize", lambda row: prepare_audio(row, dry_run, realign)),
        ("align", lambda row: align_song(row, dry_run, realign)),
        ("lyrics", lambda row: lyrics_song(row, dry_run)),
        ("render", lambda row: render_song(row, dry_run, True)),
    )
    for name, action in steps:
        print(f"== {name} ==")
        each(name, song_rows, action)


def summary() -> int:
    if not FAILED:
        return 0
    print(f"\n{len(FAILED)} song(s) were skipped; every other song ran to the end:")
    for sid, reason in FAILED.items():
        print(f"  {sid}: {reason}")
    print("Fix the cause and rerun the same command; finished work is reused.")
    return 1


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("command", choices=("check", "download", "normalize", "align", "lyrics", "edit", "render", "process", "cleanup", "qa"))
    parser.add_argument("songs", nargs="*", help="optional song ids from songs.csv; default is every song")
    parser.add_argument("--only", default="", help="comma-separated song ids to work on (same as listing them after the command)")
    parser.add_argument("--dry-run", action="store_true")
    parser.add_argument("--reprocess", action="store_true", help="redo the step even when its output already exists (download, normalize, align, render, process)")
    parser.add_argument("--realign", action="store_true", help="with process: also repeat word alignment instead of reusing existing timing")
    parser.add_argument("--drop-source", action="store_true")
    parser.add_argument("--trending", action="store_true", help="show regional YouTube Top Songs and add selected entries to songs.csv")
    parser.add_argument("--pick", action="store_true", help="interactively choose chart songs; trending mode otherwise adds all candidates")
    parser.add_argument("--regions", default="", help="comma-separated chart region codes: my,id,au,ca,sg,tw,hk,jp,kr,in,cn,us,gb")
    args = parser.parse_intermixed_args()
    select(args.songs + args.only.split(","))
    if args.command == "check": check()
    elif args.command == "download": download(args.dry_run, args.trending, args.regions, args.pick, args.reprocess)
    elif args.command == "normalize": normalize(args.dry_run, args.reprocess)
    elif args.command == "align": align_stage(args.dry_run, args.reprocess)
    elif args.command == "lyrics": lyrics(args.dry_run)
    elif args.command == "edit": each("edit", rows(), edit)
    elif args.command == "render": render(args.dry_run, args.reprocess)
    elif args.command == "process": process(args.dry_run, args.reprocess, args.realign)
    elif args.command == "cleanup": cleanup(args.dry_run, args.drop_source)
    else: qa()
    return summary()


if __name__ == "__main__":
    sys.exit(main())
