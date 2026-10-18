"""PolyKara desktop window: edit the song list and run the pipeline without a terminal.

The window runs the same command-line pipeline in a child process and shows
its output, so everything the CLI does (skipping finished work, per-song
failure isolation, the end-of-run summary) works the same way here.
"""
from __future__ import annotations

import csv
import os
import queue
import re
import shutil
import subprocess
import sys
import threading
import tkinter as tk
import tomllib
from pathlib import Path
from tkinter import filedialog, messagebox, ttk
from tkinter.scrolledtext import ScrolledText

from .app import prepare
from .config import ASS, CONFIG_FILE, HOME, MANIFEST, OUTPUT, ROOT
from .manifest import REQUIRED_COLUMNS, find_source

COLUMNS = ["id", "url", "title", "artist", "lyricist", "composer", "producer", "template", "language", "subtitle_langs", "lyrics_file", "word_timing_file", "logo_file", "prompt_ms", "align_model", "lyric_sources", "lyric_pages", "voice"]
DEFAULTS = {"template": "solo", "prompt_ms": "1000", "align_model": "auto"}
# (label, field, kind): kind is "text", a list of choices, or "file".
FIELDS = [
    ("YouTube or video URL", "url", "text"),
    ("Title", "title", "text"),
    ("Singer", "artist", "text"),
    ("Composer", "composer", "text"),
    ("Lyricist", "lyricist", "text"),
    ("Subtitles by", "producer", "text"),
    ("Language", "language", ["en", "zh", "yue", "ja", "ko", "ms", "id", "th", "vi", "tl", "hi", "ta", "es", "fr", "de"]),
    ("Voice (colour)", "voice", ["", "female", "male", "duet"]),
    ("Lyrics file (optional)", "lyrics_file", "file"),
    ("Lyrics web pages (optional, | between)", "lyric_pages", "text"),
    ("Logo image (optional)", "logo_file", "file"),
    ("Song id", "id", "text"),
]
# (label, command, takes song ids)
ACTIONS = [
    ("Make videos (all steps)", "process", True),
    ("Download only", "download", True),
    ("Make subtitles only", "lyrics", True),
    ("Render videos only", "render", True),
    ("Check lyrics accuracy", "verify", True),
    ("Status report", "qa", True),
    ("Check tools", "check", False),
    ("Self-test", "selftest", False),
]
NO_WINDOW = getattr(subprocess, "CREATE_NO_WINDOW", 0)


def cli_command() -> list[str]:
    if getattr(sys, "frozen", False):
        return [str(Path(sys.executable).with_name("polykara-cli.exe" if os.name == "nt" else "polykara-cli"))]
    return [sys.executable, str(ROOT / "pipeline.py")]


def open_path(path: Path) -> None:
    if os.name == "nt":
        os.startfile(str(path))  # type: ignore[attr-defined]
    elif sys.platform == "darwin":
        subprocess.Popen(["open", str(path)])
    else:
        subprocess.Popen(["xdg-open", str(path)])


def read_songs() -> tuple[list[str], list[dict[str, str]]]:
    if not MANIFEST.exists():
        return list(COLUMNS), []
    with MANIFEST.open(newline="", encoding="utf-8-sig") as handle:
        reader = csv.DictReader(handle)
        fields = list(reader.fieldnames or [])
        songs = [{key: (value or "") for key, value in row.items() if isinstance(key, str)} for row in reader]
    fields += [name for name in COLUMNS if name not in fields]
    return fields, songs


def write_songs(fields: list[str], songs: list[dict[str, str]]) -> None:
    MANIFEST.parent.mkdir(parents=True, exist_ok=True)
    partial = MANIFEST.with_name(MANIFEST.name + ".tmp")
    with partial.open("w", newline="", encoding="utf-8") as handle:
        writer = csv.DictWriter(handle, fieldnames=fields, extrasaction="ignore")
        writer.writeheader()
        for song in songs:
            writer.writerow({name: song.get(name, "") for name in fields})
    os.replace(partial, MANIFEST)


def song_state(sid: str) -> str:
    if (OUTPUT / f"{sid}.mp4").exists():
        return "video ready"
    if (ASS / f"{sid}.edited.ass").exists() or (ASS / f"{sid}.ass").exists():
        return "subtitles ready"
    if find_source(sid):
        return "downloaded"
    return "waiting"


def new_id(title: str, taken: set[str]) -> str:
    base = re.sub(r"[^a-z0-9]+", "-", title.casefold()).strip("-")[:40] or "song"
    candidate, number = base, 2
    while candidate in taken:
        candidate, number = f"{base}-{number}", number + 1
    return candidate


