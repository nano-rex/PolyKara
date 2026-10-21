//! Timing editor tab: time lyrics by hand while the song plays, in the style of Sayatoo.
//!
//! Hold Space while each highlighted word (or character) is sung: pressing
//! marks where it starts and letting go marks where it ends. The timeline under
//! the lyrics shows the waveform and every timed word; drag a word to move it or
//! drag its edges to change where it starts and ends. Saving writes
//! work/lyrics/<id>.timed.lrc, which the pipeline then uses ahead of every other
//! lyric source.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver, Sender};

use polykara::json::Value;
use polykara::model::{self, clock, lines_from_json, lines_json, KeyMode, Line, Place, Session, MIN_TOKEN_MS, OPEN_TOKEN_MS};
use polykara::player::Player;
use polykara::tools::waveform;

use crate::app::{Shared, Song, Want, DIALOG_CLASS, IDCANCEL, IDOK, PANE_CLASS, PLAY_TIMER};
use crate::win::{self, ffi::*, rgb, Canvas};

const SPEEDS: &[(&str, f64)] = &[("100%", 1.0), ("85%", 0.85), ("75%", 0.75), ("60%", 0.6), ("50%", 0.5)];
const MODES: &[(&str, KeyMode)] = &[("Hold (press = start, release = end)", KeyMode::Hold), ("Tap (each press starts the next word)", KeyMode::Tap)];
const LYRIC_FACE: &str = "Microsoft YaHei UI";
const HELP: &str = "Enter: play / pause    Hold Space: time the highlighted word    Backspace: undo the last word and go back    \
←/→: 3 s back / forward    ↑/↓: previous / next line    Shift+←/→: nudge the selected word 10 ms    Ctrl+Z: undo    Ctrl+S: save";
// Timeline rows at 96 DPI: ruler, waveform, words, lines.
const RULER: i32 = 18;
const WAVE_TOP: i32 = 22;
const WAVE_BOTTOM: i32 = 112;
const WORD_TOP: i32 = 116;
const WORD_BOTTOM: i32 = 146;
const LINE_TOP: i32 = 150;
const LINE_BOTTOM: i32 = 166;
const TIMELINE_HEIGHT: i32 = LINE_BOTTOM + 6 + 18;
const EDGE_PX: i32 = 5;

const ID_SONG: u16 = 201;
const ID_SAVE: u16 = 202;
const ID_BUILD: u16 = 203;
const ID_PREVIEW: u16 = 204;
const ID_TEXT: u16 = 205;
const ID_READINGS: u16 = 206;
const ID_UNDO: u16 = 207;
const ID_CLEAR: u16 = 208;
const ID_PLAY: u16 = 210;
const ID_SPEED: u16 = 212;
const ID_KEYS: u16 = 213;
const ID_DELAY: u16 = 214;
const ID_ZOOM_OUT: u16 = 220;
const ID_ZOOM_IN: u16 = 221;
const ID_DIALOG_TEXT: u16 = 300;
const ID_WORD: u16 = 301;
const ID_READING: u16 = 302;
const ID_ADD_WORD: u16 = 303;
const ID_LIBRARY_TEXT: u16 = 304;

/// What to do once the timing is saved.
pub enum Then {
    Nothing,
    Build,
    Preview(String),
}

pub struct AfterSave {
    sid: String,
    version: u64,
    then: Then,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Part {
    Start,
    End,
    Move,
}

struct Drag {
    place: Place,
    part: Part,
    x: i32,
    start: i64,
    end: i64,
}

enum Dialog {
    Lyrics { hwnd: HWND, text: HWND, controls: Vec<HWND> },
    Readings { hwnd: HWND, language: String, current: HWND, word: HWND, reading: HWND, text: HWND, problems: HWND, controls: Vec<HWND> },
}

impl Dialog {
    fn hwnd(&self) -> HWND {
        match self {
            Dialog::Lyrics { hwnd, .. } | Dialog::Readings { hwnd, .. } => *hwnd,
        }
    }
}

/// Where a token was drawn in the lyrics list, in list coordinates.
struct Placed {
    place: Place,
    left: i32,
    top: i32,
    right: i32,
    bottom: i32,
}

/// A group of tokens under one reading in the preview: (first token, count, reading).
fn reading_groups(count: usize, shown: &[Option<String>]) -> Vec<(usize, usize, Option<String>)> {
    let mut groups = Vec::new();
    let mut index = 0;
    while index < count {
        let mut size = 1;
        let reading = shown.get(index).cloned().flatten();
        if matches!(&reading, Some(text) if !text.is_empty()) {
            while index + size < count && matches!(shown.get(index + size), Some(Some(text)) if text.is_empty()) {
                size += 1;
            }
        }
        groups.push((index, size, reading.filter(|text| !text.is_empty())));
        index += size;
    }
    groups
}

pub struct EditorPage {
    controls: Vec<HWND>,
    song_label: HWND,
    song: HWND,
    buttons: Vec<HWND>,
    play_button: HWND,
    time: HWND,
    speed_label: HWND,
    speed: HWND,
    keys_label: HWND,
    keys: HWND,
    delay_label: HWND,
    delay: HWND,
    source: HWND,
    lyrics: HWND,
    stage: HWND,
    hint: HWND,
    zoom_out: HWND,
    zoom_in: HWND,
    timeline: HWND,
    help: HWND,
    dpi: u32,
    lyric_font: HFONT,
    small_font: HFONT,
    ruler_font: HFONT,
    stage_fonts: HashMap<(i32, bool), HFONT>,

    player: Player,
    ffmpeg: PathBuf,
    sid: Option<String>,
    row: Song,
    loading: Option<String>,
    session: Session,
    media: Option<PathBuf>,
    duration: i64,
    peaks: Vec<u16>,
    peak_max: u16,
    playhead: i64,
    was_playing: bool,
    language: String,
    library: bool,
    sung_colour: COLORREF,
    edited_ass: PathBuf,
    song_ids: Vec<String>,
    saving: Option<u64>,
    readings: HashMap<(String, Vec<String>), Vec<Option<String>>>,
    readings_asked: HashSet<(String, Vec<String>)>,
    now_token: Option<Place>,
    lyrics_scroll: i32,
    lyrics_height: i32,
    placed: Vec<Placed>,
    scroll_to: Option<Place>,
    stage_drawn: String,
    pps: f64,
    left: i32,
    drag: Option<Drag>,
    waves: (Sender<(String, Vec<u16>)>, Receiver<(String, Vec<u16>)>),
    dialog: Option<Dialog>,
}

fn pane(shared: &mut Shared, style: u32) -> HWND {
    let hwnd = win::create(WS_EX_CLIENTEDGE, PANE_CLASS, "", WS_CHILD | WS_VISIBLE | style, shared.main, 0);
    shared.controls.push(hwnd);
    hwnd
}

impl EditorPage {
    pub fn create(shared: &mut Shared) -> EditorPage {
        let main = shared.main;
        let song_label = shared.styled(win::label(main, "Song"));
        let song = shared.styled(win::combo(main, ID_SONG, &[], false));
        let buttons: Vec<HWND> = [(ID_SAVE, "Save"), (ID_BUILD, "Save and make subtitles"), (ID_PREVIEW, "Preview on video"), (ID_TEXT, "Lyrics text…"), (ID_READINGS, "Word readings…"), (ID_UNDO, "Undo"), (ID_CLEAR, "Clear line timing")]
            .iter()
            .map(|(id, text)| shared.styled(win::button(main, *id, text)))
            .collect();
        let play_button = shared.styled(win::button(main, ID_PLAY, "▶ Play"));
        let time = win::label(main, "00:00.000 / 00:00.000");
        win::set_font(time, shared.mono);
        shared.controls.push(time);
        let speed_label = shared.styled(win::label(main, "Speed"));
        let speed = shared.styled(win::combo(main, ID_SPEED, &SPEEDS.iter().map(|(name, _)| *name).collect::<Vec<_>>(), false));
        win::select(speed, Some(0));
        let keys_label = shared.styled(win::label(main, "Keys"));
        let keys = shared.styled(win::combo(main, ID_KEYS, &MODES.iter().map(|(name, _)| *name).collect::<Vec<_>>(), false));
        win::select(keys, Some(0));
        let delay_label = shared.styled(win::label(main, "Reaction delay (ms)"));
        let delay = shared.styled(win::edit(main, ID_DELAY, "0", ES_NUMBER));
        let source = shared.styled(win::child("STATIC", "", SS_RIGHT | SS_NOPREFIX | SS_ENDELLIPSIS | SS_CENTERIMAGE, main, 0));
        let lyrics = pane(shared, WS_VSCROLL);
        let stage = pane(shared, 0);
        let hint = shared.styled(win::label(main, "Timeline: click to move the play position, drag a word or its edges to change it, Ctrl+wheel to zoom"));
        let zoom_out = shared.styled(win::button(main, ID_ZOOM_OUT, "Zoom out"));
        let zoom_in = shared.styled(win::button(main, ID_ZOOM_IN, "Zoom in"));
        let timeline = pane(shared, WS_HSCROLL);
        let help = shared.styled(win::note(main, HELP));
        let mut controls = vec![song_label, song];
        controls.extend(buttons.iter().copied());
        controls.extend([play_button, time, speed_label, speed, keys_label, keys, delay_label, delay, source, lyrics, stage, hint, zoom_out, zoom_in, timeline, help]);
        let dpi = shared.dpi;
        let mut page = EditorPage {
            controls,
            song_label,
            song,
            buttons,
            play_button,
            time,
            speed_label,
            speed,
            keys_label,
            keys,
            delay_label,
            delay,
            source,
            lyrics,
            stage,
            hint,
            zoom_out,
            zoom_in,
            timeline,
            help,
            dpi,
            lyric_font: 0,
            small_font: 0,
            ruler_font: 0,
            stage_fonts: HashMap::new(),
            player: Player::new("ffplay".into()),
            ffmpeg: "ffmpeg".into(),
            sid: None,
            row: Song::new(),
            loading: None,
            session: Session::default(),
            media: None,
            duration: 0,
            peaks: Vec::new(),
            peak_max: 1,
            playhead: 0,
            was_playing: false,
            language: String::new(),
            library: false,
            sung_colour: rgb(255, 0, 0),
            edited_ass: PathBuf::new(),
            song_ids: Vec::new(),
            saving: None,
            readings: HashMap::new(),
            readings_asked: HashSet::new(),
            now_token: None,
            lyrics_scroll: 0,
            lyrics_height: 0,
            placed: Vec::new(),
            scroll_to: None,
            stage_drawn: String::new(),
            pps: 150.0,
            left: 0,
            drag: None,
            waves: channel(),
            dialog: None,
        };
        page.make_fonts();
        page.pps = 150.0 * dpi as f64 / 96.0;
        page
    }

