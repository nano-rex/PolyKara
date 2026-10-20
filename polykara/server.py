"""Local web server for the karaoke studio (timing editor) and the music player.

Binds to localhost only. Media is served only for known songs.csv ids and scanned
library tracks, never for arbitrary paths.
"""
from __future__ import annotations

import json
import mimetypes
import os
import re
import subprocess
import sys
import threading
import time
from http import HTTPStatus
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from urllib.parse import parse_qs, quote, urlparse

from .ass import ass
from .config import OUTPUT, ROOT, WORK, load_config
from .library import scan, store_lyrics, track_lyrics
from .manifest import rows
from .project import EXPORTERS, from_json, load_entries, project_path, save_entries, split_units, to_json

WEB = Path(__file__).resolve().parent / "web"
PEAKS = WORK / "peaks"
PEAK_RATE = 100
PAGES = {"/": "studio.html", "/studio": "studio.html", "/player": "player.html", "/lyrics": "desklyric.html"}
JOBS: dict[str, dict] = {}


def song_rows() -> dict[str, dict]:
    return {row["id"]: row for row in rows()}


def song_media(sid: str) -> Path | None:
    candidates = [path for path in sorted((WORK / "raw").glob(f"{sid}.*")) if path.suffix.lower() not in {".json", ".part", ".ytdl"}]
    return candidates[0] if candidates else None


def peaks(media: Path) -> list[int]:
    """Peak envelope at PEAK_RATE values per second, 0-255, cached by file size and mtime."""
    stat = media.stat()
    cache = PEAKS / f"{media.stem}-{stat.st_size}-{int(stat.st_mtime)}.json"
    if cache.exists():
        return json.loads(cache.read_text(encoding="utf-8"))
    sample_rate = 8000
    raw = subprocess.run(["ffmpeg", "-v", "quiet", "-i", str(media), "-vn", "-ac", "1", "-ar", str(sample_rate), "-f", "s16le", "-"], capture_output=True, check=False).stdout
    step = sample_rate // PEAK_RATE
    samples = memoryview(raw).cast("h") if len(raw) % 2 == 0 else memoryview(raw[:-1]).cast("h")
    values = [min(255, max((abs(value) for value in samples[index:index + step]), default=0) >> 7) for index in range(0, len(samples), step)]
    PEAKS.mkdir(parents=True, exist_ok=True)
    cache.write_text(json.dumps(values), encoding="utf-8")
    return values


def song_summary(row: dict) -> dict:
    sid = row["id"]
    return {
        "id": sid, "title": row.get("title", ""), "artist": row.get("artist", ""), "language": row.get("language", ""),
        "media": song_media(sid) is not None, "project": project_path(sid).exists(),
        "ass": (WORK / "ass" / f"{sid}.edited.ass").exists() or (WORK / "ass" / f"{sid}.auto.ass").exists(),
        "mp4": (OUTPUT / f"{sid}.mp4").exists(),
    }


def start_render(sid: str) -> dict:
    job = JOBS.get(sid)
    if job and job["state"] == "running":
        return job
    job = {"state": "running", "log": [], "started": time.time()}
    JOBS[sid] = job

    def work() -> None:
        env = {**os.environ, "PYTHONUNBUFFERED": "1"}  # keep the command echo in order with FFmpeg output
        process = subprocess.Popen([sys.executable, str(ROOT / "pipeline.py"), "render", "--song", sid, "--reprocess"], cwd=ROOT, env=env, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True, errors="replace")
        for line in process.stdout:
            job["log"] = (job["log"] + [line.rstrip()])[-200:]
        job["state"] = "done" if process.wait() == 0 else "failed"

    threading.Thread(target=work, daemon=True).start()
    return job


