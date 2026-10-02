from __future__ import annotations

import json
import re
from html.parser import HTMLParser
from pathlib import Path
from urllib.error import HTTPError
from urllib.parse import quote, urlencode
from urllib.request import Request, urlopen
from .config import EXTERNAL_LYRICS, RAW, load_config

USER_AGENT = "PolyKara/1.0 (+https://github.com/nano-rex/PolyKara)"
NETEASE_HEADERS = {"Referer": "https://music.163.com/"}
TIMED_LINE = re.compile(r"\s*\[\d+:\d{2}")
CREDIT_LINE = re.compile(r"\s*(?:\[[\d:.]+\])+\s*[^:：\[\]]{1,20}\s[:：]\s")
VOID_TAGS = {"area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "source", "track", "wbr"}
# Video-title decorations that are not part of the song name: (Official Video), [MV], 【官方】...
TITLE_NOISE = re.compile(r"\s*[(\[【（][^)\]】）]*(?:\b(?:official|video|mv|m/v|lyrics?|audio|visuali[sz]er|hd|4k|remaster(?:ed)?)\b|官方|完整版|高清|歌词|歌詞|字幕)[^)\]】）]*[)\]】）]", re.IGNORECASE)


def lyric_sources(row: dict[str, str]) -> list[str]:
    configured = row.get("lyric_sources", "").strip()
    if configured:
        return [item.strip().lower() for item in configured.split(",") if item.strip()]
    return [str(item).strip().lower() for item in load_config().get("lyrics", {}).get("sources", []) if str(item).strip()]


def fetch_json(url: str, headers: dict[str, str] | None = None):
    with urlopen(Request(url, headers={"User-Agent": USER_AGENT, **(headers or {})}), timeout=15) as response:
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


def closest(candidates: list[dict], duration: float | None) -> list[dict]:
    """Order records by how close their length is to the video's."""
    def distance(item: dict) -> float:
        try:
            return abs(float(item["duration"]) - duration) if duration else 0.0
        except (KeyError, TypeError, ValueError):
            return 1e9

    return sorted(candidates, key=distance)


def lrclib_lookup(title: str, artist: str, duration: float | None) -> dict:
    """Return the LRCLIB record that best fits the song, preferring synced lyrics of the same length."""
    candidates = []
    try:
        candidates.append(fetch_json(f"https://lrclib.net/api/get?{urlencode({'artist_name': artist, 'track_name': clean_title(title)})}"))
    except HTTPError as exc:
        if exc.code != 404:
            raise
    exact = [item for item in candidates if isinstance(item, dict) and item.get("syncedLyrics") and metadata_matches(title, artist, item)]
    if not exact or (duration and abs(float(exact[0].get("duration") or 0) - duration) > 2):
        # A different cut of the same song has shifted timestamps, so look for a closer one.
        found = fetch_json(f"https://lrclib.net/api/search?{urlencode({'track_name': clean_title(title), 'artist_name': artist})}")
        if isinstance(found, list):
            candidates.extend(found)
    matching = [item for item in candidates if isinstance(item, dict) and metadata_matches(title, artist, item)]
    synced = closest([item for item in matching if item.get("syncedLyrics")], duration)
    return synced[0] if synced else matching[0] if matching else {}


def netease_lookup(title: str, artist: str, duration: float | None) -> dict:
    """Return {"lyrics", "duration"} from NetEase Cloud Music's public search, or {} when nothing matches."""
    query = urlencode({"s": f"{clean_title(title)} {artist}".strip(), "type": 1, "limit": 10, "offset": 0})
    found = fetch_json(f"https://music.163.com/api/search/get?{query}", NETEASE_HEADERS)
    songs = (found.get("result") or {}).get("songs") or [] if isinstance(found, dict) else []
    records = [{"id": song.get("id"), "trackName": song.get("name"), "artistName": ", ".join(str(item.get("name") or "") for item in song.get("artists") or []), "duration": (song.get("duration") or 0) / 1000} for song in songs if isinstance(song, dict)]
    for record in closest([item for item in records if item["id"] and metadata_matches(title, artist, item)], duration)[:3]:
        data = fetch_json(f"https://music.163.com/api/song/lyric?{urlencode({'id': record['id'], 'lv': 1, 'kv': 1, 'tv': -1})}", NETEASE_HEADERS)
        text = str(((data.get("lrc") or {}).get("lyric") or "") if isinstance(data, dict) else "")
        # NetEase prepends credit lines such as "[00:00.000] 作词 : name"; they are not lyrics.
        lines = [line for line in text.splitlines() if not CREDIT_LINE.match(line)]
        if any(TIMED_LINE.match(line) for line in lines):
            return {"lyrics": "\n".join(lines), "duration": record["duration"]}
    return {}


def provider_files(row: dict[str, str]) -> list[tuple[str, str, Path]]:
    """Lyric files already fetched for a song as (source, kind, path); kind is "lrc" or "plain"."""
    sid, found = row["id"], []
    for name in ("lrclib", "netease"):
        path = EXTERNAL_LYRICS / f"{sid}.{name}.lrc"
        # Earlier versions saved the LRCLIB result as <id>.lrc.
        legacy = external_lyrics_path(row)
        if path.exists():
            found.append((name, "lrc", path))
        elif name == "lrclib" and legacy.exists():
            found.append((name, "lrc", legacy))
    for name in ("lrclib", "lyrics-ovh"):
        path = EXTERNAL_LYRICS / f"{sid}.{name}.txt"
        if path.exists() and not any(item[0] == name for item in found):
            found.append((name, "plain", path))
    found += [(path.name[len(sid) + 1:-4], "plain", path) for path in sorted(EXTERNAL_LYRICS.glob(f"{sid}.webpage*.txt"))]
    return found


def webpage_path(row: dict[str, str], index: int) -> Path:
    return external_webpage_path(row) if index == 0 else EXTERNAL_LYRICS / f"{row['id']}.webpage-{index + 1}.txt"


def warn_duration(row: dict[str, str], source: str, found: object, duration: float | None) -> None:
    try:
        difference = abs(float(found) - duration) if duration else 0.0
    except (TypeError, ValueError):
        return
    if difference > 5:
        print(f"WARN {row['id']}: {source} lyrics are for a recording {difference:.0f}s longer or shorter than this video; line timing may be offset")


def fetch_external_lyrics(row: dict[str, str]) -> bool:
    """Fetch lyrics from every configured source that has no file yet.

    Each source is saved to its own file so the files can be compared with
    each other. Returns False when a source could not be reached, so the
    caller knows the lookup is worth repeating.
    """
    sid, title, artist = row["id"], row["title"].strip(), row["artist"].strip()
    EXTERNAL_LYRICS.mkdir(parents=True, exist_ok=True)
    have = {name for name, _, _ in provider_files(row)}
    duration, complete = media_duration(row), True

    def save(name: str, suffix: str, text: str) -> None:
        (EXTERNAL_LYRICS / f"{sid}.{name}.{suffix}").write_text(text.strip() + "\n", encoding="utf-8")
        print(f"{sid}: saved {'synced' if suffix == 'lrc' else 'plain'} lyrics from {name}")

    for name in lyric_sources(row):
        try:
            if name == "lrclib" and "lrclib" not in have:
                record = lrclib_lookup(title, artist, duration)
                if str(record.get("syncedLyrics") or "").strip():
                    warn_duration(row, "LRCLIB", record.get("duration"), duration)
                    save("lrclib", "lrc", str(record["syncedLyrics"]))
                elif str(record.get("plainLyrics") or "").strip():
                    save("lrclib", "txt", str(record["plainLyrics"]))
                else:
                    print(f"{sid}: lrclib has no lyrics for this song")
            elif name == "netease" and "netease" not in have:
                record = netease_lookup(title, artist, duration)
                if record:
                    warn_duration(row, "NetEase", record.get("duration"), duration)
                    save("netease", "lrc", record["lyrics"])
                else:
                    print(f"{sid}: netease has no synced lyrics for this song")
            elif name == "lyrics.ovh" and "lyrics-ovh" not in have:
                result = fetch_json(f"https://api.lyrics.ovh/v1/{quote(artist, safe='')}/{quote(clean_title(title), safe='')}")
                if str(result.get("lyrics") or "").strip():
                    save("lyrics-ovh", "txt", str(result["lyrics"]))
                else:
                    print(f"{sid}: lyrics.ovh has no lyrics for this song")
            elif name == "webpage":
                for index, page in enumerate(item.strip() for item in row.get("lyric_pages", "").split("|") if item.strip()):
                    target = webpage_path(row, index)
                    if target.exists():
                        continue
                    extracted = extract_lyrics_page(fetch_html(page))
                    if extracted:
                        target.write_text(extracted + "\n", encoding="utf-8")
                        print(f"{sid}: extracted lyrics section from webpage")
            elif name not in {"lrclib", "netease", "lyrics.ovh", "webpage"}:
                print(f"{sid}: unknown lyric source {name!r}; expected lrclib, netease, lyrics.ovh, or webpage")
        except HTTPError as exc:
            if exc.code == 404:
                print(f"{sid}: {name} has no lyrics for this song")
            else:
                complete = False
                print(f"{sid}: lyric source {name} unavailable ({exc})")
        except Exception as exc:
            complete = False
            print(f"{sid}: lyric source {name} unavailable ({exc})")
    return complete
