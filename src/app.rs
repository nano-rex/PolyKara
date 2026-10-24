//! The main window: its two tabs, the engine and pipeline runs they share, and message routing.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicIsize, Ordering};
use std::sync::Arc;
use std::time::Instant;

use polykara::engine::{Cli, Engine};
use polykara::json::Value;
use polykara::runner::{step_name, Event, Runner};
use polykara::tools::Wake;

use crate::editor::{AfterSave, EditorPage};
use crate::settings::SettingsWindow;
use crate::songs::SongsPage;
use crate::win::{self, ffi::*};

pub type Song = HashMap<String, String>;

pub const MAIN_CLASS: &str = "PolyKaraMain";
pub const PANE_CLASS: &str = "PolyKaraPane";
pub const DIALOG_CLASS: &str = "PolyKaraDialog";
const WM_WAKE: u32 = WM_APP + 1;
const POLL_TIMER: usize = 1;
pub const PLAY_TIMER: usize = 2;
const ID_TABS: u16 = 10;
const ID_SETTINGS: u16 = 11;
pub const IDOK: u16 = 1;
pub const IDCANCEL: u16 = 2;

static MAIN: AtomicIsize = AtomicIsize::new(0);

thread_local! {
    static APP: RefCell<Option<App>> = RefCell::new(None);
}

/// Run `action` on the app, unless it is already busy (a message box or file dialog is open).
fn with_app<R>(action: impl FnOnce(&mut App) -> R) -> Option<R> {
    APP.with(|cell| match cell.try_borrow_mut() {
        Ok(mut app) => app.as_mut().map(action),
        Err(_) => None,
    })
}

/// Start-up options: `--open <song id>` opens a song in the timing editor, `--settings` the settings.
#[derive(Clone, Debug, Default)]
pub struct Options {
    pub open: Option<String>,
    pub settings: bool,
}

impl Options {
    pub fn from_args(args: impl Iterator<Item = String>) -> Options {
        let mut options = Options::default();
        let mut args = args.peekable();
        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--open" => options.open = args.next(),
                "--settings" => options.settings = true,
                _ => {}
            }
        }
        options
    }
}

#[derive(Clone, Debug, Default)]
pub struct Folders {
    pub raw: PathBuf,
    pub audio: PathBuf,
    pub ass: PathBuf,
    pub output: PathBuf,
    pub lyrics: PathBuf,
}

/// What the engine says about this installation.
#[derive(Clone, Debug, Default)]
pub struct Hello {
    pub version: String,
    pub home: PathBuf,
    pub manifest: PathBuf,
    pub config: PathBuf,
    pub folders: Folders,
    pub media_suffixes: Vec<String>,
    pub tools: HashMap<String, String>,
}

impl Hello {
    fn from_json(value: &Value) -> Hello {
        let path = |key: &str| PathBuf::from(value.get(key).str());
        let folder = |key: &str| PathBuf::from(value.get("folders").get(key).str());
        Hello {
            version: value.get("version").str().to_string(),
            home: path("home"),
            manifest: path("manifest"),
            config: path("config"),
            folders: Folders { raw: folder("raw"), audio: folder("audio"), ass: folder("ass"), output: folder("output"), lyrics: folder("lyrics") },
            media_suffixes: value.get("media_suffixes").strings(),
            tools: value.get("tools").pairs().iter().filter_map(|(name, place)| place.as_str().map(|place| (name.clone(), place.to_string()))).collect(),
        }
    }
}

/// Why a request was sent to the engine, so its answer reaches the right place.
pub enum Want {
    Hello,
    Songs,
    AddFiles(usize),
    Describe,
    Relative(String),
    NewId,
    Load(String),
    Audio(String),
    Saved(AfterSave),
    Retext,
    Readings(String, Vec<Vec<String>>),
    Library,
    LibrarySaved,
    SettingsForm,
    SettingsText,
    SettingsSaved,
    Fonts,
}

fn names(folder: &Path) -> HashSet<String> {
    std::fs::read_dir(folder).map(|entries| entries.filter_map(|entry| entry.ok()).map(|entry| entry.file_name().to_string_lossy().into_owned()).collect()).unwrap_or_default()
}

