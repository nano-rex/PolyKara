//! Songs tab: the song list, the song dialog, and running the pipeline on the songs.

use std::collections::HashSet;

use polykara::json::Value;

use crate::app::{Shared, Song, Want, DIALOG_CLASS, IDCANCEL, IDOK};
use crate::win::{self, ffi::*};

/// (label, pipeline command, takes song ids)
const ACTIONS: &[(&str, &str, bool)] = &[
    ("Make videos (all steps)", "process", true),
    ("Download only", "download", true),
    ("Make subtitles only", "lyrics", true),
    ("Render videos only", "render", true),
    ("Check lyrics accuracy", "verify", true),
    ("Status report", "qa", true),
    ("Check tools", "check", false),
    ("Self-test", "selftest", false),
];
const LANGUAGES: &[&str] = &["en", "zh", "yue", "ja", "ko", "ms", "id", "th", "vi", "tl", "hi", "ta", "es", "fr", "de"];
const VOICES: &[&str] = &["", "female", "male", "duet"];
const DEFAULTS: &[(&str, &str)] = &[("template", "solo"), ("prompt_ms", "1000"), ("align_model", "auto")];
const COLUMNS: &[(&str, i32)] = &[("Song id", 190), ("Title", 300), ("Singer", 220), ("Language", 80), ("Voice", 80), ("Status", 150)];

const ID_ADD: u16 = 101;
const ID_ADD_FILES: u16 = 102;
const ID_EDIT: u16 = 103;
const ID_REMOVE: u16 = 104;
const ID_TIME: u16 = 105;
const ID_EDIT_ASS: u16 = 106;
const ID_PLAY: u16 = 107;
const ID_VIDEOS: u16 = 108;
const ID_DATA: u16 = 109;
const ID_YTDLP: u16 = 110;
const ID_LIST: u16 = 120;
const ID_ACTION: u16 = 130;
const ID_SELECTED_ONLY: u16 = 131;
const ID_REPROCESS: u16 = 132;
const ID_REALIGN: u16 = 133;
const ID_RUN: u16 = 134;
const ID_STOP: u16 = 135;
const ID_LOG: u16 = 140;
const ID_FIELD: u16 = 1000;
const ID_BROWSE: u16 = 1100;

enum Kind {
    Text,
    Choices(&'static [&'static str]),
    File,
}

const FIELDS: &[(&str, &str, Kind)] = &[
    ("Video URL or media file", "url", Kind::File),
    ("Title", "title", Kind::Text),
    ("Singer", "artist", Kind::Text),
    ("Composer", "composer", Kind::Text),
    ("Lyricist", "lyricist", Kind::Text),
    ("Subtitles by", "producer", Kind::Text),
    ("Language", "language", Kind::Choices(LANGUAGES)),
    ("Voice (colour)", "voice", Kind::Choices(VOICES)),
    ("Lyrics file (optional)", "lyrics_file", Kind::File),
    ("Lyrics web pages (optional, | between)", "lyric_pages", Kind::Text),
    ("Logo image (optional)", "logo_file", Kind::File),
    ("Song id", "id", Kind::Text),
];

