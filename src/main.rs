mod ass;
mod config;
mod lyricformats;
mod manifest;
mod model;
mod project;
mod verify;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use config::Paths;
use model::{Entry, Song};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Parser, Debug)]
#[command(
    name = "polykara",
    version,
    about = "Multilingual karaoke subtitle automation"
)]
struct Cli {
    #[command(subcommand)]
    command: CommandLine,
}
#[derive(Subcommand, Debug)]
enum CommandLine {
    Check,
    Add {
        files: Vec<PathBuf>,
    },
    Verify {
        #[arg(long, default_value_t = 3)]
        min_sources: usize,
        #[arg(long, default_value_t = 0.6)]
        threshold: f64,
    },
    Normalize {
        #[arg(long)]
        reprocess: bool,
    },
    Align {
        #[arg(long)]
        reprocess: bool,
    },
    Lyrics,
    Edit {
        ids: Vec<String>,
    },
    Render {
        #[arg(long)]
        reprocess: bool,
    },
    Process {
        #[arg(long)]
        reprocess: bool,
        #[arg(long)]
        realign: bool,
    },
    Cleanup {
        #[arg(long)]
        drop_source: bool,
    },
    Qa,
    Selftest,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let paths = Paths::discover();
    paths.ensure_work()?;
    match cli.command {
        CommandLine::Check => check(&paths),
        CommandLine::Add { files } => add(&paths, &files),
        CommandLine::Verify {
            min_sources,
            threshold,
        } => verify(&paths, min_sources, threshold),
        CommandLine::Normalize { reprocess } => normalize(&paths, reprocess),
        CommandLine::Align { reprocess } => align(&paths, reprocess),
        CommandLine::Lyrics => lyrics(&paths),
        CommandLine::Edit { ids } => edit(&paths, &ids),
        CommandLine::Render { reprocess } => render(&paths, reprocess),
        CommandLine::Process { reprocess, realign } => process(&paths, reprocess, realign),
        CommandLine::Cleanup { drop_source } => cleanup(&paths, drop_source),
        CommandLine::Qa => qa(&paths),
        CommandLine::Selftest => selftest(&paths),
    }
}