/// What is done for a song, from the file names in the output, subtitle and download folders.
pub fn song_state(sid: &str, output: &HashSet<String>, ass: &HashSet<String>, raw: &HashSet<String>, suffixes: &[String]) -> &'static str {
    if output.contains(&format!("{sid}.mp4")) {
        return "video ready";
    }
    if [format!("{sid}.edited.ass"), format!("{sid}.auto.ass"), format!("{sid}.ass")].iter().any(|name| ass.contains(name)) {
        return "subtitles ready";
    }
    let prefix = format!("{sid}.");
    let downloaded = raw.iter().any(|name| match name.strip_prefix(&prefix) {
        Some(extension) => !extension.contains('.') && suffixes.iter().any(|suffix| suffix.eq_ignore_ascii_case(&format!(".{extension}"))),
        None => false,
    });
    if downloaded {
        "downloaded"
    } else {
        "waiting"
    }
}

pub fn song_json(fields: &[String], song: &Song) -> Value {
    let mut pairs: Vec<(String, Value)> = fields.iter().map(|field| (field.clone(), Value::from(song.get(field).cloned().unwrap_or_default()))).collect();
    for (key, value) in song {
        if !fields.contains(key) {
            pairs.push((key.clone(), Value::from(value)));
        }
    }
    Value::Object(pairs)
}

/// What both tabs share: the engine, the song list, the current run and the window's fonts.
pub struct Shared {
    pub main: HWND,
    pub status: HWND,
    pub log: HWND,
    pub engine: Engine<Want>,
    pub cli: Option<Cli>,
    pub hello: Option<Hello>,
    pub fields: Vec<String>,
    pub songs: Vec<Song>,
    pub runner: Runner,
    pub wake: Wake,
    /// Songs queued or being worked on in the current run, with what is happening to them.
    pub working: HashMap<String, String>,
    pub current: Option<String>,
    pub states: HashMap<String, &'static str>,
    states_read: Option<Instant>,
    /// The song list changed and the table must be filled again.
    pub songs_changed: bool,
    /// Only the status column changed.
    pub states_changed: bool,
    pub dpi: u32,
    pub font: HFONT,
    pub bold: HFONT,
    pub mono: HFONT,
    /// Every control, so a DPI change can give them all the new font.
    pub controls: Vec<HWND>,
}

impl Shared {
    pub fn px(&self, value: i32) -> i32 {
        value * self.dpi as i32 / 96
    }

    /// Give a new control the window font and remember it.
    pub fn styled(&mut self, hwnd: HWND) -> HWND {
        win::set_font(hwnd, self.font);
        self.controls.push(hwnd);
        hwnd
    }

    pub fn tool(&self, name: &str) -> PathBuf {
        self.hello.as_ref().and_then(|hello| hello.tools.get(name).cloned()).map(PathBuf::from).unwrap_or_else(|| PathBuf::from(name))
    }

    pub fn home(&self) -> Option<PathBuf> {
        self.hello.as_ref().map(|hello| hello.home.clone())
    }

    pub fn visible_songs(&self) -> Vec<&Song> {
        self.songs.iter().filter(|song| matches!(song.get("id"), Some(id) if !id.is_empty() && id != "example")).collect()
    }

    pub fn song(&self, sid: &str) -> Option<&Song> {
        self.songs.iter().find(|song| song.get("id").map(String::as_str) == Some(sid))
    }

    pub fn set_status(&self, text: &str) {
        win::set_text(self.status, text);
    }

    pub fn write(&self, text: &str) {
        win::append(self.log, &format!("{text}\n"));
    }

    pub fn info(&self, text: &str) {
        win::info(self.main, text);
    }

    pub fn warn(&self, text: &str) {
        win::warn(self.main, text);
    }

    pub fn save_songs(&mut self) {
        let songs = Value::Array(self.songs.iter().map(|song| song_json(&self.fields, song)).collect());
        let fields = Value::from(self.fields.clone());
        self.engine.ask("save_songs", Value::object(vec![("fields", fields), ("songs", songs)]), Want::Songs);
        self.songs_changed = true;
        self.refresh_states();
    }

