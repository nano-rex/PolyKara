//! The PolyKara command line: the pipeline, the engine the desktop app talks
//! to, and the self-test.
//!
//! Every step works on songs.csv, and a song that fails is set aside with its
//! reason while the rest of the batch carries on.

use std::path::{Path, PathBuf};

use polykara::{config, localmedia, manifest, pipeline, server};

const COMMANDS: &[&str] = &["check", "download", "verify", "normalize", "align", "lyrics", "edit", "preview", "render", "process", "cleanup", "qa"];

fn usage() -> String {
    format!(
        "PolyKara: karaoke videos from songs and their lyrics.\n\
         \n\
         Usage: polykara-cli COMMAND [SONG...] [options]\n\
         \n\
         Commands: {}\n\
           add FILE...   add media files on this computer as songs\n\
           selftest      check the tools and render a test frame\n\
           engine        answer the desktop app's JSON requests on stdin\n\
         \n\
         Options:\n\
           --only IDS      comma-separated song ids (same as listing them)\n\
           --dry-run       print what would run\n\
           --reprocess     redo a step whose output already exists\n\
           --realign       with process: repeat word alignment\n\
           --drop-source   with cleanup: remove source media that was rendered\n\
           --at SECONDS    with preview: where to start\n\
           --trending      show regional YouTube Top Songs and add entries\n\
           --pick          choose chart songs; otherwise add all of them\n\
           --regions CODES comma-separated chart regions",
        COMMANDS.join(", ")
    )
}

fn main() {
    prepare();
    std::process::exit(run());
}

/// The program carries its own tools: put bin/ first on PATH and fill the
/// user's PolyKara folder with the defaults the first time it is used.
fn prepare() {
    let paths = config::paths();
    let bin = paths.root.join("bin");
    if bin.is_dir() {
        let current = std::env::var_os("PATH").unwrap_or_default();
        let listed = std::env::split_paths(&current).any(|folder| folder == bin);
        if !listed {
            let mut folders = vec![bin];
            folders.extend(std::env::split_paths(&current));
            if let Ok(joined) = std::env::join_paths(folders) {
                std::env::set_var("PATH", joined);
            }
        }
    }
    drop_demo_songs(&paths.manifest);
    let defaults = paths.root.join("defaults");
    if defaults.is_dir() {
        let _ = std::fs::create_dir_all(&paths.home);
        if let Ok(entries) = std::fs::read_dir(&defaults) {
            for entry in entries.filter_map(|entry| entry.ok()) {
                let target = paths.home.join(entry.file_name());
                if target.exists() {
                    continue;
                }
                if entry.path().is_dir() {
                    copy_tree(&entry.path(), &target);
                } else {
                    let _ = std::fs::copy(entry.path(), &target);
                }
            }
        }
    }
}

fn copy_tree(source: &Path, target: &Path) {
    if std::fs::create_dir_all(target).is_err() {
        return;
    }
    if let Ok(entries) = std::fs::read_dir(source) {
        for entry in entries.filter_map(|entry| entry.ok()) {
            let path = entry.path();
            let destination = target.join(entry.file_name());
            if path.is_dir() {
                copy_tree(&path, &destination);
            } else {
                let _ = std::fs::copy(&path, &destination);
            }
        }
    }
}

/// Earlier versions put two demo songs in a new songs.csv; remove them once.
fn drop_demo_songs(manifest: &Path) {
    let text = match std::fs::read_to_string(manifest) {
        Ok(text) => text,
        Err(_) => return,
    };
    let demos = [
        ("amazing-grace-vocal", "https://www.youtube.com/watch?v=24cWZeB4ldE"),
        ("silent-night-vocal", "https://www.youtube.com/watch?v=25NY8yTolMY"),
    ];
    let kept: Vec<&str> = text
        .lines()
        .filter(|line| !demos.iter().any(|(sid, url)| line.starts_with(&format!("{sid},{url},"))))
        .collect();
    if kept.len() != text.lines().count() {
        let partial = manifest.with_file_name(format!("{}.tmp", manifest.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default()));
        if std::fs::write(&partial, format!("{}\n", kept.join("\n"))).is_ok() {
            let _ = std::fs::rename(&partial, manifest);
        }
    }
}

