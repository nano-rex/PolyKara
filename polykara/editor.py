"""Timing editor tab: time lyrics by hand while the song plays, in the style of Sayatoo.

Hold Space while each highlighted word (or character) is sung: pressing
marks where it starts and letting go marks where it ends. The timeline under
the lyrics shows the waveform and every timed word; drag a word to move it or
drag its edges to change where it starts and ends. Saving writes
work/lyrics/<id>.timed.lrc, which the pipeline then uses ahead of every other
lyric source.
"""
from __future__ import annotations

import copy
import os
import queue
import subprocess
import threading
import tkinter as tk
import tkinter.font as tkfont
from pathlib import Path
from tkinter import messagebox, ttk
from tkinter.scrolledtext import ScrolledText

import functools

from . import readings, timing
from .config import AUDIO, ASS, load_config
from .romanize import base_language, ruby_groups
from .localmedia import is_local, local_path, probe
from .timing import Line, Player, Token

SPEEDS = {"100%": 1.0, "85%": 0.85, "75%": 0.75, "60%": 0.6, "50%": 0.5}
MODES = {"Hold (press = start, release = end)": "hold", "Tap (each press starts the next word)": "tap"}
# Sung colours of the preview, as in the rendered video (female, male, both).
VOICE_COLOURS = {"female": "#fc0000", "male": "#142d88", "duet": "#127e18", "女": "#fc0000", "男": "#142d88", "合": "#127e18"}
UI_FONT = "Microsoft YaHei UI" if os.name == "nt" else "Noto Sans CJK SC"
RULER, WAVE_TOP, WAVE_BOTTOM, WORD_TOP, WORD_BOTTOM, LINE_TOP, LINE_BOTTOM = 18, 22, 112, 116, 146, 150, 166
EDGE_PX = 5
HELP = ("Enter: play / pause    Hold Space: time the highlighted word    Backspace: undo the last word and go back    "
        "←/→: 3 s back / forward    ↑/↓: previous / next line    Shift+←/→: nudge the selected word 10 ms    Ctrl+Z: undo    Ctrl+S: save")


def clock(ms: int) -> str:
    ms = max(0, int(ms))
    return f"{ms // 60000:02d}:{ms // 1000 % 60:02d}.{ms % 1000:03d}"


class LyricsDialog(tk.Toplevel):
    def __init__(self, parent: tk.Misc, text: str):
        super().__init__(parent)
        self.title("Lyrics text")
        self.geometry("620x640")
        self.transient(parent)
        self.result: str | None = None
        ttk.Label(self, padding=8, wraplength=580, text="One sung line per row. Start a row with [singer: 女], [singer: 男] or [singer: 合] for duets. "
                  "Lines whose words did not change keep their timing.").pack(fill="x")
        self.text = ScrolledText(self, wrap="word", undo=True, font=(UI_FONT, 12))
        self.text.pack(fill="both", expand=True, padx=8)
        self.text.insert("1.0", text)
        buttons = ttk.Frame(self, padding=8)
        buttons.pack(fill="x")
        ttk.Button(buttons, text="Cancel", command=self.destroy).pack(side="right")
        ttk.Button(buttons, text="Apply", command=self.apply).pack(side="right", padx=6)
        self.grab_set()
        self.text.focus_set()

    def apply(self) -> None:
        self.result = self.text.get("1.0", "end-1c")
        self.destroy()


@functools.lru_cache(maxsize=512)
def line_readings(text_tokens: tuple[str, ...], language: str, stamp: tuple) -> tuple[str | None, ...]:
    """The reading shown above each token (None when it needs none), from the romanization engines and the word library."""
    settings = dict(load_config()["romanization"])
    try:
        groups = ruby_groups([(0, 0, token) for token in text_tokens], language, settings) or []
    except Exception:
        groups = []
    shown: list[str | None] = []
    for group in groups:
        shown.append(group["roman"])
        shown.extend([""] * (len(group["tokens"]) - 1))
    # Tokens that were only spaces or punctuation have no group.
    return tuple(shown) if len(shown) == len(text_tokens) else tuple([None] * len(text_tokens))


def library_stamp(language: str) -> tuple:
    return tuple(readings._stamp(path) for path in (readings.BUILTIN / f"{language}.txt", readings.user_file(language)))


