//! Regional YouTube Top Songs, added to songs.csv (charts.py).
//!
//! The chart endpoint answers with a private nesting of "trackViews"; the rows
//! are found wherever they sit, so a layout change does not break the picker.

use std::collections::HashSet;
use std::io::Write;

use crate::config::{paths, tool};
use crate::csv::Row;
use crate::json::Value;
use crate::manifest;

pub const REGIONS: &[(&str, &str, &str, &str)] = &[
    ("my", "Malaysia", "ms", "en.*"),
    ("id", "Indonesia", "id", "id,en.*"),
    ("au", "Australia", "en", "en.*"),
    ("ca", "Canada", "en", "en.*"),
    ("sg", "Singapore", "en", "en.*,zh.*,ms.*"),
    ("tw", "Taiwan", "zh", "zh.*,en.*"),
    ("hk", "Hong Kong", "yue", "yue,zh.*,en.*"),
    ("jp", "Japan", "ja", "ja,en.*"),
    ("kr", "South Korea", "ko", "ko,en.*"),
    ("in", "India", "hi", "hi,en.*"),
    ("cn", "China", "zh", "zh.*,en.*"),
    ("us", "USA", "en", "en.*"),
    ("gb", "UK", "en", "en.*"),
];

const CHART_ENDPOINT: &str = "https://charts.youtube.com/youtubei/v1/browse?alt=json&key=AIzaSyCzEW7JUJdSql0-2V4tHUb6laYm4iAE_dM";

pub fn region(code: &str) -> Option<(&'static str, &'static str, &'static str)> {
    REGIONS.iter().find(|item| item.0 == code).map(|item| (item.1, item.2, item.3))
}

/// The first "trackViews" list anywhere in the response.
fn track_views(value: &Value) -> &[Value] {
    match value {
        Value::Object(pairs) => {
            for (key, child) in pairs {
                if key == "trackViews" {
                    if let Value::Array(items) = child {
                        return items;
                    }
                }
            }
            for (_, child) in pairs {
                let found = track_views(child);
                if !found.is_empty() {
                    return found;
                }
            }
            &[]
        }
        Value::Array(items) => {
            for child in items {
                let found = track_views(child);
                if !found.is_empty() {
                    return found;
                }
            }
            &[]
        }
        _ => &[],
    }
}

pub struct ChartSong {
    pub rank: usize,
    pub title: String,
    pub artist: String,
    pub url: String,
}

/// One region's chart, in rank order.
pub fn chart_rows(code: &str) -> Result<Vec<ChartSong>, String> {
    if region(code).is_none() {
        return Err(format!("Unknown region {code}; choose from {}", REGIONS.iter().map(|item| item.0).collect::<Vec<_>>().join(", ")));
    }
    let body = Value::object(vec![
        ("browseId", Value::from("FEmusic_analytics_charts_home")),
        (
            "context",
            Value::object(vec![
                ("capabilities", Value::object(Vec::<(String, Value)>::new())),
                (
                    "client",
                    Value::object(vec![
                        ("clientName", Value::from("WEB_MUSIC_ANALYTICS")),
                        ("clientVersion", Value::from("0.2")),
                        ("experimentIds", Value::Array(Vec::new())),
                        ("experimentsToken", Value::from("")),
                        ("gl", Value::from(code.to_uppercase())),
                        ("hl", Value::from("en")),
                        ("theme", Value::from("MUSIC")),
                    ]),
                ),
                ("request", Value::object(vec![("internalExperimentFlags", Value::Array(Vec::new()))])),
            ]),
        ),
        (
            "query",
            Value::from(format!(
                "chart_params_type=WEEK&perspective=CHART&flags=viral_video_chart&selected_chart=TRACKS&chart_params_id=weekly:0:0:{code}"
            )),
        ),
    ])
    .dump();
    let mut command = crate::config::command("curl");
    command
        .args(["-sS", "-L", "--compressed", "--max-time", "20", "-H", "Content-Type: application/json", "-A", "PolyKara/1.0", "--data-binary", "@-", CHART_ENDPOINT])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    let mut child = command.spawn().map_err(|error| format!("{} could not be started ({error})", tool("curl").display()))?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin.write_all(body.as_bytes()).map_err(|error| error.to_string())?;
    }
    let output = child.wait_with_output().map_err(|error| error.to_string())?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_string());
    }
    let data = Value::parse(&String::from_utf8_lossy(&output.stdout)).map_err(|error| format!("YouTube chart answer is not JSON ({error})"))?;
    let items = track_views(&data);
    if items.is_empty() {
        let keys: Vec<&str> = match &data {
            Value::Object(pairs) => pairs.iter().map(|(key, _)| key.as_str()).collect(),
            _ => vec![],
        };
        return Err(format!("YouTube chart response has no trackViews (top-level: {})", keys.join(", ")));
    }
    let mut found = Vec::new();
    for (number, item) in items.iter().take(10).enumerate() {
        let video = ["encryptedVideoId", "id", "videoId"].iter().map(|key| item.get(key).str().to_string()).find(|value| !value.is_empty()).unwrap_or_default();
        let title = ["title", "name"].iter().map(|key| item.get(key).str().to_string()).find(|value| !value.is_empty()).unwrap_or_default();
        let artist = match item.get("artists") {
            Value::String(text) => text.clone(),
            Value::Array(items) => items.iter().filter_map(|artist| artist.get("name").as_str()).filter(|name| !name.is_empty()).collect::<Vec<_>>().join(", "),
            _ => String::new(),
        };
        if !video.is_empty() && !title.is_empty() {
            found.push(ChartSong { rank: number + 1, title, artist: if artist.is_empty() { "Unknown Artist".into() } else { artist }, url: format!("https://www.youtube.com/watch?v={video}") });
        }
    }
    Ok(found)
}

