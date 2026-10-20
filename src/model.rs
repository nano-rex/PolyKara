use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct Word {
    pub start: i64,
    pub end: i64,
    pub text: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct Entry {
    pub start: i64,
    pub end: i64,
    pub text: String,
    pub words: Vec<Word>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Song {
    pub id: String,
    pub url: String,
    pub title: String,
    pub artist: String,
    pub lyricist: String,
    pub composer: String,
    pub producer: String,
    pub language: String,
    pub voice: String,
    pub lyrics_file: String,
    pub word_timing_file: String,
    pub subtitle_langs: String,
    pub logo_file: String,
}