class SongDialog(tk.Toplevel):
    def __init__(self, parent: "App", song: dict[str, str] | None, taken: set[str]):
        super().__init__(parent)
        self.title("Edit song" if song else "Add song")
        self.transient(parent)
        self.resizable(True, False)
        self.result: dict[str, str] | None = None
        self.original = dict(song or {})
        self.taken = taken - {self.original.get("id", "")}
        self.values: dict[str, tk.StringVar] = {}
        body = ttk.Frame(self, padding=12)
        body.pack(fill="both", expand=True)
        body.columnconfigure(1, weight=1)
        for number, (label, field, kind) in enumerate(FIELDS):
            ttk.Label(body, text=label).grid(row=number, column=0, sticky="w", padx=(0, 8), pady=3)
            variable = tk.StringVar(value=self.original.get(field, ""))
            self.values[field] = variable
            if isinstance(kind, list):
                widget = ttk.Combobox(body, textvariable=variable, values=kind, width=48)
            else:
                widget = ttk.Entry(body, textvariable=variable, width=50)
            widget.grid(row=number, column=1, sticky="ew", pady=3)
            if kind == "file":
                ttk.Button(body, text="Browse…", command=lambda v=variable: self.browse(v)).grid(row=number, column=2, padx=(6, 0))
        ttk.Label(body, text="Leave the song id empty to make one from the title.", foreground="#666").grid(row=len(FIELDS), column=1, sticky="w")
        buttons = ttk.Frame(body)
        buttons.grid(row=len(FIELDS) + 1, column=0, columnspan=3, sticky="e", pady=(12, 0))
        ttk.Button(buttons, text="Cancel", command=self.destroy).pack(side="right")
        ttk.Button(buttons, text="Save", command=self.save).pack(side="right", padx=6)
        self.bind("<Return>", lambda _: self.save())
        self.bind("<Escape>", lambda _: self.destroy())
        self.grab_set()

    def browse(self, variable: tk.StringVar) -> None:
        chosen = filedialog.askopenfilename(parent=self, initialdir=str(HOME))
        if chosen:
            path = Path(chosen)
            try:
                chosen = str(path.relative_to(HOME))
            except ValueError:
                pass
            variable.set(chosen)

    def save(self) -> None:
        song = {**DEFAULTS, **self.original, **{field: variable.get().strip() for field, variable in self.values.items()}}
        if not song["url"]:
            messagebox.showerror("PolyKara", "Enter the video URL.", parent=self)
            return
        if not song["language"]:
            messagebox.showerror("PolyKara", "Choose the language the song is sung in.", parent=self)
            return
        if not song["id"]:
            song["id"] = new_id(song.get("title", ""), self.taken)
        if not re.fullmatch(r"[A-Za-z0-9_-]+", song["id"]):
            messagebox.showerror("PolyKara", "The song id may only use letters, digits, - and _.", parent=self)
            return
        if song["id"] in self.taken:
            messagebox.showerror("PolyKara", f"Another song already uses the id {song['id']}.", parent=self)
            return
        self.result = song
        self.destroy()


class SettingsDialog(tk.Toplevel):
    def __init__(self, parent: tk.Tk):
        super().__init__(parent)
        self.title(f"Settings — {CONFIG_FILE}")
        self.geometry("820x640")
        self.text = ScrolledText(self, wrap="none", undo=True, font=("Consolas", 10) if os.name == "nt" else ("Monospace", 10))
        self.text.pack(fill="both", expand=True)
        self.text.insert("1.0", CONFIG_FILE.read_text(encoding="utf-8") if CONFIG_FILE.exists() else "")
        buttons = ttk.Frame(self, padding=8)
        buttons.pack(fill="x")
        ttk.Label(buttons, text="Fonts, colours, layout, card and render settings. Changes apply to the next run.").pack(side="left")
        ttk.Button(buttons, text="Close", command=self.destroy).pack(side="right")
        ttk.Button(buttons, text="Save", command=self.save).pack(side="right", padx=6)

    def save(self) -> None:
        content = self.text.get("1.0", "end-1c")
        try:
            tomllib.loads(content)
        except tomllib.TOMLDecodeError as exc:
            messagebox.showerror("PolyKara", f"The settings are not valid and were not saved:\n{exc}", parent=self)
            return
        CONFIG_FILE.write_text(content, encoding="utf-8")
        messagebox.showinfo("PolyKara", "Settings saved.", parent=self)


