use anyhow::Result;
use serde_json::{Map, Value};
use std::env;
use std::fs;
use std::path::PathBuf;

#[derive(Clone, Debug)]
pub struct Paths {
    pub root: PathBuf,
    pub home: PathBuf,
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
}

impl Paths {
    pub fn discover() -> Self {
        let root = env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        let home = env::var_os("POLYKARA_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| root.clone());
        let work = home.join("work");
        Self {
            root,
            home: home.clone(),
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
            work,
        }
    }

    pub fn ensure_work(&self) -> Result<()> {
        for path in [
            &self.home,
            &self.work,
            &self.raw,
            &self.audio,
            &self.subtitles,
            &self.align,
            &self.ass,
            &self.output,
            &self.metadata,
            &self.lyrics,
        ] {
            fs::create_dir_all(path)?;
        }
        Ok(())
    }
}

pub fn load_config(paths: &Paths) -> Value {
    let mut root = default_config();
    if let Ok(text) = fs::read_to_string(&paths.config) {
        if let Ok(value) = text.parse::<toml::Table>() {
            merge(&mut root, toml_to_json(value));
        }
    }
    root
}

fn merge(base: &mut Value, overlay: Value) {
    if let (Some(base), Some(overlay)) = (base.as_object_mut(), overlay.as_object()) {
        for (key, value) in overlay {
            if base.get(key).map(Value::is_object).unwrap_or(false) && value.is_object() {
                merge(base.get_mut(key).unwrap(), value.clone());
            } else {
                base.insert(key.clone(), value.clone());
            }
        }
    }
}

fn toml_to_json(table: toml::Table) -> Value {
    Value::Object(table.into_iter().map(|(k, v)| (k, toml_value(v))).collect())
}
fn toml_value(value: toml::Value) -> Value {
    match value {
        toml::Value::String(v) => Value::String(v),
        toml::Value::Integer(v) => Value::Number(v.into()),
        toml::Value::Float(v) => serde_json::Number::from_f64(v)
            .map(Value::Number)
            .unwrap_or(Value::Null),
        toml::Value::Boolean(v) => Value::Bool(v),
        toml::Value::Datetime(v) => Value::String(v.to_string()),
        toml::Value::Array(v) => Value::Array(v.into_iter().map(toml_value).collect()),
        toml::Value::Table(v) => toml_to_json(v),
    }
}

fn default_config() -> Value {
    let mut root = Map::new();
    root.insert(
        "video".into(),
        serde_json::json!({"play_res_x": 1920, "play_res_y": 1080, "extend_intro": true}),
    );
    root.insert("title".into(), serde_json::json!({"enabled": true, "text": "{title}  |  {artist}", "font": "Arial", "size": 42, "duration_ms": 6000, "fade_in_ms": 300, "fade_out_ms": 300, "horizontal": "center", "vertical": "top", "margin_l": 40, "margin_r": 40, "margin_v": 40, "bold": true, "outline": 2, "shadow": 1}));
    root.insert("credit".into(), serde_json::json!({"enabled": true, "text": "作词：{lyricist}    作曲：{composer}    字幕制作：{producer}", "font": "Arial", "size": 28, "duration_ms": 6000, "fade_in_ms": 300, "fade_out_ms": 300, "horizontal": "center", "vertical": "bottom", "margin_l": 40, "margin_r": 40, "margin_v": 70, "outline": 2, "shadow": 1}));
    root.insert("lyric".into(), serde_json::json!({"font": "Arial", "size": 58, "color": "&H00FF0000", "secondary_color": "&H00FFFFFF", "outline_color": "&H80000000", "back_color": "&H50000000", "alignment": 2, "margin_l": 80, "margin_r": 80, "margin_v": 150, "bold": true, "outline": 3, "shadow": 1}));
    root.insert("layout".into(), serde_json::json!({"mode": "single", "lead_in_ms": 1500, "top_alignment": 1, "bottom_alignment": 3}));
    root.insert(
        "timing".into(),
        serde_json::json!({"long_pause_ms": 30000, "dot_interval_ms": 1000}),
    );
    root.insert(
        "lyrics".into(),
        serde_json::json!({"sources": ["lrclib", "lyrics.ovh", "webpage"]}),
    );
    Value::Object(root)
}

pub fn value<'a>(config: &'a Value, section: &str, key: &str) -> Option<&'a Value> {
    config.get(section)?.get(key)
}
pub fn string(config: &Value, section: &str, key: &str, fallback: &str) -> String {
    value(config, section, key)
        .and_then(Value::as_str)
        .unwrap_or(fallback)
        .to_string()
}
pub fn int(config: &Value, section: &str, key: &str, fallback: i64) -> i64 {
    value(config, section, key)
        .and_then(Value::as_i64)
        .unwrap_or(fallback)
}
pub fn boolean(config: &Value, section: &str, key: &str, fallback: bool) -> bool {
    value(config, section, key)
        .and_then(Value::as_bool)
        .unwrap_or(fallback)
}
