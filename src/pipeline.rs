//! The command-line pipeline: download, normalize, align, lyrics, render.
//!
//! One step runs for every song in turn. A song that fails (or that cannot be
//! made yet, e.g. no lyrics) is set aside with its reason and the rest of the
//! batch carries on, which is what the window shows in its log.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use crate::align;
use crate::ass::ass;
use crate::charts;
use crate::config::{self, load_config, paths};
use crate::csv::Row;
use crate::json::Value;
use crate::localmedia::{bring_in, is_local, probe};
use crate::manifest::{self, find_source, get, source};
use crate::providers;
use crate::romanize;
use crate::subtitle::{self, Entry};
use crate::verify::{self, Kind, LyricCheck};

pub const NO_LYRICS: &str = "no lyrics available from any source; add a lyrics_file or lyric_pages in songs.csv";
const KARAOKE_TAG: &str = "{\\k";

static FAILED: Mutex<Vec<(String, String)>> = Mutex::new(Vec::new());
static UNVERIFIED: Mutex<Vec<(String, String)>> = Mutex::new(Vec::new());
static SKIPPED: Mutex<Vec<(String, String)>> = Mutex::new(Vec::new());

/// Why a song stopped: it cannot be made yet (skip), or a step failed.
pub enum Failure {
    Skip(String),
    Fail(String),
}

pub type Step = Result<(), Failure>;

fn put(list: &Mutex<Vec<(String, String)>>, sid: &str, reason: &str) {
    let mut list = list.lock().unwrap_or_else(|error| error.into_inner());
    list.retain(|(id, _)| id != sid);
    list.push((sid.to_string(), reason.to_string()));
}

fn drop_entry(list: &Mutex<Vec<(String, String)>>, sid: &str) {
    let mut list = list.lock().unwrap_or_else(|error| error.into_inner());
    list.retain(|(id, _)| id != sid);
}

fn has(list: &Mutex<Vec<(String, String)>>, sid: &str) -> bool {
    list.lock().unwrap_or_else(|error| error.into_inner()).iter().any(|(id, _)| id == sid)
}

fn each(step: &str, songs: &[Row], action: impl Fn(&Row) -> Step) {
    for row in songs {
        let sid = get(row, "id").to_string();
        if has(&FAILED, &sid) || has(&SKIPPED, &sid) {
            continue;
        }
        // The window reads these lines to show which song is being worked on.
        println!(">> {step} {sid}");
        match action(row) {
            Ok(()) => {}
            Err(Failure::Skip(reason)) => {
                put(&SKIPPED, &sid, &reason);
                println!("SKIP {sid}: {reason}; continuing with the next song");
            }
            Err(Failure::Fail(reason)) => {
                put(&FAILED, &sid, &format!("{step}: {reason}"));
                println!("SKIP {sid}: {step} failed ({reason}); continuing with the next song");
            }
        }
    }
}

fn fail(reason: impl Into<String>) -> Failure {
    Failure::Fail(reason.into())
}

fn auto_path(sid: &str) -> PathBuf {
    paths().ass.join(format!("{sid}.auto.ass"))
}

fn edited_path(sid: &str) -> PathBuf {
    paths().ass.join(format!("{sid}.edited.ass"))
}

fn output_path(sid: &str) -> PathBuf {
    paths().output.join(format!("{sid}.mp4"))
}

fn modified(path: &Path) -> std::time::SystemTime {
    std::fs::metadata(path).and_then(|meta| meta.modified()).unwrap_or(std::time::UNIX_EPOCH)
}

fn newer(path: &Path, than: &Path) -> bool {
    path.exists() && than.exists() && modified(path) > modified(than)
}

fn has_karaoke(path: &Path) -> bool {
    path.exists() && std::fs::read_to_string(path).map(|text| text.contains(KARAOKE_TAG)).unwrap_or(false)
}

fn require_lyrics(row: &Row) -> Step {
    match verify::check_lyrics(row) {
        Ok(report) if report.primary().is_some() => Ok(()),
        _ => Err(Failure::Skip(NO_LYRICS.into())),
    }
}

// ---------------------------------------------------------------- download

fn download_subtitles(row: &Row, dry_run: bool) -> Result<(), String> {
    let sid = get(row, "id");
    let marker = paths().subtitles.join(format!("{sid}.checked"));
    if marker.exists() || subtitle::downloaded_subtitle(row, false).is_some() || subtitle::downloaded_subtitle(row, true).is_some() {
        return Ok(());
    }
    let language = get(row, "language").to_lowercase().replace('_', "-");
    let language = language.split('-').next().unwrap_or("");
    // "all" would also request every auto-translated track and trips YouTube rate limits.
    let configured = get(row, "subtitle_langs");
    let langs = if !configured.is_empty() {
        configured.to_string()
    } else if !language.is_empty() {
        format!("{language}.*")
    } else {
        "all".into()
    };
    let folder = paths().subtitles.clone();
    let mut base: Vec<String> = ["--no-playlist", "--restrict-filenames", "--skip-download", "--ignore-errors", "--retries", "3", "--fragment-retries", "3", "--sleep-requests", "1", "--sub-format", "vtt", "--no-overwrites", "-P"]
        .iter()
        .map(|item| item.to_string())
        .collect();
    base.push(format!("subtitle:{}", folder.display()));
    let mut args = base.clone();
    args.extend(["--write-subs".to_string(), "--sub-langs".into(), langs, "-o".into(), format!("subtitle:{sid}.%(ext)s"), get(row, "url").into()]);
    config::run("yt-dlp", &args, dry_run, None)?;
    let auto_on = load_config().get("lyrics").bool_or("auto_captions", true);
    if !language.is_empty() && auto_on && (dry_run || subtitle::downloaded_subtitle(row, true).is_none()) {
        let mut args = base;
        args.extend(["--write-auto-subs".to_string(), "--sub-langs".into(), format!("{language}.*"), "-o".into(), format!("subtitle:{sid}.auto.%(ext)s"), get(row, "url").into()]);
        config::run("yt-dlp", &args, dry_run, None)?;
    }
    if !dry_run {
        if let Some(parent) = marker.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        std::fs::write(&marker, "").map_err(|error| error.to_string())?;
    }
    Ok(())
}