    pub fn refresh_states(&mut self) {
        let hello = match &self.hello {
            Some(hello) => hello,
            None => return,
        };
        let (output, ass, raw) = (names(&hello.folders.output), names(&hello.folders.ass), names(&hello.folders.raw));
        let states: HashMap<String, &'static str> = self.songs.iter().filter_map(|song| song.get("id")).map(|sid| (sid.clone(), song_state(sid, &output, &ass, &raw, &hello.media_suffixes))).collect();
        if states != self.states {
            self.states = states;
            self.states_changed = true;
        }
        self.states_read = Some(Instant::now());
    }

    pub fn state(&self, sid: &str) -> String {
        self.working.get(sid).cloned().unwrap_or_else(|| self.states.get(sid).copied().unwrap_or("waiting").to_string())
    }

    /// Run the PolyKara command line with these arguments, showing its output in the log.
    pub fn start(&mut self, args: Vec<String>) -> bool {
        let cli = match self.cli.clone() {
            Some(cli) => cli,
            None => {
                self.warn("The PolyKara command line (polykara-cli) was not found next to this program.");
                return false;
            }
        };
        let shown = cli.shown(&args);
        self.start_command(cli.command(args), shown)
    }

    pub fn start_command(&mut self, mut command: std::process::Command, shown: String) -> bool {
        if self.runner.running() {
            self.info("Wait for the current run to finish, or stop it.");
            return false;
        }
        if let Some(home) = self.home() {
            std::fs::create_dir_all(&home).ok();
            command.current_dir(&home).env("POLYKARA_HOME", &home);
        }
        self.write("");
        self.write(&format!("$ {shown}"));
        match self.runner.start(command, &self.wake) {
            Ok(()) => {
                self.set_status("Running…");
                true
            }
            Err(error) => {
                self.write(&format!("Could not start: {error}"));
                false
            }
        }
    }

