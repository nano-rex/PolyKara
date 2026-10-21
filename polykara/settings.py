"""Settings window: polykara.toml as a form, with the raw file one tab away.

Every setting PolyKara knows (its defaults merged with the file) gets a field
chosen by its value: a tick box, a number, a list of choices, a font list,
or a colour button for ASS colours (&HAABBGGRR). The comment above a setting
in polykara.toml is shown as its help. Saving rewrites only the values that
changed, in place, so the file keeps its comments and layout.
"""
from __future__ import annotations

import os
import re
import tkinter as tk
import tkinter.font as tkfont
import tomllib
from tkinter import colorchooser, filedialog, messagebox, ttk
from tkinter.scrolledtext import ScrolledText

from .config import CONFIG_FILE, HOME
from .settingsfile import ASS_COLOUR, CHOICES, COLOUR_WORDS, POSITIONS, SECTION_NAMES, TABS, ass_to_hex, comments, current_values, hex_to_ass, label_for, literal, set_value  # noqa: F401

class ScrollFrame(ttk.Frame):
    def __init__(self, parent):
        super().__init__(parent)
        self.canvas = tk.Canvas(self, highlightthickness=0, borderwidth=0)
        bar = ttk.Scrollbar(self, orient="vertical", command=self.canvas.yview)
        self.body = ttk.Frame(self.canvas, padding=8)
        self.body.bind("<Configure>", lambda _: self.canvas.configure(scrollregion=self.canvas.bbox("all")))
        window = self.canvas.create_window((0, 0), window=self.body, anchor="nw")
        self.canvas.bind("<Configure>", lambda event: self.canvas.itemconfigure(window, width=event.width))
        self.canvas.configure(yscrollcommand=bar.set)
        self.canvas.pack(side="left", fill="both", expand=True)
        bar.pack(side="right", fill="y")
        for sequence in ("<MouseWheel>", "<Button-4>", "<Button-5>"):
            self.bind_all(sequence, self.wheel, add="+")

    def wheel(self, event) -> None:
        if not self.winfo_viewable():
            return
        widget = self.winfo_containing(event.x_root, event.y_root)
        while widget is not None and widget is not self:
            widget = widget.master
        if widget is self:
            step = -1 if getattr(event, "num", 0) == 4 or getattr(event, "delta", 0) > 0 else 1
            self.canvas.yview_scroll(step * 3, "units")