/// Why a media file cannot be used, or None when it has video, audio and a duration.
fn media_problem(path: &Path, audio_only: bool) -> Option<String> {
    if config::find_tool("ffprobe").is_none() {
        // Without a verdict the file is kept rather than downloaded again.
        return None;
    }
    let info = probe(path);
    if info.duration <= 0.0 {
        return Some("it cannot be read".into());
    }
    // A video-only file is what an interrupted or failed merge leaves behind.
    if !info.audio {
        return Some("it has no audio track".into());
    }
    if !info.video && !audio_only {
        return Some("it has no video track".into());
    }
    None
}

/// Records that every lyric source was tried, so a miss is not requested again each run.
fn lyrics_marker(sid: &str) -> PathBuf {
    paths().lyrics.join(format!("{sid}.checked"))
}

fn download_lyrics(row: &Row, dry_run: bool, force: bool) -> Result<(), String> {
    let sid = get(row, "id");
    let report = match verify::check_lyrics(row) {
        Ok(report) => report,
        Err(problem) => {
            println!("WARN {sid}: {problem}");
            return Ok(());
        }
    };
    if report.files.len() >= report.min_sources && !force {
        println!("{sid}: {} usable lyric files found; skipping lyric download", report.files.len());
        report_check(sid, &report, dry_run, false);
        return Ok(());
    }
    if lyrics_marker(sid).exists() && !force {
        println!("{sid}: {} of {} lyric files; every source was already tried (download --reprocess tries again)", report.files.len(), report.min_sources);
        report_check(sid, &report, dry_run, false);
        return Ok(());
    }
    println!("{sid}: {} of {} lyric files; looking for more", report.files.len(), report.min_sources);
    // Subtitles and external lyrics are optional inputs: report problems and carry on.
    let mut complete = true;
    if !dry_run {
        let metadata = paths().metadata.join(format!("{sid}.json"));
        if let Some(parent) = metadata.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let data = Value::Object(row.iter().map(|(key, value)| (key.clone(), Value::from(value))).collect());
        let _ = std::fs::write(metadata, data.dump() + "\n");
        complete = providers::fetch_external_lyrics(row);
    }
    // The lyric providers are asked first; YouTube is only contacted when files are still missing.
    let still_missing = verify::check_lyrics(row).map(|check| check.files.len() < check.min_sources).unwrap_or(true);
    if !is_local(get(row, "url")) && (dry_run || force || still_missing) {
        let attempt = (|| -> Result<(), String> {
            config::require_current_ytdlp()?;
            if force && !dry_run {
                let _ = std::fs::remove_file(paths().subtitles.join(format!("{sid}.checked")));
            }
            download_subtitles(row, dry_run)
        })();
        if let Err(problem) = attempt {
            complete = false;
            println!("WARN {sid}: subtitle download failed ({problem}); continuing with other lyric sources");
        }
    }
    if dry_run {
        return Ok(());
    }
    if complete {
        let marker = lyrics_marker(sid);
        if let Some(parent) = marker.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = std::fs::write(marker, "");
    }
    let report = match verify::check_lyrics(row) {
        Ok(report) => report,
        Err(problem) => {
            println!("WARN {sid}: {problem}");
            return Ok(());
        }
    };
    report_check(sid, &report, dry_run, false);
    if report.files.len() < report.min_sources {
        println!("{sid}: to reach {} lyric files, add a lyrics_file or lyric_pages (URLs separated by |) in songs.csv", report.min_sources);
    }
    Ok(())
}

fn download_song(row: &Row, dry_run: bool, force: bool) -> Step {
    let sid = get(row, "id");
    if is_local(get(row, "url")) {
        // A file on this computer is placed first, so the lyrics search knows its length.
        let found = find_source(sid);
        match found {
            Some(media) if !force && media_problem(&media, true).is_none() => println!("{sid}: media found and playable; skipping"),
            _ if dry_run => println!("{sid}: would copy {} into {}", get(row, "url"), paths().raw.display()),
            _ => {
                let media = bring_in(row, &paths().raw).map_err(fail)?;
                if let Some(problem) = media_problem(&media, true) {
                    let _ = std::fs::remove_file(&media);
                    return Err(fail(format!("{} is not usable ({problem})", get(row, "url"))));
                }
                println!("{sid}: using local file {}", get(row, "url"));
            }
        }
        download_lyrics(row, dry_run, force).map_err(fail)?;
        if !dry_run && verify::check_lyrics(row).map(|check| check.primary().is_none()).unwrap_or(true) {
            return Err(Failure::Skip(NO_LYRICS.into()));
        }
        return Ok(());
    }
    // Lyrics come first: a song without any is set aside before its video is downloaded.
    download_lyrics(row, dry_run, force).map_err(fail)?;
    if !dry_run && verify::check_lyrics(row).map(|check| check.primary().is_none()).unwrap_or(true) {
        return Err(Failure::Skip(NO_LYRICS.into()));
    }
    let mut media = find_source(sid);
    if let Some(problem) = media.as_deref().and_then(|path| media_problem(path, false)) {
        if let Some(path) = &media {
            println!("{sid}: {} is not usable ({problem}); removing it and downloading again", path.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default());
        }
        if !dry_run {
            if let Some(path) = &media {
                let _ = std::fs::remove_file(path);
            }
        }
        media = None;
    }
    if media.is_some() {
        println!("{sid}: media found and playable; skipping media download");
        return Ok(());
    }
    config::require_current_ytdlp().map_err(fail)?;
    let target = paths().raw.join(format!("{sid}.%(ext)s"));
    let args: Vec<String> = [
        "--no-playlist",
        "--restrict-filenames",
        "--format",
        "bv*+ba/b",
        "--merge-output-format",
        "mp4",
        "--write-info-json",
        "--retries",
        "3",
        "--fragment-retries",
        "3",
        "--sleep-requests",
        "1",
        "--no-overwrites",
        "-o",
    ]
    .iter()
    .map(|item| item.to_string())
    .chain(std::iter::once(target.display().to_string()))
    .chain(std::iter::once(get(row, "url").to_string()))
    .collect();
    config::run("yt-dlp", &args, dry_run, None).map_err(fail)?;
    if !dry_run {
        let media = find_source(sid).ok_or_else(|| fail("yt-dlp finished without producing a merged media file; check that ffmpeg is installed"))?;
        if let Some(problem) = media_problem(&media, false) {
            return Err(fail(format!("downloaded {} is not usable ({problem}); update yt-dlp and install its JavaScript runtime (deno), then rerun", media.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default())));
        }
    }
    Ok(())
}