    fn px(&self, value: i32) -> i32 {
        value * self.dpi as i32 / 96
    }

    fn make_fonts(&mut self) {
        for font in [self.lyric_font, self.small_font, self.ruler_font].iter().chain(self.stage_fonts.values()) {
            if *font != 0 {
                unsafe { DeleteObject(*font) };
            }
        }
        self.stage_fonts.clear();
        self.lyric_font = win::font(LYRIC_FACE, 12.5, false, self.dpi);
        self.small_font = win::font(LYRIC_FACE, 8.0, false, self.dpi);
        self.ruler_font = win::font("Consolas", 7.5, false, self.dpi);
    }

    pub fn dpi_changed(&mut self, shared: &Shared) {
        let ratio = shared.dpi as f64 / self.dpi as f64;
        self.dpi = shared.dpi;
        self.pps *= ratio;
        self.make_fonts();
        win::set_font(self.time, shared.mono);
    }

    /// A preview font `height` pixels tall.
    fn stage_font(&mut self, height: i32, bold: bool) -> HFONT {
        let face = LYRIC_FACE;
        *self.stage_fonts.entry((height, bold)).or_insert_with(|| {
            let wide = win::wide(face);
            unsafe { CreateFontW(-height.max(6), 0, 0, 0, if bold { FW_BOLD } else { FW_NORMAL }, 0, 0, 0, DEFAULT_CHARSET, 0, 0, CLEARTYPE_QUALITY, 0, wide.as_ptr()) }
        })
    }

    pub fn set_tools(&mut self, ffplay: PathBuf, ffmpeg: PathBuf) {
        self.player = Player::new(ffplay);
        self.ffmpeg = ffmpeg;
    }

    pub fn show(&self, visible: bool) {
        for control in &self.controls {
            win::show(*control, visible);
        }
    }

    pub fn focus(&self) {
        unsafe { SetFocus(self.lyrics) };
    }

    pub fn layout(&self, shared: &Shared, x: i32, y: i32, width: i32, height: i32) {
        let px = |value: i32| shared.px(value);
        win::place(self.song_label, x, y + px(4), px(38), px(24));
        win::place(self.song, x + px(40), y + px(3), px(300), px(400));
        let mut left = x + px(350);
        for button in &self.buttons {
            let wide = win::measure_with(*button, shared.font, &win::text(*button)).0 + px(20);
            win::place(*button, left, y + px(2), wide, px(28));
            left += wide + px(4);
        }
        let row = y + px(36);
        win::place(self.play_button, x, row, px(90), px(28));
        win::place(self.time, x + px(100), row + px(2), px(190), px(24));
        let mut left = x + px(296);
        for (label, control, wide) in [(self.speed_label, self.speed, px(80)), (self.keys_label, self.keys, px(280)), (self.delay_label, self.delay, px(56))] {
            let label_width = win::measure_with(label, shared.font, &win::text(label)).0 + px(6);
            win::place(label, left, row + px(2), label_width, px(24));
            left += label_width;
            win::place(control, left, row + px(2), wide, if control == self.delay { px(24) } else { px(300) });
            left += wide + px(14);
        }
        win::place(self.source, left, row + px(2), (x + width - left).max(0), px(24));
        let help_height = px(40);
        let bottom = y + height;
        win::place(self.help, x, bottom - help_height, width, help_height);
        let timeline_top = bottom - help_height - px(TIMELINE_HEIGHT);
        win::place(self.timeline, x, timeline_top, width, px(TIMELINE_HEIGHT));
        let hint_top = timeline_top - px(30);
        win::place(self.hint, x, hint_top + px(2), width - px(200), px(24));
        win::place(self.zoom_in, x + width - px(94), hint_top, px(94), px(26));
        win::place(self.zoom_out, x + width - px(192), hint_top, px(94), px(26));
        let middle_top = row + px(36);
        let middle_height = hint_top - px(6) - middle_top;
        let lyrics_width = width * 58 / 100;
        win::place(self.lyrics, x, middle_top, lyrics_width, middle_height);
        win::place(self.stage, x + lyrics_width + px(8), middle_top, width - lyrics_width - px(8), middle_height);
    }

    pub fn dialog(&self) -> Option<HWND> {
        self.dialog.as_ref().map(Dialog::hwnd)
    }

    pub fn stop(&mut self) {
        self.player.stop();
    }

    /// Grey text for hints and help; every other label keeps the normal colour.
    pub fn static_colour(&self, hdc: HDC, hwnd: HWND) -> LRESULT {
        unsafe {
            if [self.hint, self.help, self.source].contains(&hwnd) {
                SetTextColor(hdc, rgb(0x66, 0x66, 0x66));
            }
            SetBkColor(hdc, GetSysColor(COLOR_BTNFACE));
            GetSysColorBrush(COLOR_BTNFACE)
        }
    }

    fn redraw_all(&mut self) {
        self.stage_drawn.clear();
        for pane in [self.lyrics, self.stage, self.timeline] {
            win::redraw(pane);
        }
    }

    fn changed(&mut self) {
        self.redraw_all();
    }

    // Songs
    fn list_songs(&mut self, shared: &Shared) {
        let songs = shared.visible_songs();
        self.song_ids = songs.iter().map(|song| song.get("id").cloned().unwrap_or_default()).collect();
        let items: Vec<String> = songs.iter().map(|song| format!("{}  {}", song.get("id").cloned().unwrap_or_default(), song.get("title").cloned().unwrap_or_default())).collect();
        win::set_items(self.song, &items.iter().map(String::as_str).collect::<Vec<_>>());
        let current = self.loading.as_ref().or(self.sid.as_ref());
        win::select(self.song, current.and_then(|sid| self.song_ids.iter().position(|item| item == sid)));
    }

    /// Ask to save changes: Some(true) to go on, Some(false) when a save was started, None to stay.
    pub fn confirm_discard(&mut self, shared: &mut Shared) -> Option<bool> {
        if !self.session.dirty {
            return Some(true);
        }
        match win::confirm_or_cancel(shared.main, "Save the timing changes first?") {
            Some(true) => {
                if self.save(Then::Nothing, shared) {
                    Some(false)
                } else {
                    None
                }
            }
            Some(false) => Some(true),
            None => None,
        }
    }