class Handler(BaseHTTPRequestHandler):
    server_version = "PolyKara"
    library: dict[str, dict] = {}

    def log_message(self, format: str, *args) -> None:
        if getattr(self.server, "verbose", False):
            super().log_message(format, *args)

    # -- helpers ---------------------------------------------------------
    def send_json(self, value, status: int = 200) -> None:
        body = json.dumps(value, ensure_ascii=False).encode("utf-8")
        self.send_response(status)
        self.send_header("Content-Type", "application/json; charset=utf-8")
        self.send_header("Content-Length", str(len(body)))
        self.send_header("Cache-Control", "no-store")
        self.end_headers()
        self.wfile.write(body)

    def send_text(self, text: str, filename: str, content_type: str = "text/plain") -> None:
        body = text.encode("utf-8")
        self.send_response(200)
        self.send_header("Content-Type", f"{content_type}; charset=utf-8")
        self.send_header("Content-Length", str(len(body)))
        self.send_header("Content-Disposition", f"attachment; filename*=UTF-8''{quote(filename)}")
        self.end_headers()
        self.wfile.write(body)

    def fail(self, status: int, message: str) -> None:
        self.send_json({"error": message}, status)

    def body(self) -> bytes:
        length = int(self.headers.get("Content-Length") or 0)
        if length > 64 * 1024 * 1024:
            raise ValueError("request too large")
        return self.rfile.read(length)

    def send_file(self, path: Path, content_type: str | None = None) -> None:
        size = path.stat().st_size
        start, end = 0, size - 1
        match = re.fullmatch(r"bytes=(\d*)-(\d*)", self.headers.get("Range", "").strip())
        if match and (match.group(1) or match.group(2)):
            if match.group(1):
                start = int(match.group(1))
                end = min(int(match.group(2)), size - 1) if match.group(2) else size - 1
            else:
                start = max(0, size - int(match.group(2)))
            if start > end:
                self.send_response(HTTPStatus.REQUESTED_RANGE_NOT_SATISFIABLE)
                self.send_header("Content-Range", f"bytes */{size}")
                self.end_headers()
                return
            self.send_response(HTTPStatus.PARTIAL_CONTENT)
            self.send_header("Content-Range", f"bytes {start}-{end}/{size}")
        else:
            self.send_response(200)
        self.send_header("Content-Type", content_type or mimetypes.guess_type(path.name)[0] or "application/octet-stream")
        self.send_header("Accept-Ranges", "bytes")
        self.send_header("Content-Length", str(end - start + 1))
        self.end_headers()
        if self.command == "HEAD":
            return
        with path.open("rb") as handle:
            handle.seek(start)
            remaining = end - start + 1
            try:
                while remaining > 0:
                    chunk = handle.read(min(1 << 16, remaining))
                    if not chunk:
                        break
                    self.wfile.write(chunk)
                    remaining -= len(chunk)
            except (BrokenPipeError, ConnectionResetError):
                pass

    def track(self, tid: str) -> dict | None:
        if not Handler.library:
            Handler.library = {item["id"]: item for item in scan(ROOT)}
        return Handler.library.get(tid)

    # -- routing ---------------------------------------------------------
    def handle_one_request(self) -> None:
        try:
            super().handle_one_request()
        except (BrokenPipeError, ConnectionResetError):
            self.close_connection = True
        except Exception as exc:  # report instead of dropping the connection
            print(f"server error: {exc!r}")
            try:
                self.fail(500, f"{type(exc).__name__}: {exc}")
            except OSError:
                self.close_connection = True

    def do_HEAD(self) -> None:
        self.do_GET()

    def do_GET(self) -> None:
        url = urlparse(self.path)
        path, query = url.path, parse_qs(url.query)
        try:
            if path in PAGES:
                return self.send_file(WEB / PAGES[path], "text/html; charset=utf-8")
            if path.startswith("/static/"):
                name = path[len("/static/"):]
                target = (WEB / name).resolve()
                if WEB not in target.parents or not target.is_file():
                    return self.fail(404, "not found")
                kind = "text/javascript" if target.suffix == ".js" else None
                return self.send_file(target, kind)
            if path == "/api/config":
                return self.send_json(load_config())
            if path == "/api/songs":
                return self.send_json([song_summary(row) for row in rows()])
            match = re.fullmatch(r"/api/song/([A-Za-z0-9_-]+)(?:/(peaks|export/(\w+)))?", path)
            if match:
                row = song_rows().get(match.group(1))
                if row is None:
                    return self.fail(404, "unknown song")
                media = song_media(row["id"])
                if match.group(2) == "peaks":
                    return self.send_json({"rate": PEAK_RATE, "peaks": peaks(media) if media else []})
                entries, source = load_entries(row, ROOT)
                if match.group(3):
                    fmt = match.group(3).lower()
                    if fmt == "ass":
                        return self.send_text(ass(row, entries), f"{row['id']}.ass")
                    if fmt not in EXPORTERS:
                        return self.fail(400, f"unknown export format {fmt}")
                    return self.send_text(EXPORTERS[fmt](entries), f"{row['id']}.{fmt}")
                video = bool(media and media.suffix.lower() in {".mp4", ".mkv", ".webm", ".mov"})
                return self.send_json({"row": row, "source": source, **to_json(entries), "media": f"/media/song/{row['id']}" if media else "", "video": video, "job": JOBS.get(row["id"])})
            match = re.fullmatch(r"/media/song/([A-Za-z0-9_-]+)", path)
            if match:
                media = song_media(match.group(1)) if match.group(1) in song_rows() else None
                return self.send_file(media) if media else self.fail(404, "no media")
            if path == "/api/library":
                if query.get("refresh"):
                    Handler.library = {}
                    tracks = scan(ROOT, refresh=True)
                    Handler.library = {item["id"]: item for item in tracks}
                else:
                    self.track("")
                return self.send_json([{key: value for key, value in item.items() if key not in {"size", "mtime"}} for item in Handler.library.values()])
            match = re.fullmatch(r"/(media|api)/track/([0-9a-f]{16})(/lyrics)?", path)
            if match:
                item = self.track(match.group(2))
                if item is None:
                    return self.fail(404, "unknown track")
                if match.group(1) == "media":
                    return self.send_file(Path(item["path"]))
                return self.send_json(track_lyrics(item, online=query.get("online", ["1"])[0] != "0"))
            match = re.fullmatch(r"/api/jobs/([A-Za-z0-9_-]+)", path)
            if match:
                return self.send_json(JOBS.get(match.group(1)) or {"state": "idle", "log": []})
            if path == "/api/units":
                return self.send_json(split_units(query.get("text", [""])[0]))
            return self.fail(404, "not found")
        except (BrokenPipeError, ConnectionResetError):
            return None
        except SystemExit as exc:
            return self.fail(400, str(exc))

    def do_PUT(self) -> None:
        match = re.fullmatch(r"/api/song/([A-Za-z0-9_-]+)", urlparse(self.path).path)
        if not match:
            return self.fail(404, "not found")
        row = song_rows().get(match.group(1))
        if row is None:
            return self.fail(404, "unknown song")
        try:
            entries = from_json(json.loads(self.body().decode("utf-8")))
            edited = save_entries(row, entries)
        except (ValueError, TypeError, KeyError) as exc:
            return self.fail(400, f"invalid project: {exc}")
        saved = edited.relative_to(ROOT) if edited.is_relative_to(ROOT) else edited
        return self.send_json({"saved": str(saved), "lines": len(entries)})

    def do_POST(self) -> None:
        path = urlparse(self.path).path
        match = re.fullmatch(r"/api/song/([A-Za-z0-9_-]+)/render", path)
        if match:
            if match.group(1) not in song_rows():
                return self.fail(404, "unknown song")
            return self.send_json(start_render(match.group(1)))
        match = re.fullmatch(r"/api/track/([0-9a-f]{16})/lyrics", path)
        if match:
            if self.track(match.group(1)) is None:
                return self.fail(404, "unknown track")
            suffix = "." + (self.headers.get("X-Lyric-Format") or "lrc").lstrip(".")
            store_lyrics(match.group(1), suffix, self.body())
            Handler.library[match.group(1)]["lyrics"] = True
            return self.send_json(track_lyrics(Handler.library[match.group(1)], online=False))
        return self.fail(404, "not found")


def serve(host: str = "127.0.0.1", port: int = 8765, page: str = "/", open_browser: bool = True) -> None:
    server = ThreadingHTTPServer((host, port), Handler)
    url = f"http://{host}:{server.server_address[1]}{page}"
    print(f"PolyKara {'player' if page == '/player' else 'studio'}: {url}  (Ctrl+C to stop)")
    if open_browser:
        import webbrowser
        threading.Timer(0.5, webbrowser.open, args=(url,)).start()
    try:
        server.serve_forever()
    except KeyboardInterrupt:
        pass
    finally:
        server.server_close()
