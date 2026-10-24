//! Settings window: polykara.toml as a form, with the raw file one tab away.
//!
//! The engine says which settings exist and how each is edited (a tick box, a
//! number, a list of choices, a font, an ASS colour, a file), with the comment
//! above it in the file as its help. Saving writes only the values that
//! changed, in place, so the file keeps its comments and layout.

use std::path::Path;

use polykara::json::Value;

use crate::app::{Shared, Want, DIALOG_CLASS, IDCANCEL, IDOK, PANE_CLASS};
use crate::win::{self, ffi::*};

const ID_TABS: u16 = 50;
const ID_FIELD: u16 = 2000;
const ID_BUTTON: u16 = 6000;

#[derive(Clone, PartialEq)]
enum Shown {
    Tick(bool),
    Text(String),
}

struct Field {
    section: String,
    key: String,
    kind: String,
    original: Shown,
    control: HWND,
    /// Where the colour swatch is drawn in the page, before scrolling.
    swatch: Option<RECT>,
}

/// A placed child of a page, before scrolling.
struct Placed {
    hwnd: HWND,
    x: i32,
    y: i32,
    width: i32,
    height: i32,
}

struct Page {
    pane: HWND,
    children: Vec<Placed>,
    height: i32,
    scroll: i32,
}

/// An ASS colour, &HAABBGGRR, as (alpha, COLORREF). COLORREF is 0x00BBGGRR, so the order already matches.
pub fn ass_colour(text: &str) -> Option<(u32, COLORREF)> {
    let digits = text.trim().strip_prefix("&H").or_else(|| text.trim().strip_prefix("&h"))?;
    if digits.len() != 8 {
        return None;
    }
    let value = u32::from_str_radix(digits, 16).ok()?;
    Some((value >> 24, value & 0x00FF_FFFF))
}

pub fn ass_text(alpha: u32, colour: COLORREF) -> String {
    format!("&H{:02X}{:06X}", alpha & 0xFF, colour & 0x00FF_FFFF)
}

/// A path written relative to the PolyKara folder when it is inside it, with / between folders.
pub fn relative_to(home: &Path, path: &Path) -> String {
    let shown = path.strip_prefix(home).unwrap_or(path);
    shown.to_string_lossy().replace('\\', "/")
}

pub struct SettingsWindow {
    hwnd: HWND,
    tabs: HWND,
    advanced: HWND,
    hint: HWND,
    save: HWND,
    close: HWND,
    tooltip: HWND,
    names: Vec<String>,
    pages: Vec<Page>,
    fields: Vec<Field>,
    fonts: Vec<String>,
    /// The file text the form shows.
    text: String,
    current: usize,
    dpi: u32,
    font: HFONT,
    bold: HFONT,
}

impl SettingsWindow {
    pub fn open(shared: &mut Shared) -> SettingsWindow {
        let px = |value: i32| shared.px(value);
        let title = format!("Settings — {}", shared.hello.as_ref().map(|hello| hello.config.display().to_string()).unwrap_or_default());
        let hwnd = win::top_window(DIALOG_CLASS, &title, WS_OVERLAPPEDWINDOW | WS_CLIPCHILDREN, shared.main, px(940), px(720));
        let tabs = win::child("SysTabControl32", "", WS_CLIPSIBLINGS | WS_TABSTOP, hwnd, ID_TABS);
        let advanced = win::multiline(hwnd, 0, "", false);
        let hint = win::label(hwnd, "Point at a setting to see what it does. Changes apply to the next run.");
        let save = win::child("BUTTON", "Save", WS_TABSTOP | BS_PUSHBUTTON, hwnd, IDOK);
        let close = win::button(hwnd, IDCANCEL, "Close");
        for control in [tabs, hint, save, close] {
            win::set_font(control, shared.font);
        }
        win::set_font(advanced, shared.mono);
        let tooltip = win::tooltip(hwnd);
        shared.engine.ask("settings_form", Value::object::<&str>(vec![]), Want::SettingsForm);
        shared.engine.ask("fonts", Value::object::<&str>(vec![]), Want::Fonts);
        let window = SettingsWindow { hwnd, tabs, advanced, hint, save, close, tooltip, names: Vec::new(), pages: Vec::new(), fields: Vec::new(), fonts: Vec::new(), text: String::new(), current: 0, dpi: shared.dpi, font: shared.font, bold: shared.bold };
        window.layout();
        win::enable(shared.main, false);
        win::show(hwnd, true);
        window
    }