pub fn download(selected: &[String], dry_run: bool, trending: bool, regions: &str, pick: bool, force: bool) -> Result<(), String> {
    if trending {
        let selected: Vec<String> = if regions.trim().is_empty() {
            config::enabled_regions()
        } else {
            regions.split(',').map(|item| item.trim().to_lowercase()).filter(|item| !item.is_empty()).collect()
        };
        return charts::add_selected(&selected, pick);
    }
    let songs = manifest::rows(selected)?;
    for folder in [paths().raw.clone(), paths().subtitles.clone(), paths().metadata.clone(), paths().lyrics.clone()] {
        std::fs::create_dir_all(folder).map_err(|error| error.to_string())?;
    }
    each("download", &songs, |row| download_song(row, dry_run, force));
    Ok(())
}

// --------------------------------------------------------------- normalize

fn normalize_song(row: &Row, dry_run: bool, force: bool) -> Step {
    let sid = get(row, "id");
    require_lyrics(row)?;
    let media = source(sid).map_err(fail)?;
    let audio = paths().audio.join(format!("{sid}.flac"));
    if let Some(problem) = media_problem(&media, is_local(get(row, "url"))) {
        let name = media.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default();
        return Err(fail(format!("{name} is not usable ({problem}); run download again to replace it")));
    }
    if !force && audio.exists() && modified(&audio) >= modified(&media) {
        println!("{sid}: normalized audio is up to date");
        return Ok(());
    }
    std::fs::create_dir_all(paths().audio.clone()).map_err(|error| fail(error.to_string()))?;
    // Encode to a temporary name so an interrupted run never leaves a truncated file behind.
    let partial = paths().audio.join(format!("{sid}.partial.flac"));
    let args: Vec<String> = [
        "-hide_banner",
        "-loglevel",
        "warning",
        "-y",
        "-i",
    ]
    .iter()
    .map(|item| item.to_string())
    .chain(std::iter::once(media.display().to_string()))
    .chain(["-vn", "-ac", "2", "-ar", "48000", "-c:a", "flac"].iter().map(|item| item.to_string()))
    .chain(std::iter::once(partial.display().to_string()))
    .collect();
    let result = config::run("ffmpeg", &args, dry_run, None);
    let outcome = match result {
        Ok(()) if !dry_run => std::fs::rename(&partial, &audio).map_err(|error| fail(error.to_string())),
        Ok(()) => Ok(()),
        Err(problem) => Err(fail(problem)),
    };
    let _ = std::fs::remove_file(&partial);
    outcome
}

fn prepare_audio(row: &Row, dry_run: bool, realign: bool) -> Step {
    if !realign && align::is_aligned(row) {
        println!("{}: word timing already exists; audio not needed", get(row, "id"));
        return Ok(());
    }
    normalize_song(row, dry_run, false)
}

pub fn normalize(selected: &[String], dry_run: bool, force: bool) -> Result<(), String> {
    let songs = manifest::rows(selected)?;
    each("normalize", &songs, |row| normalize_song(row, dry_run, force));
    Ok(())
}

// ------------------------------------------------------------------ lyrics

fn lyric_source(row: &Row) -> Option<verify::LyricFile> {
    verify::check_lyrics(row).ok().and_then(|report| report.primary().cloned())
}

fn save_check(sid: &str, report: &LyricCheck, audio: Option<f64>) {
    let folder = paths().lyrics.clone();
    let _ = std::fs::create_dir_all(&folder);
    let mut data = match report.to_json() {
        Value::Object(pairs) => pairs,
        _ => Vec::new(),
    };
    data.push(("audio_agreement".into(), audio.map(|value| Value::Number((value * 1000.0).round() / 1000.0)).unwrap_or(Value::Null)));
    let _ = std::fs::write(folder.join(format!("{sid}.check.json")), Value::Object(data).dump() + "\n");
}

fn report_check(sid: &str, report: &LyricCheck, dry_run: bool, enforce: bool) {
    let primary = match report.primary() {
        Some(primary) => primary,
        None => {
            println!("WARN {sid}: lyric check: no usable lyric file");
            return;
        }
    };
    println!("{sid}: lyric check: {} usable file(s), rendering from {} ({})", report.files.len(), primary.name, primary.path.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default());
    for item in &report.files {
        if !report.is_primary(item) {
            println!("{sid}:   {} ({}): {:.0}% agreement{}", item.name, item.path.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default(), item.score * 100.0, if item.agrees { "" } else { " -> DIFFERS" });
        }
    }
    if !dry_run {
        save_check(sid, report, None);
    }
    if report.verified() {
        println!("{sid}: lyrics VERIFIED, {} lyric files agree", report.agreeing());
        drop_entry(&UNVERIFIED, sid);
        return;
    }
    if enforce && load_config().get("lyrics").bool_or("require_verified", false) {
        // Reported by the caller as a failed step.
        put(&FAILED, sid, &format!("lyrics not verified: {}", report.problem()));
        return;
    }
    put(&UNVERIFIED, sid, &report.problem());
    println!("WARN {sid}: lyrics NOT verified: {}", report.problem());
}

pub fn verify(selected: &[String], dry_run: bool) -> Result<(), String> {
    let songs = manifest::rows(selected)?;
    each("verify", &songs, |row| {
        if let Ok(report) = verify::check_lyrics(row) {
            report_check(get(row, "id"), &report, dry_run, false);
        }
        Ok(())
    });
    Ok(())
}

fn needs_alignment(kind: &Kind, entries: &[Entry]) -> bool {
    matches!(kind, Kind::Plain) || entries.iter().any(|entry| entry.words.is_empty())
}