    pub fn open(&mut self, sid: &str, shared: &mut Shared) {
        if shared.song(sid).is_none() {
            return;
        }
        if self.sid.as_deref() == Some(sid) && !self.session.lines.is_empty() {
            return;
        }
        if self.confirm_discard(shared).is_none() {
            self.list_songs(shared);
            return;
        }
        self.pause(shared);
        self.loading = Some(sid.to_string());
        self.list_songs(shared);
        win::set_text(self.source, "Loading…");
        shared.engine.ask("load", Value::object(vec![("id", Value::from(sid))]), Want::Load(sid.to_string()));
    }

    fn opened(&mut self, sid: String, value: Value, shared: &mut Shared) {
        self.player.stop();
        self.sid = Some(sid.clone());
        self.row = value.get("row").pairs().iter().map(|(key, value)| (key.clone(), value.str().to_string())).collect();
        self.session = Session::new(lines_from_json(value.get("lines")));
        self.media = value.get("media").as_str().map(PathBuf::from);
        self.language = value.get("language").str().to_string();
        self.library = value.get("library").as_bool().unwrap_or(false);
        self.sung_colour = win::hex_colour(value.get("sung_colour").str()).unwrap_or(rgb(255, 0, 0));
        self.edited_ass = PathBuf::from(value.get("edited_ass").str());
        self.peaks.clear();
        self.peak_max = 1;
        self.duration = 0;
        self.playhead = 0;
        self.left = 0;
        self.lyrics_scroll = 0;
        self.drag = None;
        self.now_token = None;
        self.scroll_to = self.session.cursor;
        self.readings.clear();
        self.readings_asked.clear();
        let label = value.get("label").str();
        win::set_text(self.source, &if label.is_empty() { "No lyrics yet".to_string() } else { format!("Lyrics: {label}") });
        self.ask_readings(shared);
        self.redraw_all();
        let problem = value.get("problem").str();
        if !problem.is_empty() {
            shared.warn(problem);
        }
        match &self.media {
            None => shared.info(&format!("{sid} has not been downloaded yet. Run Download for it on the Songs tab, then open it here again.")),
            Some(media) => {
                let media = Value::from(media.to_string_lossy().into_owned());
                shared.engine.ask("prepare_audio", Value::object(vec![("id", Value::from(&sid)), ("media", media)]), Want::Audio(sid.clone()));
            }
        }
        if self.session.lines.is_empty() {
            shared.info("No lyrics were found for this song. Paste them in the next window, then time them while the song plays.");
            self.edit_text(shared);
        }
    }

    // Readings above the characters, from the engine.
    fn ask_readings(&mut self, shared: &mut Shared) {
        if self.language.is_empty() {
            return;
        }
        let missing: Vec<Vec<String>> = self
            .session
            .lines
            .iter()
            .map(|line| line.tokens.iter().map(|token| token.text.clone()).collect::<Vec<String>>())
            .filter(|tokens| {
                let key = (self.language.clone(), tokens.clone());
                !self.readings.contains_key(&key) && !self.readings_asked.contains(&key)
            })
            .collect::<HashSet<_>>()
            .into_iter()
            .collect();
        if missing.is_empty() {
            return;
        }
        for tokens in &missing {
            self.readings_asked.insert((self.language.clone(), tokens.clone()));
        }
        let lines = Value::Array(missing.iter().map(|tokens| Value::from(tokens.clone())).collect());
        shared.engine.ask("readings", Value::object(vec![("language", Value::from(&self.language)), ("lines", lines)]), Want::Readings(self.language.clone(), missing));
    }

    fn line_readings(&self, line: &Line) -> Option<&Vec<Option<String>>> {
        let tokens: Vec<String> = line.tokens.iter().map(|token| token.text.clone()).collect();
        self.readings.get(&(self.language.clone(), tokens))
    }

    /// Engine answers for this tab. Returns whether a save succeeded (for quitting after it).
    pub fn answer(&mut self, want: Want, result: Result<Value, String>, shared: &mut Shared) -> bool {
        match want {
            Want::Load(sid) => {
                if self.loading.as_deref() != Some(sid.as_str()) {
                    return false;
                }
                self.loading = None;
                match result {
                    Ok(value) => self.opened(sid, value, shared),
                    Err(error) => {
                        win::set_text(self.source, "");
                        shared.warn(&format!("{sid} could not be opened: {error}"));
                    }
                }
                self.list_songs(shared);
            }
            Want::Audio(sid) => {
                if self.sid.as_deref() != Some(sid.as_str()) {
                    return false;
                }
                match result {
                    Ok(value) => {
                        let media = PathBuf::from(value.get("media").str());
                        self.duration = value.get("duration").as_i64().unwrap_or(0);
                        self.media = Some(media.clone());
                        let (ffmpeg, sender, wake) = (self.ffmpeg.clone(), self.waves.0.clone(), shared.wake.clone());
                        std::thread::spawn(move || {
                            let peaks = waveform(&ffmpeg, &media).unwrap_or_default();
                            let _ = sender.send((sid, peaks));
                            wake();
                        });
                        self.redraw_all();
                    }
                    Err(error) => shared.warn(&format!("The audio could not be read: {error}")),
                }
            }
            Want::Readings(language, asked) => {
                if let Ok(value) = result {
                    for (tokens, shown) in asked.into_iter().zip(value.get("readings").items()) {
                        let shown: Vec<Option<String>> = shown.items().iter().map(|item| item.as_str().map(str::to_string)).collect();
                        self.readings.insert((language.clone(), tokens), shown);
                    }
                    self.stage_drawn.clear();
                    win::redraw(self.stage);
                    self.show_current_readings();
                }
            }
            Want::Retext => match result {
                Ok(value) => {
                    self.session.replace_lines(lines_from_json(value.get("lines")));
                    self.scroll_to = self.session.cursor;
                    self.ask_readings(shared);
                    self.changed();
                }
                Err(error) => shared.warn(&format!("The lyrics could not be changed: {error}")),
            },
            Want::Library => match result {
                Ok(value) => self.open_readings(value, shared),
                Err(error) => shared.info(&error),
            },
            Want::LibrarySaved => match result {
                Ok(value) => {
                    let problems = value.get("problems").strings();
                    if let Some(Dialog::Readings { problems: label, .. }) = &self.dialog {
                        win::set_text(*label, &if problems.is_empty() { String::new() } else { format!("Not used: {}", problems.join("; ")) });
                    }
                    // The library changed: every reading may be different now.
                    self.readings.clear();
                    self.readings_asked.clear();
                    self.ask_readings(shared);
                }
                Err(error) => shared.warn(&format!("The word list could not be saved: {error}")),
            },
            Want::Saved(after) => {
                self.saving = None;
                match result {
                    Ok(value) => {
                        let current = self.sid.as_deref() == Some(after.sid.as_str());
                        if current && self.session.version == after.version {
                            self.session.dirty = false;
                        }
                        shared.set_status(&format!("Saved timing to {}", value.get("path").str()));
                        if current {
                            win::set_text(self.source, &format!("Lyrics: your saved timing ({})", value.get("name").str()));
                        }
                        let estimated = value.get("estimated").as_i64().unwrap_or(0);
                        if estimated > 0 {
                            shared.info(&format!(
                                "Saved {}.\n\n{estimated} line(s) had no timing yet and were placed between their neighbours; time them before rendering.",
                                value.get("name").str()
                            ));
                        }
                        match after.then {
                            Then::Nothing => {}
                            Then::Build => {
                                if self.aside_edited_ass(shared) {
                                    shared.start(vec!["lyrics".into(), after.sid]);
                                    shared.set_status("Making subtitles… (log on the Songs tab). Then use Render videos only.");
                                }
                            }
                            Then::Preview(at) => {
                                if self.aside_edited_ass(shared) {
                                    shared.start(vec!["preview".into(), after.sid, "--at".into(), at]);
                                }
                            }
                        }
                        return true;
                    }
                    Err(error) => shared.warn(&format!("The timing was not saved: {error}")),
                }
            }
            _ => {}
        }
        false
    }

    /// Waveforms finished in the background.
    pub fn poll(&mut self) {
        while let Ok((sid, peaks)) = self.waves.1.try_recv() {
            if self.sid.as_deref() == Some(sid.as_str()) {
                self.peak_max = peaks.iter().copied().max().unwrap_or(1).max(1);
                self.peaks = peaks;
                win::redraw(self.timeline);
            }
        }
    }

    // Playback
    fn speed_value(&self) -> f64 {
        SPEEDS[win::selection(self.speed).unwrap_or(0).min(SPEEDS.len() - 1)].1
    }

    fn mode(&self) -> KeyMode {
        MODES[win::selection(self.keys).unwrap_or(0).min(MODES.len() - 1)].1
    }

    fn delay_ms(&self) -> i64 {
        win::text(self.delay).trim().parse::<i64>().unwrap_or(0).clamp(0, 2000)
    }