class ReadingsDialog(tk.Toplevel):
    """Edit the user's word library, seeing the readings of the song's lines as they are now."""

    def __init__(self, parent: "TimingEditor", language: str):
        super().__init__(parent)
        self.editor, self.language = parent, language
        self.title(f"Word readings — {language}")
        self.geometry("760x700")
        self.transient(parent)
        name = {"zh": "pinyin", "yue": "Jyutping", "ja": "romaji"}[language]
        ttk.Label(self, padding=8, wraplength=720, text=f"Readings ({name}) shown above the characters of this song. When one is wrong, add the word "
                  f"with the right reading below; it is used for every song from now on.").pack(fill="x")
        self.current = ScrolledText(self, height=12, wrap="word", font=(UI_FONT, 12))
        self.current.pack(fill="both", expand=True, padx=8)
        add = ttk.Frame(self, padding=(8, 8, 8, 0))
        add.pack(fill="x")
        ttk.Label(add, text="Word").pack(side="left")
        self.word = tk.StringVar()
        ttk.Entry(add, textvariable=self.word, width=12, font=(UI_FONT, 12)).pack(side="left", padx=4)
        ttk.Label(add, text="Reading").pack(side="left")
        self.reading = tk.StringVar()
        ttk.Entry(add, textvariable=self.reading, width=28).pack(side="left", padx=4)
        ttk.Button(add, text="Add", command=self.add).pack(side="left")
        hint = {"zh": "one syllable per character: 重来 = chóng lái (or chong2 lai2)", "yue": "one syllable per character: 銀行 = ngan4 hong4", "ja": "the whole word in kana or romaji: 君 = きみ"}[language]
        ttk.Label(add, text=hint, foreground="#666").pack(side="left", padx=8)
        ttk.Label(self, padding=(8, 8, 8, 2), text=f"Your words ({readings.user_file(language)}):").pack(fill="x")
        self.text = ScrolledText(self, height=12, wrap="none", undo=True, font=(UI_FONT, 11))
        self.text.pack(fill="both", expand=True, padx=8)
        path = readings.user_file(language)
        self.text.insert("1.0", path.read_text(encoding="utf-8-sig") if path.exists() else f"# Your {name} readings: one word per line, the word then its reading.\n")
        self.problems = ttk.Label(self, padding=(8, 4), foreground="#b71c1c", wraplength=720)
        self.problems.pack(fill="x")
        buttons = ttk.Frame(self, padding=8)
        buttons.pack(fill="x")
        ttk.Button(buttons, text="Close", command=self.destroy).pack(side="right")
        ttk.Button(buttons, text="Save", command=self.save).pack(side="right", padx=6)
        ttk.Label(buttons, text=f"Built-in list: {len(readings.parse((readings.BUILTIN / f'{language}.txt').read_text(encoding='utf-8'), language)[0]) if (readings.BUILTIN / f'{language}.txt').exists() else 0} words", foreground="#666").pack(side="left")
        self.show()

    def show(self) -> None:
        self.current.configure(state="normal")
        self.current.delete("1.0", "end")
        stamp = library_stamp(self.language)
        for number, line in enumerate(self.editor.lines, 1):
            tokens = tuple(token.text for token in line.tokens)
            shown = line_readings(tokens, self.language, stamp)
            parts = [f"{token.strip()}({reading})" if reading else token for token, reading in zip(tokens, shown)]
            self.current.insert("end", f"{number:>3}  " + "".join(parts) + "\n")
        self.current.configure(state="disabled")

    def add(self) -> None:
        word, reading = self.word.get().strip(), " ".join(self.reading.get().split())
        if not word or not reading:
            return
        content = self.text.get("1.0", "end-1c").rstrip("\n")
        self.text.delete("1.0", "end")
        self.text.insert("1.0", content + f"\n{word} {reading}\n")
        self.word.set("")
        self.reading.set("")
        self.save()

    def save(self) -> None:
        content = self.text.get("1.0", "end-1c")
        problems = readings.parse(content, self.language)[1]
        self.problems.configure(text="Not used: " + "; ".join(problems) if problems else "")
        path = readings.user_file(self.language)
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(content.rstrip("\n") + "\n", encoding="utf-8")
        self.show()
        self.editor.draw_stage()


