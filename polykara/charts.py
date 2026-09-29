from __future__ import annotations

import csv
import json
import re
from pathlib import Path
from urllib.request import Request, urlopen

from .config import MANIFEST, enabled_regions

REGIONS = {
    "my": ("Malaysia", "ms", "en.*"),
    "id": ("Indonesia", "id", "id,en.*"),
    "au": ("Australia", "en", "en.*"),
    "ca": ("Canada", "en", "en.*"),
    "sg": ("Singapore", "en", "en.*,zh.*,ms.*"),
    "tw": ("Taiwan", "zh", "zh.*,en.*"),
    "hk": ("Hong Kong", "yue", "yue,zh.*,en.*"),
    "jp": ("Japan", "ja", "ja,en.*"),
    "kr": ("South Korea", "ko", "ko,en.*"),
    "in": ("India", "hi", "hi,en.*"),
    "cn": ("China", "zh", "zh.*,en.*"),
    "us": ("USA", "en", "en.*"),
    "gb": ("UK", "en", "en.*"),
}
CHART_ENDPOINT = "https://charts.youtube.com/youtubei/v1/browse?alt=json&key=AIzaSyCzEW7JUJdSql0-2V4tHUb6laYm4iAE_dM"


def chart_rows(region: str) -> list[dict[str, str]]:
    if region not in REGIONS:
        raise ValueError(f"Unknown region {region}; choose from {', '.join(REGIONS)}")
    body = {
        "browseId": "FEmusic_analytics_charts_home",
        "context": {"capabilities": {}, "client": {"clientName": "WEB_MUSIC_ANALYTICS", "clientVersion": "0.2", "experimentIds": [], "experimentsToken": "", "gl": region.upper(), "hl": "en", "theme": "MUSIC"}, "request": {"internalExperimentFlags": []}},
        "query": f"chart_params_type=WEEK&perspective=CHART&flags=viral_video_chart&selected_chart=TRACKS&chart_params_id=weekly:0:0:{region}",
    }
    request = Request(CHART_ENDPOINT, data=json.dumps(body).encode(), headers={"Content-Type": "application/json", "User-Agent": "PolyKara/1.0"}, method="POST")
    with urlopen(request, timeout=20) as response:
        data = json.loads(response.read().decode("utf-8"))
    section = data["contents"]["sectionListRenderer"]["contents"][0]["musicAnalyticsSectionRenderer"]
    items = section["trackTypes"][0]["trackViews"][:10]
    result = []
    for rank, item in enumerate(items, 1):
        video_id = item.get("encryptedVideoId") or item.get("id")
        title = item.get("title") or item.get("name") or ""
        artist = ", ".join(artist.get("name", "") for artist in item.get("artists", []) if artist.get("name"))
        if video_id and title:
            result.append({"rank": str(rank), "title": title, "artist": artist or "Unknown Artist", "url": f"https://www.youtube.com/watch?v={video_id}"})
    return result


def slug(value: str) -> str:
    value = re.sub(r"[^A-Za-z0-9]+", "-", value.casefold()).strip("-")
    return value or "song"


def add_selected(regions: list[str], pick: bool = False) -> None:
    if pick and not sys_stdin_tty():
        raise SystemExit("Trending selection requires an interactive terminal; rerun with a terminal attached")
    candidates = []
    for region in regions:
        print(f"\n== {REGIONS[region][0]} ==")
        try:
            found = chart_rows(region)
        except Exception as exc:
            print(f"SKIP {region}: unable to load chart ({exc})")
            continue
        for item in found:
            index = len(candidates) + 1
            candidates.append((region, item))
            print(f"{index:02d}. #{item['rank']} {item['title']} — {item['artist']}\n    {item['url']}")
    if not candidates:
        print("No chart candidates found")
        return
    if pick:
        answer = input("\nSelect numbers to add (comma-separated), 'a' for all, or Enter to cancel: ").strip().lower()
        if not answer:
            print("No songs added")
            return
        selected = set(range(1, len(candidates) + 1)) if answer == "a" else {int(value) for value in answer.split(",") if value.strip().isdigit()}
    else:
        print("\nAutomatic mode: adding all listed chart songs")
        selected = set(range(1, len(candidates) + 1))
    selected = {value for value in selected if 1 <= value <= len(candidates)}
    if not selected:
        print("No valid selections")
        return
    with MANIFEST.open(newline="", encoding="utf-8-sig") as handle:
        reader = csv.DictReader(handle)
        fieldnames = reader.fieldnames or []
        existing = list(reader)
    existing_urls = {row.get("url", "").strip() for row in existing}
    existing_ids = {row.get("id", "").strip() for row in existing}
    added = 0
    for number in sorted(selected):
        region, item = candidates[number - 1]
        if item["url"] in existing_urls:
            print(f"SKIP existing URL: {item['title']}")
            continue
        base = slug(f"{item['artist']}-{item['title']}")
        sid, suffix = base, 2
        while sid in existing_ids:
            sid, suffix = f"{base}-{suffix}", suffix + 1
        language, subtitle_langs = REGIONS[region][1], REGIONS[region][2]
        row = {name: "" for name in fieldnames}
        row.update({"id": sid, "url": item["url"], "title": item["title"], "artist": item["artist"], "template": "solo", "language": language, "subtitle_langs": subtitle_langs, "prompt_ms": "1000", "align_model": "auto", "lyric_sources": "lrclib,lyrics.ovh,webpage"})
        existing.append(row)
        existing_urls.add(item["url"])
        existing_ids.add(sid)
        added += 1
        print(f"ADD {sid}: {item['title']} — {item['artist']}")
    if added:
        with MANIFEST.open("w", newline="", encoding="utf-8") as handle:
            writer = csv.DictWriter(handle, fieldnames=fieldnames)
            writer.writeheader()
            writer.writerows(existing)
        print(f"Added {added} song(s) to songs.csv. Run `python3 pipeline.py download` to fetch them.")


def sys_stdin_tty() -> bool:
    import sys
    return sys.stdin.isatty()
