//! Where PolyKara keeps things, its settings, and running the tools it needs.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex, RwLock};
use std::time::SystemTime;

use crate::toml::{self, Item};

pub const MEDIA_SUFFIXES: &[&str] = &[".mp4", ".mkv", ".webm", ".mov", ".m4v", ".flv", ".avi", ".ts", ".mpg", ".mpeg", ".vob", ".wmv", ".mp3", ".m4a", ".flac", ".wav", ".ogg", ".opus", ".aac", ".wma"];
pub const LONG_PAUSE_MS: i64 = 30_000;
pub const DOT_INTERVAL_MS: i64 = 1_000;
pub const MAX_SINGERS: usize = 16;
pub const SPEAKER_PALETTE: [&str; 16] = [
    "&H00B4771F", "&H000E7FFF", "&H002CA02C", "&H002827D6", "&H00BD6794", "&H004B568C", "&H00C277E3", "&H007F7F7F",
    "&H0022BDBC", "&H00CFBE17", "&H00808000", "&H0020A5DA", "&H00616FFF", "&H0002DEA4", "&H0082004B", "&H00C000C0",
];

/// Every setting PolyKara knows, with its default. polykara.toml overrides any of them.
pub const DEFAULT_CONFIG: &str = r#"
[video]
play_res_x = 1920
play_res_y = 1080
extend_intro = true

[render]
video_codec = "libx264"
preset = "medium"
crf = 18
pixel_format = "yuv420p"
audio_codec = "aac"
audio_bitrate = "192k"
extra_args = []
fit = "blur"
blur_sigma = 30
background_color = "black"

[logo]
file = ""
width = 220
position = "top-right"
margin_x = 40
margin_y = 40
opacity = 1.0

[title]
enabled = true
text = "{title}  |  {artist}"
font = "Arial"
size = 42
color = "&H00FFFFFF"
horizontal = "center"
vertical = "top"
margin_l = 40
margin_r = 40
margin_v = 40
start_ms = 0
duration_ms = 6000
fade_in_ms = 300
fade_out_ms = 300
bold = true
italic = false
outline = 2
shadow = 1

[credit]
enabled = true
text = "作词：{lyricist}    作曲：{composer}    字幕制作：{producer}"
font = "Arial"
size = 28
color = "&H00FFFFFF"
horizontal = "center"
vertical = "bottom"
margin_l = 40
margin_r = 40
margin_v = 70
start_ms = 0
duration_ms = 6000
fade_in_ms = 300
fade_out_ms = 300
bold = false
italic = false
outline = 2
shadow = 1

[lyric]
font = "Arial"
size = 58
color = "&H00FF0000"
secondary_color = "&H00FFFFFF"
outline_color = "&H80000000"
back_color = "&H50000000"
alignment = 2
margin_l = 80
margin_r = 80
margin_v = 150
bold = true
italic = false
outline = 3
shadow = 1

[karaoke]
active_color = "&H00FF0000"
inactive_color = "&H00FFFFFF"
tag = "kf"

[romanization]
enabled = true
languages = ["zh", "yue", "ja", "ko", "hi", "ta", "bn", "gu", "kn", "ml", "mr", "ne", "pa", "sa", "te", "or"]
font = "Arial"
size = 30
color = "&H00FFFFFF"
outline_color = "&H80000000"
back_color = "&H50000000"
horizontal = "center"
vertical = "bottom"
margin_l = 80
margin_r = 80
margin_v = 225
bold = false
italic = false
outline = 2
shadow = 1
tone = "numbers"

[lyrics]
sources = ["lrclib", "lyrics.ovh", "webpage"]

[alignment]
model = "auto"
reserve_memory_gib = 2
base_min_budget_gib = 2
small_min_budget_gib = 4
medium_min_budget_gib = 8
large_min_budget_gib = 12
cpu_threads = 0

[timing]
long_pause_ms = 30000
dot_interval_ms = 1000

[singers]
max = 16

[prompt]
font = "Arial"
size = 44
color = "&H00FFFFFF"
alignment = 1
margin_l = 80
margin_r = 80
margin_v = 220
bold = true
italic = false
outline = 2
shadow = 1

[watermark]
enabled = false
text = "PolyKara"
font = "Arial"
size = 24
color = "&H80FFFFFF"
alignment = 9
margin_l = 40
margin_r = 40
margin_v = 40
bold = false
italic = false
outline = 1
shadow = 0

[charts]
enabled_regions = ["my", "id", "au", "ca", "sg", "tw", "hk", "jp", "kr", "in", "cn", "us", "gb"]

[layout]
mode = "single"
lead_in_ms = 1500
top_alignment = 1
bottom_alignment = 3

[effects]
preset = "none"

[effects.params]

[live]
target = "/dev/video10"
audio_output = "none"
audio_device = "default"

[player]
library = ["~/Music", "work/output"]
"#;