class TimingEditor(ttk.Frame):
    def __init__(self, parent: tk.Misc, app):
        super().__init__(parent)
        self.app = app
        self.player = Player()
        self.row: dict[str, str] | None = None
        self.lines: list[Line] = []
        self.media: Path | None = None
        self.duration = 0
        self.peaks: list[int] = []
        self.peak_max = 1
        self.cursor: tuple[int, int] | None = None
        self.selected: tuple[int, int] | None = None
        self.holding: tuple[int, int] | None = None
        self.pending_release: str | None = None
        self.playhead = 0
        self.history: list[tuple[list[Line], tuple[int, int] | None]] = []
        self.dirty = False
        self.drag: dict | None = None
        self.ranges: dict[tuple[int, int], tuple[str, str]] = {}
        self.results: queue.Queue = queue.Queue()
        self.was_playing = False
        self.now_token: tuple[int, int] | None = None

        top = ttk.Frame(self, padding=(8, 8, 8, 0))
        top.pack(fill="x")
        ttk.Label(top, text="Song").pack(side="left")
        self.song = tk.StringVar()
        self.song_box = ttk.Combobox(top, textvariable=self.song, state="readonly", width=40, postcommand=self.list_songs)
        self.song_box.pack(side="left", padx=(4, 4))
        self.song_box.bind("<<ComboboxSelected>>", lambda _: self.open(self.song.get().split("  ", 1)[0]))
        for text, command in (("Save", self.save), ("Save and make subtitles", self.save_and_build), ("Preview on video", self.preview), ("Lyrics text…", self.edit_text), ("Word readings…", self.edit_readings)):
            ttk.Button(top, text=text, command=command).pack(side="left", padx=(0, 4))
        ttk.Button(top, text="Undo", command=self.undo).pack(side="left", padx=(0, 4))
        ttk.Button(top, text="Clear line timing", command=self.clear_line).pack(side="left")

        options = ttk.Frame(self, padding=(8, 6, 8, 0))
        options.pack(fill="x")
        self.play_button = ttk.Button(options, text="▶ Play", width=9, command=self.toggle)
        self.play_button.pack(side="left")
        self.time_label = ttk.Label(options, text="00:00.000 / 00:00.000", width=24, font=("Consolas", 10) if os.name == "nt" else ("Monospace", 10))
        self.time_label.pack(side="left", padx=8)
        ttk.Label(options, text="Speed").pack(side="left")
        self.speed = tk.StringVar(value="100%")
        speed_box = ttk.Combobox(options, textvariable=self.speed, values=list(SPEEDS), state="readonly", width=6)
        speed_box.pack(side="left", padx=(4, 12))
        speed_box.bind("<<ComboboxSelected>>", lambda _: self.restart())
        ttk.Label(options, text="Keys").pack(side="left")
        self.mode = tk.StringVar(value=next(iter(MODES)))
        ttk.Combobox(options, textvariable=self.mode, values=list(MODES), state="readonly", width=34).pack(side="left", padx=(4, 12))
        ttk.Label(options, text="Reaction delay (ms)").pack(side="left")
        self.delay = tk.IntVar(value=0)
        ttk.Spinbox(options, from_=0, to=500, increment=10, textvariable=self.delay, width=5).pack(side="left", padx=4)
        self.source_label = ttk.Label(options, text="", foreground="#666")
        self.source_label.pack(side="right")

        ttk.Label(self, text=HELP, foreground="#555", padding=(8, 0, 8, 6), wraplength=1050).pack(side="bottom", fill="x")
        lower = ttk.Frame(self, padding=(8, 0, 8, 4))
        lower.pack(side="bottom", fill="x")
        panes = ttk.PanedWindow(self, orient="vertical")
        panes.pack(fill="both", expand=True, padx=8, pady=6)
        upper = ttk.PanedWindow(panes, orient="horizontal")
        self.text = ScrolledText(upper, wrap="word", cursor="arrow", font=(UI_FONT, 13), spacing1=4, spacing3=4, padx=6)
        self.text.tag_configure("number", foreground="#999", font=(UI_FONT, 9))
        self.text.tag_configure("speaker", foreground="#a05000")
        self.text.tag_configure("untimed", foreground="#9a9a9a")
        self.text.tag_configure("timed", foreground="#1b5e20")
        self.text.tag_configure("open", foreground="#e65100")
        self.text.tag_configure("now", background="#ffe082")
        self.text.tag_configure("cursor", background="#4fc3f7", foreground="#000000")
        self.text.tag_configure("selected", underline=True)
        self.text.tag_raise("cursor")
        self.text.bind("<Button-1>", self.text_click)
        self.text.configure(state="disabled")
        upper.add(self.text, weight=3)
        self.stage = tk.Canvas(upper, background="#101418", height=220, highlightthickness=0)
        self.stage.bind("<Configure>", lambda _: self.draw_stage())
        upper.add(self.stage, weight=2)
        panes.add(upper, weight=3)

        zoom = ttk.Frame(lower)
        zoom.pack(fill="x")
        ttk.Label(zoom, text="Timeline: click to move the play position, drag a word or its edges to change it, Ctrl+wheel to zoom", foreground="#666").pack(side="left")
        ttk.Button(zoom, text="Zoom in", command=lambda: self.zoom(1.5)).pack(side="right")
        ttk.Button(zoom, text="Zoom out", command=lambda: self.zoom(1 / 1.5)).pack(side="right", padx=4)
        self.pps = 150.0
        self.timeline = tk.Canvas(lower, background="#1e2328", height=LINE_BOTTOM + 6, highlightthickness=0)
        self.timeline.pack(fill="x")
        self.scroll = ttk.Scrollbar(lower, orient="horizontal", command=self.scroll_timeline)
        self.scroll.pack(fill="x")
        self.timeline.configure(xscrollcommand=self.scroll.set)
        self.timeline.bind("<Configure>", lambda _: self.draw_timeline())
        self.timeline.bind("<Button-1>", self.press_timeline)
        self.timeline.bind("<B1-Motion>", self.drag_timeline)
        self.timeline.bind("<ButtonRelease-1>", self.release_timeline)
        self.timeline.bind("<Double-Button-1>", self.double_timeline)
        self.timeline.bind("<Motion>", self.hover_timeline)
        for sequence in ("<MouseWheel>", "<Button-4>", "<Button-5>"):
            self.timeline.bind(sequence, self.wheel)

        root = self.winfo_toplevel()
        for sequence, handler in (("<KeyPress-space>", self.key_down), ("<KeyRelease-space>", self.key_up), ("<Return>", lambda e: self.toggle()), ("<BackSpace>", self.step_back),
                                  ("<Left>", lambda e: self.seek_by(-3000)), ("<Right>", lambda e: self.seek_by(3000)), ("<Up>", lambda e: self.jump_line(-1)), ("<Down>", lambda e: self.jump_line(1)),
                                  ("<Shift-Left>", lambda e: self.nudge(-10)), ("<Shift-Right>", lambda e: self.nudge(10)), ("<Control-z>", lambda e: self.undo()), ("<Control-s>", lambda e: self.save()),
                                  ("<Escape>", lambda e: self.pause())):
            root.bind(sequence, lambda event, handler=handler: self.key(event, handler), add="+")
        self.ticking = self.after(40, self.tick)

    # Keyboard: only while this tab is showing and no text box has the focus.
    def active(self, event=None) -> bool:
        if not self.winfo_viewable() or self.row is None:
            return False
        widget = event.widget if event is not None else self.focus_get()
        if isinstance(widget, (tk.Entry, ttk.Entry, ttk.Combobox, ttk.Spinbox)):
            return False
        return not (isinstance(widget, tk.Text) and widget is not self.text)

    def key(self, event, handler):
        if not self.active(event):
            return None
        handler(event)
        return "break"

    # Songs
    def list_songs(self) -> None:
        self.song_box.configure(values=[f"{song['id']}  {song.get('title', '')}" for song in self.app.visible_songs()])

    def confirm_discard(self) -> bool:
        if not self.dirty:
            return True
        answer = messagebox.askyesnocancel("PolyKara", "Save the timing changes first?")
        if answer is None:
            return False
        return self.save() if answer else True

    def open(self, sid: str) -> None:
        row = next((song for song in self.app.songs if song.get("id") == sid), None)
        if row is None or (self.row is not None and self.row["id"] == sid and self.lines):
            return
        if not self.confirm_discard():
            self.song.set(f"{self.row['id']}  {self.row.get('title', '')}" if self.row else "")
            return
        self.player.stop()
        try:
            lines, label = timing.load(row)
        except Exception as exc:  # a broken lyric file must not close the editor
            lines, label = [], ""
            messagebox.showwarning("PolyKara", f"The lyrics of {sid} could not be read ({exc}). Paste them with Lyrics text….")
        self.row, self.lines, self.history, self.dirty = row, lines, [], False
        self.song.set(f"{sid}  {row.get('title', '')}")
        self.media = timing.media_for(sid) or (local_path(row["url"]) if is_local(row.get("url", "")) and local_path(row["url"]).is_file() else None)
        self.peaks, self.peak_max, self.duration, self.playhead = [], 1, 0, 0
        self.cursor = self.first_untimed()
        self.selected = None
        self.source_label.configure(text=f"Lyrics: {label}" if label else "No lyrics yet")
        if self.media is None:
            messagebox.showinfo("PolyKara", f"{sid} has not been downloaded yet. Run Download for it on the Songs tab, then open it here again.")
        else:
            threading.Thread(target=self.load_audio, args=(sid, self.media), daemon=True).start()
        self.render_text()
        self.draw_timeline()
        self.draw_stage()
        if not lines:
            messagebox.showinfo("PolyKara", "No lyrics were found for this song. Paste them in the next window, then time them while the song plays.")
            self.edit_text()

    def load_audio(self, sid: str, media: Path) -> None:
        """Extract the audio once (exact seeking in video files), then read its duration and waveform."""
        try:
            audio = AUDIO / f"{sid}.flac"
            if media != audio and probe(media)["video"]:
                AUDIO.mkdir(parents=True, exist_ok=True)
                partial = AUDIO / f"{sid}.partial.flac"
                subprocess.run(["ffmpeg", "-hide_banner", "-loglevel", "error", "-y", "-i", str(media), "-vn", "-ac", "2", "-ar", "48000", "-c:a", "flac", str(partial)], check=False, timeout=900, creationflags=timing.NO_WINDOW)
                if partial.exists() and partial.stat().st_size:
                    os.replace(partial, audio)
                    media = audio
                partial.unlink(missing_ok=True)
            duration = int(probe(media)["duration"] * 1000)
            peaks = timing.waveform(media)
            self.results.put((sid, media, duration, peaks))
        except Exception as exc:
            self.results.put((sid, None, 0, str(exc)))

    def first_untimed(self) -> tuple[int, int] | None:
        for li, line in enumerate(self.lines):
            for ti, token in enumerate(line.tokens):
                if not token.timed:
                    return li, ti
        return (0, 0) if self.lines else None

    # Editing helpers
    def remember(self) -> None:
        self.history.append((copy.deepcopy(self.lines), self.cursor))
        del self.history[:-200]
        self.dirty = True

    def undo(self) -> None:
        if not self.history:
            return
        self.lines, self.cursor = self.history.pop()
        self.holding, self.selected = None, None
        self.changed()

    def changed(self) -> None:
        self.render_text()
        self.draw_timeline()
        self.draw_stage()

    def token(self, place: tuple[int, int] | None) -> Token | None:
        if place is None or place[0] >= len(self.lines) or place[1] >= len(self.lines[place[0]].tokens):
            return None
        return self.lines[place[0]].tokens[place[1]]

    def following(self, place: tuple[int, int]) -> tuple[int, int] | None:
        li, ti = place
        if ti + 1 < len(self.lines[li].tokens):
            return li, ti + 1
        return (li + 1, 0) if li + 1 < len(self.lines) else None

    def previous(self, place: tuple[int, int] | None) -> tuple[int, int] | None:
        if place is None:
            if not self.lines:
                return None
            return len(self.lines) - 1, len(self.lines[-1].tokens) - 1
        li, ti = place
        if ti > 0:
            return li, ti - 1
        return (li - 1, len(self.lines[li - 1].tokens) - 1) if li > 0 else None

    def edit_text(self) -> None:
        if self.row is None:
            messagebox.showinfo("PolyKara", "Choose a song first.")
            return
        dialog = LyricsDialog(self, timing.lines_to_text(self.lines))
        self.wait_window(dialog)
        if dialog.result is None or dialog.result == timing.lines_to_text(self.lines):
            return
        self.remember()
        self.lines = timing.retext(self.lines, dialog.result)
        self.cursor = self.first_untimed()
        self.changed()

    def language(self) -> str:
        return base_language((self.row or {}).get("language", "") or "")

    def edit_readings(self) -> None:
        if self.row is None:
            messagebox.showinfo("PolyKara", "Choose a song first.")
            return
        language = self.language()
        if language not in readings.LANGUAGES:
            messagebox.showinfo("PolyKara", "The word library is for songs in Mandarin (zh), Cantonese (yue) and Japanese (ja). Set the song's language on the Songs tab.")
            return
        ReadingsDialog(self, language)

    def clear_line(self) -> None:
        place = self.selected or self.cursor
        if place is None:
            return
        self.remember()
        line = self.lines[place[0]]
        for token in line.tokens:
            token.start = token.end = None
        line.start = line.end = None
        self.cursor = (place[0], 0)
        self.changed()

    # Playback
    def speed_value(self) -> float:
        return SPEEDS.get(self.speed.get(), 1.0)

    def position(self) -> int:
        return self.player.position() if self.player.playing else self.playhead

    def play(self, at: int | None = None) -> None:
        if self.media is None:
            return
        self.playhead = max(0, at if at is not None else self.playhead)
        try:
            self.player.play(self.media, self.playhead, self.speed_value())
        except OSError as exc:
            messagebox.showerror("PolyKara", f"ffplay could not be started ({exc}).")
            return
        self.play_button.configure(text="⏸ Pause")

    def pause(self) -> None:
        if self.player.playing:
            self.playhead = self.player.position()
        self.player.stop()
        self.finish_hold(self.playhead)
        self.play_button.configure(text="▶ Play")

    def toggle(self) -> None:
        self.pause() if self.player.playing else self.play()

    def restart(self) -> None:
        if self.player.playing:
            self.play(self.player.position())

    def seek(self, ms: int) -> None:
        ms = max(0, min(ms, self.duration or ms))
        if self.player.playing:
            self.play(ms)
        else:
            self.playhead = ms
        self.follow(ms, force=True)
        self.draw_stage()

    def seek_by(self, delta: int, _event=None) -> None:
        self.seek(self.position() + delta)

    def jump_line(self, step: int) -> None:
        if not self.lines:
            return
        li = (self.cursor or (len(self.lines), 0))[0] + step
        li = max(0, min(li, len(self.lines) - 1))
        self.cursor = (li, 0)
        span = self.lines[li].span()
        if span:
            self.seek(span[0] - 1500)
        self.render_text()

    # Timing with the keyboard
    def key_down(self, _event=None) -> None:
        if self.pending_release is not None:
            # A release immediately followed by a press is key auto-repeat (X11), not a new press.
            self.after_cancel(self.pending_release)
            self.pending_release = None
            return
        if self.holding is not None:
            return  # auto-repeat on Windows sends presses only
        if not self.player.playing:
            self.play()
            return
        token = self.token(self.cursor)
        if token is None:
            return
        at = self.player.position() - max(0, int(self.delay.get() or 0))
        self.remember()
        li, ti = self.cursor
        line = self.lines[li]
        if self.mode_value() == "tap" and ti > 0:
            before = line.tokens[ti - 1]
            if before.start is not None and before.end is None:
                before.end = max(at, before.start + timing.MIN_TOKEN_MS)
        # Later words of this line timed before this point are from an earlier take.
        for later in line.tokens[ti + 1:]:
            if later.start is not None and later.start < at:
                later.start = later.end = None
        token.start, token.end = at, None
        last = ti == len(line.tokens) - 1
        if self.mode_value() == "hold" or last:
            self.holding = self.cursor
        else:
            self.cursor = self.following(self.cursor)
        self.selected = (li, ti)
        self.changed()

    def key_up(self, _event=None) -> None:
        if self.holding is None:
            return
        at = self.player.position()
        if self.pending_release is not None:
            self.after_cancel(self.pending_release)
        self.pending_release = self.after(35, lambda: self.release(at))

    def release(self, at: int) -> None:
        self.pending_release = None
        self.finish_hold(at - max(0, int(self.delay.get() or 0)))

    def finish_hold(self, at: int) -> None:
        if self.holding is None:
            return
        token = self.token(self.holding)
        if token is not None and token.start is not None:
            token.end = max(at, token.start + timing.MIN_TOKEN_MS)
        self.cursor = self.following(self.holding)
        self.holding = None
        self.changed()

    def mode_value(self) -> str:
        return MODES.get(self.mode.get(), "hold")

    def step_back(self, _event=None) -> None:
        """Undo the last timed word and go back to just before it."""
        if self.holding is not None:
            return
        place = self.previous(self.cursor)
        token = self.token(place)
        if token is None:
            return
        self.remember()
        start = token.start
        token.start = token.end = None
        self.cursor = place
        self.changed()
        if start is not None:
            self.seek(start - 2000)

    def nudge(self, delta: int) -> None:
        token = self.token(self.selected)
        if token is None or not token.timed:
            return
        self.remember()
        token.start += delta
        token.end += delta
        self.changed()

    # Saving
    def save(self) -> bool:
        if self.row is None:
            return False
        self.close_open()
        if not any(line.span() for line in self.lines):
            messagebox.showinfo("PolyKara", "Nothing is timed yet.")
            return False
        path, estimated = timing.save(self.row, self.lines)
        self.dirty = False
        note = f"\n\n{estimated} line(s) had no timing yet and were placed between their neighbours; time them before rendering." if estimated else ""
        self.app.status.set(f"Saved timing to {path}")
        if estimated:
            messagebox.showinfo("PolyKara", f"Saved {path.name}.{note}")
        self.source_label.configure(text=f"Lyrics: your saved timing ({path.name})")
        return True

    def close_open(self) -> None:
        """Give a word whose key was never released an end: the next word's start, or 400 ms."""
        for li, line in enumerate(self.lines):
            for ti, token in enumerate(line.tokens):
                if token.start is not None and token.end is None:
                    after = line.tokens[ti + 1].start if ti + 1 < len(line.tokens) else None
                    token.end = after if after is not None and after > token.start else token.start + 400

    def aside_edited_ass(self) -> bool:
        """A hand-edited ASS file overrides the timing editor; offer to set it aside."""
        edited = ASS / f"{self.row['id']}.edited.ass"
        if not edited.exists():
            return True
        if not messagebox.askyesno("PolyKara", f"{edited.name} (edited in a subtitle editor) is used for the video instead of this timing. Rename it to {edited.name}.bak so this timing is used?"):
            return False
        os.replace(edited, edited.with_name(edited.name + ".bak"))
        return True

    def save_and_build(self) -> None:
        if self.save() and self.aside_edited_ass():
            self.app.start(self.app.cli_command() + ["lyrics", self.row["id"]])
            self.app.status.set("Making subtitles… (log on the Songs tab). Then use Render videos only.")

    def preview(self) -> None:
        if self.row is None or not self.save() or not self.aside_edited_ass():
            return
        self.pause()
        self.app.start(self.app.cli_command() + ["preview", self.row["id"], "--at", f"{max(0, self.playhead - 2000) / 1000:.2f}"])

    # Lyrics list
    def render_text(self) -> None:
        self.text.configure(state="normal")
        self.text.delete("1.0", "end")
        self.ranges.clear()
        for li, line in enumerate(self.lines):
            self.text.insert("end", f"{li + 1:>3}  ", "number")
            if line.speaker:
                self.text.insert("end", f"{line.speaker}: ", "speaker")
            for ti, token in enumerate(line.tokens):
                start = self.text.index("end-1c")
                state = "timed" if token.timed else "open" if token.start is not None else "untimed"
                tags = [state, f"t{li}_{ti}"]
                if (li, ti) == self.cursor:
                    tags.append("cursor")
                if (li, ti) == self.selected:
                    tags.append("selected")
                self.text.insert("end", token.text, tuple(tags))
                self.ranges[(li, ti)] = (start, self.text.index("end-1c"))
            self.text.insert("end", "\n")
        self.text.configure(state="disabled")
        self.now_token = None
        if self.cursor in self.ranges:
            self.text.see(self.ranges[self.cursor][0])

    def text_click(self, event) -> str:
        index = self.text.index(f"@{event.x},{event.y}")
        for name in self.text.tag_names(index):
            if name.startswith("t") and "_" in name:
                li, ti = (int(part) for part in name[1:].split("_"))
                self.cursor, self.selected = (li, ti), (li, ti)
                token = self.lines[li].tokens[ti]
                span = self.lines[li].span()
                target = token.start if token.start is not None else span[0] if span else None
                self.render_text()
                self.draw_timeline()
                if target is not None:
                    self.seek(target - 1500)
                break
        self.focus_set()
        return "break"

    def mark_now(self, ms: int) -> None:
        place = None
        for li, line in enumerate(self.lines):
            for ti, token in enumerate(line.tokens):
                if token.start is not None and token.start <= ms < (token.end if token.end is not None else token.start + 400):
                    place = (li, ti)
        if place == self.now_token:
            return
        self.text.tag_remove("now", "1.0", "end")
        self.now_token = place
        if place in self.ranges:
            self.text.tag_add("now", *self.ranges[place])
            if self.holding is None:
                self.text.see(self.ranges[place][0])

    # Preview of the sung line
    def stage_lines(self, ms: int) -> list[int]:
        """The line being sung (or coming next) and the one after it."""
        # Only words timed here count: line times from other sources may not fit this recording.
        current = None
        for li, line in enumerate(self.lines):
            starts = [token.start for token in line.tokens if token.start is not None]
            if starts and min(starts) - 1500 <= ms:
                current = li
        waiting = self.cursor[0] if self.cursor else None
        if current is None:
            current = waiting if waiting is not None else 0
        else:
            ends = [token.end for token in self.lines[current].tokens if token.end is not None]
            finished = ends and ms > max(ends) + 800 and all(token.timed for token in self.lines[current].tokens)
            if finished and current + 1 < len(self.lines):
                current = waiting if waiting is not None and waiting > current else current + 1
        return [li for li in (current, current + 1) if li < len(self.lines)]

    def draw_stage(self) -> None:
        canvas = self.stage
        canvas.delete("all")
        width, height = max(200, canvas.winfo_width()), max(120, canvas.winfo_height())
        if not self.lines:
            canvas.create_text(width / 2, height / 2, text="Choose a song", fill="#778", font=(UI_FONT, 14))
            return
        ms = self.position()
        size = max(14, min(34, height // 6))
        for row_number, li in enumerate(self.stage_lines(ms)):
            line = self.lines[li]
            colour = VOICE_COLOURS.get(line.speaker or (self.row or {}).get("voice", ""), "#fc0000")
            font = (UI_FONT, size, "bold")
            y = height * (0.35 if row_number == 0 else 0.7)
            language = self.language()
            tokens = tuple(token.text for token in line.tokens)
            shown = line_readings(tokens, language, library_stamp(language)) if language else ()
            # Groups of tokens under one reading: (first token, token count, reading).
            groups, index = [], 0
            while index < len(tokens):
                count = 1
                while index + count < len(tokens) and index < len(shown) and shown[index] and shown[index + count] == "":
                    count += 1
                groups.append((index, count, shown[index] if index < len(shown) else None))
                index += count
            scale = 1.0
            for _attempt in range(3):
                font = (UI_FONT, max(8, int(size * scale)), "bold")
                ruby_font = (UI_FONT, max(7, int(size * scale * 0.42)))
                measure, ruby_measure = tkfont.Font(font=font), tkfont.Font(font=ruby_font)
                # Each group is as wide as its characters or its reading, whichever is wider.
                layout, total = [], 0.0
                for first, count, reading in groups:
                    widths = [measure.measure(text) for text in tokens[first:first + count]]
                    need = ruby_measure.measure(reading) + ruby_measure.measure(" ") if reading else 0
                    pad = max(0.0, need - sum(widths)) / count
                    layout.append((first, count, reading, total, [w + pad for w in widths], pad))
                    total += sum(widths) + pad * count
                if total <= width * 0.94:
                    break
                scale *= width * 0.94 / total  # shrink a line wider than the preview
            # Staggered like the video: the first row leans left, the second right.
            spare = max(0, width - total - 24)
            left = 12 + (spare * 0.8 if row_number else spare * 0.2)
            for first, count, reading, offset, advances, pad in layout:
                x = left + offset
                group_sung = line.tokens[first].start is not None and ms >= line.tokens[first].start
                if reading:
                    canvas.create_text(x + sum(advances) / 2, y - font[1] * 0.95, text=reading, fill=colour if group_sung else "#e0e0e0", font=ruby_font)
                for number in range(count):
                    token, advance = line.tokens[first + number], advances[number]
                    sung = token.start is not None and ms >= token.start
                    inner = x + pad / 2
                    canvas.create_text(inner + 2, y + 2, text=token.text, anchor="w", fill="#000000", font=font)
                    canvas.create_text(inner, y, text=token.text, anchor="w", fill=colour if sung else "#ffffff", font=font)
                    if token.start is not None and token.start <= ms and (token.end is None or ms < token.end):
                        end_ms = token.end if token.end is not None else ms + 1
                        share = min(1.0, (ms - token.start) / max(1, end_ms - token.start))
                        canvas.create_rectangle(x, y + font[1] * 0.85, x + advance * share, y + font[1] * 0.85 + 4, fill=colour, outline="")
                    x += advance
        canvas.create_text(10, height - 10, anchor="sw", text=clock(ms), fill="#667", font=("Consolas", 10))

    # Timeline
    def x_of(self, ms: int) -> float:
        return ms * self.pps / 1000

    def ms_of(self, x: float) -> int:
        return int(x * 1000 / self.pps)

    def scroll_timeline(self, *args) -> None:
        self.timeline.xview(*args)
        self.draw_timeline()

    def zoom(self, factor: float, around: float | None = None) -> None:
        anchor = self.ms_of(self.timeline.canvasx(around if around is not None else self.timeline.winfo_width() / 2))
        self.pps = max(20.0, min(1500.0, self.pps * factor))
        self.update_region()
        width = max(1, self.timeline.winfo_width())
        total = max(1.0, self.x_of(self.duration) + 50)
        self.timeline.xview_moveto(max(0.0, (self.x_of(anchor) - (around if around is not None else width / 2)) / total))
        self.draw_timeline()

    def update_region(self) -> None:
        end = max(self.duration, max((span[1] for line in self.lines if (span := line.span())), default=0) + 5000)
        self.timeline.configure(scrollregion=(0, 0, self.x_of(end) + 50, LINE_BOTTOM + 6))

    def follow(self, ms: int, force: bool = False) -> None:
        """Scroll the timeline so the play position stays in view."""
        left, width = self.timeline.canvasx(0), self.timeline.winfo_width()
        x = self.x_of(ms)
        if force or x < left or x > left + width * 0.85:
            first, _ = self.timeline.xview()
            region = float(self.timeline.cget("scrollregion").split()[2]) if self.timeline.cget("scrollregion") else 1.0
            self.timeline.xview_moveto(max(0.0, (x - width * 0.2) / max(1.0, region)))
            if self.timeline.xview()[0] != first:
                self.draw_timeline()

    def draw_timeline(self) -> None:
        canvas = self.timeline
        self.update_region()
        canvas.delete("all")
        left, width = canvas.canvasx(0), max(1, canvas.winfo_width())
        right = left + width
        start_ms, end_ms = self.ms_of(left), self.ms_of(right)
        step = next(value for value in (100, 250, 500, 1000, 2000, 5000, 10000, 30000, 60000) if self.x_of(value) >= 60)
        for tick in range(start_ms - start_ms % step, end_ms + step, step):
            x = self.x_of(tick)
            canvas.create_line(x, 0, x, RULER - 4, fill="#556")
            canvas.create_text(x + 3, 1, text=clock(tick)[:5] if step >= 1000 else clock(tick)[:-1], anchor="nw", fill="#889", font=("Consolas", 8))
        if self.peaks:
            middle, half = (WAVE_TOP + WAVE_BOTTOM) / 2, (WAVE_BOTTOM - WAVE_TOP) / 2
            for px in range(int(left), int(right), 2):
                low, high = self.ms_of(px) // 10, max(self.ms_of(px) // 10 + 1, self.ms_of(px + 2) // 10)
                chunk = self.peaks[low:high]
                if chunk:
                    level = max(chunk) / self.peak_max * half
                    canvas.create_line(px, middle - level, px, middle + level, fill="#3d6f8f")
        # Lines with no word timed yet are drawn first and dashed: their times come from the lyric file.
        order = sorted(range(len(self.lines)), key=lambda li: any(token.start is not None for token in self.lines[li].tokens))
        for li in order:
            line = self.lines[li]
            span = line.span()
            if span is None or span[1] < start_ms - 1000 or span[0] > end_ms + 1000:
                continue
            guessed = not any(token.start is not None for token in line.tokens)
            canvas.create_rectangle(self.x_of(span[0]), LINE_TOP, self.x_of(span[1]), LINE_BOTTOM, fill="" if guessed else "#37474f", outline="#607d8b", dash=(3, 3) if guessed else None, tags=("line", f"l{li}"))
            canvas.create_text(self.x_of(span[0]) + 3, (LINE_TOP + LINE_BOTTOM) / 2, text=f"{li + 1}. {line.text}", anchor="w", fill="#78909c" if guessed else "#cfd8dc", font=(UI_FONT, 8), tags=("line", f"l{li}"))
            for ti, token in enumerate(line.tokens):
                if token.start is None:
                    continue
                end = token.end if token.end is not None else max(token.start + 100, self.position() if self.holding == (li, ti) else token.start + 400)
                fill = "#ff8f00" if (li, ti) == self.selected else "#e65100" if token.end is None else "#2e7d32" if li % 2 == 0 else "#1565c0"
                x1, x2 = self.x_of(token.start), self.x_of(end)
                canvas.create_rectangle(x1, WORD_TOP, x2, WORD_BOTTOM, fill=fill, outline="#e0e0e0", tags=("word", f"w{li}_{ti}"))
                if x2 - x1 > 10:
                    canvas.create_text((x1 + x2) / 2, (WORD_TOP + WORD_BOTTOM) / 2, text=token.text.strip(), fill="#ffffff", font=(UI_FONT, 9), tags=("word", f"w{li}_{ti}"))
        x = self.x_of(self.position())
        canvas.create_line(x, 0, x, LINE_BOTTOM + 6, fill="#ff5252", width=2, tags=("playhead",))

    def word_at(self, x: float, y: float) -> tuple[int, int] | None:
        if not WORD_TOP <= y <= WORD_BOTTOM:
            return None
        for item in reversed(self.timeline.find_overlapping(x - 1, y - 1, x + 1, y + 1)):
            for name in self.timeline.gettags(item):
                if name.startswith("w") and "_" in name:
                    li, ti = (int(part) for part in name[1:].split("_"))
                    return li, ti
        return None

    def edge(self, place: tuple[int, int], x: float) -> str:
        token = self.token(place)
        if token is None or not token.timed:
            return "move"
        if abs(x - self.x_of(token.start)) <= EDGE_PX:
            return "start"
        if abs(x - self.x_of(token.end)) <= EDGE_PX:
            return "end"
        return "move"

    def hover_timeline(self, event) -> None:
        x = self.timeline.canvasx(event.x)
        place = self.word_at(x, event.y)
        shape = "sb_h_double_arrow" if place and self.edge(place, x) != "move" else "fleur" if place else "xterm"
        if str(self.timeline.cget("cursor")) != shape:
            self.timeline.configure(cursor=shape)

    def press_timeline(self, event) -> None:
        self.timeline.focus_set()
        x = self.timeline.canvasx(event.x)
        place = self.word_at(x, event.y)
        if place is None:
            if LINE_TOP <= event.y <= LINE_BOTTOM:
                for item in self.timeline.find_overlapping(x - 1, event.y - 1, x + 1, event.y + 1):
                    for name in self.timeline.gettags(item):
                        if name.startswith("l") and name[1:].isdigit():
                            self.cursor = self.selected = (int(name[1:]), 0)
                            self.render_text()
            self.drag = None
            self.seek(self.ms_of(x))
            self.draw_timeline()
            return
        token = self.token(place)
        if token is None or token.start is None:
            return
        self.remember()
        if token.end is None:
            token.end = token.start + 400
        self.selected = place
        self.drag = {"place": place, "part": self.edge(place, x), "x": x, "start": token.start, "end": token.end}
        self.render_text()
        self.draw_timeline()

    def limits(self, place: tuple[int, int]) -> tuple[int, int]:
        """How far a word may move: up to its timed neighbours."""
        before, after = self.token(self.previous(place)), self.token(self.following(place))
        low = before.end if before is not None and before.end is not None else 0
        high = after.start if after is not None and after.start is not None else 10 ** 9
        return low, high

    def drag_timeline(self, event) -> None:
        if not self.drag:
            return
        token = self.token(self.drag["place"])
        delta = self.ms_of(self.timeline.canvasx(event.x) - self.drag["x"])
        low, high = self.limits(self.drag["place"])
        start, end = self.drag["start"], self.drag["end"]
        if self.drag["part"] == "start":
            token.start = max(low, min(start + delta, end - timing.MIN_TOKEN_MS))
        elif self.drag["part"] == "end":
            token.end = min(high, max(end + delta, start + timing.MIN_TOKEN_MS))
        else:
            delta = max(low - start, min(delta, high - end))
            token.start, token.end = start + delta, end + delta
        self.draw_timeline()
        self.time_label.configure(text=f"{clock(token.start)} – {clock(token.end)}")

    def release_timeline(self, _event) -> None:
        if self.drag:
            self.drag = None
            self.changed()

    def double_timeline(self, event) -> None:
        place = self.word_at(self.timeline.canvasx(event.x), event.y)
        token = self.token(place)
        if token is not None and token.start is not None:
            self.drag = None
            self.cursor = self.selected = place
            self.render_text()
            self.seek(token.start - 500)

    def wheel(self, event) -> str:
        delta = 1 if getattr(event, "num", 0) == 4 or getattr(event, "delta", 0) > 0 else -1
        if event.state & 0x4:  # Control
            self.zoom(1.25 if delta > 0 else 0.8, event.x)
        else:
            self.timeline.xview_scroll(-delta * 3, "units")
            self.draw_timeline()
        return "break"

    # Periodic work: audio results and playback display.
    def tick(self) -> None:
        try:
            while True:
                sid, media, duration, peaks = self.results.get_nowait()
                if self.row is None or sid != self.row["id"]:
                    continue
                if media is None:
                    messagebox.showwarning("PolyKara", f"The audio could not be read: {peaks}")
                    continue
                self.media, self.duration = media, duration
                self.peaks, self.peak_max = peaks, max(peaks or [1]) or 1
                self.draw_timeline()
        except queue.Empty:
            pass
        playing = self.player.playing
        if playing or self.was_playing:
            ms = self.position()
            if not playing:
                self.pause()
                ms = self.playhead
            self.time_label.configure(text=f"{clock(ms)} / {clock(self.duration)}")
            self.timeline.coords("playhead", self.x_of(ms), 0, self.x_of(ms), LINE_BOTTOM + 6)
            self.follow(ms)
            if self.holding is not None:
                self.draw_timeline()
            self.mark_now(ms)
            self.draw_stage()
        elif self.drag is None:
            self.time_label.configure(text=f"{clock(self.playhead)} / {clock(self.duration)}")
        self.was_playing = playing
        self.ticking = self.after(40, self.tick)

    def close(self) -> bool:
        if not self.confirm_discard():
            return False
        self.player.stop()
        return True

    def destroy(self) -> None:
        self.after_cancel(self.ticking)
        self.player.stop()
        super().destroy()