    fn position(&mut self) -> i64 {
        if self.player.playing() {
            self.player.position()
        } else {
            self.playhead
        }
    }

    fn play(&mut self, at: Option<i64>, shared: &Shared) {
        let media = match &self.media {
            Some(media) => media.clone(),
            None => return,
        };
        self.playhead = at.unwrap_or(self.playhead).max(0);
        if let Err(error) = self.player.play(&media, self.playhead, self.speed_value()) {
            shared.warn(&format!("ffplay could not be started ({error})."));
            return;
        }
        win::set_text(self.play_button, "‖ Pause");
        unsafe { SetTimer(shared.main, PLAY_TIMER, 15, std::ptr::null()) };
    }

    fn pause(&mut self, shared: &Shared) {
        if self.player.playing() {
            self.playhead = self.player.position();
        }
        self.player.stop();
        self.session.release(self.playhead);
        win::set_text(self.play_button, "▶ Play");
        unsafe { KillTimer(shared.main, PLAY_TIMER) };
        self.show_time(self.playhead);
        self.changed();
    }

    fn toggle(&mut self, shared: &Shared) {
        if self.player.playing() {
            self.pause(shared);
        } else {
            self.play(None, shared);
        }
    }

    fn restart(&mut self, shared: &Shared) {
        if self.player.playing() {
            let at = self.player.position();
            self.play(Some(at), shared);
        }
    }

    fn seek(&mut self, ms: i64, shared: &Shared) {
        let ms = ms.max(0).min(if self.duration > 0 { self.duration } else { ms.max(0) });
        if self.player.playing() {
            self.play(Some(ms), shared);
        } else {
            self.playhead = ms;
        }
        self.follow(ms, true);
        self.show_time(ms);
        self.changed();
    }

    fn show_time(&self, ms: i64) {
        win::set_text(self.time, &format!("{} / {}", clock(ms), clock(self.duration)));
    }

    fn jump_line(&mut self, step: i64, shared: &Shared) {
        if self.session.lines.is_empty() {
            return;
        }
        let current = self.session.cursor.map(|(li, _)| li as i64).unwrap_or(self.session.lines.len() as i64);
        let li = (current + step).clamp(0, self.session.lines.len() as i64 - 1) as usize;
        self.session.cursor = Some((li, 0));
        self.scroll_to = self.session.cursor;
        if let Some((start, _)) = self.session.lines[li].span() {
            self.seek(start - 1500, shared);
        }
        win::redraw(self.lyrics);
    }

    /// Ticks while playing: the play position, the sung word and the preview.
    pub fn tick(&mut self, shared: &mut Shared) {
        let playing = self.player.playing();
        if playing || self.was_playing {
            let mut ms = self.position();
            if !playing {
                self.pause(shared);
                ms = self.playhead;
            }
            self.show_time(ms);
            self.follow(ms, false);
            win::redraw(self.timeline);
            let now = model::sung_at(&self.session.lines, ms);
            if now != self.now_token {
                self.now_token = now;
                if self.session.holding.is_none() && now.is_some() {
                    self.scroll_to = now;
                }
                win::redraw(self.lyrics);
            }
            let signature = self.stage_signature(ms);
            if signature != self.stage_drawn {
                win::redraw(self.stage);
            }
        } else {
            unsafe { KillTimer(shared.main, PLAY_TIMER) };
        }
        self.was_playing = playing;
    }

    // Keys
    /// A key for the editor; true when it was used.
    pub fn key(&mut self, key: u32, down: bool, repeat: bool, shared: &mut Shared) -> bool {
        if self.sid.is_none() {
            return false;
        }
        let (control, shift) = (win::key_down(VK_CONTROL), win::key_down(VK_SHIFT));
        if key == VK_SPACE {
            if down && !repeat {
                self.key_down(shared);
            } else if !down {
                self.key_up();
            }
            return true;
        }
        if !down {
            return [VK_RETURN, VK_BACK, VK_LEFT, VK_RIGHT, VK_UP, VK_DOWN, VK_ESCAPE].contains(&key);
        }
        match key {
            VK_RETURN => self.toggle(shared),
            VK_BACK => self.step_back(shared),
            VK_LEFT if shift => self.nudge(-10),
            VK_RIGHT if shift => self.nudge(10),
            VK_LEFT => {
                let at = self.position() - 3000;
                self.seek(at, shared);
            }
            VK_RIGHT => {
                let at = self.position() + 3000;
                self.seek(at, shared);
            }
            VK_UP => self.jump_line(-1, shared),
            VK_DOWN => self.jump_line(1, shared),
            VK_ESCAPE => self.pause(shared),
            0x5A if control => self.undo(),
            0x53 if control => {
                self.save(Then::Nothing, shared);
            }
            _ => return false,
        }
        true
    }

    fn key_down(&mut self, shared: &Shared) {
        if self.session.holding.is_some() {
            return;
        }
        if !self.player.playing() {
            self.play(None, shared);
            return;
        }
        let at = self.player.position() - self.delay_ms();
        self.session.press(at, self.mode());
        self.scroll_to = self.session.cursor;
        self.changed();
    }

    fn key_up(&mut self) {
        if self.session.holding.is_none() {
            return;
        }
        let at = self.position() - self.delay_ms();
        self.session.release(at);
        self.scroll_to = self.session.cursor;
        self.changed();
    }

    fn step_back(&mut self, shared: &Shared) {
        if let Some(start) = self.session.step_back() {
            self.scroll_to = self.session.cursor;
            self.changed();
            if let Some(start) = start {
                self.seek(start - 2000, shared);
            }
        }
    }

    fn nudge(&mut self, delta: i64) {
        if self.session.nudge(delta) {
            self.changed();
        }
    }

    fn undo(&mut self) {
        if self.session.undo() {
            self.changed();
        }
    }

    // Saving
    /// Start saving; false when there is nothing to save.
    fn save(&mut self, then: Then, shared: &mut Shared) -> bool {
        let sid = match &self.sid {
            Some(sid) => sid.clone(),
            None => return false,
        };
        model::close_open(&mut self.session.lines);
        if !self.session.anything_timed() {
            shared.info("Nothing is timed yet.");
            return false;
        }
        self.saving = Some(self.session.version);
        let after = AfterSave { sid: sid.clone(), version: self.session.version, then };
        shared.engine.ask("save", Value::object(vec![("id", Value::from(sid)), ("lines", lines_json(&self.session.lines))]), Want::Saved(after));
        true
    }

    /// A hand-edited ASS file overrides the timing editor; offer to set it aside.
    fn aside_edited_ass(&self, shared: &Shared) -> bool {
        if !self.edited_ass.is_file() {
            return true;
        }
        let name = self.edited_ass.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default();
        if !win::confirm(shared.main, &format!("{name} (edited in a subtitle editor) is used for the video instead of this timing. Rename it to {name}.bak so this timing is used?")) {
            return false;
        }
        let mut backup = self.edited_ass.clone().into_os_string();
        backup.push(".bak");
        match std::fs::rename(&self.edited_ass, backup) {
            Ok(()) => true,
            Err(error) => {
                shared.warn(&format!("{name} could not be renamed: {error}"));
                false
            }
        }
    }

    pub fn command(&mut self, id: u16, code: u32, shared: &mut Shared) {
        match (id, code) {
            (ID_SONG, CBN_DROPDOWN) => self.list_songs(shared),
            (ID_SONG, CBN_SELCHANGE) => {
                if let Some(sid) = win::selection(self.song).and_then(|index| self.song_ids.get(index).cloned()) {
                    self.open(&sid, shared);
                }
                self.focus();
            }
            (ID_SPEED, CBN_SELCHANGE) => {
                self.restart(shared);
                self.focus();
            }
            (ID_KEYS, CBN_SELCHANGE) => self.focus(),
            (_, BN_CLICKED) => {
                match id {
                    ID_PLAY => self.toggle(shared),
                    ID_ZOOM_IN => self.zoom(1.5, None),
                    ID_ZOOM_OUT => self.zoom(1.0 / 1.5, None),
                    _ if self.sid.is_none() && [ID_SAVE, ID_BUILD, ID_PREVIEW, ID_TEXT, ID_READINGS, ID_UNDO, ID_CLEAR].contains(&id) => shared.info("Choose a song first."),
                    ID_SAVE => {
                        self.save(Then::Nothing, shared);
                    }
                    ID_BUILD => {
                        self.save(Then::Build, shared);
                    }
                    ID_PREVIEW => {
                        self.pause(shared);
                        let at = format!("{:.2}", (self.playhead - 2000).max(0) as f64 / 1000.0);
                        self.save(Then::Preview(at), shared);
                    }
                    ID_TEXT => self.edit_text(shared),
                    ID_READINGS => self.edit_readings(shared),
                    ID_UNDO => self.undo(),
                    ID_CLEAR => {
                        if self.session.clear_line() {
                            self.changed();
                        }
                    }
                    _ => return,
                }
                if id != ID_TEXT && id != ID_READINGS {
                    self.focus();
                }
            }
            _ => {}
        }
    }