fn require_verified(report: &LyricCheck) -> Step {
    if !report.verified() && load_config().get("lyrics").bool_or("require_verified", false) {
        return Err(fail(format!("lyrics not verified: {}", report.problem())));
    }
    Ok(())
}

fn align_song(row: &Row, dry_run: bool, force: bool) -> Step {
    let sid = get(row, "id");
    require_lyrics(row)?;
    if !force && align::is_aligned(row) {
        println!("{sid}: word timing is up to date; skipping alignment");
        return Ok(());
    }
    // Alignment is the slowest step, so it only runs for songs that have lyrics to time.
    let found = lyric_source(row).ok_or_else(|| Failure::Skip(NO_LYRICS.into()))?;
    let report = verify::check_lyrics(row).map_err(fail)?;
    require_verified(&report)?;
    if !needs_alignment(&found.kind, &found.entries) {
        println!("{sid}: lyrics already carry word timing; alignment not needed");
        return Ok(());
    }
    if !dry_run && !paths().audio.join(format!("{sid}.flac")).exists() {
        normalize_song(row, false, false)?;
    }
    align::align_row(row, dry_run).map_err(fail)
}

pub fn align_stage(selected: &[String], dry_run: bool, force: bool) -> Result<(), String> {
    let songs = manifest::rows(selected)?;
    each("align", &songs, |row| align_song(row, dry_run, force));
    Ok(())
}

pub fn timestamp(value: i64) -> String {
    format!("{:02}:{:02}", value / 60000, value / 1000 % 60)
}

fn lyrics_song(row: &Row, dry_run: bool) -> Step {
    let sid = get(row, "id");
    let report = verify::check_lyrics(row).map_err(fail)?;
    let primary = report.primary().cloned().ok_or_else(|| Failure::Skip(NO_LYRICS.into()))?;
    let (kind, label, path) = (primary.kind.clone(), primary.label.clone(), primary.path.clone());
    let mut entries = primary.entries.clone();
    let mut estimated: Vec<i64> = Vec::new();
    // The accuracy check comes first: it must be visible even when alignment later fails.
    report_check(sid, &report, dry_run, true);
    if has(&FAILED, sid) {
        return Err(fail(format!("lyrics not verified: {}", report.problem())));
    }
    if needs_alignment(&kind, &entries) {
        let mut timing_path = subtitle::word_json(row);
        if timing_path.is_none() {
            if dry_run {
                return Err(fail("no word-level timing found (a dry run does not align)"));
            }
            println!("{sid}: word timing missing; starting automatic alignment");
            if !paths().audio.join(format!("{sid}.flac")).exists() {
                normalize_song(row, false, false)?;
            }
            align::align_row(row, false).map_err(fail)?;
            timing_path = subtitle::word_json(row);
        }
        let timing_path = timing_path.ok_or_else(|| fail("alignment produced no word-level timing"))?;
        let config = load_config();
        let timing = config.get("timing");
        let units = subtitle::load_units(&timing_path).map_err(fail)?;
        // The recognised audio is an independent witness that these are the words of this recording.
        let heard: Vec<String> = units.iter().map(|unit| unit.2.clone()).collect();
        let audio = verify::agreement(&primary.tokens, &heard);
        println!("{sid}: {:.0}% of the lyric words were also recognised in the audio", audio * 100.0);
        if !dry_run {
            save_check(sid, &report, Some(audio));
        }
        if matches!(kind, Kind::Plain) {
            let text = subtitle::read_text(&path).map_err(fail)?;
            entries = subtitle::plain_entries(&text, &units);
        } else {
            entries = subtitle::time_entries(&entries, &units, timing.int_or("max_tail_ms", 0), timing.int_or("tail_hold_ms", 1000), &mut estimated);
        }
    }
    if entries.is_empty() {
        return Err(fail(format!("no timed lines found in {}", path.display())));
    }
    let text = ass(row, &entries).map_err(fail)?;
    let target = auto_path(sid);
    for language in romanize::take_missing() {
        println!("WARN {sid}: no romanized row for {language} lyrics; install the romanization data");
    }
    // Leave an unchanged file alone so its modification time keeps meaning "lyrics changed".
    let same = target.exists() && std::fs::read_to_string(&target).map(|current| current == text).unwrap_or(false);
    if !dry_run && !same {
        std::fs::create_dir_all(paths().ass.clone()).map_err(|error| fail(error.to_string()))?;
        std::fs::write(&target, &text).map_err(|error| fail(error.to_string()))?;
    }
    let word_lines = entries.iter().filter(|entry| !entry.words.is_empty()).count();
    println!("{sid}: {} lines from {label}, {word_lines} with word timing -> {}", entries.len(), target.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default());
    if !estimated.is_empty() {
        let shown = estimated.iter().take(10).map(|value| timestamp(*value)).collect::<Vec<_>>().join(", ");
        println!("{sid}: {} line(s) had no recognised words and use evenly spaced timing; check them in the editor: {shown}{}", estimated.len(), if estimated.len() > 10 { ", ..." } else { "" });
    }
    if edited_path(sid).exists() && !dry_run {
        println!("{sid}: {}.edited.ass exists and is used for rendering; it is never overwritten", sid);
    }
    Ok(())
}

pub fn lyrics(selected: &[String], dry_run: bool) -> Result<(), String> {
    let songs = manifest::rows(selected)?;
    each("lyrics", &songs, |row| lyrics_song(row, dry_run));
    Ok(())
}

// ------------------------------------------------------------------ render

fn render_subtitle(row: &Row) -> PathBuf {
    let sid = get(row, "id");
    let (edited, auto) = (edited_path(sid), auto_path(sid));
    if has_karaoke(&edited) {
        return edited;
    }
    if edited.exists() && has_karaoke(&auto) {
        println!("{sid}: edited ASS has no word timing; using automatic ASS");
        return auto;
    }
    if edited.exists() {
        edited
    } else {
        auto
    }
}

