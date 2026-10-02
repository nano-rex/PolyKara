from __future__ import annotations

import functools
import importlib.util
import json
import shutil
import subprocess
from .config import ALIGN, AUDIO, load_config, run, total_memory_bytes
from .subtitle import word_json


def alignment_model(row: dict[str, str]) -> str:
    settings = load_config().get("alignment", {})
    configured = row.get("align_model", "").strip().lower() or str(settings.get("model", "auto")).lower()
    if configured != "auto":
        return configured
    total_gib = total_memory_bytes() / (1024 ** 3)
    reserve_gib = max(0, float(settings.get("reserve_memory_gib", 2)))
    budget_gib = max(0, total_gib - reserve_gib)
    large_min = float(settings.get("large_min_budget_gib", 12))
    medium_min = float(settings.get("medium_min_budget_gib", 8))
    small_min = float(settings.get("small_min_budget_gib", 4))
    base_min = float(settings.get("base_min_budget_gib", 2))
    model = "large-v3" if budget_gib >= large_min else "medium" if budget_gib >= medium_min else "small" if budget_gib >= small_min else "base" if budget_gib >= base_min else "tiny"
    print(f"{row['id']}: detected {total_gib:.1f} GiB RAM; reserving {reserve_gib:.1f} GiB for the system, using {budget_gib:.1f} GiB for alignment -> {model}")
    return model


def align_row(row: dict[str, str], dry_run: bool) -> None:
    ALIGN.mkdir(parents=True, exist_ok=True)
    language, audio = row.get("language", "").strip(), AUDIO / f"{row['id']}.flac"
    if not language:
        raise RuntimeError("language is missing")
    if not audio.exists():
        raise RuntimeError(f"normalized audio is missing: {audio}")
    settings = load_config().get("alignment", {})
    model = alignment_model(row)
    # Whisper takes bare language codes, and only large-v3 knows Cantonese.
    tag = language.lower().replace("_", "-")
    language = "yue" if tag in {"yue", "zh-hk", "zh-yue"} else tag.split("-", 1)[0]
    if language == "yue" and not model.startswith("large-v3"):
        language = "zh"
    device = row.get("device", "").strip().lower() or str(settings.get("device", "auto")).lower()
    compute = row.get("compute_type", "").strip() or str(settings.get("compute_type", "int8"))
    if device == "auto":
        try:
            import torch
            device = "cuda" if torch.cuda.is_available() else "cpu"
        except ImportError:
            device = "cpu"
    if shutil.which("whisperx") is not None:
        print(f"{row['id']}: using WhisperX model {model}")
        run(["whisperx", str(audio), "--model", model, "--language", language, "--device", device, "--compute_type", compute, "--output_format", "json", "--output_dir", str(ALIGN), "--return_char_alignments"], dry_run)
        return
    if dry_run:
        raise RuntimeError("WhisperX and faster-whisper are not installed")
    if importlib.util.find_spec("faster_whisper") is None:
        raise RuntimeError("WhisperX and faster-whisper are not installed; install requirements-align.txt")
    print(f"{row['id']}: using faster-whisper model {model} (word timestamps)")
    cpu_threads = int(settings.get("cpu_threads", 0) or 0)
    transcriber = _whisper_model(model, device, compute, int(settings.get("num_workers", 1) or 1), cpu_threads if device == "cpu" else 0)
    segments, _ = transcriber.transcribe(str(audio), language=language, word_timestamps=True, vad_filter=True)
    output = []
    for segment in segments:
        words = [{"start": word.start, "end": word.end, "word": word.word} for word in (segment.words or []) if word.start is not None and word.end is not None]
        output.append({"start": segment.start, "end": segment.end, "text": segment.text, "words": words})
    if not any(segment["words"] for segment in output):
        raise RuntimeError("faster-whisper returned no word timestamps")
    (ALIGN / f"{row['id']}.json").write_text(json.dumps({"segments": output}, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")


@functools.lru_cache(maxsize=1)
def _whisper_model(model: str, device: str, compute: str, num_workers: int, cpu_threads: int):
    """Keep the most recent model loaded so a batch of songs does not reload it per song."""
    from faster_whisper import WhisperModel

    options = {"cpu_threads": cpu_threads} if cpu_threads > 0 else {}
    return WhisperModel(model, device=device, compute_type=compute, num_workers=num_workers, **options)


def is_aligned(row: dict[str, str]) -> bool:
    """True when word timing exists and is not older than the normalized audio."""
    timing, audio = word_json(row), AUDIO / f"{row['id']}.flac"
    if timing is None:
        return False
    return not audio.exists() or timing.stat().st_mtime >= audio.stat().st_mtime


def align(rows: list[dict[str, str]], dry_run: bool, force: bool = False) -> list[str]:
    """Align every row, isolating failures per song. Returns the ids that were skipped."""
    skipped = []
    for row in rows:
        if not force and is_aligned(row):
            print(f"{row['id']}: word timing is up to date; skipping alignment")
            continue
        try:
            align_row(row, dry_run)
        except (RuntimeError, subprocess.CalledProcessError, OSError, ValueError) as exc:
            print(f"SKIP {row['id']}: alignment unavailable ({exc})")
            skipped.append(row["id"])
    if skipped:
        print("Skipped alignment: " + ", ".join(skipped))
    return skipped