class App(tk.Tk):
    def __init__(self):
        super().__init__()
        self.title(f"PolyKara — {HOME}")
        self.geometry("1100x720")
        self.minsize(800, 500)
        self.process: subprocess.Popen | None = None
        self.lines: queue.Queue[str | None] = queue.Queue()
        self.fields, self.songs = read_songs()

        toolbar = ttk.Frame(self, padding=(8, 8, 8, 0))
        toolbar.pack(fill="x")
        for text, command in (("Add song", self.add_song), ("Edit", self.edit_song), ("Remove", self.remove_song)):
            ttk.Button(toolbar, text=text, command=command).pack(side="left", padx=(0, 4))
        ttk.Separator(toolbar, orient="vertical").pack(side="left", fill="y", padx=8)
        for text, command in (("Edit subtitles", self.edit_subtitles), ("Play video", self.play_video), ("Open videos folder", lambda: self.open_folder(OUTPUT)), ("Open data folder", lambda: self.open_folder(HOME))):
            ttk.Button(toolbar, text=text, command=command).pack(side="left", padx=(0, 4))
        ttk.Button(toolbar, text="Settings", command=lambda: SettingsDialog(self)).pack(side="right")
        ttk.Button(toolbar, text="Update yt-dlp", command=self.update_ytdlp).pack(side="right", padx=4)

        panes = ttk.PanedWindow(self, orient="vertical")
        panes.pack(fill="both", expand=True, padx=8, pady=8)
        table = ttk.Frame(panes)
        self.tree = ttk.Treeview(table, columns=("title", "artist", "language", "voice", "state"), show="tree headings", selectmode="extended")
        self.tree.heading("#0", text="Song id")
        for column, text, width in (("title", "Title", 260), ("artist", "Singer", 200), ("language", "Language", 80), ("voice", "Voice", 80), ("state", "Status", 120)):
            self.tree.heading(column, text=text)
            self.tree.column(column, width=width, stretch=column in {"title", "artist"})
        self.tree.column("#0", width=200)
        scroll = ttk.Scrollbar(table, orient="vertical", command=self.tree.yview)
        self.tree.configure(yscrollcommand=scroll.set)
        self.tree.pack(side="left", fill="both", expand=True)
        scroll.pack(side="right", fill="y")
        self.tree.bind("<Double-1>", lambda _: self.edit_song())
        panes.add(table, weight=3)

        lower = ttk.Frame(panes)
        controls = ttk.Frame(lower)
        controls.pack(fill="x", pady=(0, 6))
        self.action = tk.StringVar(value=ACTIONS[0][0])
        ttk.Combobox(controls, textvariable=self.action, values=[label for label, _, _ in ACTIONS], state="readonly", width=26).pack(side="left")
        self.selected_only = tk.BooleanVar(value=False)
        self.reprocess = tk.BooleanVar(value=False)
        self.realign = tk.BooleanVar(value=False)
        ttk.Checkbutton(controls, text="Selected songs only", variable=self.selected_only).pack(side="left", padx=8)
        ttk.Checkbutton(controls, text="Redo finished work", variable=self.reprocess).pack(side="left")
        ttk.Checkbutton(controls, text="Redo word timing", variable=self.realign).pack(side="left", padx=8)
        self.stop_button = ttk.Button(controls, text="Stop", command=self.stop, state="disabled")
        self.stop_button.pack(side="right")
        self.run_button = ttk.Button(controls, text="Run", command=self.run_action)
        self.run_button.pack(side="right", padx=6)
        self.log = ScrolledText(lower, height=12, state="disabled", wrap="word", font=("Consolas", 9) if os.name == "nt" else ("Monospace", 9))
        self.log.pack(fill="both", expand=True)
        panes.add(lower, weight=2)

        self.status = tk.StringVar(value=f"Songs: {MANIFEST}")
        ttk.Label(self, textvariable=self.status, anchor="w", padding=(8, 0, 8, 6)).pack(fill="x")
        self.refresh()
        self.protocol("WM_DELETE_WINDOW", self.close)
        self.after(100, self.drain)

    # Song list
    def visible_songs(self) -> list[dict[str, str]]:
        return [song for song in self.songs if song.get("id") and song.get("id") != "example"]

    def refresh(self) -> None:
        chosen = set(self.tree.selection())
        self.tree.delete(*self.tree.get_children())
        for song in self.visible_songs():
            sid = song["id"]
            self.tree.insert("", "end", iid=sid, text=sid, values=(song.get("title", ""), song.get("artist", ""), song.get("language", ""), song.get("voice", ""), song_state(sid)))
        self.tree.selection_set([sid for sid in chosen if self.tree.exists(sid)])

    def selected_ids(self) -> list[str]:
        return list(self.tree.selection())

    def add_song(self) -> None:
        dialog = SongDialog(self, None, {song.get("id", "") for song in self.songs})
        self.wait_window(dialog)
        if dialog.result:
            self.songs.append(dialog.result)
            self.save_songs()

    def edit_song(self) -> None:
        ids = self.selected_ids()
        if len(ids) != 1:
            messagebox.showinfo("PolyKara", "Select one song to edit.")
            return
        index = next(number for number, song in enumerate(self.songs) if song.get("id") == ids[0])
        dialog = SongDialog(self, self.songs[index], {song.get("id", "") for song in self.songs})
        self.wait_window(dialog)
        if dialog.result:
            self.songs[index] = dialog.result
            self.save_songs()

    def remove_song(self) -> None:
        ids = set(self.selected_ids())
        if ids and messagebox.askyesno("PolyKara", f"Remove {len(ids)} song(s) from the list? Downloaded files and videos are kept."):
            self.songs = [song for song in self.songs if song.get("id") not in ids]
            self.save_songs()

    def save_songs(self) -> None:
        missing = REQUIRED_COLUMNS - set(self.fields)
        self.fields += sorted(missing)
        write_songs(self.fields, self.songs)
        self.refresh()

    # Running the pipeline
    def run_action(self) -> None:
        label = self.action.get()
        _, command, takes_ids = next(item for item in ACTIONS if item[0] == label)
        args = [command]
        if takes_ids and self.selected_only.get():
            ids = self.selected_ids()
            if not ids:
                messagebox.showinfo("PolyKara", "Select the songs to work on, or untick 'Selected songs only'.")
                return
            args += ids
        if self.reprocess.get() and command in {"download", "render", "process"}:
            args.append("--reprocess")
        if self.realign.get() and command == "process":
            args.append("--realign")
        self.start(cli_command() + args)

    def update_ytdlp(self) -> None:
        self.start([shutil.which("yt-dlp") or "yt-dlp", "-U"])

    def edit_subtitles(self) -> None:
        ids = self.selected_ids()
        if len(ids) != 1:
            messagebox.showinfo("PolyKara", "Select one song whose subtitles you want to edit.")
            return
        self.start(cli_command() + ["edit", ids[0]])

    def play_video(self) -> None:
        ids = self.selected_ids()
        video = OUTPUT / f"{ids[0]}.mp4" if len(ids) == 1 else None
        if video is None or not video.exists():
            messagebox.showinfo("PolyKara", "Select one song whose video is ready.")
            return
        open_path(video)

    def open_folder(self, folder: Path) -> None:
        folder.mkdir(parents=True, exist_ok=True)
        open_path(folder)

    def start(self, command: list[str]) -> None:
        if self.process is not None:
            messagebox.showinfo("PolyKara", "Wait for the current run to finish, or stop it.")
            return
        env = {**os.environ, "PYTHONUNBUFFERED": "1", "PYTHONUTF8": "1", "POLYKARA_HOME": str(HOME)}
        self.write(f"\n$ {' '.join(command)}\n")
        try:
            self.process = subprocess.Popen(command, cwd=str(HOME), env=env, stdin=subprocess.DEVNULL, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, encoding="utf-8", errors="replace", bufsize=1, creationflags=NO_WINDOW, start_new_session=os.name != "nt")
        except OSError as exc:
            self.process = None
            self.write(f"Could not start: {exc}\n")
            return
        self.run_button.configure(state="disabled")
        self.stop_button.configure(state="normal")
        self.status.set("Running…")
        threading.Thread(target=self.pump, args=(self.process,), daemon=True).start()

    def pump(self, process: subprocess.Popen) -> None:
        assert process.stdout is not None
        for line in process.stdout:
            self.lines.put(line)
        process.wait()
        self.lines.put(None)

    def drain(self) -> None:
        try:
            while True:
                line = self.lines.get_nowait()
                if line is None:
                    code = self.process.returncode if self.process else 0
                    self.process = None
                    self.run_button.configure(state="normal")
                    self.stop_button.configure(state="disabled")
                    self.status.set("Finished" if code == 0 else f"Finished with problems (exit code {code}); see the summary above")
                    self.write(f"[finished, exit code {code}]\n")
                    self.refresh()
                else:
                    self.write(line)
        except queue.Empty:
            pass
        self.after(100, self.drain)

    def write(self, text: str) -> None:
        self.log.configure(state="normal")
        self.log.insert("end", text)
        self.log.see("end")
        self.log.configure(state="disabled")

    def stop(self) -> None:
        process = self.process
        if process is None:
            return
        if os.name == "nt":
            # ffmpeg and yt-dlp run as children; stop the whole tree.
            subprocess.run(["taskkill", "/T", "/F", "/PID", str(process.pid)], capture_output=True, creationflags=NO_WINDOW)
        else:
            import signal

            try:
                os.killpg(process.pid, signal.SIGTERM)
            except ProcessLookupError:
                pass
        self.write("[stopping…]\n")

    def close(self) -> None:
        if self.process is not None:
            if not messagebox.askyesno("PolyKara", "A run is still going. Stop it and quit?"):
                return
            self.stop()
        self.destroy()


def main() -> int:
    prepare()
    App().mainloop()
    return 0


if __name__ == "__main__":
    sys.exit(main())