class SettingsDialog(tk.Toplevel):
    def __init__(self, parent: tk.Misc):
        super().__init__(parent)
        self.title(f"Settings — {CONFIG_FILE}")
        self.geometry("900x700")
        self.transient(parent)
        self.original = CONFIG_FILE.read_text(encoding="utf-8") if CONFIG_FILE.exists() else ""
        try:
            self.values = current_values(self.original)
        except tomllib.TOMLDecodeError as exc:
            messagebox.showwarning("PolyKara", f"polykara.toml has a mistake, so only the Advanced tab can be used until it is fixed:\n{exc}", parent=self)
            self.values = None
        self.help = comments(self.original)
        self.fields: dict[tuple[str, str], tuple[tk.Variable, object]] = {}
        self.fonts = sorted({name for name in tkfont.families(self) if not name.startswith("@")}, key=str.casefold)

        self.tabs = ttk.Notebook(self)
        self.tabs.pack(fill="both", expand=True, padx=8, pady=(8, 0))
        if self.values is not None:
            shown = set()
            for name, sections in TABS:
                frame = ScrollFrame(self.tabs)
                self.tabs.add(frame, text=name)
                for section in sections:
                    if isinstance(self.values.get(section), dict):
                        self.section(frame.body, section)
                        shown.add(section)
            others = [section for section, value in self.values.items() if isinstance(value, dict) and section not in shown]
            if others:
                frame = ScrollFrame(self.tabs)
                self.tabs.add(frame, text="Other")
                for section in others:
                    self.section(frame.body, section)
        advanced = ttk.Frame(self.tabs)
        self.tabs.add(advanced, text="Advanced (file)")
        self.text = ScrolledText(advanced, wrap="none", undo=True, font=("Consolas", 10) if os.name == "nt" else ("Monospace", 10))
        self.text.pack(fill="both", expand=True)
        self.text.insert("1.0", self.original)
        self.tabs.bind("<<NotebookTabChanged>>", self.tab_changed)
        self.on_text = self.tabs.index("end") - 1 if self.values is None else None
        if self.values is None:
            self.tabs.select(advanced)

        self.hint = tk.StringVar(value="Point at a setting to see what it does. Changes apply to the next run.")
        ttk.Label(self, textvariable=self.hint, wraplength=860, foreground="#444", padding=(10, 6)).pack(fill="x")
        buttons = ttk.Frame(self, padding=8)
        buttons.pack(fill="x")
        ttk.Button(buttons, text="Close", command=self.destroy).pack(side="right")
        ttk.Button(buttons, text="Save", command=self.save).pack(side="right", padx=6)
        self.last_tab = self.tabs.index("current")

    # Building the form
    def section(self, parent: ttk.Frame, section: str) -> None:
        box = ttk.LabelFrame(parent, text=f"{SECTION_NAMES.get(section, section)}  [{section}]", padding=8)
        box.pack(fill="x", pady=(0, 10))
        box.columnconfigure(1, weight=1)
        for row, (key, value) in enumerate(self.values[section].items()):
            if isinstance(value, dict):
                continue
            label = ttk.Label(box, text=label_for(key))
            label.grid(row=row, column=0, sticky="w", padx=(0, 10), pady=2)
            widget, variable = self.field(box, section, key, value)
            widget.grid(row=row, column=1, sticky="w", pady=2)
            help_text = self.help.get((section, key)) or self.help.get((section, ""), "")
            if key.endswith("color"):
                help_text = (help_text + " " if help_text else "") + "Colours are written &H<alpha><blue><green><red> in hex; alpha 00 is solid, FF invisible. Choose… picks one."
            for item in (label, widget):
                item.bind("<Enter>", lambda _e, text=help_text, name=f"[{section}] {key}": self.hint.set(f"{name}: {text}" if text else name))
            self.fields[(section, key)] = (variable, value)

    def field(self, parent, section: str, key: str, value):
        if isinstance(value, bool):
            variable = tk.BooleanVar(value=value)
            return ttk.Checkbutton(parent, variable=variable), variable
        if isinstance(value, list):
            variable = tk.StringVar(value=", ".join(str(item) for item in value))
            return ttk.Entry(parent, textvariable=variable, width=60), variable
        if isinstance(value, (int, float)):
            variable = tk.StringVar(value=repr(value))
            return ttk.Entry(parent, textvariable=variable, width=12), variable
        variable = tk.StringVar(value=str(value))
        if (section, key) in CHOICES or key in POSITIONS:
            return ttk.Combobox(parent, textvariable=variable, values=CHOICES.get((section, key)) or POSITIONS[key], width=20), variable
        if key == "font":
            return ttk.Combobox(parent, textvariable=variable, values=self.fonts, width=30), variable
        if key.endswith("color") or ASS_COLOUR.match(str(value)):
            return self.colour_field(parent, section, key, variable), variable
        if key == "file":
            frame = ttk.Frame(parent)
            ttk.Entry(frame, textvariable=variable, width=48).pack(side="left")
            ttk.Button(frame, text="Browse…", command=lambda: self.browse(variable)).pack(side="left", padx=4)
            return frame, variable
        return ttk.Entry(parent, textvariable=variable, width=60 if key == "text" else 24), variable

    def colour_field(self, parent, section: str, key: str, variable: tk.StringVar):
        frame = ttk.Frame(parent)
        swatch = tk.Label(frame, width=4, relief="solid", borderwidth=1)
        swatch.pack(side="left")
        entry = ttk.Combobox(frame, textvariable=variable, values=COLOUR_WORDS.get((section, key), []), width=14) if (section, key) in COLOUR_WORDS else ttk.Entry(frame, textvariable=variable, width=14)
        entry.pack(side="left", padx=4)

        def paint(*_):
            colour = ass_to_hex(variable.get())
            swatch.configure(background=colour or "#d9d9d9", text="" if colour else "?")

        def pick():
            current = variable.get().strip()
            chosen = colorchooser.askcolor(color=ass_to_hex(current) or "#ffffff", parent=self, title=label_for(key))[1]
            if chosen:
                match = ASS_COLOUR.match(current)
                variable.set(hex_to_ass(chosen, match.group(1).upper() if match else "00"))

        ttk.Button(frame, text="Choose…", command=pick).pack(side="left")
        variable.trace_add("write", paint)
        paint()
        return frame

    def browse(self, variable: tk.StringVar) -> None:
        chosen = filedialog.askopenfilename(parent=self, initialdir=str(HOME), filetypes=[("Images", "*.png *.jpg *.jpeg *.webp *.bmp"), ("All files", "*.*")])
        if chosen:
            from .localmedia import relative_to_home
            from pathlib import Path

            variable.set(relative_to_home(Path(chosen)).replace("\\", "/"))

    # Reading the form back
    def parse(self, section: str, key: str, raw, original):
        if isinstance(original, bool):
            return bool(raw)
        if isinstance(original, list):
            items = [item.strip() for item in str(raw).split(",") if item.strip()]
            return [int(item) if re.fullmatch(r"-?\d+", item) and all(isinstance(old, int) for old in original) and original else item for item in items]
        if isinstance(original, (int, float)):
            text = str(raw).strip()
            try:
                return int(text) if isinstance(original, int) and re.fullmatch(r"-?\d+", text) else float(text)
            except ValueError:
                raise ValueError(f"[{section}] {label_for(key)} must be a number") from None
        return str(raw)

    def form_text(self) -> str:
        """The file text with every changed field written in."""
        text = self.text.get("1.0", "end-1c")
        for (section, key), (variable, original) in self.fields.items():
            value = self.parse(section, key, variable.get(), original)
            if value != original:
                text = set_value(text, section, key, value)
        return text

    def tab_changed(self, _event=None) -> None:
        """Moving between the form and the file keeps both in step."""
        advanced = self.tabs.index("end") - 1
        now = self.tabs.index("current")
        try:
            if now == advanced and self.last_tab != advanced and self.values is not None:
                text = self.form_text()
                self.text.delete("1.0", "end")
                self.text.insert("1.0", text)
                self.reload(text)
            elif self.last_tab == advanced and now != advanced:
                self.reload(self.text.get("1.0", "end-1c"))
        except (ValueError, tomllib.TOMLDecodeError) as exc:
            messagebox.showerror("PolyKara", str(exc), parent=self)
        self.last_tab = now

    def reload(self, text: str) -> None:
        if self.values is None:
            return
        values = current_values(text)
        for (section, key), (variable, _) in self.fields.items():
            value = values.get(section, {}).get(key)
            if isinstance(value, bool):
                variable.set(value)
            elif isinstance(value, list):
                variable.set(", ".join(str(item) for item in value))
            elif isinstance(value, (int, float)):
                variable.set(repr(value))
            elif value is not None:
                variable.set(str(value))
            self.fields[(section, key)] = (variable, value)

    def save(self) -> None:
        try:
            on_file = self.tabs.index("current") == self.tabs.index("end") - 1
            content = self.text.get("1.0", "end-1c") if on_file or self.values is None else self.form_text()
            tomllib.loads(content)
        except ValueError as exc:  # TOMLDecodeError is a ValueError too
            messagebox.showerror("PolyKara", f"The settings were not saved:\n{exc}", parent=self)
            return
        CONFIG_FILE.parent.mkdir(parents=True, exist_ok=True)
        CONFIG_FILE.write_text(content, encoding="utf-8")
        self.text.delete("1.0", "end")
        self.text.insert("1.0", content)
        if self.values is None:
            self.values = current_values(content)
        else:
            self.reload(content)
        from .config import load_config

        load_config.cache_clear()
        messagebox.showinfo("PolyKara", "Settings saved. They apply to the next run.", parent=self)