    // Dialogs
    fn edit_text(&mut self, shared: &mut Shared) {
        if self.sid.is_none() {
            return;
        }
        let text = model::lines_to_text(&self.session.lines);
        self.open_lyrics(&text, shared);
    }

    fn open_lyrics(&mut self, text: &str, shared: &Shared) {
        if self.dialog.is_some() {
            return;
        }
        let px = |value: i32| shared.px(value);
        let hwnd = win::top_window(DIALOG_CLASS, "Lyrics text", WS_OVERLAPPEDWINDOW | WS_CLIPCHILDREN, shared.main, px(640), px(680));
        let note = win::note(hwnd, "One sung line per row. Start a row with [singer: 女], [singer: 男] or [singer: 合] for duets. Lines whose words did not change keep their timing.");
        let edit = win::multiline(hwnd, ID_DIALOG_TEXT, text, false);
        let apply = win::child("BUTTON", "Apply", WS_TABSTOP | BS_PUSHBUTTON, hwnd, IDOK);
        let cancel = win::button(hwnd, IDCANCEL, "Cancel");
        win::set_font(edit, self.lyric_font);
        for control in [note, apply, cancel] {
            win::set_font(control, shared.font);
        }
        let dialog = Dialog::Lyrics { hwnd, text: edit, controls: vec![note, edit, apply, cancel] };
        self.dialog = Some(dialog);
        self.layout_dialog(shared.dpi);
        win::enable(shared.main, false);
        win::show(hwnd, true);
        unsafe { SetFocus(edit) };
    }

    fn edit_readings(&mut self, shared: &mut Shared) {
        if !self.library {
            shared.info("The word library is for songs in Mandarin (zh), Cantonese (yue) and Japanese (ja). Set the song's language on the Songs tab.");
            return;
        }
        shared.engine.ask("library", Value::object(vec![("language", Value::from(&self.language))]), Want::Library);
    }

    fn open_readings(&mut self, value: Value, shared: &Shared) {
        if self.dialog.is_some() {
            return;
        }
        let px = |value: i32| shared.px(value);
        let name = value.get("name").str();
        let hwnd = win::top_window(DIALOG_CLASS, &format!("Word readings — {}", self.language), WS_OVERLAPPEDWINDOW | WS_CLIPCHILDREN, shared.main, px(780), px(740));
        let note = win::note(hwnd, &format!("Readings ({name}) shown above the characters of this song. When one is wrong, add the word with the right reading below; it is used for every song from now on."));
        let current = win::multiline(hwnd, 0, "", true);
        let word_label = win::label(hwnd, "Word");
        let word = win::edit(hwnd, ID_WORD, "", 0);
        let reading_label = win::label(hwnd, "Reading");
        let reading = win::edit(hwnd, ID_READING, "", 0);
        let add = win::child("BUTTON", "Add", WS_TABSTOP | BS_DEFPUSHBUTTON, hwnd, ID_ADD_WORD);
        let hint = win::label(hwnd, value.get("hint").str());
        let file = win::label(hwnd, &format!("Your words ({}):", value.get("user_file").str()));
        let text = win::multiline(hwnd, ID_LIBRARY_TEXT, value.get("text").str(), false);
        let problems = win::note(hwnd, "");
        let builtin = win::label(hwnd, &format!("Built-in list: {} words", value.get("builtin_count").as_i64().unwrap_or(0)));
        let save = win::child("BUTTON", "Save", WS_TABSTOP | BS_PUSHBUTTON, hwnd, IDOK);
        let close = win::button(hwnd, IDCANCEL, "Close");
        for control in [current, word, text] {
            win::set_font(control, self.lyric_font);
        }
        let controls = vec![note, current, word_label, word, reading_label, reading, add, hint, file, text, problems, builtin, save, close];
        for control in &controls {
            if ![current, word, text].contains(control) {
                win::set_font(*control, shared.font);
            }
        }
        self.dialog = Some(Dialog::Readings { hwnd, language: self.language.clone(), current, word, reading, text, problems, controls });
        self.layout_dialog(shared.dpi);
        self.show_current_readings();
        win::enable(shared.main, false);
        win::show(hwnd, true);
        unsafe { SetFocus(word) };
    }

    /// The song's lines with the reading after each character, in the word readings window.
    fn show_current_readings(&self) {
        let current = match &self.dialog {
            Some(Dialog::Readings { current, .. }) => *current,
            _ => return,
        };
        let mut text = String::new();
        for (number, line) in self.session.lines.iter().enumerate() {
            let shown = self.line_readings(line);
            text.push_str(&format!("{:>3}  ", number + 1));
            for (index, token) in line.tokens.iter().enumerate() {
                match shown.and_then(|shown| shown.get(index)).cloned().flatten() {
                    Some(reading) if !reading.is_empty() => text.push_str(&format!("{}({reading})", token.text.trim())),
                    _ => text.push_str(&token.text),
                }
            }
            text.push('\n');
        }
        win::set_text(current, &text);
    }

    fn layout_dialog(&self, dpi: u32) {
        let px = |value: i32| value * dpi as i32 / 96;
        match &self.dialog {
            Some(Dialog::Lyrics { hwnd, controls, .. }) => {
                let (width, height) = win::client_size(*hwnd);
                win::place(controls[0], px(10), px(8), width - px(20), px(40));
                win::place(controls[1], px(10), px(52), width - px(20), height - px(100));
                win::place(controls[2], width - px(210), height - px(40), px(96), px(28));
                win::place(controls[3], width - px(106), height - px(40), px(96), px(28));
            }
            Some(Dialog::Readings { hwnd, controls, .. }) => {
                let (width, height) = win::client_size(*hwnd);
                let half = (height - px(250)) / 2;
                win::place(controls[0], px(10), px(8), width - px(20), px(40));
                win::place(controls[1], px(10), px(50), width - px(20), half);
                let row = px(56) + half;
                win::place(controls[2], px(10), row, px(40), px(26));
                win::place(controls[3], px(52), row, px(120), px(26));
                win::place(controls[4], px(182), row, px(56), px(26));
                win::place(controls[5], px(240), row, px(200), px(26));
                win::place(controls[6], px(448), row, px(70), px(26));
                win::place(controls[7], px(10), row + px(30), width - px(20), px(22));
                win::place(controls[8], px(10), row + px(56), width - px(20), px(22));
                win::place(controls[9], px(10), row + px(80), width - px(20), half);
                let bottom = row + px(84) + half;
                win::place(controls[10], px(10), bottom, width - px(20), px(36));
                win::place(controls[11], px(10), height - px(38), px(300), px(26));
                win::place(controls[12], width - px(210), height - px(40), px(96), px(28));
                win::place(controls[13], width - px(106), height - px(40), px(96), px(28));
            }
            None => {}
        }
    }

    fn close_dialog(&mut self, shared: &Shared) {
        if let Some(dialog) = self.dialog.take() {
            win::enable(shared.main, true);
            unsafe {
                DestroyWindow(dialog.hwnd());
                SetForegroundWindow(shared.main);
            }
            self.focus();
        }
    }

    fn save_library(&mut self, shared: &mut Shared) {
        if let Some(Dialog::Readings { language, text, .. }) = &self.dialog {
            let content = win::text(*text);
            shared.engine.ask("save_library", Value::object(vec![("language", Value::from(language)), ("text", Value::from(content))]), Want::LibrarySaved);
        }
    }

