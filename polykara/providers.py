from __future__ import annotations

import json
import re
from difflib import SequenceMatcher
from html.parser import HTMLParser
from pathlib import Path
from urllib.parse import quote, urlencode
from urllib.request import Request, urlopen
from .config import EXTERNAL_LYRICS, load_config


def lyric_sources(row: dict[str, str]) -> list[str]:
    configured = row.get("lyric_sources", "").strip()
    if configured:
        return [item.strip().lower() for item in configured.split(",") if item.strip()]
    return [str(item).strip().lower() for item in load_config().get("lyrics", {}).get("sources", []) if str(item).strip()]


def fetch_json(url: str) -> dict:
    with urlopen(Request(url, headers={"User-Agent": "PolyKara/1.0 (+https://github.com/nano-rex/PolyKara)"}), timeout=15) as response:
        return json.loads(response.read().decode("utf-8"))


def fetch_html(url: str) -> str:
    with urlopen(Request(url, headers={"User-Agent": "PolyKara/1.0 (+https://github.com/nano-rex/PolyKara)"}), timeout=15) as response:
        return response.read().decode(response.headers.get_content_charset() or "utf-8", errors="replace")


class LyricsPageParser(HTMLParser):
    def __init__(self) -> None:
        super().__init__(convert_charrefs=True)
        self.depth = 0
        self.active: int | None = None
        self.current: list[str] = []
        self.candidates: list[str] = []

    def handle_starttag(self, tag: str, attrs: list[tuple[str, str | None]]) -> None:
        self.depth += 1
        values = dict(attrs)
        marker = values.get("data-lyrics-container") == "true" or "Lyrics__Container" in (values.get("class") or "") or "song_body-lyrics" in (values.get("class") or "")
        if marker and self.active is None:
            self.active, self.current = self.depth, []
        elif self.active is not None and tag in {"br", "div", "p", "section"}:
            self.current.append("\n")

    def handle_endtag(self, tag: str) -> None:
        if self.active is not None and self.depth == self.active:
            text = re.sub(r"\n{3,}", "\n\n", "".join(self.current)).strip()
            if text:
                self.candidates.append(text)
            self.active, self.current = None, []
        self.depth = max(0, self.depth - 1)

    def handle_data(self, data: str) -> None:
        if self.active is not None:
            self.current.append(data)


def extract_lyrics_page(html: str) -> str:
    parser = LyricsPageParser()
    parser.feed(html)
    return max(parser.candidates, key=len, default="")


def lyric_text(value: str) -> str:
    value = re.sub(r"\[\d{1,2}:\d{2}(?:[.:]\d{1,3})?\]|<\d{1,2}:\d{2}(?:[.:]\d{1,3})?>", " ", value or "")
    value = re.sub(r"[^\w\u3000-\u9fff\u3040-\u30ff\uac00-\ud7af]+", " ", value.casefold())
    return " ".join(value.split())


def external_lyrics_path(row: dict[str, str]) -> Path:
    return EXTERNAL_LYRICS / f"{row['id']}.lrc"


def external_webpage_path(row: dict[str, str]) -> Path:
    return EXTERNAL_LYRICS / f"{row['id']}.webpage.txt"


def fetch_external_lyrics(row: dict[str, str]) -> Path | None:
    output = external_lyrics_path(row)
    if output.exists():
        print(f"{row['id']}: using cached external lyrics {output.name}")
        return output
    title, artist, lrclib, plain = row["title"].strip(), row["artist"].strip(), {}, []
    for name in lyric_sources(row):
        try:
            if name == "lrclib":
                lrclib = fetch_json(f"https://lrclib.net/api/get?{urlencode({'artist_name': artist, 'track_name': title})}")
                if lrclib.get("plainLyrics"): plain.append(str(lrclib["plainLyrics"]))
            elif name == "lyrics.ovh":
                result = fetch_json(f"https://api.lyrics.ovh/v1/{quote(artist, safe='')}/{quote(title, safe='')}")
                if result.get("lyrics"): plain.append(str(result["lyrics"]))
            elif name == "webpage":
                for page in [item.strip() for item in row.get("lyric_pages", "").split("|") if item.strip()]:
                    extracted = extract_lyrics_page(fetch_html(page))
                    if extracted:
                        plain.append(extracted)
                        external_webpage_path(row).write_text(extracted + "\n", encoding="utf-8")
                        print(f"{row['id']}: extracted lyrics section from webpage")
        except Exception as exc:
            print(f"{row['id']}: lyric source {name} unavailable ({exc})")
    synced = str(lrclib.get("syncedLyrics") or "").strip()
    if not synced:
        print(f"{row['id']}: no synced lyrics found from configured external sources")
        return None
    if not (lyric_text(title) in lyric_text(str(lrclib.get("trackName") or title)) and lyric_text(artist) in lyric_text(str(lrclib.get("artistName") or artist))):
        print(f"SKIP {row['id']}: external lyrics metadata does not match manifest")
        return None
    comparisons = [SequenceMatcher(None, lyric_text(synced), lyric_text(item)).ratio() for item in plain if lyric_text(item)]
    if comparisons and max(comparisons) < 0.55:
        print(f"SKIP {row['id']}: external lyric sources disagree; review lyrics manually")
        return None
    output.write_text(synced + "\n", encoding="utf-8")
    (EXTERNAL_LYRICS / f"{row['id']}.json").write_text(json.dumps({"source": "lrclib", "validation_sources": lyric_sources(row), "validation_similarity": max(comparisons) if comparisons else None, "title": lrclib.get("trackName", title), "artist": lrclib.get("artistName", artist)}, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(f"{row['id']}: downloaded validated synced lyrics from LRCLIB")
    return output