pub fn valid_id(sid: &str) -> bool {
    !sid.is_empty() && sid.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

pub enum Action {
    TimeLyrics(String),
}

struct SongDialog {
    hwnd: HWND,
    /// The song being edited (its id before editing), or None for a new song.
    editing: Option<String>,
    original: Song,
    fields: Vec<HWND>,
    taken: HashSet<String>,
}

impl SongDialog {
    fn open(song: Option<&Song>, shared: &Shared) -> SongDialog {
        let original = song.cloned().unwrap_or_default();
        let editing = song.and_then(|song| song.get("id").cloned());
        let taken = shared.songs.iter().filter_map(|song| song.get("id").cloned()).filter(|sid| Some(sid) != editing.as_ref()).collect();
        let px = |value: i32| shared.px(value);
        let (label_width, field_width, row) = (px(250), px(430), px(30));
        let width = px(12) + label_width + field_width + px(100) + px(28);
        let height = px(12) + row * FIELDS.len() as i32 + px(90) + px(40);
        let hwnd = win::top_window(DIALOG_CLASS, if editing.is_some() { "Edit song" } else { "Add song" }, WS_CAPTION | WS_SYSMENU | WS_CLIPCHILDREN, shared.main, width, height);
        let mut fields = Vec::new();
        for (index, (label, field, kind)) in FIELDS.iter().enumerate() {
            let y = px(12) + row * index as i32;
            let text = win::label(hwnd, label);
            win::set_font(text, shared.font);
            win::place(text, px(12), y, label_width - px(8), px(24));
            let value = original.get(*field).cloned().unwrap_or_default();
            let control = match kind {
                Kind::Choices(choices) => {
                    let combo = win::combo(hwnd, ID_FIELD + index as u16, choices, true);
                    win::set_text(combo, &value);
                    combo
                }
                _ => win::edit(hwnd, ID_FIELD + index as u16, &value, 0),
            };
            win::set_font(control, shared.font);
            win::place(control, px(12) + label_width, y, field_width, if matches!(kind, Kind::Choices(_)) { px(300) } else { px(24) });
            if let Kind::File = kind {
                let browse = win::button(hwnd, ID_BROWSE + index as u16, "Browse…");
                win::set_font(browse, shared.font);
                win::place(browse, px(18) + label_width + field_width, y, px(90), px(25));
            }
            fields.push(control);
        }
        let bottom = px(12) + row * FIELDS.len() as i32;
        let hint = win::label(hwnd, "Leave the song id empty to make one from the title.");
        win::set_font(hint, shared.font);
        win::place(hint, px(12) + label_width, bottom, field_width, px(22));
        let save = win::child("BUTTON", "Save", WS_TABSTOP | BS_DEFPUSHBUTTON, hwnd, IDOK);
        let cancel = win::button(hwnd, IDCANCEL, "Cancel");
        for (number, button) in [save, cancel].iter().enumerate() {
            win::set_font(*button, shared.font);
            win::place(*button, px(12) + label_width + field_width - px(196) + px(100) * number as i32, bottom + px(34), px(94), px(28));
        }
        win::enable(shared.main, false);
        win::show(hwnd, true);
        unsafe { SetFocus(fields[0]) };
        SongDialog { hwnd, editing, original, fields, taken }
    }

    fn index(field: &str) -> Option<usize> {
        FIELDS.iter().position(|(_, name, _)| *name == field)
    }

    fn value(&self, field: &str) -> String {
        Self::index(field).map(|index| win::text(self.fields[index]).trim().to_string()).unwrap_or_default()
    }

    fn set(&self, field: &str, value: &str) {
        if let Some(index) = Self::index(field) {
            win::set_text(self.fields[index], value);
        }
    }

    /// The finished song, or why it cannot be saved yet. Ok(None) means the id is being made.
    fn finish(&self, shared: &mut Shared) -> Result<Option<Song>, String> {
        if self.value("url").is_empty() {
            return Err("Enter the video URL, or choose a media file with Browse.".into());
        }
        if self.value("language").is_empty() {
            return Err("Choose the language the song is sung in.".into());
        }
        let sid = self.value("id");
        if sid.is_empty() {
            let taken: Vec<String> = self.taken.iter().cloned().collect();
            shared.engine.ask("new_id", Value::object(vec![("title", Value::from(self.value("title"))), ("taken", Value::from(taken))]), Want::NewId);
            return Ok(None);
        }
        if !valid_id(&sid) {
            return Err("The song id may only use letters, digits, - and _.".into());
        }
        if self.taken.contains(&sid) {
            return Err(format!("Another song already uses the id {sid}."));
        }
        let mut song: Song = DEFAULTS.iter().map(|(key, value)| (key.to_string(), value.to_string())).collect();
        song.extend(self.original.clone());
        for (_, field, _) in FIELDS {
            song.insert(field.to_string(), self.value(field));
        }
        Ok(Some(song))
    }

    fn close(&self, shared: &Shared) {
        win::enable(shared.main, true);
        unsafe {
            DestroyWindow(self.hwnd);
            SetForegroundWindow(shared.main);
        }
    }
}

pub struct SongsPage {
    controls: Vec<HWND>,
    toolbar: Vec<HWND>,
    list: HWND,
    action: HWND,
    selected_only: HWND,
    reprocess: HWND,
    realign: HWND,
    run: HWND,
    stop: HWND,
    log: HWND,
    /// The song id of each row of the table.
    rows: Vec<String>,
    dialog: Option<SongDialog>,
}

impl SongsPage {
    pub fn create(shared: &mut Shared) -> SongsPage {
        let main = shared.main;
        let names = [
            (ID_ADD, "Add song"),
            (ID_ADD_FILES, "Add files"),
            (ID_EDIT, "Edit"),
            (ID_REMOVE, "Remove"),
            (ID_TIME, "Time lyrics"),
            (ID_EDIT_ASS, "Edit ASS file"),
            (ID_PLAY, "Play video"),
            (ID_VIDEOS, "Open videos folder"),
            (ID_DATA, "Open data folder"),
            (ID_YTDLP, "Update yt-dlp"),
        ];
        let toolbar: Vec<HWND> = names.iter().map(|(id, name)| shared.styled(win::button(main, *id, name))).collect();
        let list = shared.styled(win::create(WS_EX_CLIENTEDGE, "SysListView32", "", WS_CHILD | WS_VISIBLE | WS_TABSTOP | LVS_REPORT | LVS_SHOWSELALWAYS, main, ID_LIST));
        win::list_columns(list, &COLUMNS.iter().map(|(name, width)| (*name, shared.px(*width))).collect::<Vec<_>>());
        let action = shared.styled(win::combo(main, ID_ACTION, &ACTIONS.iter().map(|(label, _, _)| *label).collect::<Vec<_>>(), false));
        win::select(action, Some(0));
        let selected_only = shared.styled(win::checkbox(main, ID_SELECTED_ONLY, "Selected songs only"));
        let reprocess = shared.styled(win::checkbox(main, ID_REPROCESS, "Redo finished work"));
        let realign = shared.styled(win::checkbox(main, ID_REALIGN, "Redo word timing"));
        let run = shared.styled(win::child("BUTTON", "Run", WS_TABSTOP | BS_DEFPUSHBUTTON, main, ID_RUN));
        let stop = shared.styled(win::button(main, ID_STOP, "Stop"));
        let log = win::multiline(main, ID_LOG, "", true);
        win::set_font(log, shared.mono);
        shared.log = log;
        let mut controls = toolbar.clone();
        controls.extend([list, action, selected_only, reprocess, realign, run, stop, log]);
        SongsPage { controls, toolbar, list, action, selected_only, reprocess, realign, run, stop, log, rows: Vec::new(), dialog: None }
    }

    pub fn show(&self, visible: bool) {
        for control in &self.controls {
            win::show(*control, visible);
        }
    }

    pub fn layout(&self, shared: &Shared, x: i32, y: i32, width: i32, height: i32) {
        let px = |value: i32| shared.px(value);
        let mut left = x;
        for (index, button) in self.toolbar.iter().enumerate() {
            let text = win::text(*button);
            let wide = win::measure_with(*button, shared.font, &text).0 + px(22);
            if index == 4 || index == 9 {
                left += px(12);
            }
            win::place(*button, left, y + px(2), wide, px(28));
            left += wide + px(4);
        }
        let log_height = (height * 38 / 100).max(px(120));
        let controls_top = y + height - log_height - px(36);
        let list_top = y + px(36);
        win::place(self.list, x, list_top, width, controls_top - list_top - px(6));
        // Title and singer share what the fixed columns leave.
        let fixed: i32 = [0usize, 3, 4, 5].iter().map(|index| px(COLUMNS[*index].1)).sum();
        let spare = (width - fixed - px(24)).max(px(260));
        win::list_width(self.list, 1, spare * 55 / 100);
        win::list_width(self.list, 2, spare * 45 / 100);
        win::place(self.action, x, controls_top + px(3), px(240), px(300));
        let mut left = x + px(252);
        for check in [self.selected_only, self.reprocess, self.realign] {
            let wide = win::measure_with(check, shared.font, &win::text(check)).0 + px(30);
            win::place(check, left, controls_top + px(4), wide, px(24));
            left += wide + px(8);
        }
        win::place(self.stop, x + width - px(90), controls_top + px(2), px(90), px(28));
        win::place(self.run, x + width - px(186), controls_top + px(2), px(90), px(28));
        win::place(self.log, x, controls_top + px(36), width, log_height);
    }

    pub fn dialog(&self) -> Option<HWND> {
        self.dialog.as_ref().map(|dialog| dialog.hwnd)
    }

    /// Fill the table from the song list, keeping the selection.
    pub fn fill(&mut self, shared: &Shared) {
        let chosen: HashSet<String> = self.selected();
        let songs = shared.visible_songs();
        self.rows = songs.iter().map(|song| song.get("id").cloned().unwrap_or_default()).collect();
        let cell = |song: &Song, key: &str| song.get(key).cloned().unwrap_or_default();
        let table: Vec<Vec<String>> = songs.iter().map(|song| {
            let sid = cell(song, "id");
            vec![sid.clone(), cell(song, "title"), cell(song, "artist"), cell(song, "language"), cell(song, "voice"), shared.state(&sid)]
        }).collect();
        win::list_fill(self.list, &table);
        self.select_ids(&chosen);
    }

    pub fn update_states(&self, shared: &Shared) {
        for (row, sid) in self.rows.iter().enumerate() {
            win::list_set(self.list, row, 5, &shared.state(sid));
        }
        // Keep the song being worked on in view.
        if let Some(row) = shared.current.as_ref().and_then(|sid| self.rows.iter().position(|item| item == sid)) {
            unsafe { SendMessageW(self.list, LVM_ENSUREVISIBLE, row, 0) };
        }
    }

    pub fn update_buttons(&self, shared: &Shared) {
        let running = shared.runner.running();
        win::enable(self.run, !running && shared.hello.is_some());
        win::enable(self.stop, running);
    }

    fn selected(&self) -> HashSet<String> {
        win::list_selected(self.list).into_iter().filter_map(|row| self.rows.get(row).cloned()).collect()
    }

    fn select_ids(&self, ids: &HashSet<String>) {
        let rows: Vec<usize> = self.rows.iter().enumerate().filter(|(_, sid)| ids.contains(*sid)).map(|(row, _)| row).collect();
        win::list_select(self.list, &rows);
    }

    fn one_selected(&self, shared: &Shared, what: &str) -> Option<String> {
        let chosen = self.selected();
        if chosen.len() == 1 {
            chosen.into_iter().next()
        } else {
            shared.info(what);
            None
        }
    }

    /// Songs added from media files: select them and say what to check.
    pub fn added(&mut self, value: &Value, chosen: usize, shared: &Shared) {
        let added = value.get("added").strings();
        self.select_ids(&added.iter().cloned().collect());
        let skipped = chosen.saturating_sub(added.len());
        let note = if skipped > 0 { format!("\n{skipped} file(s) were already in the list.") } else { String::new() };
        if added.is_empty() {
            shared.info(&format!("No songs were added.{note}"));
        } else {
            shared.info(&format!(
                "Added {} song(s). Title and singer were read from each file's tags or name and are used to search for lyrics.\n\nCheck the language, singer and voice of the new songs (double-click to edit) before making videos.{note}",
                added.len()
            ));
        }
    }

    pub fn answer(&mut self, want: Want, result: Result<Value, String>, shared: &mut Shared) {
        let dialog = match &self.dialog {
            Some(dialog) => dialog,
            None => return,
        };
        let value = match result {
            Ok(value) => value,
            Err(error) => {
                win::warn(dialog.hwnd, &error);
                return;
            }
        };
        match want {
            Want::Describe => {
                // Fill in what the file itself says, without overwriting anything typed already.
                for name in ["title", "artist", "language"] {
                    if dialog.value(name).is_empty() {
                        dialog.set(name, value.get(name).str());
                    }
                }
                dialog.set("url", value.get("url").str());
            }
            Want::Relative(field) => dialog.set(&field, value.get("path").str()),
            Want::NewId => {
                dialog.set("id", value.get("id").as_str().unwrap_or("song"));
                self.save_dialog(shared);
            }
            _ => {}
        }
    }

    fn save_dialog(&mut self, shared: &mut Shared) {
        let dialog = match &self.dialog {
            Some(dialog) => dialog,
            None => return,
        };
        match dialog.finish(shared) {
            Err(problem) => win::warn(dialog.hwnd, &problem),
            Ok(None) => {}
            Ok(Some(song)) => {
                let sid = song.get("id").cloned().unwrap_or_default();
                let index = dialog.editing.as_ref().and_then(|old| shared.songs.iter().position(|song| song.get("id") == Some(old)));
                match index {
                    Some(index) => shared.songs[index] = song,
                    None => shared.songs.push(song),
                }
                dialog.close(shared);
                self.dialog = None;
                shared.save_songs();
                self.fill(shared);
                shared.songs_changed = false;
                self.select_ids(&[sid].into_iter().collect());
            }
        }
    }

    pub fn dialog_message(&mut self, message: u32, wparam: WPARAM, _lparam: LPARAM, shared: &mut Shared) -> Option<LRESULT> {
        match message {
            WM_COMMAND => {
                let id = win::loword(wparam) as u16;
                if id == IDOK {
                    self.save_dialog(shared);
                } else if id == IDCANCEL {
                    if let Some(dialog) = self.dialog.take() {
                        dialog.close(shared);
                    }
                } else if (ID_BROWSE..ID_BROWSE + FIELDS.len() as u16).contains(&id) {
                    let field = FIELDS[(id - ID_BROWSE) as usize].1;
                    let owner = self.dialog.as_ref().map(|dialog| dialog.hwnd).unwrap_or(shared.main);
                    let home = shared.home();
                    let chosen = win::open_files(owner, "Choose a file", home.as_deref(), &[("All files", "*.*")], false);
                    if let Some(path) = chosen.into_iter().next() {
                        let path = Value::from(path.to_string_lossy().into_owned());
                        if field == "url" {
                            let taken: Vec<String> = self.dialog.as_ref().map(|dialog| dialog.taken.iter().cloned().collect()).unwrap_or_default();
                            shared.engine.ask("describe", Value::object(vec![("path", path), ("taken", Value::from(taken))]), Want::Describe);
                        } else {
                            shared.engine.ask("relative", Value::object(vec![("path", path)]), Want::Relative(field.to_string()));
                        }
                    }
                } else {
                    return None;
                }
                Some(0)
            }
            WM_CLOSE => {
                if let Some(dialog) = self.dialog.take() {
                    dialog.close(shared);
                }
                Some(0)
            }
            _ => None,
        }
    }

    fn run_action(&self, shared: &mut Shared) {
        let (_, command, takes_ids) = ACTIONS[win::selection(self.action).unwrap_or(0).min(ACTIONS.len() - 1)];
        let selected_only = win::checked(self.selected_only);
        let ids: Vec<String> = if selected_only {
            let chosen: Vec<String> = self.rows.iter().filter(|sid| self.selected().contains(*sid)).cloned().collect();
            if chosen.is_empty() {
                shared.info("Select the songs to work on, or untick 'Selected songs only'.");
                return;
            }
            chosen
        } else {
            self.rows.clone()
        };
        let mut args = vec![command.to_string()];
        if takes_ids && selected_only {
            args.extend(ids.iter().cloned());
        }
        if win::checked(self.reprocess) && ["download", "render", "process"].contains(&command) {
            args.push("--reprocess".into());
        }
        if win::checked(self.realign) && command == "process" {
            args.push("--realign".into());
        }
        if shared.start(args) && takes_ids {
            for sid in ids {
                shared.working.insert(sid, "queued".into());
            }
            shared.states_changed = true;
        }
    }

    fn edit_song(&mut self, shared: &Shared) {
        if let Some(sid) = self.one_selected(shared, "Select one song to edit.") {
            let song = shared.song(&sid).cloned();
            self.dialog = Some(SongDialog::open(song.as_ref(), shared));
        }
    }

    pub fn command(&mut self, id: u16, code: u32, shared: &mut Shared) -> Option<Action> {
        if code != BN_CLICKED {
            return None;
        }
        match id {
            ID_ADD => self.dialog = Some(SongDialog::open(None, shared)),
            ID_ADD_FILES => {
                let patterns: Vec<String> = shared.hello.as_ref().map(|hello| hello.media_suffixes.iter().map(|suffix| format!("*{suffix}")).collect()).unwrap_or_default();
                let home = shared.home();
                let paths = win::open_files(shared.main, "Add songs from media files", home.as_deref(), &[("Video and audio files", &patterns.join(";")), ("All files", "*.*")], true);
                if !paths.is_empty() {
                    let chosen = paths.len();
                    let paths: Vec<String> = paths.iter().map(|path| path.to_string_lossy().into_owned()).collect();
                    shared.engine.ask("add_files", Value::object(vec![("paths", Value::from(paths))]), Want::AddFiles(chosen));
                }
            }
            ID_EDIT => self.edit_song(shared),
            ID_REMOVE => {
                let chosen = self.selected();
                if !chosen.is_empty() && win::confirm(shared.main, &format!("Remove {} song(s) from the list? Downloaded files and videos are kept.", chosen.len())) {
                    shared.songs.retain(|song| !matches!(song.get("id"), Some(sid) if chosen.contains(sid)));
                    shared.save_songs();
                }
            }
            ID_TIME => return self.one_selected(shared, "Select one song to time its lyrics.").map(Action::TimeLyrics),
            ID_EDIT_ASS => {
                if let Some(sid) = self.one_selected(shared, "Select one song whose subtitles you want to edit.") {
                    shared.start(vec!["edit".into(), sid]);
                }
            }
            ID_PLAY => {
                let chosen = self.selected();
                let video = match (chosen.len(), &shared.hello) {
                    (1, Some(hello)) => Some(hello.folders.output.join(format!("{}.mp4", chosen.iter().next().unwrap()))),
                    _ => None,
                };
                match video {
                    Some(video) if video.is_file() => win::open_path(&video),
                    _ => shared.info("Select one song whose video is ready."),
                }
            }
            ID_VIDEOS => {
                if let Some(hello) = &shared.hello {
                    std::fs::create_dir_all(&hello.folders.output).ok();
                    win::open_path(&hello.folders.output);
                }
            }
            ID_DATA => {
                if let Some(hello) = &shared.hello {
                    win::open_path(&hello.home);
                }
            }
            ID_YTDLP => {
                let program = shared.tool("yt-dlp");
                let mut command = std::process::Command::new(&program);
                command.arg("-U");
                polykara::tools::quiet(&mut command);
                shared.start_command(command, format!("{} -U", program.display()));
            }
            ID_RUN => self.run_action(shared),
            ID_STOP => {
                shared.runner.stop();
                shared.write("[stopping…]");
            }
            _ => {}
        }
        None
    }

    pub fn notify(&mut self, header: &NMHDR, shared: &mut Shared) {
        if header.hwndFrom == self.list && (header.code == NM_DBLCLK || header.code == NM_RETURN) && !win::list_selected(self.list).is_empty() {
            self.edit_song(shared);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn song_ids_are_checked() {
        assert!(valid_id("my-song_2"));
        assert!(!valid_id("my song"));
        assert!(!valid_id(""));
        assert!(!valid_id("歌"));
    }
}