    pub fn dialog_message(&mut self, message: u32, wparam: WPARAM, lparam: LPARAM, shared: &mut Shared) -> Option<LRESULT> {
        match message {
            // Enter in the word readings window adds the word (DM_GETDEFID: DC_HASDEFID and the button).
            0x0400 if matches!(self.dialog, Some(Dialog::Readings { .. })) => return Some((0x534B << 16) | ID_ADD_WORD as isize),
            WM_SIZE => self.layout_dialog(shared.dpi),
            WM_CLOSE => self.close_dialog(shared),
            WM_COMMAND => {
                let id = win::loword(wparam) as u16;
                match (&self.dialog, id) {
                    // Escape must not throw away pasted lyrics or words: only the buttons close.
                    (_, IDCANCEL) if lparam == 0 => {}
                    (_, IDCANCEL) => self.close_dialog(shared),
                    (Some(Dialog::Lyrics { text, .. }), IDOK) => {
                        let content = win::text(*text);
                        self.close_dialog(shared);
                        if content != model::lines_to_text(&self.session.lines) {
                            shared.engine.ask("retext", Value::object(vec![("lines", lines_json(&self.session.lines)), ("text", Value::from(content))]), Want::Retext);
                        }
                    }
                    (Some(Dialog::Readings { .. }), IDOK) => self.save_library(shared),
                    (Some(Dialog::Readings { word, reading, text, .. }), ID_ADD_WORD) => {
                        let (word_text, reading_text) = (win::text(*word).trim().to_string(), win::text(*reading).split_whitespace().collect::<Vec<_>>().join(" "));
                        if word_text.is_empty() || reading_text.is_empty() {
                            return Some(0);
                        }
                        let content = win::text(*text).trim_end_matches('\n').to_string();
                        win::set_text(*text, &format!("{content}\n{word_text} {reading_text}\n"));
                        win::set_text(*word, "");
                        win::set_text(*reading, "");
                        unsafe { SetFocus(*word) };
                        self.save_library(shared);
                    }
                    _ => return None,
                }
            }
            _ => return None,
        }
        Some(0)
    }

    // The lyrics list
    fn paint_lyrics(&mut self, canvas: &Canvas) {
        canvas.fill(0, 0, canvas.width, canvas.height, rgb(255, 255, 255));
        let (indent, gap) = (self.px(44), self.px(8));
        canvas.use_font(self.lyric_font);
        let row_height = canvas.measure("Ag汉").1 + self.px(8);
        // Lay out every token (wrapping long lines), then draw the visible ones.
        let mut placed = Vec::new();
        let mut speakers = Vec::new();
        let mut y = gap;
        for (li, line) in self.session.lines.iter().enumerate() {
            let mut x = indent;
            if !line.speaker.is_empty() {
                let label = format!("{}: ", line.speaker);
                let width = canvas.measure(&label).0;
                speakers.push((li, x, y, label));
                x += width;
            }
            for (ti, token) in line.tokens.iter().enumerate() {
                let width = canvas.measure(&token.text).0.max(self.px(4));
                if x + width > canvas.width - gap && x > indent {
                    x = indent;
                    y += row_height;
                }
                placed.push(Placed { place: (li, ti), left: x, top: y, right: x + width, bottom: y + row_height });
                x += width;
            }
            y += row_height + self.px(2);
        }
        self.lyrics_height = y + gap;
        if let Some(target) = self.scroll_to.take() {
            if let Some(found) = placed.iter().find(|item| item.place == target) {
                if found.top < self.lyrics_scroll || found.bottom > self.lyrics_scroll + canvas.height {
                    self.lyrics_scroll = (found.top - canvas.height / 3).max(0);
                }
            }
        }
        self.lyrics_scroll = win::set_scroll(self.lyrics, SB_VERT, self.lyrics_height, canvas.height, self.lyrics_scroll);
        let offset = self.lyrics_scroll;
        let line_starts: Vec<(usize, i32)> = placed.iter().filter(|item| item.place.1 == 0).map(|item| (item.place.0, item.top)).collect();
        canvas.use_font(self.small_font);
        for (li, top) in &line_starts {
            if top - offset > -row_height && top - offset < canvas.height {
                canvas.text(self.px(6), top - offset + self.px(5), &format!("{:>3}", li + 1), rgb(0x99, 0x99, 0x99));
            }
        }
        canvas.use_font(self.lyric_font);
        for (_, x, top, label) in &speakers {
            canvas.text(*x, top - offset + self.px(3), label, rgb(0xa0, 0x50, 0x00));
        }
        for item in &placed {
            let (top, bottom) = (item.top - offset, item.bottom - offset);
            if bottom < 0 || top > canvas.height {
                continue;
            }
            let token = &self.session.lines[item.place.0].tokens[item.place.1];
            let mut colour = if token.timed() {
                rgb(0x1b, 0x5e, 0x20)
            } else if token.start.is_some() {
                rgb(0xe6, 0x51, 0x00)
            } else {
                rgb(0x9a, 0x9a, 0x9a)
            };
            if Some(item.place) == self.session.cursor {
                canvas.fill(item.left, top + self.px(1), item.right, bottom - self.px(1), rgb(0x4f, 0xc3, 0xf7));
                colour = rgb(0, 0, 0);
            } else if Some(item.place) == self.now_token {
                canvas.fill(item.left, top + self.px(1), item.right, bottom - self.px(1), rgb(0xff, 0xe0, 0x82));
            }
            canvas.text(item.left, top + self.px(3), &token.text, colour);
            if Some(item.place) == self.session.selected {
                canvas.fill(item.left, bottom - self.px(3), item.right, bottom - self.px(2), colour);
            }
        }
        if self.session.lines.is_empty() {
            canvas.use_font(win_font_ui());
            canvas.text(self.px(12), self.px(12), if self.sid.is_some() { "No lyrics: use Lyrics text… to paste them." } else { "Choose a song above, or select one on the Songs tab and press Time lyrics." }, rgb(0x77, 0x77, 0x77));
        }
        self.placed = placed;
    }

    fn click_lyrics(&mut self, x: i32, y: i32, shared: &Shared) {
        let y = y + self.lyrics_scroll;
        let hit = self.placed.iter().find(|item| item.left <= x && x < item.right && item.top <= y && y < item.bottom).map(|item| item.place);
        unsafe { SetFocus(self.lyrics) };
        let (li, ti) = match hit {
            Some(place) => place,
            None => return,
        };
        self.session.cursor = Some((li, ti));
        self.session.selected = Some((li, ti));
        let token = &self.session.lines[li].tokens[ti];
        let target = token.start.or_else(|| self.session.lines[li].span().map(|(start, _)| start));
        self.changed();
        if let Some(target) = target {
            self.seek(target - 1500, shared);
        }
    }

    // The preview of the sung line
    fn stage_signature(&mut self, ms: i64) -> String {
        let (width, height) = win::client_size(self.stage);
        let shown = model::stage_lines(&self.session.lines, ms, self.session.cursor);
        let mut signature = format!("{width}x{height} {:?} {:?}", self.sid, shown);
        for li in &shown {
            let line = &self.session.lines[*li];
            signature.push_str(&format!("|{}:{}", line.text(), self.line_readings(line).is_some()));
            for token in &line.tokens {
                let state = match (token.start, token.end) {
                    (None, _) => 0,
                    (Some(start), _) if ms < start => 0,
                    (Some(_), Some(end)) if ms >= end => 1,
                    // Being sung: the bar under it grows in tenths.
                    (Some(start), end) => 2 + ((ms - start) * 10 / (end.unwrap_or(ms + 1) - start).max(1)).min(9),
                };
                signature.push_str(&state.to_string());
            }
        }
        signature
    }