    fn follow(&mut self, event: Event) {
        match event {
            Event::Output(line) => self.write(&line),
            Event::Progress { step, id } => {
                // The song before is done with: show its new state.
                if let Some(previous) = self.current.take() {
                    if previous != id {
                        self.working.remove(&previous);
                        self.refresh_states();
                    }
                }
                self.working.insert(id.clone(), step_name(&step));
                self.current = Some(id);
                self.states_changed = true;
            }
            Event::Finished(code) => {
                let code = code.unwrap_or(-1);
                self.set_status(&if code == 0 { "Finished".to_string() } else { format!("Finished with problems (exit code {code}); see the summary above") });
                self.write(&format!("[finished, exit code {code}]"));
                self.working.clear();
                self.current = None;
                self.states_changed = true;
                // A chart download may have added songs.
                self.engine.ask("songs", Value::object::<&str>(vec![]), Want::Songs);
                self.refresh_states();
            }
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Songs,
    Editor,
}

pub struct App {
    pub shared: Shared,
    tabs: HWND,
    settings_button: HWND,
    tab: Tab,
    songs: SongsPage,
    editor: EditorPage,
    settings: Option<SettingsWindow>,
    options: Options,
    /// Quit once the timing being saved is written.
    quit_after_save: bool,
}

fn ui_fonts(dpi: u32) -> (HFONT, HFONT, HFONT) {
    (win::font("Segoe UI", 9.0, false, dpi), win::font("Segoe UI", 9.0, true, dpi), win::font("Consolas", 9.0, false, dpi))
}

impl App {
    fn create(options: Options) -> App {
        let main = win::top_window(MAIN_CLASS, "PolyKara", WS_OVERLAPPEDWINDOW | WS_CLIPCHILDREN, 0, 1240, 800);
        MAIN.store(main, Ordering::SeqCst);
        let dpi = win::dpi(main);
        // Resize for the screen's scaling (the window was created at 96 DPI sizes).
        unsafe { SetWindowPos(main, 0, 0, 0, 1240 * dpi as i32 / 96, 800 * dpi as i32 / 96, SWP_NOZORDER | SWP_NOACTIVATE | 0x0002) };
        let (font, bold, mono) = ui_fonts(dpi);
        let wake: Wake = Arc::new(|| {
            unsafe { PostMessageW(MAIN.load(Ordering::SeqCst), WM_WAKE, 0, 0) };
        });
        let cli = Cli::find();
        let home = std::env::var_os("POLYKARA_HOME").map(PathBuf::from);
        let mut engine = Engine::start(cli.as_ref().unwrap_or(&Cli { program: "polykara-cli".into(), prefix: Vec::new() }), home.as_deref(), wake.clone());
        if cli.is_none() && engine.failure.is_some() {
            engine.failure = Some("The PolyKara command line (polykara-cli) was not found next to this program. Reinstall PolyKara.".into());
        }
        engine.ask("hello", Value::object::<&str>(vec![]), Want::Hello);
        let status = win::label(main, "Starting…");
        let mut shared = Shared {
            main,
            status,
            log: 0,
            engine,
            cli,
            hello: None,
            fields: Vec::new(),
            songs: Vec::new(),
            runner: Runner::default(),
            wake,
            working: HashMap::new(),
            current: None,
            states: HashMap::new(),
            states_read: None,
            songs_changed: false,
            states_changed: false,
            dpi,
            font,
            bold,
            mono,
            controls: Vec::new(),
        };
        shared.styled(status);
        let tabs = shared.styled(win::child("SysTabControl32", "", WS_CLIPSIBLINGS | WS_TABSTOP, main, ID_TABS));
        win::add_tab(tabs, 0, "   Songs   ");
        win::add_tab(tabs, 1, "   Timing editor   ");
        let settings_button = shared.styled(win::button(main, ID_SETTINGS, "Settings"));
        let songs = SongsPage::create(&mut shared);
        let editor = EditorPage::create(&mut shared);
        unsafe { SetTimer(main, POLL_TIMER, 100, std::ptr::null()) };
        let mut app = App { shared, tabs, settings_button, tab: Tab::Songs, songs, editor, settings: None, options, quit_after_save: false };
        app.show_tab(Tab::Songs);
        app.layout();
        app
    }

    fn show_tab(&mut self, tab: Tab) {
        self.tab = tab;
        win::set_tab(self.tabs, if tab == Tab::Songs { 0 } else { 1 });
        self.songs.show(tab == Tab::Songs);
        self.editor.show(tab == Tab::Editor);
        if tab == Tab::Editor {
            self.editor.focus();
        }
    }

    fn layout(&mut self) {
        let shared = &self.shared;
        let (width, height) = win::client_size(shared.main);
        let (tab_height, status_height, gap) = (shared.px(30), shared.px(24), shared.px(8));
        win::place(self.tabs, gap, shared.px(4), width - 2 * gap, tab_height);
        win::place(self.settings_button, width - gap - shared.px(90), shared.px(5), shared.px(86), tab_height - shared.px(6));
        win::place(shared.status, gap, height - status_height, width - 2 * gap, status_height);
        let (top, bottom) = (shared.px(4) + tab_height + shared.px(4), height - status_height);
        self.songs.layout(shared, gap, top, width - 2 * gap, bottom - top);
        self.editor.layout(shared, gap, top, width - 2 * gap, bottom - top);
    }

    fn open_settings(&mut self) {
        if self.settings.is_none() && self.shared.hello.is_some() {
            self.settings = Some(SettingsWindow::open(&mut self.shared));
        }
    }

    fn open_in_editor(&mut self, sid: &str) {
        self.show_tab(Tab::Editor);
        self.editor.open(sid, &mut self.shared);
    }

    fn poll(&mut self) {
        for (want, result) in self.shared.engine.poll() {
            self.answer(want, result);
        }
        self.editor.poll();
        for event in self.shared.runner.poll() {
            self.shared.follow(event);
        }
        if self.tab == Tab::Songs && matches!(self.shared.states_read, Some(read) if read.elapsed().as_secs() >= 3) {
            self.shared.refresh_states();
        }
        self.after();
    }

    /// Bring the song table up to date after anything changed.
    fn after(&mut self) {
        if self.shared.songs_changed {
            self.shared.songs_changed = false;
            self.shared.states_changed = false;
            self.songs.fill(&self.shared);
        } else if self.shared.states_changed {
            self.shared.states_changed = false;
            self.songs.update_states(&self.shared);
        }
        self.songs.update_buttons(&self.shared);
    }

    fn answer(&mut self, want: Want, result: Result<Value, String>) {
        match want {
            Want::Hello => match result {
                Ok(value) => {
                    let hello = Hello::from_json(&value);
                    let title = win::wide(&format!("PolyKara {} — {}", hello.version, hello.home.display()));
                    unsafe { SetWindowTextW(self.shared.main, title.as_ptr()) };
                    self.shared.set_status(&format!("Songs: {}", hello.manifest.display()));
                    self.shared.hello = Some(hello);
                    let (ffplay, ffmpeg) = (self.shared.tool("ffplay"), self.shared.tool("ffmpeg"));
                    self.editor.set_tools(ffplay, ffmpeg);
                    self.shared.engine.ask("songs", Value::object::<&str>(vec![]), Want::Songs);
                    if self.options.settings {
                        self.open_settings();
                    }
                }
                Err(error) => {
                    self.shared.set_status("The PolyKara engine did not start; see the log");
                    self.shared.write(&error);
                    self.shared.warn(&error);
                }
            },
            Want::Songs | Want::AddFiles(_) => match result {
                Ok(value) => {
                    if !value.get("fields").is_null() {
                        self.shared.fields = value.get("fields").strings();
                    }
                    if !value.get("songs").is_null() {
                        self.shared.songs = value.get("songs").items().iter().map(|song| song.pairs().iter().map(|(key, value)| (key.clone(), value.str().to_string())).collect()).collect();
                    }
                    self.shared.songs_changed = true;
                    self.shared.refresh_states();
                    if let Want::AddFiles(chosen) = want {
                        self.after();
                        self.songs.added(&value, chosen, &self.shared);
                    }
                    if let Some(sid) = self.options.open.take() {
                        self.after();
                        self.open_in_editor(&sid);
                    }
                }
                Err(error) => {
                    self.shared.write(&format!("Songs: {error}"));
                    self.shared.warn(&format!("The song list could not be read or saved:\n{error}"));
                }
            },
            Want::Describe | Want::Relative(_) | Want::NewId => self.songs.answer(want, result, &mut self.shared),
            Want::SettingsForm | Want::SettingsText | Want::SettingsSaved | Want::Fonts => {
                if let Some(settings) = &mut self.settings {
                    settings.answer(want, result, &mut self.shared);
                }
            }
            want => {
                let quit = matches!(want, Want::Saved(_)) && self.quit_after_save;
                let saved = self.editor.answer(want, result, &mut self.shared);
                if quit {
                    self.quit_after_save = false;
                    if saved {
                        self.quit();
                    }
                }
            }
        }
    }

    fn quit(&mut self) {
        self.editor.stop();
        unsafe { DestroyWindow(self.shared.main) };
    }

    fn close(&mut self) {
        if self.quit_after_save {
            return;
        }
        match self.editor.confirm_discard(&mut self.shared) {
            Some(true) => {}
            Some(false) => {
                // Saving: quit when the engine has written the file.
                self.quit_after_save = true;
                return;
            }
            None => return,
        }
        if self.shared.runner.running() {
            if !win::confirm(self.shared.main, "A run is still going. Stop it and quit?") {
                return;
            }
            self.shared.runner.stop();
        }
        self.quit();
    }

    /// Keys for the timing editor, taken before the focused control sees them. True when used.
    fn editor_key(&mut self, msg: &MSG) -> bool {
        if self.tab != Tab::Editor || self.dialog_open() {
            return false;
        }
        if unsafe { GetAncestor(msg.hwnd, 2) } != self.shared.main {
            return false;
        }
        // Typing in a text box (or a combo box's text) keeps its keys.
        if win::class_name(unsafe { GetFocus() }).eq_ignore_ascii_case("Edit") {
            return false;
        }
        let repeat = msg.lParam & (1 << 30) != 0;
        self.editor.key(msg.wParam as u32, msg.message == WM_KEYDOWN, repeat, &mut self.shared)
    }

    fn dialog_open(&self) -> bool {
        self.settings.is_some() || self.songs.dialog().is_some() || self.editor.dialog().is_some()
    }

    /// The dialog window that should get Tab and Enter handling, if one is open.
    fn active_dialog(&self) -> Option<HWND> {
        self.editor.dialog().or_else(|| self.songs.dialog()).or_else(|| self.settings.as_ref().map(SettingsWindow::hwnd))
    }

    fn main_message(&mut self, message: u32, wparam: WPARAM, lparam: LPARAM) -> Option<LRESULT> {
        match message {
            WM_SIZE => self.layout(),
            WM_GETMINMAXINFO => {
                let info = unsafe { &mut *(lparam as *mut MINMAXINFO) };
                info.ptMinTrackSize = POINT { x: self.shared.px(900), y: self.shared.px(560) };
            }
            WM_DPICHANGED => {
                self.shared.dpi = win::hiword(wparam);
                let (font, bold, mono) = ui_fonts(self.shared.dpi);
                let old = [self.shared.font, self.shared.bold, self.shared.mono];
                self.shared.font = font;
                self.shared.bold = bold;
                self.shared.mono = mono;
                for control in &self.shared.controls {
                    win::set_font(*control, font);
                }
                win::set_font(self.shared.log, mono);
                self.editor.dpi_changed(&self.shared);
                for object in old {
                    unsafe { DeleteObject(object) };
                }
                let rect = unsafe { &*(lparam as *const RECT) };
                unsafe { SetWindowPos(self.shared.main, 0, rect.left, rect.top, rect.right - rect.left, rect.bottom - rect.top, SWP_NOZORDER | SWP_NOACTIVATE) };
            }
            WM_TIMER if wparam == POLL_TIMER => self.poll(),
            WM_TIMER if wparam == PLAY_TIMER => self.editor.tick(&mut self.shared),
            WM_WAKE => self.poll(),
            WM_CLOSE => self.close(),
            WM_COMMAND => {
                let (id, code) = (win::loword(wparam) as u16, win::hiword(wparam));
                if id == ID_SETTINGS && code == BN_CLICKED {
                    self.open_settings();
                } else if let Some(action) = self.songs.command(id, code, &mut self.shared) {
                    match action {
                        crate::songs::Action::TimeLyrics(sid) => self.open_in_editor(&sid),
                    }
                } else {
                    self.editor.command(id, code, &mut self.shared);
                }
                self.after();
            }
            WM_NOTIFY => {
                let header = unsafe { &*(lparam as *const NMHDR) };
                if header.hwndFrom == self.tabs && header.code == TCN_SELCHANGE {
                    let tab = if win::current_tab(self.tabs) == 0 { Tab::Songs } else { Tab::Editor };
                    self.show_tab(tab);
                } else {
                    self.songs.notify(header, &mut self.shared);
                    self.after();
                }
            }
            WM_CTLCOLORSTATIC if lparam as HWND == self.shared.log => unsafe {
                // The log is a read-only text box: keep it white like an ordinary one.
                SetBkColor(wparam as HDC, win::rgb(255, 255, 255));
                return Some(GetStockObject(0));
            },
            WM_CTLCOLORSTATIC => return Some(self.editor.static_colour(wparam as HDC, lparam as HWND)),
            _ => return None,
        }
        Some(0)
    }

    fn pane_message(&mut self, hwnd: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> Option<LRESULT> {
        let handled = self.editor.pane_message(hwnd, message, wparam, lparam, &mut self.shared);
        let handled = match (handled, self.settings.as_mut()) {
            (None, Some(settings)) => settings.pane_message(hwnd, message, wparam, lparam, &self.shared),
            (handled, _) => handled,
        };
        self.after();
        handled
    }

    fn dialog_message(&mut self, hwnd: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> Option<LRESULT> {
        let handled = if self.songs.dialog() == Some(hwnd) {
            self.songs.dialog_message(message, wparam, lparam, &mut self.shared)
        } else if self.editor.dialog() == Some(hwnd) {
            self.editor.dialog_message(message, wparam, lparam, &mut self.shared)
        } else if let Some(settings) = self.settings.as_mut().filter(|settings| settings.hwnd() == hwnd) {
            let (handled, open) = settings.message(message, wparam, lparam, &mut self.shared);
            if !open {
                self.settings = None;
            }
            handled
        } else {
            None
        };
        self.after();
        handled
    }
}

unsafe extern "system" fn main_proc(hwnd: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if message == WM_DESTROY && hwnd == MAIN.load(Ordering::SeqCst) {
        PostQuitMessage(0);
        return 0;
    }
    if message == WM_CLOSE && APP.with(|cell| cell.try_borrow_mut().is_err()) {
        // A message box or file dialog is open; quitting waits for it.
        return 0;
    }
    match with_app(|app| app.main_message(message, wparam, lparam)).flatten() {
        Some(result) => result,
        None => DefWindowProcW(hwnd, message, wparam, lparam),
    }
}

unsafe extern "system" fn pane_proc(hwnd: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match with_app(|app| app.pane_message(hwnd, message, wparam, lparam)).flatten() {
        Some(result) => result,
        None => DefWindowProcW(hwnd, message, wparam, lparam),
    }
}

unsafe extern "system" fn dialog_proc(hwnd: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match with_app(|app| app.dialog_message(hwnd, message, wparam, lparam)).flatten() {
        Some(result) => result,
        // Closing a dialog is decided by its owner; never let Windows destroy it behind the app's back.
        None if message == WM_CLOSE => 0,
        None => DefWindowProcW(hwnd, message, wparam, lparam),
    }
}

/// Create the window and run until it closes.
pub fn run(options: Options) -> i32 {
    win::init_controls();
    let face = unsafe { GetSysColorBrush(COLOR_BTNFACE) };
    win::register_class(MAIN_CLASS, Some(main_proc), face);
    win::register_class(PANE_CLASS, Some(pane_proc), 0);
    win::register_class(DIALOG_CLASS, Some(dialog_proc), face);
    let app = App::create(options);
    let main = app.shared.main;
    APP.with(|cell| *cell.borrow_mut() = Some(app));
    unsafe {
        ShowWindow(main, SW_SHOWNORMAL);
    }
    let mut msg = MSG::default();
    loop {
        let got = unsafe { GetMessageW(&mut msg, 0, 0, 0) };
        if got <= 0 {
            break;
        }
        if msg.message == WM_KEYDOWN || msg.message == WM_KEYUP {
            if with_app(|app| app.editor_key(&msg)) == Some(true) {
                continue;
            }
        }
        if let Some(dialog) = with_app(|app| app.active_dialog()).flatten() {
            if unsafe { IsDialogMessageW(dialog, &mut msg) } != 0 {
                continue;
            }
        }
        unsafe {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
    APP.with(|cell| cell.borrow_mut().take());
    msg.wParam as i32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn song_state_reads_the_folders() {
        let set = |names: &[&str]| names.iter().map(|name| name.to_string()).collect::<HashSet<String>>();
        let suffixes = vec![".mp4".to_string(), ".flac".to_string()];
        assert_eq!(song_state("a", &set(&["a.mp4"]), &set(&[]), &set(&[]), &suffixes), "video ready");
        assert_eq!(song_state("a", &set(&[]), &set(&["a.auto.ass"]), &set(&[]), &suffixes), "subtitles ready");
        assert_eq!(song_state("a", &set(&[]), &set(&[]), &set(&["a.MP4"]), &suffixes), "downloaded");
        // a.part.mp4 is an unfinished download; ab.mp4 is another song's.
        assert_eq!(song_state("a", &set(&[]), &set(&[]), &set(&["a.part.mp4", "ab.mp4"]), &suffixes), "waiting");
    }

    #[test]
    fn options_read_the_command_line() {
        let options = Options::from_args(["--open", "song", "--settings"].iter().map(|arg| arg.to_string()));
        assert_eq!(options.open.as_deref(), Some("song"));
        assert!(options.settings);
    }

    #[test]
    fn songs_are_written_in_column_order() {
        let song: Song = [("title", "T"), ("id", "x"), ("extra", "e")].iter().map(|(key, value)| (key.to_string(), value.to_string())).collect();
        let value = song_json(&["id".to_string(), "title".to_string(), "url".to_string()], &song);
        assert_eq!(value.dump(), r#"{"id":"x","title":"T","url":"","extra":"e"}"#);
    }
}