/// The folders PolyKara uses.
#[derive(Clone, Debug)]
pub struct Paths {
    /// The user's PolyKara folder: songs.csv, polykara.toml and work\.
    pub home: PathBuf,
    /// The program's own files: bin\, data\, web\, readings\, defaults\.
    pub root: PathBuf,
    pub manifest: PathBuf,
    pub config: PathBuf,
    pub work: PathBuf,
    pub raw: PathBuf,
    pub audio: PathBuf,
    pub subtitles: PathBuf,
    pub align: PathBuf,
    pub ass: PathBuf,
    pub output: PathBuf,
    pub metadata: PathBuf,
    pub lyrics: PathBuf,
    pub models: PathBuf,
}

impl Paths {
    pub fn new(home: PathBuf, root: PathBuf) -> Paths {
        let work = home.join("work");
        Paths {
            manifest: home.join("songs.csv"),
            config: home.join("polykara.toml"),
            raw: work.join("raw"),
            audio: work.join("audio"),
            subtitles: work.join("subtitles"),
            align: work.join("align"),
            ass: work.join("ass"),
            output: work.join("output"),
            metadata: work.join("metadata"),
            lyrics: work.join("lyrics"),
            models: home.join("models"),
            work,
            home,
            root,
        }
    }
}

static PATHS: RwLock<Option<Arc<Paths>>> = RwLock::new(None);
static TEST_LOCK: Mutex<()> = Mutex::new(());

fn user_home() -> PathBuf {
    std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME")).map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."))
}

/// "~/Music" with the user's home folder put in.
pub fn expand_user(value: &str) -> PathBuf {
    match value.strip_prefix('~') {
        Some(rest) => user_home().join(rest.trim_start_matches(['/', '\\'])),
        None => PathBuf::from(value),
    }
}

/// The folder with the program's own files: next to the program when installed, else the checkout.
fn find_root() -> (PathBuf, bool) {
    if let Ok(exe) = std::env::current_exe() {
        if let Some(folder) = exe.parent() {
            if folder.join("defaults").is_dir() {
                return (folder.to_path_buf(), true);
            }
            for place in folder.ancestors() {
                if place.join("Cargo.toml").is_file() && place.join("polykara.toml").is_file() {
                    return (place.to_path_buf(), false);
                }
            }
        }
    }
    if let Ok(here) = std::env::current_dir() {
        for place in here.ancestors() {
            if place.join("Cargo.toml").is_file() && place.join("polykara.toml").is_file() {
                return (place.to_path_buf(), false);
            }
        }
        return (here, false);
    }
    (PathBuf::from("."), false)
}

/// The folders, worked out once: POLYKARA_HOME, else Documents\PolyKara when installed, else the checkout.
pub fn paths() -> Arc<Paths> {
    if let Some(paths) = PATHS.read().unwrap().as_ref() {
        return paths.clone();
    }
    let (root, installed) = find_root();
    let home = match std::env::var("POLYKARA_HOME") {
        Ok(value) if !value.trim().is_empty() => expand_user(value.trim()),
        _ if installed => user_home().join("Documents").join("PolyKara"),
        _ => root.clone(),
    };
    let paths = Arc::new(Paths::new(home, root));
    *PATHS.write().unwrap() = Some(paths.clone());
    paths
}

/// Run `action` with PolyKara's folders in `home` (tests). Calls are serialised.
pub fn with_home<R>(home: &Path, action: impl FnOnce() -> R) -> R {
    let _guard = TEST_LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let root = find_root().0;
    let before = PATHS.read().unwrap().clone();
    *PATHS.write().unwrap() = Some(Arc::new(Paths::new(home.to_path_buf(), root)));
    clear_config_cache();
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(action));
    *PATHS.write().unwrap() = before;
    clear_config_cache();
    match result {
        Ok(value) => value,
        Err(panic) => std::panic::resume_unwind(panic),
    }
}

/// A path from songs.csv or the settings: relative ones are inside the PolyKara folder.
pub fn resolve(value: &str) -> PathBuf {
    let path = expand_user(value);
    if path.is_absolute() {
        path
    } else {
        paths().home.join(path)
    }
}

pub fn defaults() -> Item {
    toml::parse(DEFAULT_CONFIG).expect("the built-in settings are valid TOML")
}

/// Defaults with the user's file merged in.
pub fn merged(text: &str) -> Result<Item, String> {
    let mut config = defaults();
    toml::merge(&mut config, &toml::parse(text)?);
    Ok(config)
}

type Cached = Option<(PathBuf, Option<SystemTime>, Arc<Item>)>;
static CONFIG: Mutex<Cached> = Mutex::new(None);

pub fn clear_config_cache() {
    *CONFIG.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) = None;
}