    pub fn hwnd(&self) -> HWND {
        self.hwnd
    }

    fn px(&self, value: i32) -> i32 {
        value * self.dpi as i32 / 96
    }

    fn page_area(&self) -> RECT {
        let mut rect = win::tab_area(self.tabs);
        let mut corner = RECT::default();
        unsafe { GetWindowRect(self.tabs, &mut corner) };
        let mut origin = POINT { x: corner.left, y: corner.top };
        unsafe { ScreenToClient(self.hwnd, &mut origin) };
        rect.left += origin.x;
        rect.right += origin.x;
        rect.top += origin.y;
        rect.bottom += origin.y;
        rect
    }

    fn layout(&self) {
        let (width, height) = win::client_size(self.hwnd);
        win::place(self.tabs, self.px(8), self.px(8), width - self.px(16), height - self.px(100));
        let area = self.page_area();
        for page in &self.pages {
            win::place(page.pane, area.left, area.top, area.right - area.left, area.bottom - area.top);
        }
        win::place(self.advanced, area.left, area.top, area.right - area.left, area.bottom - area.top);
        win::place(self.hint, self.px(12), height - self.px(88), width - self.px(24), self.px(40));
        win::place(self.save, width - self.px(212), height - self.px(42), self.px(96), self.px(30));
        win::place(self.close, width - self.px(108), height - self.px(42), self.px(96), self.px(30));
        for index in 0..self.pages.len() {
            self.scroll_page(index, 0);
        }
    }

    fn show_current(&self) {
        let advanced = self.current >= self.pages.len();
        for (index, page) in self.pages.iter().enumerate() {
            win::show(page.pane, index == self.current);
        }
        win::show(self.advanced, advanced);
    }

