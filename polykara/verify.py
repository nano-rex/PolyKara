"""Collect every lyric file a song has and check that they agree with each other."""
from __future__ import annotations

import functools
from dataclasses import dataclass, field
from difflib import SequenceMatcher
from pathlib import Path

from .config import load_config
from .manifest import SongError, resolve
from .providers import provider_files
from .subtitle import downloaded_subtitle, parse_lrc, parse_timed_subtitle, split_speaker, token_key, tokenise

# A file with fewer words than this cannot confirm or contradict anything.
MIN_TOKENS = 10


@dataclass
class LyricFile:
    name: str
    label: str
    kind: str  # "lrc", "subtitle", or "plain" (untimed text)
    path: Path
    entries: list = field(default_factory=list)
    tokens: tuple[str, ...] = ()
    score: float = 1.0
    agrees: bool = True


@dataclass
class LyricCheck:
    files: list[LyricFile]
    primary: LyricFile | None
    min_sources: int

    @property
    def agreeing(self) -> int:
        return sum(1 for item in self.files if item.agrees)

    @property
    def verified(self) -> bool:
        return self.primary is not None and self.agreeing >= self.min_sources

    def problem(self) -> str:
        if self.primary is None:
            return "no lyric file"
        if len(self.files) < self.min_sources:
            return f"only {len(self.files)} lyric file(s), {self.min_sources} needed for an accuracy check"
        return f"only {self.agreeing} of {len(self.files)} lyric files agree, {self.min_sources} needed"

    def as_dict(self) -> dict:
        return {
            "verified": self.verified, "min_sources": self.min_sources, "agreeing": self.agreeing,
            "primary": self.primary.name if self.primary else None,
            "files": [{"source": item.name, "file": item.path.name, "words": len(item.tokens), "agreement": round(item.score, 3), "agrees": item.agrees} for item in self.files],
        }


def comparison_tokens(text: str) -> tuple[str, ...]:
    """Words (or CJK characters) of a lyric text in a form that ignores case, punctuation, and script variant."""
    try:
        # Optional: lets Traditional and Simplified Chinese versions of the same lyrics agree.
        import zhconv
        text = zhconv.convert(text, "zh-hans")
    except ImportError:
        pass
    return tuple(key for key in (token_key(token) for token in tokenise(text)) if key)


def agreement(first: tuple[str, ...], second: tuple[str, ...]) -> float:
    """Share of the shorter text found, in order, in the longer one.

    Measuring against the shorter text keeps a file that omits repeated
    choruses from counting as a disagreement.
    """
    if not first or not second:
        return 0.0
    matched = sum(block.size for block in SequenceMatcher(None, first, second, autojunk=False).get_matching_blocks())
    return matched / min(len(first), len(second))


def load_file(name: str, label: str, kind: str, path: Path) -> LyricFile | None:
    """Parse one lyric file; None when it has too little text to be usable."""
    try:
        content = path.read_text(encoding="utf-8-sig")
    except (OSError, UnicodeDecodeError):
        return None
    entries = parse_lrc(content) if kind == "lrc" else parse_timed_subtitle(content) if kind == "subtitle" else []
    text = "\n".join(split_speaker(line)[1] for _, _, line, _ in entries) if kind != "plain" else content
    tokens = comparison_tokens(text)
    if len(tokens) < MIN_TOKENS or (kind != "plain" and not entries):
        return None
    return LyricFile(name, label, kind, path, entries, tokens)


def candidates(row: dict[str, str]) -> list[tuple[str, str, str, Path]]:
    """Every lyric file of a song as (name, label, kind, path), most trusted first."""
    found: list[tuple[str, str, str, Path]] = []
    raw = row.get("lyrics_file", "")
    if raw:
        path = resolve(raw)
        if not path.exists():
            raise SongError(f"lyrics_file not found: {path}")
        kind = {".lrc": "lrc", ".srt": "subtitle", ".vtt": "subtitle", ".txt": "plain"}.get(path.suffix.lower())
        if kind is None:
            raise SongError(f"lyrics_file must be .lrc, .srt, .vtt, or .txt: {path}")
        found.append(("manifest", "manifest lyrics_file", kind, path))
    manual = downloaded_subtitle(row)
    if manual is not None:
        found.append(("subtitle", "downloaded subtitle", "subtitle", manual))
    external = provider_files(row)
    found += [(name, f"{name} synced lyrics", kind, path) for name, kind, path in external if kind == "lrc"]
    automatic = downloaded_subtitle(row, auto=True)
    if automatic is not None and load_config()["lyrics"].get("auto_captions", True):
        found.append(("auto-captions", "automatic captions", "subtitle", automatic))
    found += [(name, f"{name} lyrics", kind, path) for name, kind, path in external if kind == "plain"]
    return found


def check_files(files: list[LyricFile], min_sources: int, threshold: float, fixed_primary: bool = False) -> LyricCheck:
    """Choose the lyric file to render from and mark which other files agree with it."""
    if not files:
        return LyricCheck([], None, min_sources)
    scores = {(a, b): agreement(files[a].tokens, files[b].tokens) for a in range(len(files)) for b in range(a + 1, len(files))}

    def supporters(index: int) -> int:
        return 1 + sum(1 for (a, b), score in scores.items() if index in (a, b) and score >= threshold)

    timed = [index for index, item in enumerate(files) if item.kind != "plain"]
    if fixed_primary or not timed:
        chosen = 0
    else:
        # The most trusted timed file that enough other files confirm, else the best-confirmed one.
        chosen = next((index for index in timed if supporters(index) >= min_sources), max(timed, key=lambda index: (supporters(index), -index)))
    for index, item in enumerate(files):
        item.score = 1.0 if index == chosen else scores[(min(index, chosen), max(index, chosen))]
        item.agrees = item.score >= threshold
    return LyricCheck(files, files[chosen], min_sources)


@functools.lru_cache(maxsize=64)
def _check(found: tuple, stamps: tuple, min_sources: int, threshold: float, fixed_primary: bool) -> LyricCheck:
    files = [item for item in (load_file(*entry) for entry in found) if item is not None]
    # A manifest file that turned out unusable must not hand its priority to the next file.
    fixed = fixed_primary and bool(files) and files[0].name == "manifest"
    return check_files(files, min_sources, threshold, fixed)


def check_lyrics(row: dict[str, str]) -> LyricCheck:
    """Accuracy check for one song. Results are cached until a lyric file changes."""
    settings = load_config()["lyrics"]
    found = tuple(candidates(row))
    stamps = tuple(path.stat().st_mtime_ns for _, _, _, path in found)
    return _check(found, stamps, max(1, int(settings.get("min_sources", 3))), float(settings.get("agreement_threshold", 0.6)), bool(row.get("lyrics_file", "")))