fn edit(row: &Row) -> Step {
    let sid = get(row, "id");
    let (auto, edited) = (auto_path(sid), edited_path(sid));
    if !auto.exists() && !edited.exists() {
        require_lyrics(row)?;
        return Err(fail(format!("missing {}.auto.ass; run lyrics first", sid)));
    }
    if !edited.exists() {
        std::fs::copy(&auto, &edited).map_err(|error| fail(error.to_string()))?;
        println!("Created editable copy: {}", edited.display());
    }
    let gui = ["aegisub", "Aegisub", "subtitleedit"].iter().find_map(|name| config::find_tool(name));
    match gui {
        Some(program) => {
            let _ = std::process::Command::new(program).arg(&edited).spawn();
        }
        None => println!("Open this file in Aegisub or Subtitle Edit: {}", edited.display()),
    }
    Ok(())
}

fn video_size(path: &Path) -> Result<(u32, u32), String> {
    let output = config::output("ffprobe", &["-v", "error", "-select_streams", "v:0", "-show_entries", "stream=width,height", "-of", "csv=p=0:s=x", &path.to_string_lossy()])?;
    let text = String::from_utf8_lossy(&output.stdout);
    let (width, height) = text.trim().split_once('x').ok_or_else(|| format!("cannot read the video size of {}", path.display()))?;
    Ok((width.trim().parse().unwrap_or(0), height.trim().parse().unwrap_or(0)))
}

fn logo_for(row: &Row) -> Result<Option<PathBuf>, String> {
    let configured = get(row, "logo_file").trim();
    let raw = if configured.is_empty() {
        load_config().get("logo").str_or("file", "")
    } else {
        configured.to_string()
    };
    if raw.is_empty() {
        return Ok(None);
    }
    let path = config::resolve(&raw);
    if !path.exists() {
        return Err(format!("logo file not found: {}", path.display()));
    }
    Ok(Some(path))
}

/// Scale and place the logo in PlayRes units so it matches the subtitle layout at any resolution.
fn logo_filters(media: &Path, video: &str, frame_width: Option<u32>, logo_input: usize) -> Result<Vec<String>, String> {
    let config = load_config();
    let settings = config.get("logo");
    let frame = match frame_width {
        Some(width) => width as f64,
        None => video_size(media)?.0 as f64,
    };
    let play_res_x = config.get("video").int_or("play_res_x", 1920).max(1) as f64;
    let scale = frame / play_res_x;
    let width = (settings.float_or("width", 220.0) * scale).round().max(2.0);
    let margin_x = (settings.float_or("margin_x", 40.0) * scale).round();
    let margin_y = (settings.float_or("margin_y", 40.0) * scale).round();
    let position = settings.str_or("position", "top-right").to_lowercase();
    let x = if position.contains("left") {
        format!("{margin_x}")
    } else if position.contains("right") {
        format!("W-w-{margin_x}")
    } else {
        "(W-w)/2".into()
    };
    let y = if position.contains("top") {
        format!("{margin_y}")
    } else if position.contains("bottom") {
        format!("H-h-{margin_y}")
    } else {
        "(H-h)/2".into()
    };
    let opacity = settings.float_or("opacity", 1.0).clamp(0.0, 1.0);
    Ok(vec![
        format!("[{logo_input}:v]scale={width}:-1,format=rgba,colorchannelmixer=aa={opacity}[logo]"),
        format!("{video}[logo]overlay={x}:{y}[branded]"),
    ])
}

/// Bring the video to the PlayRes frame size; returns the filters and the new width.
fn fit_filters(media: &Path, video: &str) -> Result<(Vec<String>, Option<u32>), String> {
    let config = load_config();
    let fit = config.get("render").str_or("fit", "blur").to_lowercase();
    if fit == "none" {
        return Ok((Vec::new(), None));
    }
    let width = config.get("video").int_or("play_res_x", 1920) as u32;
    let height = config.get("video").int_or("play_res_y", 1080) as u32;
    let (source_width, source_height) = video_size(media)?;
    if (source_width, source_height) == (width, height) {
        return Ok((Vec::new(), None));
    }
    let fitted = format!("scale={width}:{height}:force_original_aspect_ratio=decrease:force_divisible_by=2");
    if (source_width as f64 * height as f64 - source_height as f64 * width as f64).abs() <= 0.01 * source_height as f64 * width as f64 {
        return Ok((vec![format!("{video}scale={width}:{height},setsar=1[fitted]")], Some(width)));
    }
    if fit == "pad" {
        return Ok((vec![format!("{video}{fitted},pad={width}:{height}:(ow-iw)/2:(oh-ih)/2,setsar=1[fitted]")], Some(width)));
    }
    let sigma = config.get("render").float_or("blur_sigma", 30.0);
    Ok((
        vec![
            format!("{video}split[fit_bg][fit_fg]"),
            format!("[fit_bg]scale={width}:{height}:force_original_aspect_ratio=increase,crop={width}:{height},gblur=sigma={sigma},eq=brightness=-0.06[fit_blurred]"),
            format!("[fit_fg]{fitted}[fit_front]"),
            "[fit_blurred][fit_front]overlay=(W-w)/2:(H-h)/2,setsar=1[fitted]".into(),
        ],
        Some(width),
    ))
}

/// ffmpeg inputs for a still background when the media has no video, and the picture to size it by.
fn background_inputs(sid: &str, media: &Path, dry_run: bool) -> (Vec<String>, Option<PathBuf>) {
    let info = probe(media);
    if info.video {
        return (Vec::new(), Some(media.to_path_buf()));
    }
    let config = load_config();
    let width = config.get("video").int_or("play_res_x", 1920);
    let height = config.get("video").int_or("play_res_y", 1080);
    if info.cover {
        let cover = paths().raw.join(format!("{sid}.cover.png"));
        if !dry_run && !cover.exists() {
            let args: Vec<String> = vec!["-hide_banner".into(), "-loglevel".into(), "error".into(), "-y".into(), "-i".into(), media.display().to_string(), "-map".into(), "0:v:0".into(), "-frames:v".into(), "1".into(), cover.display().to_string()];
            let _ = config::run("ffmpeg", &args, false, None);
        }
        if dry_run || cover.exists() {
            return (vec!["-loop".into(), "1".into(), "-framerate".into(), "25".into(), "-i".into(), cover.display().to_string()], Some(cover));
        }
    }
    let colour = config.get("render").str_or("background_color", "black");
    // Already the frame size, so there is nothing to fit.
    (vec!["-f".into(), "lavfi".into(), "-i".into(), format!("color=c={colour}:s={width}x{height}:r=25")], None)
}

