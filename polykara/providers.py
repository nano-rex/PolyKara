from __future__ import annotations

import json
import re
from difflib import SequenceMatcher
from html.parser import HTMLParser
from pathlib import Path
from urllib.error import HTTPError
from urllib.parse import quote, urlencode
from urllib.request import Request, urlopen
from .config import EXTERNAL_LYRICS, RAW, load_config

USER_AGENT = "PolyKara/1.0 (+https://github.com/nano-rex/PolyKara)"
VOID_TAGS = {"area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "source", "track", "wbr"}
# Video-title decorations that are not part of the song name: (Official Video), [MV], 【官方】...
TITLE_NOISE = re.compile(r"\s*[(\[【（][^)\]】）]*(?:\b(?:official|video|mv|m/v|lyrics?|audio|visuali[sz]er|hd|4k|remaster(?:ed)?)\b|官方|完整版|高清|歌词|歌詞|字幕)[^)\]】）]*[)\]】）]", re.IGNORECASE)


def lyric_sources(row: dict[str, str]) -> list[str]:
    configured = row.get("lyric_sources", "").strip()
    if configured:
        return [item.strip().lower() for item in configured.split(",") if item.strip()]
    return [str(item).strip().lower() for item in load_config().get("lyrics", {}).get("sources", []) if str(item).strip()]


def fetch_json(url: str):
    with urlopen(Request(url, headers={"User-Agent": USER_AGENT}), timeout=15) as response:
        return json.loads(response.read().decode("utf-8"))


def fetch_html(url: str) -> str:
    with urlopen(Request(url, headers={"User-Agent": USER_AGENT}), timeout=15) as response:
        return response.read().decode(response.headers.get_content_charset() or "utf-8", errors="replace")


class LyricsPageParser(HTMLParser):
    def __init__(self) -> None:
        super().__init__(convert_charrefs=True)
        self.depth = 0
        self.active: int | None = None
        self.current: list[str] = []
        self.candidates: list[str] = []

    def handle_starttag(self, tag: str, attrs: list[tuple[str, str | None]]) -> None:
        if self.active is not None and tag in {"br", "div", "p", "section"}:
            self.current.append("\n")
        # Void elements never get an end tag; counting them would unbalance the depth.
        if tag in VOID_TAGS:
            return
        self.depth += 1
        values = dict(attrs)
        marker = values.get("data-lyrics-container") == "true" or "Lyrics__Container" in (values.get("class") or "") or "song_body-lyrics" in (values.get("class") or "")
        if marker and self.active is None:
            self.active, self.current = self.depth, []

    def handle_startendtag(self, tag: str, attrs: list[tuple[str, str | None]]) -> None:
        if self.active is not None and tag == "br":
            self.current.append("\n")

    def handle_endtag(self, tag: str) -> None:
        if tag in VOID_TAGS:
            return
        if self.active is not None and self.depth == self.active:
            self.finish()
        self.depth = max(0, self.depth - 1)

    def finish(self) -> None:
        text = re.sub(r"\n{3,}", "\n\n", "".join(self.current)).strip()
        if text:
            self.candidates.append(text)
        self.active, self.current = None, []

    def handle_data(self, data: str) -> None:
        if self.active is not None:
            self.current.append(data)


def extract_lyrics_page(html: str) -> str:
    parser = LyricsPageParser()
    parser.feed(html)
    parser.close()
    if parser.active is not None:
        parser.finish()
    # Lyric pages split one song over several containers; keep them all, in page order.
    return "\n".join(parser.candidates)


def lyric_text(value: str) -> str:
    value = re.sub(r"\[\d{1,2}:\d{2}(?:[.:]\d{1,3})?\]|<\d{1,2}:\d{2}(?:[.:]\d{1,3})?>", " ", value or "")
    value = re.sub(r"[^\w\u3000-\u9fff\u3040-\u30ff\uac00-\ud7af]+", " ", value.casefold())
    return " ".join(value.split())


def external_lyrics_path(row: dict[str, str]) -> Path:
    return EXTERNAL_LYRICS / f"{row['id']}.lrc"


def external_webpage_path(row: dict[str, str]) -> Path:
    return EXTERNAL_LYRICS / f"{row['id']}.webpage.txt"


def clean_title(title: str) -> str:
    return TITLE_NOISE.sub("", title).strip() or title.strip()


def media_duration(row: dict[str, str]) -> float | None:
    """Length of the downloaded video in seconds, from yt-dlp's info JSON."""
    try:
        return float(json.loads((RAW / f"{row['id']}.info.json").read_text(encoding="utf-8"))["duration"])
    except (OSError, ValueError, KeyError, TypeError):
        return None


def metadata_matches(title: str, artist: str, candidate: dict) -> bool:
    found_title, found_artist = lyric_text(str(candidate.get("trackName") or "")), lyric_text(str(candidate.get("artistName") or ""))
    wanted_title = lyric_text(clean_title(title))
    if not wanted_title or not found_title or not (wanted_title in found_title or found_title in wanted_title):
        return False
    # Chart rows list several artists; one of them appearing on either side is enough.
    names = [lyric_text(name) for name in re.split(r",|&|\bfeat\.?|\bft\.?|\bx\b", artist, flags=re.IGNORECASE)]
    return not found_artist or any(name and (name in found_artist or found_artist in name) for name in names)


def lrclib_lookup(title: str, artist: str, duration: float | None) -> dict:
    """Return the LRCLIB record that best fits the song, preferring synced lyrics of the same length."""
    def distance(item: dict) -> float:
        try:
            return abs(float(item["duration"]) - duration) if duration else 0.0
        except (KeyError, TypeError, ValueError):
            return 1e9

    candidates = []
    try:
        candidates.append(fetch_json(f"https://lrclib.net/api/get?{urlencode({'artist_name': artist, 'track_name': clean_title(title)})}"))
    except HTTPError as exc:
        if exc.code != 404:
            raise
    exact = [item for item in candidates if isinstance(item, dict) and item.get("syncedLyrics") and metadata_matches(title, artist, item)]
    if not exact or distance(exact[0]) > 2:
        # A different cut of the same song has shifted timestamps, so look for a closer one.
        found = fetch_json(f"https://lrclib.net/api/search?{urlencode({'track_name': clean_title(title), 'artist_name': artist})}")
        if isinstance(found, list):
            candidates.extend(found)
    matching = [item for item in candidates if isinstance(item, dict) and metadata_matches(title, artist, item)]
    synced = sorted((item for item in matching if item.get("syncedLyrics")), key=distance)
    return synced[0] if synced else matching[0] if matching else {}


def fetch_external_lyrics(row: dict[str, str]) -> Path | None:
    output = external_lyrics_path(row)
    if output.exists():
        print(f"{row['id']}: using cached external lyrics {output.name}")
        return output
    EXTERNAL_LYRICS.mkdir(parents=True, exist_ok=True)
    title, artist, lrclib, plain = row["title"].strip(), row["artist"].strip(), {}, []
    duration = media_duration(row)
    for name in lyric_sources(row):
        try:
            if name == "lrclib":
                lrclib = lrclib_lookup(title, artist, duration)
                if lrclib.get("plainLyrics"): plain.append(str(lrclib["plainLyrics"]))
            elif name == "lyrics.ovh":
                result = fetch_json(f"https://api.lyrics.ovh/v1/{quote(artist, safe='')}/{quote(clean_title(title), safe='')}")
                if result.get("lyrics"): plain.append(str(result["lyrics"]))
            elif name == "webpage":
                for page in [item.strip() for item in row.get("lyric_pages", "").split("|") if item.strip()]:
                    extracted = extract_lyrics_page(fetch_html(page))
                    if extracted:
                        plain.append(extracted)
                        external_webpage_path(row).write_text(extracted + "\n", encoding="utf-8")
                        print(f"{row['id']}: extracted lyrics section from webpage")
            else:
                print(f"{row['id']}: unknown lyric source {name!r}; expected lrclib, lyrics.ovh, or webpage")
        except Exception as exc:
            print(f"{row['id']}: lyric source {name} unavailable ({exc})")
    synced = str(lrclib.get("syncedLyrics") or "").strip()
    if not synced:
        print(f"{row['id']}: no synced lyrics found from configured external sources")
        return None
    comparisons = [SequenceMatcher(None, lyric_text(synced), lyric_text(item)).ratio() for item in plain if lyric_text(item)]
    if comparisons and max(comparisons) < 0.55:
        print(f"SKIP {row['id']}: external lyric sources disagree; review lyrics manually")
        return None
    try:
        difference = abs(float(lrclib["duration"]) - duration) if duration else None
    except (KeyError, TypeError, ValueError):
        difference = None
    if difference is not None and difference > 5:
        print(f"WARN {row['id']}: LRCLIB lyrics are for a recording {difference:.0f}s longer or shorter than this video; line timing may be offset")
    output.write_text(synced + "\n", encoding="utf-8")
    (EXTERNAL_LYRICS / f"{row['id']}.json").write_text(json.dumps({"source": "lrclib", "validation_sources": lyric_sources(row), "validation_similarity": max(comparisons) if comparisons else None, "title": lrclib.get("trackName", title), "artist": lrclib.get("artistName", artist), "duration_difference_s": difference}, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(f"{row['id']}: downloaded validated synced lyrics from LRCLIB")
    return output