/// The value of an option: after an "=" or as the next argument.
fn take(args: &[String], index: &mut usize, item: &str) -> Option<String> {
    if let Some((_, value)) = item.split_once('=') {
        return Some(value.to_string());
    }
    *index += 1;
    args.get(*index).cloned()
}

fn run() -> i32 {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let first = match args.first() {
        Some(first) => first.as_str(),
        None => {
            println!("{}", usage());
            return 2;
        }
    };
    match first {
        "selftest" => return pipeline::selftest(),
        "engine" => return server::serve(),
        "add" => {
            let files: Vec<PathBuf> = args[1..].iter().map(PathBuf::from).collect();
            if files.is_empty() {
                println!("Usage: polykara-cli add FILE...");
                return 2;
            }
            return match localmedia::add_files(&files) {
                Ok(added) => {
                    for row in &added {
                        let artist = manifest::get(row, "artist");
                        println!(
                            "added {}: {} - {} [{}] from {}",
                            manifest::get(row, "id"),
                            if artist.is_empty() { "(singer unknown)" } else { artist },
                            manifest::get(row, "title"),
                            manifest::get(row, "language"),
                            manifest::get(row, "url")
                        );
                    }
                    println!("{} song(s) added to songs.csv; check their language, singer and voice before processing", added.len());
                    0
                }
                Err(problem) => {
                    println!("{problem}");
                    1
                }
            };
        }
        "help" | "--help" | "-h" => {
            println!("{}", usage());
            return 0;
        }
        _ => {}
    }
    let mut command: Option<String> = None;
    let mut selected: Vec<String> = Vec::new();
    let (mut dry_run, mut reprocess, mut realign, mut drop_source, mut trending, mut pick) = (false, false, false, false, false, false);
    let mut regions = String::new();
    let mut at = 0.0f64;
    let mut index = 0;
    while index < args.len() {
        let item = args[index].as_str();
        let name = item.split('=').next().unwrap_or(item);
        match name {
            "--dry-run" => dry_run = true,
            "--reprocess" => reprocess = true,
            "--realign" => realign = true,
            "--drop-source" => drop_source = true,
            "--trending" => trending = true,
            "--pick" => pick = true,
            "--only" => {
                if let Some(value) = take(&args, &mut index, item) {
                    selected.extend(value.split(',').map(|item| item.trim().to_string()).filter(|item| !item.is_empty()));
                }
            }
            "--regions" => regions = take(&args, &mut index, item).unwrap_or_default(),
            "--at" => at = take(&args, &mut index, item).and_then(|value| value.trim().parse().ok()).unwrap_or(0.0),
            other if other.starts_with("--") => {
                println!("Unknown option {other}\n\n{}", usage());
                return 2;
            }
            _ => {
                if command.is_none() {
                    command = Some(item.to_string());
                } else {
                    selected.push(item.to_string());
                }
            }
        }
        index += 1;
    }
    let command = match command {
        Some(command) => command,
        None => {
            println!("{}", usage());
            return 2;
        }
    };
    if !COMMANDS.contains(&command.as_str()) {
        println!("Unknown command {command}\n\n{}", usage());
        return 2;
    }
    let selection: Vec<String> = selected;
    let result = match command.as_str() {
        "check" => manifest::check(false),
        "download" => pipeline::download(&selection, dry_run, trending, &regions, pick, reprocess),
        "verify" => pipeline::verify(&selection, dry_run),
        "normalize" => pipeline::normalize(&selection, dry_run, reprocess),
        "align" => pipeline::align_stage(&selection, dry_run, reprocess),
        "lyrics" => pipeline::lyrics(&selection, dry_run),
        "edit" => pipeline::edit_all(&selection),
        "preview" => pipeline::preview_all(&selection, at),
        "render" => pipeline::render(&selection, dry_run, reprocess),
        "process" => pipeline::process(&selection, dry_run, reprocess, realign),
        "cleanup" => pipeline::cleanup(&selection, dry_run, drop_source),
        _ => pipeline::qa(&selection),
    };
    if let Err(problem) = result {
        println!("{problem}");
        return 1;
    }
    pipeline::summary()
}