/// The settings, reread when polykara.toml changes. A broken file is reported and the defaults are used.
pub fn load_config() -> Arc<Item> {
    let file = paths().config.clone();
    let stamp = std::fs::metadata(&file).and_then(|meta| meta.modified()).ok();
    let mut cache = CONFIG.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some((path, when, config)) = cache.as_ref() {
        if *path == file && *when == stamp {
            return config.clone();
        }
    }
    let config = match std::fs::read_to_string(&file) {
        Ok(text) => match merged(text.trim_start_matches('\u{feff}')) {
            Ok(config) => config,
            Err(problem) => {
                eprintln!("WARN polykara.toml: {problem}; using the default settings");
                defaults()
            }
        },
        Err(_) => defaults(),
    };
    let config = Arc::new(config);
    *cache = Some((file, stamp, config.clone()));
    config
}

pub fn enabled_regions() -> Vec<String> {
    load_config().get("charts").get("enabled_regions").items().iter().map(|item| item.text().to_lowercase()).collect()
}

fn exe_name(name: &str) -> String {
    if cfg!(windows) && !name.to_lowercase().ends_with(".exe") {
        format!("{name}.exe")
    } else {
        name.to_string()
    }
}

/// Where a tool is: the program's bin folder first, then PATH. None when it is not installed.
pub fn find_tool(name: &str) -> Option<PathBuf> {
    let file = exe_name(name);
    let bundled = paths().root.join("bin").join(&file);
    if bundled.is_file() {
        return Some(bundled);
    }
    let search = std::env::var_os("PATH")?;
    std::env::split_paths(&search).map(|folder| folder.join(&file)).find(|candidate| candidate.is_file())
}

/// The tool to run: its full path when found, else its bare name (so the error names it).
pub fn tool(name: &str) -> PathBuf {
    find_tool(name).unwrap_or_else(|| PathBuf::from(exe_name(name)))
}

/// A command for a tool that never opens a console window of its own.
pub fn command(name: &str) -> Command {
    let mut command = Command::new(tool(name));
    crate::tools::quiet(&mut command);
    command
}

pub fn shown(program: &str, args: &[String]) -> String {
    std::iter::once(program.to_string()).chain(args.iter().cloned()).collect::<Vec<_>>().join(" ")
}

/// Run a tool, printing the command first; an error names the tool and its exit status.
pub fn run(name: &str, args: &[String], dry_run: bool, folder: Option<&Path>) -> Result<(), String> {
    println!("$ {}", shown(name, args));
    if dry_run {
        return Ok(());
    }
    let mut command = command(name);
    command.args(args).stdin(Stdio::null());
    if let Some(folder) = folder {
        command.current_dir(folder);
    }
    let status = command.status().map_err(|error| format!("{name} could not be started ({error})"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("{name} exited with status {}", status.code().unwrap_or(-1)))
    }
}

/// A tool's output (stdout), or an error when it cannot be started.
pub fn output(name: &str, args: &[&str]) -> Result<std::process::Output, String> {
    let mut command = command(name);
    command.args(args).stdin(Stdio::null());
    command.output().map_err(|error| format!("{name} could not be started ({error})"))
}

/// yt-dlp must be from 2025 or later to download from YouTube today.
pub fn require_current_ytdlp() -> Result<(), String> {
    let version = output("yt-dlp", &["--version"]).map(|output| String::from_utf8_lossy(&output.stdout).trim().to_string()).unwrap_or_default();
    let parts: Vec<&str> = version.split('.').collect();
    let date = if parts.len() >= 3 && parts.iter().take(3).all(|part| !part.is_empty() && part.chars().all(|c| c.is_ascii_digit())) {
        format!("{}{:0>2}{:0>2}", parts[0], parts[1], parts[2]).parse::<u64>().unwrap_or(0)
    } else {
        0
    };
    if date < 20250101 {
        return Err(format!("yt-dlp {} is too old for current YouTube. Update yt-dlp, then rerun this command.", if version.is_empty() { "unknown" } else { &version }));
    }
    Ok(())
}

/// Installed memory in bytes (0 when unknown).
pub fn total_memory_bytes() -> u64 {
    crate::tools::total_memory()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_parse_and_merge() {
        let config = merged("[lyric]\nsize = 60\n[effects.params]\nscale = 130\n").unwrap();
        assert_eq!(config.get("lyric").int_or("size", 0), 60);
        assert_eq!(config.get("lyric").str_or("font", ""), "Arial");
        assert_eq!(config.get("effects").get("params").int_or("scale", 0), 130);
        assert_eq!(config.get("video").int_or("play_res_x", 0), 1920);
        assert!(merged("[broken").is_err());
    }

    #[test]
    fn relative_paths_are_in_the_polykara_folder() {
        let home = std::env::temp_dir().join("polykara-config-test");
        with_home(&home, || {
            assert_eq!(resolve("songs/a.lrc"), home.join("songs/a.lrc"));
            assert_eq!(paths().lyrics, home.join("work").join("lyrics"));
        });
    }
}
