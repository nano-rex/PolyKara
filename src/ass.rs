use crate::config::{boolean, int, string};
use crate::model::{Entry, Song};

pub fn esc(text: &str) -> String {
    text.replace('{', r"\{")
        .replace('}', r"\}")
        .replace('\\', "/")
}
pub fn at(ms: i64) -> String {
    format!(
        "{}:{:02}:{:02}.{:02}",
        ms / 3_600_000,
        (ms / 60_000) % 60,
        (ms / 1000) % 60,
        (ms % 1000) / 10
    )
}
pub fn karaoke_text(words: &[crate::model::Word], start: i64, tag: &str) -> String {
    if words.is_empty() {
        return format!("{{\\kf{}}}{}", ((1000.max(0)) / 10), esc(""));
    }
    let mut out = String::new();
    let mut cursor = start;
    for word in words {
        if word.start > cursor {
            out.push_str(&format!("{{\\k{}}}", (word.start - cursor) / 10));
        }
        out.push('{');
        out.push('\\');
        out.push_str(tag);
        out.push_str(&((word.end - word.start).max(0) / 10).to_string());
        out.push('}');
        out.push_str(&esc(&word.text));
        cursor = word.end;
    }
    out
}
pub fn document(_song: &Song, entries: &[Entry], config: &serde_json::Value) -> String {
    let res_x = int(config, "video", "play_res_x", 1920);
    let res_y = int(config, "video", "play_res_y", 1080);
    let mut out=format!("[Script Info]\n; PolyKaraIntroPaddingMs: 0\nScriptType: v4.00+\nPlayResX: {}\nPlayResY: {}\n\n[V4+ Styles]\nFormat: Name, Fontname, Fontsize, PrimaryColour, SecondaryColour, OutlineColour, BackColour, Bold, Italic, Alignment, MarginL, MarginR, MarginV, Encoding\nStyle: Lyric,{},{} ,{},&H00FFFFFF,&H80000000,&H50000000,{},0,2,80,80,150,1\n\n[Events]\nFormat: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\n",res_x,res_y,string(config,"lyric","font","Arial"),int(config,"lyric","size",58),string(config,"lyric","color","&H00FF0000"),if boolean(config,"lyric","bold",true){-1}else{0});
    for entry in entries {
        let words = if entry.words.is_empty() {
            vec![crate::model::Word {
                start: entry.start,
                end: entry.end,
                text: entry.text.clone(),
            }]
        } else {
            entry.words.clone()
        };
        out.push_str(&format!(
            "Dialogue: 0,{}, {},Lyric,,0,0,0,,{}\n",
            at(entry.start),
            at(entry.end),
            karaoke_text(&words, entry.start, "kf")
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Word;

    #[test]
    fn karaoke_wait_and_gaps_are_preserved() {
        let words = vec![
            Word {
                start: 10_500,
                end: 11_000,
                text: "Amazing ".into(),
            },
            Word {
                start: 11_400,
                end: 12_000,
                text: "grace".into(),
            },
        ];
        assert_eq!(
            karaoke_text(&words, 10_000, "kf"),
            r"{\k50}{\kf50}Amazing {\k40}{\kf60}grace"
        );
    }

    #[test]
    fn ass_contains_one_dialogue_per_entry() {
        let song = Song {
            id: "demo".into(),
            title: "Demo".into(),
            language: "en".into(),
            ..Song::default()
        };
        let entries = vec![Entry {
            start: 20_000,
            end: 24_000,
            text: "Hello".into(),
            words: vec![Word {
                start: 20_000,
                end: 24_000,
                text: "Hello".into(),
            }],
        }];
        let text = document(
            &song,
            &entries,
            &crate::config::load_config(&crate::config::Paths::discover()),
        );
        assert_eq!(
            text.lines()
                .filter(|line| line.starts_with("Dialogue:"))
                .count(),
            1
        );
        assert!(text.contains(r"{\kf400}Hello"));
    }
}