    /// Build the form from the engine's description of it.
    fn build(&mut self, value: &Value, shared: &Shared) {
        for page in self.pages.drain(..) {
            unsafe { DestroyWindow(page.pane) };
        }
        self.fields.clear();
        self.names.clear();
        self.text = value.get("text").str().to_string();
        win::set_text(self.advanced, &self.text);
        let tabs = value.get("tabs").items();
        unsafe { SendMessageW(self.tabs, 0x1309, 0, 0) }; // TCM_DELETEALLITEMS
        let dpi = self.dpi as i32;
        let px = move |value: i32| value * dpi / 96;
        for (number, tab) in tabs.iter().enumerate() {
            let name = tab.get("name").str().to_string();
            win::add_tab(self.tabs, number, &name);
            self.names.push(name);
            let pane = win::create(WS_EX_CONTROLPARENT, PANE_CLASS, "", WS_CHILD | WS_VSCROLL | WS_CLIPCHILDREN, self.hwnd, 0);
            let mut children = Vec::new();
            let mut y = px(10);
            for section in tab.get("sections").items() {
                let id = section.get("id").str();
                let header = win::label(pane, &format!("{}   [{}]", section.get("title").str(), id));
                win::set_font(header, self.bold);
                children.push(Placed { hwnd: header, x: px(10), y, width: px(700), height: px(24) });
                y += px(30);
                for item in section.get("fields").items() {
                    let index = self.fields.len();
                    let kind = item.get("kind").str().to_string();
                    let help = item.get("help").str().to_string();
                    let help = if help.is_empty() { format!("[{id}] {}", item.get("key").str()) } else { format!("[{id}] {}: {help}", item.get("key").str()) };
                    let label = win::child("STATIC", item.get("label").str(), SS_LEFT | SS_NOPREFIX | SS_CENTERIMAGE | SS_ENDELLIPSIS | 0x0100, pane, 0);
                    win::set_font(label, self.font);
                    children.push(Placed { hwnd: label, x: px(26), y, width: px(290), height: px(26) });
                    let original = match item.get("value") {
                        Value::Bool(value) => Shown::Tick(*value),
                        other => Shown::Text(other.str().to_string()),
                    };
                    let text = match &original {
                        Shown::Text(text) => text.clone(),
                        Shown::Tick(_) => String::new(),
                    };
                    let field_id = ID_FIELD + index as u16;
                    let (control, width, height) = match kind.as_str() {
                        "bool" => {
                            let check = win::checkbox(pane, field_id, "");
                            win::set_checked(check, original == Shown::Tick(true));
                            (check, px(30), px(26))
                        }
                        "choice" | "font" => {
                            let choices = if kind == "font" { self.fonts.clone() } else { item.get("choices").strings() };
                            let combo = win::combo(pane, field_id, &choices.iter().map(String::as_str).collect::<Vec<_>>(), true);
                            win::set_text(combo, &text);
                            (combo, if kind == "font" { px(280) } else { px(220) }, px(320))
                        }
                        "number" => (win::edit(pane, field_id, &text, 0), px(110), px(26)),
                        "list" => (win::edit(pane, field_id, &text, 0), px(540), px(26)),
                        "colour" => {
                            let choices = item.get("choices").strings();
                            let control = if choices.is_empty() {
                                win::edit(pane, field_id, &text, 0)
                            } else {
                                let combo = win::combo(pane, field_id, &choices.iter().map(String::as_str).collect::<Vec<_>>(), true);
                                win::set_text(combo, &text);
                                combo
                            };
                            (control, px(150), if choices.is_empty() { px(26) } else { px(200) })
                        }
                        "file" => (win::edit(pane, field_id, &text, 0), px(380), px(26)),
                        _ => (win::edit(pane, field_id, &text, 0), if item.get("wide").as_bool() == Some(true) { px(540) } else { px(280) }, px(26)),
                    };
                    win::set_font(control, self.font);
                    let left = px(330) + if kind == "colour" { px(44) } else { 0 };
                    children.push(Placed { hwnd: control, x: left, y, width, height });
                    let mut swatch = None;
                    if kind == "colour" || kind == "file" {
                        let button = win::button(pane, ID_BUTTON + index as u16, if kind == "colour" { "Choose…" } else { "Browse…" });
                        win::set_font(button, self.font);
                        children.push(Placed { hwnd: button, x: left + width + px(6), y, width: px(90), height: px(26) });
                    }
                    if kind == "colour" {
                        swatch = Some(RECT { left: px(330), top: y + px(2), right: px(330) + px(36), bottom: y + px(24) });
                    }
                    win::add_tip(self.tooltip, pane, control, &help);
                    win::add_tip(self.tooltip, pane, label, &help);
                    self.fields.push(Field { section: id.to_string(), key: item.get("key").str().to_string(), kind, original, control, swatch });
                    y += px(32);
                }
                y += px(12);
            }
            self.pages.push(Page { pane, children, height: y, scroll: 0 });
        }
        win::add_tab(self.tabs, tabs.len(), "Advanced (file)");
        let error = value.get("error").str();
        if !error.is_empty() {
            win::warn(self.hwnd, error);
        }
        self.current = if tabs.is_empty() { 0 } else { self.current.min(tabs.len()) };
        win::set_tab(self.tabs, self.current);
        self.layout();
        self.show_current();
        let _ = shared;
    }

    /// Move a page's children for its scroll position (plus `by`).
    fn scroll_page(&self, index: usize, by: i32) {
        let page = &self.pages[index];
        let (_, height) = win::client_size(page.pane);
        let scroll = win::set_scroll(page.pane, SB_VERT, page.height, height, page.scroll + by);
        for child in &page.children {
            win::place(child.hwnd, child.x, child.y - scroll, child.width, child.height);
        }
        win::redraw(page.pane);
        // The position is kept by the caller through `set_page_scroll`.
        let _ = scroll;
    }

    fn set_page_scroll(&mut self, index: usize, position: i32) {
        let (_, height) = win::client_size(self.pages[index].pane);
        self.pages[index].scroll = position.min((self.pages[index].height - height).max(0)).max(0);
        self.scroll_page(index, 0);
    }

    fn value(&self, field: &Field) -> Shown {
        match field.original {
            Shown::Tick(_) => Shown::Tick(win::checked(field.control)),
            Shown::Text(_) => Shown::Text(win::text(field.control)),
        }
    }

    /// Every field that differs from the file, as [section, key, value].
    fn changes(&self) -> Value {
        let changed = self.fields.iter().filter_map(|field| {
            let now = self.value(field);
            if now == field.original {
                return None;
            }
            let raw = match now {
                Shown::Tick(value) => Value::from(value),
                Shown::Text(text) => Value::from(text),
            };
            Some(Value::Array(vec![Value::from(&field.section), Value::from(&field.key), raw]))
        });
        Value::Array(changed.collect())
    }