/// A songs.csv id from a title.
pub fn slug(value: &str) -> String {
    let mut text = String::new();
    let mut dash = false;
    for character in value.chars() {
        if character.is_ascii_alphanumeric() {
            text.push(character.to_ascii_lowercase());
            dash = false;
        } else if !dash && !text.is_empty() {
            text.push('-');
            dash = true;
        }
    }
    let text = text.trim_matches('-').to_string();
    if text.is_empty() { "song".into() } else { text }
}

/// Fetch the charts and add the chosen songs (all of them without `pick`).
pub fn add_selected(regions: &[String], pick: bool) -> Result<(), String> {
    let unknown: Vec<&str> = regions.iter().map(String::as_str).filter(|code| region(code).is_none()).collect();
    if !unknown.is_empty() {
        return Err(format!("Unknown region code(s): {}; choose from {}", unknown.join(", "), REGIONS.iter().map(|item| item.0).collect::<Vec<_>>().join(", ")));
    }
    let mut candidates: Vec<(&str, ChartSong)> = Vec::new();
    for code in regions {
        let (name, _, _) = region(code).expect("checked above");
        println!("\n== {name} ==");
        match chart_rows(code) {
            Ok(found) => {
                for item in found {
                    println!("{:02}. #{} {} — {}\n    {}", candidates.len() + 1, item.rank, item.title, item.artist, item.url);
                    candidates.push((code, item));
                }
            }
            Err(problem) => println!("SKIP {code}: unable to load chart ({problem})"),
        }
    }
    if candidates.is_empty() {
        println!("No chart candidates found");
        return Ok(());
    }
    let selected: Vec<usize> = if pick {
        print!("\nSelect numbers to add (comma-separated), 'a' for all, or Enter to cancel: ");
        let _ = std::io::stdout().flush();
        let mut answer = String::new();
        std::io::stdin().read_line(&mut answer).map_err(|error| error.to_string())?;
        let answer = answer.trim().to_lowercase();
        if answer.is_empty() {
            println!("No songs added");
            return Ok(());
        }
        if answer == "a" {
            (1..=candidates.len()).collect()
        } else {
            answer.split(',').filter_map(|value| value.trim().parse::<usize>().ok()).filter(|value| (1..=candidates.len()).contains(value)).collect()
        }
    } else {
        println!("\nAutomatic mode: adding all listed chart songs");
        (1..=candidates.len()).collect()
    };
    if selected.is_empty() {
        println!("No valid selections");
        return Ok(());
    }
    let manifest = paths().manifest.clone();
    let (fields, mut songs) = if manifest.is_file() {
        let text = std::fs::read_to_string(&manifest).map_err(|error| error.to_string())?;
        crate::csv::read(&text)
    } else {
        (Vec::new(), Vec::new())
    };
    let mut urls: HashSet<String> = songs.iter().map(|row| manifest::get(row, "url").trim().to_string()).collect();
    let mut ids: HashSet<String> = songs.iter().map(|row| manifest::get(row, "id").trim().to_string()).collect();
    let mut added = 0;
    for number in selected {
        let (code, item) = &candidates[number - 1];
        if urls.contains(&item.url) {
            println!("SKIP existing URL: {}", item.title);
            continue;
        }
        let base = slug(&format!("{}-{}", item.artist, item.title));
        let mut sid = base.clone();
        let mut suffix = 2;
        while ids.contains(&sid) {
            sid = format!("{base}-{suffix}");
            suffix += 1;
        }
        let (_, language, subtitle_langs) = region(code).expect("checked above");
        let mut row: Row = fields.iter().map(|name| (name.clone(), String::new())).collect();
        for (key, value) in [("id", sid.clone()), ("url", item.url.clone()), ("title", item.title.clone()), ("artist", item.artist.clone()), ("template", "solo".into()), ("language", language.into()), ("subtitle_langs", subtitle_langs.into()), ("prompt_ms", "1000".into()), ("align_model", "auto".into())] {
            row.insert(key.to_string(), value);
        }
        println!("ADD {sid}: {} — {}", item.title, item.artist);
        urls.insert(item.url.clone());
        ids.insert(sid);
        songs.push(row);
        added += 1;
    }
    if added > 0 {
        manifest::write_songs(&fields, &songs)?;
        println!("Added {added} song(s) to songs.csv. Run `polykara-cli download` to fetch them.");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_plain() {
        assert_eq!(slug("Beyoncé — TEXAS HOLD 'EM"), "beyonc-texas-hold-em");
        assert_eq!(slug("!!!"), "song");
    }

    #[test]
    fn chart_rows_are_found_wherever_they_sit() {
        let data = Value::parse(r#"{"a":{"b":[{"trackViews":[{"encryptedVideoId":"x1","title":"One","artists":[{"name":"A"},{"name":"B"}]},{"id":"x2","name":"Two"}]}]}}"#).unwrap();
        let items = track_views(&data);
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].get("title").str(), "One");
        assert_eq!(items[1].get("id").str(), "x2");
    }

    #[test]
    fn regions_have_language_and_subtitles() {
        assert_eq!(region("hk"), Some(("Hong Kong", "yue", "yue,zh.*,en.*")));
        assert_eq!(region("zz"), None);
    }
}