    fn paint_stage(&mut self, canvas: &Canvas) {
        let ms = self.position();
        self.stage_drawn = self.stage_signature(ms);
        canvas.fill(0, 0, canvas.width, canvas.height, rgb(0x10, 0x14, 0x18));
        let (width, height) = (canvas.width as f64, canvas.height as f64);
        if self.session.lines.is_empty() {
            canvas.use_font(self.lyric_font);
            canvas.text_in(0, 0, canvas.width, canvas.height, "Choose a song", rgb(0x77, 0x77, 0x88), true);
            return;
        }
        let size = (canvas.height / 6).clamp(self.px(18), self.px(46));
        let colour = self.sung_colour;
        let rows = model::stage_lines(&self.session.lines, ms, self.session.cursor);
        for (row_number, li) in rows.into_iter().enumerate() {
            let line = self.session.lines[li].clone();
            let texts: Vec<String> = line.tokens.iter().map(|token| token.text.clone()).collect();
            let shown = self.line_readings(&line).cloned().unwrap_or_default();
            let groups = reading_groups(texts.len(), &shown);
            let y = (height * if row_number == 0 { 0.35 } else { 0.7 }) as i32;
            // Each group is as wide as its characters or its reading, whichever is wider; shrink lines too wide.
            let mut scale = 1.0;
            let mut layout = Vec::new();
            let mut total = 0.0;
            let (mut font, mut ruby) = (0, 0);
            for _ in 0..3 {
                font = self.stage_font(((size as f64) * scale) as i32, true);
                ruby = self.stage_font(((size as f64) * scale * 0.42) as i32, false);
                layout.clear();
                total = 0.0;
                for (first, count, reading) in &groups {
                    canvas.use_font(font);
                    let widths: Vec<f64> = texts[*first..first + count].iter().map(|text| canvas.measure(text).0 as f64).collect();
                    canvas.use_font(ruby);
                    let need = reading.as_ref().map(|reading| canvas.measure(&format!("{reading} ")).0 as f64).unwrap_or(0.0);
                    let sum: f64 = widths.iter().sum();
                    let pad = (need - sum).max(0.0) / *count as f64;
                    layout.push((*first, *count, reading.clone(), total, widths.iter().map(|width| width + pad).collect::<Vec<f64>>(), pad));
                    total += sum + pad * *count as f64;
                }
                if total <= width * 0.94 {
                    break;
                }
                scale *= width * 0.94 / total;
            }
            let font_height = ((size as f64) * scale) as i32;
            // Staggered like the video: the first row leans left, the second right.
            let spare = (width - total - 24.0).max(0.0);
            let left = 12.0 + if row_number > 0 { spare * 0.8 } else { spare * 0.2 };
            for (first, count, reading, offset, advances, pad) in layout {
                let mut x = left + offset;
                let group_sung = matches!(line.tokens[first].start, Some(start) if ms >= start);
                if let Some(reading) = reading {
                    canvas.use_font(ruby);
                    let group_width: f64 = advances.iter().sum();
                    let reading_width = canvas.measure(&reading).0 as f64;
                    canvas.text((x + (group_width - reading_width) / 2.0) as i32, y - font_height - font_height / 2, &reading, if group_sung { colour } else { rgb(0xe0, 0xe0, 0xe0) });
                }
                canvas.use_font(font);
                for number in 0..count {
                    let token = &line.tokens[first + number];
                    let advance = advances[number];
                    let sung = matches!(token.start, Some(start) if ms >= start);
                    let inner = (x + pad / 2.0) as i32;
                    let top = y - font_height / 2 - font_height / 4;
                    canvas.text(inner + 2, top + 2, &token.text, rgb(0, 0, 0));
                    canvas.text(inner, top, &token.text, if sung { colour } else { rgb(255, 255, 255) });
                    if let Some(start) = token.start {
                        if start <= ms && token.end.map_or(true, |end| ms < end) {
                            let end = token.end.unwrap_or(ms + 1);
                            let share = ((ms - start) as f64 / (end - start).max(1) as f64).min(1.0);
                            let bar = y + font_height * 3 / 4;
                            canvas.fill(x as i32, bar, (x + advance * share) as i32, bar + self.px(4), colour);
                        }
                    }
                    x += advance;
                }
            }
        }
        canvas.use_font(self.ruler_font);
        canvas.text(self.px(8), canvas.height - self.px(18), &clock(ms), rgb(0x66, 0x66, 0x77));
    }

    // The timeline
    fn x_of(&self, ms: i64) -> i32 {
        (ms as f64 * self.pps / 1000.0).round() as i32
    }

    fn ms_of(&self, x: i32) -> i64 {
        (x as f64 * 1000.0 / self.pps) as i64
    }

    fn timeline_end(&self) -> i64 {
        let last = self.session.lines.iter().filter_map(Line::span).map(|(_, end)| end).max().unwrap_or(0);
        self.duration.max(last + 5000)
    }

    fn follow(&mut self, ms: i64, force: bool) {
        let width = win::client_size(self.timeline).0;
        let x = self.x_of(ms) - self.left;
        if force || x < 0 || x > width * 85 / 100 {
            self.left = (self.x_of(ms) - width / 5).max(0);
        }
    }

    fn zoom(&mut self, factor: f64, around: Option<i32>) {
        let width = win::client_size(self.timeline).0;
        let around = around.unwrap_or(width / 2);
        let anchor = self.ms_of(self.left + around);
        let scale = self.dpi as f64 / 96.0;
        self.pps = (self.pps * factor).clamp(20.0 * scale, 1500.0 * scale);
        self.left = (self.x_of(anchor) - around).max(0);
        win::redraw(self.timeline);
    }

    /// The end shown for a word: its end, or the play position while it is held.
    fn shown_end(&mut self, place: Place) -> Option<(i64, i64)> {
        let holding = self.session.holding == Some(place);
        let position = if holding { self.position() } else { 0 };
        let token = model::token(&self.session.lines, Some(place))?;
        let start = token.start?;
        let end = match token.end {
            Some(end) => end,
            None if holding => (start + 100).max(position),
            None => start + OPEN_TOKEN_MS,
        };
        Some((start, end))
    }

    /// Lines in drawing order: those with no word timed yet first (their times come from a lyric file).
    fn drawing_order(&self) -> Vec<usize> {
        let mut order: Vec<usize> = (0..self.session.lines.len()).collect();
        order.sort_by_key(|li| self.session.lines[*li].started());
        order
    }

    fn paint_timeline(&mut self, canvas: &Canvas) {
        let dpi = self.dpi as i32;
        let px = move |value: i32| value * dpi / 96;
        let (width, height) = (canvas.width, canvas.height);
        let total = self.x_of(self.timeline_end()) + px(50);
        self.left = win::set_scroll(self.timeline, SB_HORZ, total, width, self.left);
        let left = self.left;
        canvas.fill(0, 0, width, height, rgb(0x1e, 0x23, 0x28));
        let (start_ms, end_ms) = (self.ms_of(left), self.ms_of(left + width));
        let step = [100, 250, 500, 1000, 2000, 5000, 10000, 30000, 60000].iter().copied().find(|step| self.x_of(*step) >= px(60)).unwrap_or(60000);
        canvas.use_font(self.ruler_font);
        let mut tick = start_ms - start_ms % step;
        while tick <= end_ms + step {
            let x = self.x_of(tick) - left;
            canvas.line(x, 0, x, px(RULER) - px(4), rgb(0x55, 0x55, 0x66));
            let label = clock(tick);
            canvas.text(x + px(3), 0, if step >= 1000 { &label[..5] } else { &label[..label.len() - 1] }, rgb(0x88, 0x88, 0x99));
            tick += step;
        }
        if !self.peaks.is_empty() {
            let (middle, half) = ((px(WAVE_TOP) + px(WAVE_BOTTOM)) / 2, (px(WAVE_BOTTOM) - px(WAVE_TOP)) / 2);
            let mut x = 0;
            while x < width {
                let low = (self.ms_of(left + x) / 10).max(0) as usize;
                let high = ((self.ms_of(left + x + 2) / 10) as usize).max(low + 1);
                let peak = self.peaks.get(low..high.min(self.peaks.len())).and_then(|chunk| chunk.iter().copied().max()).unwrap_or(0);
                let level = (peak as i64 * half as i64 / self.peak_max as i64) as i32;
                if level > 0 {
                    canvas.fill(x, middle - level, x + 2, middle + level + 1, rgb(0x3d, 0x6f, 0x8f));
                }
                x += 2;
            }
        }
        for li in self.drawing_order() {
            let line = self.session.lines[li].clone();
            let (span_start, span_end) = match line.span() {
                Some(span) => span,
                None => continue,
            };
            if span_end < start_ms - 1000 || span_start > end_ms + 1000 {
                continue;
            }
            let guessed = !line.started();
            let (x1, x2) = (self.x_of(span_start) - left, self.x_of(span_end) - left);
            if guessed {
                canvas.dotted_rect(x1, px(LINE_TOP), x2, px(LINE_BOTTOM), rgb(0x60, 0x7d, 0x8b));
            } else {
                canvas.rect(x1, px(LINE_TOP), x2, px(LINE_BOTTOM), Some(rgb(0x37, 0x47, 0x4f)), rgb(0x60, 0x7d, 0x8b));
            }
            canvas.use_font(self.small_font);
            let saved = canvas.clip(x1, px(LINE_TOP), x2.max(x1 + px(200)), px(LINE_BOTTOM));
            canvas.text_in(x1 + px(3), px(LINE_TOP), x2.max(x1 + px(400)), px(LINE_BOTTOM), &format!("{}. {}", li + 1, line.text()), if guessed { rgb(0x78, 0x90, 0x9c) } else { rgb(0xcf, 0xd8, 0xdc) }, false);
            canvas.unclip(saved);
            for ti in 0..line.tokens.len() {
                let (start, end) = match self.shown_end((li, ti)) {
                    Some(times) => times,
                    None => continue,
                };
                let token = &line.tokens[ti];
                let fill = if Some((li, ti)) == self.session.selected {
                    rgb(0xff, 0x8f, 0x00)
                } else if token.end.is_none() {
                    rgb(0xe6, 0x51, 0x00)
                } else if li % 2 == 0 {
                    rgb(0x2e, 0x7d, 0x32)
                } else {
                    rgb(0x15, 0x65, 0xc0)
                };
                let (x1, x2) = (self.x_of(start) - left, self.x_of(end) - left);
                canvas.rect(x1, px(WORD_TOP), x2.max(x1 + 2), px(WORD_BOTTOM), Some(fill), rgb(0xe0, 0xe0, 0xe0));
                if x2 - x1 > px(10) {
                    canvas.text_in(x1, px(WORD_TOP), x2, px(WORD_BOTTOM), token.text.trim(), rgb(255, 255, 255), true);
                }
            }
        }
        let position = self.position();
        let x = self.x_of(position) - left;
        canvas.fill(x - 1, 0, x + 1, px(LINE_BOTTOM) + px(6), rgb(0xff, 0x52, 0x52));
    }