fn songs(paths: &Paths) -> Result<Vec<Song>> {
    manifest::read(&paths.manifest)
        .with_context(|| format!("could not read {}", paths.manifest.display()))
}
fn check(paths: &Paths) -> Result<()> {
    let rows = songs(paths)?;
    if rows.is_empty() {
        println!("no songs in {}", paths.manifest.display());
    } else {
        for s in &rows {
            println!("{}: {} - {} [{}]", s.id, s.artist, s.title, s.language);
        }
    }
    Ok(())
}
fn add(paths: &Paths, files: &[PathBuf]) -> Result<()> {
    let mut rows = songs(paths)?;
    for file in files {
        let song = manifest::add_file(file, &mut rows)?;
        println!(
            "added {}: {} - {} [{}]",
            song.id, song.artist, song.title, song.language
        );
    }
    manifest::write(&paths.manifest, &rows)
}
fn verify(paths: &Paths, min_sources: usize, threshold: f64) -> Result<()> {
    let mut failed = 0;
    for song in songs(paths)? {
        let mut texts = Vec::new();
        if !song.lyrics_file.is_empty() {
            let p = PathBuf::from(&song.lyrics_file);
            if p.exists() {
                texts.push(fs::read_to_string(p)?);
            }
        }
        for p in fs::read_dir(&paths.lyrics)
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| e.path())
            .filter(|p| {
                p.file_name()
                    .and_then(|n| n.to_str())
                    .map(|n| n.starts_with(&song.id))
                    .unwrap_or(false)
            })
        {
            if let Ok(t) = fs::read_to_string(p) {
                texts.push(t);
            }
        }
        let ok = verify::compare(&texts, min_sources, threshold);
        println!(
            "{}: {} lyric source(s), {}",
            song.id,
            texts.len(),
            if ok { "verified" } else { "not verified" }
        );
        if !ok {
            failed += 1
        }
    }
    if failed > 0 {
        Err(anyhow::anyhow!(
            "{} song(s) failed lyric verification",
            failed
        ))
    } else {
        Ok(())
    }
}
fn lyrics(paths: &Paths) -> Result<()> {
    for song in songs(paths)? {
        if song.lyrics_file.is_empty() {
            println!("{}: no lyrics_file configured", song.id);
            continue;
        }
        let entries = lyricformats::parse_file(Path::new(&song.lyrics_file))?;
        println!("{}: parsed {} lyric line(s)", song.id, entries.len());
    }
    Ok(())
}
fn normalize(paths: &Paths, reprocess: bool) -> Result<()> {
    for song in songs(paths)? {
        let input = paths.raw.join(format!("{}.mp4", song.id));
        let output = paths.audio.join(format!("{}.wav", song.id));
        if output.exists() && !reprocess {
            continue;
        }
        if !input.exists() {
            println!("SKIP {}: no media at {}", song.id, input.display());
            continue;
        }
        println!("normalizing {}", song.id);
        run(
            "ffmpeg",
            &[
                "-y",
                "-i",
                input.to_str().unwrap(),
                "-vn",
                "-acodec",
                "pcm_s16le",
                output.to_str().unwrap(),
            ],
        )?;
    }
    Ok(())
}
fn align(paths: &Paths, reprocess: bool) -> Result<()> {
    for song in songs(paths)? {
        let target = paths.align.join(format!("{}.json", song.id));
        if target.exists() && !reprocess {
            continue;
        }
        if song.word_timing_file.is_empty() {
            println!("SKIP {}: Rust alignment expects word_timing_file; install a timing provider before running align",song.id);
            continue;
        }
        fs::copy(&song.word_timing_file, &target)?;
        println!("{}: imported word timing", song.id);
    }
    Ok(())
}
fn render(paths: &Paths, reprocess: bool) -> Result<()> {
    let cfg = config::load_config(paths);
    for song in songs(paths)? {
        let target = paths.ass.join(format!("{}.ass", song.id));
        if target.exists() && !reprocess {
            continue;
        }
        let (entries, source) = match project::load_or_lyrics(&paths.work, &song) {
            Ok(v) => v,
            Err(e) => {
                println!("SKIP {}: {}", song.id, e);
                continue;
            }
        };
        fs::create_dir_all(&paths.ass)?;
        fs::write(&target, ass::document(&song, &entries, &cfg))?;
        println!("{}: wrote {} from {}", song.id, target.display(), source);
    }
    Ok(())
}
fn edit(paths: &Paths, ids: &[String]) -> Result<()> {
    for song in songs(paths)?
        .into_iter()
        .filter(|s| ids.is_empty() || ids.contains(&s.id))
    {
        let p = paths.ass.join(format!("{}.ass", song.id));
        println!("{}", p.display());
        if cfg!(target_os = "windows") {
            let _ = Command::new("cmd")
                .args(["/C", "start", p.to_str().unwrap()])
                .status();
        }
    }
    Ok(())
}
fn process(paths: &Paths, reprocess: bool, realign: bool) -> Result<()> {
    check(paths)?;
    normalize(paths, reprocess)?;
    if realign {
        align(paths, true)?
    }
    lyrics(paths)?;
    render(paths, reprocess)
}
fn cleanup(paths: &Paths, drop_source: bool) -> Result<()> {
    for dir in [
        &paths.audio,
        &paths.align,
        &paths.ass,
        &paths.metadata,
        &paths.lyrics,
    ] {
        if dir.exists() {
            fs::remove_dir_all(dir)?;
        }
    }
    if drop_source && paths.raw.exists() {
        fs::remove_dir_all(&paths.raw)?
    }
    Ok(())
}
fn qa(paths: &Paths) -> Result<()> {
    for song in songs(paths)? {
        let ass = paths.ass.join(format!("{}.ass", song.id));
        let out = paths.output.join(format!("{}.mp4", song.id));
        println!("{}: ass={} output={}", song.id, ass.exists(), out.exists());
    }
    Ok(())
}
fn selftest(paths: &Paths) -> Result<()> {
    let song = Song {
        id: "selftest".into(),
        title: "PolyKara selftest".into(),
        artist: "PolyKara".into(),
        language: "en".into(),
        ..Song::default()
    };
    let entries = vec![Entry {
        start: 20_000,
        end: 24_000,
        text: "Hello world".into(),
        words: vec![
            model::Word {
                start: 20_000,
                end: 22_000,
                text: "Hello ".into(),
            },
            model::Word {
                start: 22_000,
                end: 24_000,
                text: "world".into(),
            },
        ],
    }];
    let text = ass::document(&song, &entries, &config::load_config(paths));
    let target = paths.output.join("selftest.ass");
    fs::write(&target, text)?;
    println!("selftest wrote {}", target.display());
    Ok(())
}
fn run(program: &str, args: &[&str]) -> Result<()> {
    let status = Command::new(program)
        .args(args)
        .status()
        .with_context(|| format!("could not start {}", program))?;
    if !status.success() {
        return Err(anyhow::anyhow!("{} failed with {}", program, status));
    }
    Ok(())
}