    fn on_advanced(&self) -> bool {
        self.current >= self.pages.len()
    }

    fn tab_changed(&mut self, shared: &mut Shared) {
        let now = win::current_tab(self.tabs);
        let was_advanced = self.on_advanced();
        self.current = now;
        if self.on_advanced() && !was_advanced {
            // The file, with what was changed in the form written in.
            shared.engine.ask("settings_apply", Value::object(vec![("text", Value::from(&self.text)), ("changes", self.changes())]), Want::SettingsText);
        } else if was_advanced && !self.on_advanced() {
            shared.engine.ask("settings_form", Value::object(vec![("text", Value::from(win::text(self.advanced)))]), Want::SettingsForm);
        }
        self.show_current();
    }

    pub fn answer(&mut self, want: Want, result: Result<Value, String>, shared: &mut Shared) {
        match (want, result) {
            (Want::SettingsForm, Ok(value)) => self.build(&value, shared),
            (Want::SettingsText, Ok(value)) => win::set_text(self.advanced, value.get("text").str()),
            (Want::SettingsSaved, Ok(value)) => {
                self.build(&value, shared);
                win::info(self.hwnd, "Settings saved. They apply to the next run.");
            }
            (Want::Fonts, Ok(value)) => {
                self.fonts = value.get("fonts").strings();
                let names: Vec<&str> = self.fonts.iter().map(String::as_str).collect();
                for field in self.fields.iter().filter(|field| field.kind == "font") {
                    let text = win::text(field.control);
                    win::set_items(field.control, &names);
                    win::set_text(field.control, &text);
                }
            }
            (Want::SettingsSaved, Err(error)) => win::warn(self.hwnd, &format!("The settings were not saved:\n{error}")),
            (Want::SettingsText, Err(error)) | (Want::SettingsForm, Err(error)) => {
                win::warn(self.hwnd, &error);
                // Stay on the file until it can be read.
                self.current = self.pages.len();
                win::set_tab(self.tabs, self.current);
                self.show_current();
            }
            _ => {}
        }
    }

    fn save(&mut self, shared: &mut Shared) {
        let (text, changes) = if self.on_advanced() { (win::text(self.advanced), Value::Array(Vec::new())) } else { (self.text.clone(), self.changes()) };
        shared.engine.ask("settings_save", Value::object(vec![("text", Value::from(text)), ("changes", changes)]), Want::SettingsSaved);
    }

    fn close_window(&self, shared: &Shared) {
        win::enable(shared.main, true);
        unsafe {
            DestroyWindow(self.hwnd);
            SetForegroundWindow(shared.main);
        }
    }

    /// Messages to the settings window; the bool is false once it closed.
    pub fn message(&mut self, message: u32, wparam: WPARAM, lparam: LPARAM, shared: &mut Shared) -> (Option<LRESULT>, bool) {
        match message {
            WM_SIZE => self.layout(),
            WM_CLOSE => {
                self.close_window(shared);
                return (Some(0), false);
            }
            WM_COMMAND => match win::loword(wparam) as u16 {
                // Enter and Escape (no button) neither save nor close: only the buttons do.
                IDOK | IDCANCEL if lparam == 0 => {}
                IDOK => self.save(shared),
                IDCANCEL => {
                    self.close_window(shared);
                    return (Some(0), false);
                }
                _ => return (None, true),
            },
            WM_NOTIFY => {
                let header = unsafe { &*(lparam as *const NMHDR) };
                if header.hwndFrom == self.tabs && header.code == TCN_SELCHANGE {
                    self.tab_changed(shared);
                } else {
                    return (None, true);
                }
            }
            _ => return (None, true),
        }
        (Some(0), true)
    }