    /// The word under a timeline point (in timeline coordinates).
    fn word_at(&mut self, x: i32, y: i32) -> Option<Place> {
        if !(self.px(WORD_TOP)..=self.px(WORD_BOTTOM)).contains(&y) {
            return None;
        }
        let x = x + self.left;
        let mut found = None;
        for li in self.drawing_order() {
            for ti in 0..self.session.lines[li].tokens.len() {
                if let Some((start, end)) = self.shown_end((li, ti)) {
                    let (x1, x2) = (self.x_of(start), self.x_of(end).max(self.x_of(start) + 2));
                    if x1 - 1 <= x && x <= x2 + 1 {
                        found = Some((li, ti));
                    }
                }
            }
        }
        found
    }

    fn edge(&self, place: Place, x: i32) -> Part {
        let x = x + self.left;
        match model::token(&self.session.lines, Some(place)) {
            Some(token) if token.timed() => {
                if (x - self.x_of(token.start.unwrap_or(0))).abs() <= self.px(EDGE_PX) {
                    Part::Start
                } else if (x - self.x_of(token.end.unwrap_or(0))).abs() <= self.px(EDGE_PX) {
                    Part::End
                } else {
                    Part::Move
                }
            }
            _ => Part::Move,
        }
    }

    fn press_timeline(&mut self, x: i32, y: i32, shared: &Shared) {
        unsafe {
            SetFocus(self.timeline);
            SetCapture(self.timeline);
        }
        let place = match self.word_at(x, y) {
            Some(place) => place,
            None => {
                if (self.px(LINE_TOP)..=self.px(LINE_BOTTOM)).contains(&y) {
                    let ms = self.ms_of(x + self.left);
                    let hit = self.drawing_order().into_iter().rev().find(|li| matches!(self.session.lines[*li].span(), Some((start, end)) if start <= ms && ms <= end));
                    if let Some(li) = hit {
                        self.session.cursor = Some((li, 0));
                        self.session.selected = Some((li, 0));
                        self.scroll_to = self.session.cursor;
                    }
                }
                self.drag = None;
                let at = self.ms_of(x + self.left);
                self.seek(at, shared);
                return;
            }
        };
        let (start, end) = match model::token(&self.session.lines, Some(place)) {
            Some(token) => match token.start {
                Some(start) => (start, token.end),
                None => return,
            },
            None => return,
        };
        self.session.remember();
        let end = end.unwrap_or(start + OPEN_TOKEN_MS);
        if let Some(token) = model::token_mut(&mut self.session.lines, Some(place)) {
            token.end = Some(end);
        }
        self.session.selected = Some(place);
        self.drag = Some(Drag { place, part: self.edge(place, x), x, start, end });
        self.changed();
    }

    fn drag_timeline(&mut self, x: i32) {
        let (place, part, start, end, delta) = match &self.drag {
            Some(drag) => (drag.place, drag.part, drag.start, drag.end, self.ms_of(x - drag.x + self.left) - self.ms_of(self.left)),
            None => return,
        };
        let (low, high) = model::limits(&self.session.lines, place);
        let token = match model::token_mut(&mut self.session.lines, Some(place)) {
            Some(token) => token,
            None => return,
        };
        match part {
            Part::Start => token.start = Some((start + delta).min(end - MIN_TOKEN_MS).max(low)),
            Part::End => token.end = Some((end + delta).max(start + MIN_TOKEN_MS).min(high)),
            Part::Move => {
                let delta = delta.min(high - end).max(low - start);
                token.start = Some(start + delta);
                token.end = Some(end + delta);
            }
        }
        let (start, end) = (token.start.unwrap_or(0), token.end.unwrap_or(0));
        win::set_text(self.time, &format!("{} – {}", clock(start), clock(end)));
        win::redraw(self.timeline);
    }

    fn double_timeline(&mut self, x: i32, y: i32, shared: &Shared) {
        if let Some(place) = self.word_at(x, y) {
            if let Some(start) = model::token(&self.session.lines, Some(place)).and_then(|token| token.start) {
                self.drag = None;
                self.session.cursor = Some(place);
                self.session.selected = Some(place);
                self.scroll_to = Some(place);
                self.seek(start - 500, shared);
            }
        }
    }

    fn timeline_cursor(&mut self) {
        let mut point = POINT::default();
        unsafe {
            GetCursorPos(&mut point);
            ScreenToClient(self.timeline, &mut point);
        }
        let shape = match self.word_at(point.x, point.y) {
            Some(place) if self.edge(place, point.x) != Part::Move => IDC_SIZEWE,
            Some(_) => IDC_SIZEALL,
            None => IDC_IBEAM,
        };
        win::set_cursor(shape);
    }

    pub fn pane_message(&mut self, hwnd: HWND, message: u32, wparam: WPARAM, lparam: LPARAM, shared: &mut Shared) -> Option<LRESULT> {
        if ![self.lyrics, self.stage, self.timeline].contains(&hwnd) {
            return None;
        }
        let (x, y) = win::mouse_point(lparam);
        match message {
            WM_ERASEBKGND => return Some(1),
            WM_PAINT => {
                if hwnd == self.lyrics {
                    win::paint(hwnd, |canvas| self.paint_lyrics(canvas));
                } else if hwnd == self.stage {
                    win::paint(hwnd, |canvas| self.paint_stage(canvas));
                } else {
                    win::paint(hwnd, |canvas| self.paint_timeline(canvas));
                }
            }
            WM_SIZE => win::redraw(hwnd),
            WM_VSCROLL if hwnd == self.lyrics => {
                self.lyrics_scroll = win::scrolled(hwnd, SB_VERT, wparam, self.px(30));
                win::redraw(hwnd);
            }
            WM_HSCROLL if hwnd == self.timeline => {
                self.left = win::scrolled(hwnd, SB_HORZ, wparam, self.px(40));
                win::redraw(hwnd);
            }
            WM_MOUSEWHEEL => {
                let delta = (win::hiword(wparam) as u16 as i16) as i32;
                if hwnd == self.lyrics {
                    self.lyrics_scroll = (self.lyrics_scroll - delta * self.px(60) / 120).max(0);
                    win::redraw(hwnd);
                } else if hwnd == self.timeline {
                    if win::key_down(VK_CONTROL) {
                        let mut point = POINT::default();
                        unsafe {
                            GetCursorPos(&mut point);
                            ScreenToClient(hwnd, &mut point);
                        }
                        self.zoom(if delta > 0 { 1.25 } else { 0.8 }, Some(point.x));
                    } else {
                        self.left = (self.left - delta * self.px(90) / 120).max(0);
                        win::redraw(hwnd);
                    }
                }
            }
            WM_LBUTTONDOWN if hwnd == self.lyrics => self.click_lyrics(x, y, shared),
            WM_LBUTTONDOWN if hwnd == self.timeline => self.press_timeline(x, y, shared),
            WM_LBUTTONDBLCLK if hwnd == self.timeline => self.double_timeline(x, y, shared),
            WM_MOUSEMOVE if hwnd == self.timeline && self.drag.is_some() => self.drag_timeline(x),
            WM_LBUTTONUP if hwnd == self.timeline => {
                unsafe { ReleaseCapture() };
                if self.drag.take().is_some() {
                    self.changed();
                }
            }
            WM_SETCURSOR if hwnd == self.timeline && win::loword(lparam as usize) == 1 => {
                self.timeline_cursor();
                return Some(1);
            }
            _ => return None,
        }
        Some(0)
    }
}

/// The window font, for the few texts drawn on panes in it.
fn win_font_ui() -> HFONT {
    unsafe { GetStockObject(17) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn readings_group_the_tokens_under_them() {
        let shown = vec![Some("chong".to_string()), Some("lai".to_string()), None, Some("tokyo".to_string()), Some(String::new())];
        assert_eq!(reading_groups(5, &shown), vec![(0, 1, Some("chong".into())), (1, 1, Some("lai".into())), (2, 1, None), (3, 2, Some("tokyo".into()))]);
        // Readings not known yet: every token on its own.
        assert_eq!(reading_groups(2, &[]), vec![(0, 1, None), (1, 1, None)]);
    }
}