fn encoder_args() -> Vec<String> {
    let config = load_config();
    let settings = config.get("render");
    let mut args = vec!["-c:v".to_string(), settings.str_or("video_codec", "libx264")];
    // Empty values are left out so hardware encoders with other rate-control options can be used.
    for (flag, key) in [("-preset", "preset"), ("-crf", "crf"), ("-pix_fmt", "pixel_format")] {
        let value = settings.str_or(key, "");
        if !value.trim().is_empty() {
            args.push(flag.into());
            args.push(value);
        }
    }
    args.push("-c:a".into());
    args.push(settings.str_or("audio_codec", "aac"));
    let bitrate = settings.str_or("audio_bitrate", "");
    if !bitrate.trim().is_empty() {
        args.push("-b:a".into());
        args.push(bitrate);
    }
    for item in settings.get("extra_args").items() {
        args.push(item.text());
    }
    args.extend(["-movflags".to_string(), "+faststart".to_string()]);
    args
}

fn intro_padding(text: &str) -> i64 {
    for line in text.lines() {
        if let Some(rest) = line.trim().strip_prefix("; PolyKaraIntroPaddingMs:") {
            return rest.trim().parse().unwrap_or(0);
        }
    }
    0
}

fn render_song(row: &Row, dry_run: bool, force: bool) -> Step {
    let sid = get(row, "id");
    let output = output_path(sid);
    // A subtitle that already exists can be rendered; otherwise the song needs lyrics to make one.
    if !edited_path(sid).exists() && !auto_path(sid).exists() {
        require_lyrics(row)?;
    }
    if !dry_run && !has_karaoke(&edited_path(sid)) && !has_karaoke(&auto_path(sid)) {
        println!("{sid}: no karaoke subtitle yet; generating lyrics first");
        lyrics_song(row, false)?;
    }
    let subtitle = render_subtitle(row);
    if !subtitle.exists() {
        return Err(fail("missing ASS subtitle; run lyrics first"));
    }
    let ass_text = std::fs::read_to_string(&subtitle).map_err(|error| fail(error.to_string()))?;
    if !ass_text.contains(KARAOKE_TAG) {
        return Err(fail("no word-level timing; no output rendered"));
    }
    if output.exists() && !force {
        if newer(&subtitle, &output) {
            println!("{sid}: {} changed after the last render; rendering again", subtitle.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default());
        } else {
            println!("{sid}: output already exists; skipping (use --reprocess to render again)");
            return Ok(());
        }
    }
    let media = source(sid).map_err(fail)?;
    let logo = logo_for(row).map_err(fail)?;
    let padding = intro_padding(&ass_text);
    let (inputs, picture) = background_inputs(sid, &media, dry_run);
    // Inputs: [background,] media [, logo]. A song from an audio file gets a still background.
    let sound = if inputs.is_empty() { 0 } else { 1 };
    let mut filters: Vec<String> = Vec::new();
    let mut video = "[0:v]".to_string();
    let mut audio = format!("{sound}:a:0?");
    if padding != 0 {
        filters.push(format!("{video}tpad=start_mode=clone:start_duration={}[padded]", padding as f64 / 1000.0));
        filters.push(format!("[{sound}:a:0]adelay={padding}:all=1[a]"));
        video = "[padded]".into();
        audio = "[a]".into();
    }
    let (fitted, frame_width) = match &picture {
        Some(picture) => fit_filters(picture, &video).map_err(fail)?,
        None => (Vec::new(), Some(load_config().get("video").int_or("play_res_x", 1920) as u32)),
    };
    if !fitted.is_empty() {
        filters.extend(fitted);
        video = "[fitted]".into();
    }
    if logo.is_some() {
        filters.extend(logo_filters(picture.as_deref().unwrap_or(&media), &video, frame_width, sound + 1).map_err(fail)?);
        video = "[branded]".into();
    }
    // ffmpeg runs inside the ASS directory: ids are filter-safe, absolute paths are not.
    let name = subtitle.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default();
    filters.push(format!("{video}ass={name}[v]"));
    let partial = paths().output.join(format!("{sid}.partial.mp4"));
    let mut args: Vec<String> = vec!["-hide_banner".into(), "-y".into()];
    args.extend(inputs);
    args.extend(["-i".to_string(), media.display().to_string()]);
    if let Some(logo) = &logo {
        args.extend(["-i".to_string(), logo.display().to_string()]);
    }
    // A still background never ends by itself; stop at the end of the song (plus any intro padding).
    let length = if args.iter().any(|item| item.as_str() == "-loop" || item.as_str() == "-f") {
        vec!["-t".to_string(), format!("{:.3}", probe(&media).duration + padding as f64 / 1000.0)]
    } else {
        Vec::new()
    };
    args.extend(["-filter_complex".to_string(), filters.join(";"), "-map".to_string(), "[v]".to_string(), "-map".to_string(), audio]);
    args.extend(length);
    args.extend(encoder_args());
    args.push(partial.display().to_string());
    std::fs::create_dir_all(paths().output.clone()).map_err(|error| fail(error.to_string()))?;
    let folder = subtitle.parent().map(Path::to_path_buf);
    let result = config::run("ffmpeg", &args, dry_run, folder.as_deref());
    let outcome = match result {
        Ok(()) if !dry_run => std::fs::rename(&partial, &output).map_err(|error| fail(error.to_string())).map(|_| println!("{sid}: rendered {}", output.display())),
        Ok(()) => Ok(()),
        Err(problem) => Err(fail(problem)),
    };
    let _ = std::fs::remove_file(&partial);
    outcome
}