    fn pick(&mut self, index: usize, shared: &Shared) {
        let field = match self.fields.get(index) {
            Some(field) => field,
            None => return,
        };
        if field.kind == "colour" {
            let current = win::text(field.control);
            let (alpha, start) = ass_colour(&current).unwrap_or((0, 0xFF_FFFF));
            if let Some(chosen) = win::choose_colour(self.hwnd, start) {
                win::set_text(field.control, &ass_text(alpha, chosen));
                for page in &self.pages {
                    win::redraw(page.pane);
                }
            }
        } else {
            let home = shared.home();
            let chosen = win::open_files(self.hwnd, "Choose an image", home.as_deref(), &[("Images", "*.png;*.jpg;*.jpeg;*.webp;*.bmp"), ("All files", "*.*")], false);
            if let Some(path) = chosen.into_iter().next() {
                let shown = match &home {
                    Some(home) => relative_to(home, &path),
                    None => path.to_string_lossy().into_owned(),
                };
                win::set_text(field.control, &shown);
            }
        }
    }

    /// Messages to the form pages (scrolling, painting swatches, buttons, help).
    pub fn pane_message(&mut self, hwnd: HWND, message: u32, wparam: WPARAM, _lparam: LPARAM, shared: &Shared) -> Option<LRESULT> {
        let index = self.pages.iter().position(|page| page.pane == hwnd)?;
        match message {
            WM_ERASEBKGND => return Some(1),
            WM_PAINT => {
                let scroll = self.pages[index].scroll;
                let swatches: Vec<(RECT, Option<COLORREF>)> = self
                    .fields
                    .iter()
                    .filter(|field| unsafe { GetParent(field.control) } == hwnd)
                    .filter_map(|field| field.swatch.map(|rect| (rect, ass_colour(&win::text(field.control)).map(|(_, colour)| colour))))
                    .collect();
                win::paint(hwnd, |canvas| {
                    canvas.fill(0, 0, canvas.width, canvas.height, unsafe { GetSysColor(COLOR_BTNFACE) });
                    for (rect, colour) in &swatches {
                        let (top, bottom) = (rect.top - scroll, rect.bottom - scroll);
                        match colour {
                            Some(colour) => canvas.rect(rect.left, top, rect.right, bottom, Some(*colour), win::rgb(0x40, 0x40, 0x40)),
                            None => canvas.text_in(rect.left, top, rect.right, bottom, "?", win::rgb(0x60, 0x60, 0x60), true),
                        }
                    }
                });
            }
            WM_SIZE => self.set_page_scroll(index, self.pages[index].scroll),
            WM_VSCROLL => {
                let position = win::scrolled(hwnd, SB_VERT, wparam, self.px(32));
                self.set_page_scroll(index, position);
            }
            WM_MOUSEWHEEL => {
                let delta = (win::hiword(wparam) as u16 as i16) as i32;
                let position = self.pages[index].scroll - delta * self.px(96) / 120;
                self.set_page_scroll(index, position);
            }
            WM_CTLCOLORSTATIC => unsafe {
                SetBkColor(wparam as HDC, GetSysColor(COLOR_BTNFACE));
                return Some(GetSysColorBrush(COLOR_BTNFACE));
            },
            WM_COMMAND => {
                let (id, code) = (win::loword(wparam) as u16, win::hiword(wparam));
                if (ID_BUTTON..ID_BUTTON + 4000).contains(&id) && code == BN_CLICKED {
                    self.pick((id - ID_BUTTON) as usize, shared);
                } else if (ID_FIELD..ID_FIELD + 4000).contains(&id) {
                    let field = &self.fields[(id - ID_FIELD) as usize];
                    if code == EN_CHANGE && field.kind == "colour" {
                        win::redraw(hwnd);
                    }
                    // Edit and combo box focus: show which setting it is.
                    if code == EN_SETFOCUS || code == 3 {
                        win::set_text(self.hint, &format!("[{}] {}", field.section, field.key));
                    }
                }
            }
            _ => return None,
        }
        Some(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ass_colours_read_and_write() {
        assert_eq!(ass_colour("&H00FF0000"), Some((0, 0x00FF_0000)));
        assert_eq!(ass_colour("&h80000000"), Some((0x80, 0)));
        assert_eq!(ass_colour("voice"), None);
        assert_eq!(ass_text(0x80, 0x0088_2D14), "&H80882D14");
    }

    #[test]
    fn paths_inside_the_folder_are_relative() {
        assert_eq!(relative_to(Path::new("/home/me/PolyKara"), Path::new("/home/me/PolyKara/logos/a.png")), "logos/a.png");
        assert_eq!(relative_to(Path::new("/home/me/PolyKara"), Path::new("/tmp/a.png")), "/tmp/a.png");
    }
}