fn preview(row: &Row, at: f64) -> Step {
    let sid = get(row, "id");
    lyrics_song(row, false)?;
    let subtitle = render_subtitle(row);
    if !subtitle.exists() {
        return Err(fail("missing ASS subtitle; run lyrics first"));
    }
    let media = source(sid).map_err(fail)?;
    if !probe(&media).video {
        return Err(fail("the preview needs a video; this song comes from an audio file, so render it and use Play video"));
    }
    let ass_text = std::fs::read_to_string(&subtitle).map_err(|error| fail(error.to_string()))?;
    let padding = intro_padding(&ass_text);
    let config = load_config();
    let width = config.get("video").int_or("play_res_x", 1920);
    let height = config.get("video").int_or("play_res_y", 1080);
    let name = subtitle.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default();
    let filters = format!("setpts=PTS+{}/TB,scale={width}:{height}:force_original_aspect_ratio=decrease,pad={width}:{height}:(ow-iw)/2:(oh-ih)/2,ass={name}", padding as f64 / 1000.0);
    println!("{sid}: previewing from {}; close the window or press q to stop", timestamp((at * 1000.0) as i64));
    let args: Vec<String> = vec![
        "-hide_banner".to_string(), "-loglevel".to_string(), "error".to_string(), "-autoexit".to_string(), "-x".to_string(), "1280".to_string(), "-y".to_string(), "720".to_string(),
        "-window_title".to_string(), format!("PolyKara preview - {sid}"), "-ss".to_string(), format!("{:.3}", at.max(0.0)), "-vf".to_string(), filters,
        media.to_string_lossy().into_owned(),
    ];
    let _ = config::run("ffplay", &args, false, subtitle.parent());
    Ok(())
}

pub fn render(selected: &[String], dry_run: bool, force: bool) -> Result<(), String> {
    let songs = manifest::rows(selected)?;
    each("render", &songs, |row| render_song(row, dry_run, force));
    Ok(())
}

pub fn edit_all(selected: &[String]) -> Result<(), String> {
    let songs = manifest::rows(selected)?;
    each("edit", &songs, edit);
    Ok(())
}

pub fn preview_all(selected: &[String], at: f64) -> Result<(), String> {
    let songs = manifest::rows(selected)?;
    each("preview", &songs, |row| preview(row, at));
    Ok(())
}

pub fn cleanup(selected: &[String], dry_run: bool, drop_source: bool) -> Result<(), String> {
    let mut targets: Vec<PathBuf> = std::fs::read_dir(paths().audio.clone()).map(|entries| entries.filter_map(|entry| entry.ok()).map(|entry| entry.path()).collect()).unwrap_or_default();
    targets.extend(std::fs::read_dir(paths().output.clone()).map(|entries| entries.filter_map(|entry| entry.ok()).map(|entry| entry.path()).filter(|path| path.file_name().map(|name| name.to_string_lossy().ends_with(".partial.mp4")).unwrap_or(false)).collect::<Vec<_>>()).unwrap_or_default());
    if drop_source {
        for row in manifest::rows(selected)? {
            let sid = get(&row, "id");
            if output_path(sid).exists() && edited_path(sid).exists() {
                if let Ok(entries) = std::fs::read_dir(paths().raw.clone()) {
                    targets.extend(entries.filter_map(|entry| entry.ok()).map(|entry| entry.path()).filter(|path| path.file_name().map(|name| name.to_string_lossy().starts_with(&format!("{sid}.")) && path.extension().map(|ext| !ext.eq_ignore_ascii_case("json")).unwrap_or(true)).unwrap_or(false)));
                }
            }
        }
    }
    for path in targets {
        println!("REMOVE {}", path.display());
        if !dry_run {
            let _ = std::fs::remove_file(&path);
        }
    }
    Ok(())
}

pub fn qa(selected: &[String]) -> Result<(), String> {
    for row in manifest::rows(selected)? {
        let sid = get(&row, "id");
        let selected = if edited_path(sid).exists() { edited_path(sid) } else { auto_path(sid) };
        let lyric_state = match verify::check_lyrics(&row) {
            Ok(report) => match report.primary() {
                None => "MISSING".to_string(),
                Some(primary) => format!("{}, {} ({} of {} files agree)", primary.label, if report.verified() { "verified" } else { "NOT verified" }, report.agreeing(), report.files.len()),
            },
            Err(problem) => format!("ERROR ({problem})"),
        };
        let ass_state = if selected.exists() {
            let text = std::fs::read_to_string(&selected).unwrap_or_default();
            let events: Vec<&str> = text.lines().filter(|line| line.starts_with("Dialogue:") && line.contains(",Lyric,")).collect();
            let key = |line: &str| -> Vec<String> {
                let fields: Vec<&str> = line.splitn(6, ',').collect();
                if fields.len() >= 5 && !fields[4].trim().is_empty() {
                    vec![fields[4].trim().to_string()]
                } else {
                    fields.iter().skip(1).take(2).map(|item| item.to_string()).collect()
                }
            };
            let mut lines: Vec<Vec<String>> = events.iter().map(|line| key(line)).collect();
            lines.sort();
            lines.dedup();
            let mut timed: Vec<Vec<String>> = events.iter().filter(|line| line.contains(KARAOKE_TAG)).map(|line| key(line)).collect();
            timed.sort();
            timed.dedup();
            format!("{} ({}/{} lines with word timing)", if selected == edited_path(sid) { "edited" } else { "automatic" }, timed.len(), lines.len())
        } else {
            "MISSING".into()
        };
        let output = output_path(sid);
        let mp4_state = if !output.exists() { "WAITING" } else if newer(&edited_path(sid), &output) { "STALE (subtitle edited after render)" } else { "OK" };
        println!("{sid}: media={}, lyrics={lyric_state}, timing={}, ass={ass_state}, mp4={mp4_state}", if find_source(sid).is_some() { "OK" } else { "MISSING" }, if subtitle::word_json(&row).is_some() { "OK" } else { "none" });
    }
    Ok(())
}

pub fn process(selected: &[String], dry_run: bool, force: bool, realign: bool) -> Result<(), String> {
    let mut songs = manifest::rows(selected)?;
    if !force {
        songs.retain(|row| {
            let sid = get(row, "id");
            if output_path(sid).exists() && !newer(&edited_path(sid), &output_path(sid)) {
                println!("{sid}: final MP4 is up to date; skipping (use --reprocess to rebuild)");
                false
            } else {
                true
            }
        });
    }
    println!("== check ==");
    // yt-dlp is only required when some song still has to be downloaded.
    manifest::check(songs.iter().any(|row| find_source(get(row, "id")).is_none()))?;
    if songs.is_empty() {
        println!("Nothing to do: every song already has a final MP4. Use --reprocess to build them again.");
        return Ok(());
    }
    if !align::available() && songs.iter().any(|row| subtitle::word_json(row).is_none()) {
        println!("WARN no aligner is installed: songs without word timing cannot be rendered");
    }
    for folder in [paths().raw.clone(), paths().subtitles.clone(), paths().metadata.clone(), paths().lyrics.clone()] {
        std::fs::create_dir_all(folder).map_err(|error| error.to_string())?;
    }
    println!("== download ==");
    each("download", &songs, |row| download_song(row, dry_run, false));
    println!("== normalize ==");
    each("normalize", &songs, |row| prepare_audio(row, dry_run, realign));
    println!("== align ==");
    each("align", &songs, |row| align_song(row, dry_run, realign));
    println!("== lyrics ==");
    each("lyrics", &songs, |row| lyrics_song(row, dry_run));
    println!("== render ==");
    each("render", &songs, |row| render_song(row, dry_run, true));
    Ok(())
}

pub fn summary() -> i32 {
    let unverified = UNVERIFIED.lock().unwrap_or_else(|error| error.into_inner()).clone();
    let skipped = SKIPPED.lock().unwrap_or_else(|error| error.into_inner()).clone();
    let failed = FAILED.lock().unwrap_or_else(|error| error.into_inner()).clone();
    if !unverified.is_empty() {
        println!("\n{} song(s) have lyrics that did not pass the accuracy check; review them before publishing:", unverified.len());
        for (sid, reason) in &unverified {
            println!("  {sid}: {reason}");
        }
    }
    if !skipped.is_empty() {
        println!("\n{} song(s) were set aside and can be completed later:", skipped.len());
        for (sid, reason) in &skipped {
            println!("  {sid}: {reason}");
        }
    }
    if failed.is_empty() {
        return 0;
    }
    println!("\n{} song(s) were skipped; every other song ran to the end:", failed.len());
    for (sid, reason) in &failed {
        println!("  {sid}: {reason}");
    }
    println!("Fix the cause and rerun the same command; finished work is reused.");
    1
}

/// Render one subtitle frame with the tools PolyKara would use, to check an installation.
pub fn selftest() -> i32 {
    let mut problems: Vec<String> = Vec::new();
    for name in ["ffmpeg", "ffprobe", "ffplay", "yt-dlp"] {
        let found = config::find_tool(name);
        println!("{name}: {}", found.as_ref().map(|path| path.display().to_string()).unwrap_or_else(|| "MISSING".into()));
        if found.is_none() {
            problems.push(format!("{name} is missing"));
        }
    }
    let aligner = align::shown();
    println!("aligner: {aligner}");
    if aligner == "MISSING" {
        problems.push("no aligner".into());
    }
    let mut row: Row = Row::new();
    for (key, value) in [("id", "selftest"), ("title", "PolyKara 自检"), ("artist", "歌手 & Singer"), ("composer", "作曲"), ("lyricist", "作词"), ("producer", "PolyKara"), ("language", "zh"), ("voice", "duet")] {
        row.insert(key.to_string(), value.to_string());
    }
    let timed = |start: i64, pieces: &[&str]| -> Entry {
        let words: Vec<(i64, i64, String)> = pieces.iter().enumerate().map(|(index, piece)| (start + 300 * index as i64, start + 300 * index as i64 + 280, piece.to_string())).collect();
        Entry::new(start, words.last().map(|word| word.1).unwrap_or(start), pieces.concat(), words)
    };
    let mut entries = vec![timed(5000, &["唤", "不", "醒", "原", "来"]), timed(7000, &["Hello ", "PolyKara ", "world"])];
    entries[0].text = format!("[singer: 女] {}", entries[0].text);
    let text = match ass(&row, &entries) {
        Ok(text) => text,
        Err(problem) => {
            println!("SELFTEST FAILED: {problem}");
            return 1;
        }
    };
    // The text is the tenth field of a Dialogue line; override tags such as {\k50} are dropped.
    let readings: Vec<String> = text
        .lines()
        .filter(|line| line.starts_with("Dialogue:") && line.contains(",Romanization,"))
        .map(|line| line.splitn(10, ',').nth(9).unwrap_or("").split('{').map(|part| part.split('}').next_back().unwrap_or("")).collect::<Vec<_>>().join("").trim().to_string())
        .collect();
    println!("romanization: {}", if readings.is_empty() { "MISSING".into() } else { readings.join(" ") });
    if readings.is_empty() {
        problems.push("no romanization (the pinyin data is missing)".into());
    }
    let target = paths().output.join("selftest.png");
    let _ = std::fs::create_dir_all(paths().output.clone());
    let folder = std::env::temp_dir().join(format!("polykara-selftest-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&folder);
    let _ = std::fs::write(folder.join("selftest.ass"), &text);
    let args: Vec<String> = ["-hide_banner", "-loglevel", "error", "-y", "-f", "lavfi", "-i", "color=c=0x2a3d55:s=1920x1080:r=5:d=7", "-vf", "ass=selftest.ass", "-ss", "5.9", "-frames:v", "1"]
        .iter()
        .map(|item| item.to_string())
        .chain(std::iter::once(target.display().to_string()))
        .collect();
    if let Err(problem) = config::run("ffmpeg", &args, false, Some(&folder)) {
        problems.push(format!("ffmpeg could not render subtitles ({problem})"));
    }
    let _ = std::fs::remove_dir_all(&folder);
    if target.exists() && std::fs::metadata(&target).map(|meta| meta.len() > 0).unwrap_or(false) {
        println!("rendered test frame: {}", target.display());
    } else if !problems.iter().any(|problem| problem.starts_with("ffmpeg")) {
        problems.push("ffmpeg produced no frame".into());
    }
    if problems.is_empty() {
        println!("SELFTEST OK");
        0
    } else {
        println!("SELFTEST FAILED: {}", problems.join("; "));
        1
    }
}
