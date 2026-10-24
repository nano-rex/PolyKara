//! Small safe helpers over the Windows API: controls, dialogs, and double-buffered drawing.

pub mod ffi;

use std::path::{Path, PathBuf};
use std::ptr::{null, null_mut};

use ffi::*;

pub fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Text for a single-line control: Windows edit controls need \r\n line ends.
pub fn wide_crlf(text: &str) -> Vec<u16> {
    wide(&text.replace("\r\n", "\n").replace('\n', "\r\n"))
}

pub fn path_wide(path: &Path) -> Vec<u16> {
    wide(&path.to_string_lossy())
}

pub fn rgb(red: u8, green: u8, blue: u8) -> COLORREF {
    red as u32 | (green as u32) << 8 | (blue as u32) << 16
}

/// "#rrggbb" as a COLORREF.
pub fn hex_colour(text: &str) -> Option<COLORREF> {
    let digits = text.trim().strip_prefix('#')?;
    if digits.len() != 6 {
        return None;
    }
    let value = u32::from_str_radix(digits, 16).ok()?;
    Some(rgb((value >> 16) as u8, (value >> 8) as u8, value as u8))
}

pub fn instance() -> HINSTANCE {
    unsafe { GetModuleHandleW(null()) }
}

pub fn loword(value: usize) -> u32 {
    (value & 0xFFFF) as u32
}

pub fn hiword(value: usize) -> u32 {
    ((value >> 16) & 0xFFFF) as u32
}

/// The signed x and y packed into a mouse message's lparam.
pub fn mouse_point(lparam: LPARAM) -> (i32, i32) {
    ((lparam & 0xFFFF) as u16 as i16 as i32, ((lparam >> 16) & 0xFFFF) as u16 as i16 as i32)
}

pub fn register_class(name: &str, proc_: WNDPROC, background: HBRUSH) {
    let class_name = wide(name);
    let class = WNDCLASSEXW {
        cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
        style: CS_HREDRAW | CS_VREDRAW | CS_DBLCLKS,
        lpfnWndProc: proc_,
        cbClsExtra: 0,
        cbWndExtra: 0,
        hInstance: instance(),
        hIcon: 0,
        hCursor: unsafe { LoadCursorW(0, IDC_ARROW as *const u16) },
        hbrBackground: background,
        lpszMenuName: null(),
        lpszClassName: class_name.as_ptr(),
        hIconSm: 0,
    };
    unsafe { RegisterClassExW(&class) };
}

pub fn create(ex_style: u32, class: &str, text: &str, style: u32, parent: HWND, id: u16) -> HWND {
    let (class, text) = (wide(class), wide(text));
    unsafe { CreateWindowExW(ex_style, class.as_ptr(), text.as_ptr(), style, 0, 0, 10, 10, parent, id as HMENU, instance(), null()) }
}

pub fn top_window(class: &str, title: &str, style: u32, owner: HWND, width: i32, height: i32) -> HWND {
    let (class, title) = (wide(class), wide(title));
    unsafe { CreateWindowExW(WS_EX_CONTROLPARENT, class.as_ptr(), title.as_ptr(), style, CW_USEDEFAULT, CW_USEDEFAULT, width, height, owner, 0, instance(), null()) }
}

pub fn child(class: &str, text: &str, style: u32, parent: HWND, id: u16) -> HWND {
    create(0, class, text, WS_CHILD | WS_VISIBLE | style, parent, id)
}

pub fn button(parent: HWND, id: u16, text: &str) -> HWND {
    child("BUTTON", text, WS_TABSTOP | BS_PUSHBUTTON, parent, id)
}

pub fn checkbox(parent: HWND, id: u16, text: &str) -> HWND {
    child("BUTTON", text, WS_TABSTOP | BS_AUTOCHECKBOX, parent, id)
}

pub fn label(parent: HWND, text: &str) -> HWND {
    child("STATIC", text, SS_LEFT | SS_NOPREFIX | SS_ENDELLIPSIS | SS_CENTERIMAGE, parent, 0)
}

/// Text that wraps onto more lines when it does not fit.
pub fn note(parent: HWND, text: &str) -> HWND {
    child("STATIC", text, SS_LEFT | SS_NOPREFIX, parent, 0)
}

pub fn edit(parent: HWND, id: u16, text: &str, style: u32) -> HWND {
    let hwnd = create(WS_EX_CLIENTEDGE, "EDIT", "", WS_CHILD | WS_VISIBLE | WS_TABSTOP | ES_AUTOHSCROLL | style, parent, id);
    set_text(hwnd, text);
    hwnd
}

pub fn multiline(parent: HWND, id: u16, text: &str, read_only: bool) -> HWND {
    let style = ES_MULTILINE | ES_AUTOVSCROLL | ES_WANTRETURN | ES_NOHIDESEL | WS_VSCROLL | WS_HSCROLL | if read_only { ES_READONLY } else { 0 };
    let hwnd = create(WS_EX_CLIENTEDGE, "EDIT", "", WS_CHILD | WS_VISIBLE | WS_TABSTOP | style, parent, id);
    unsafe { SendMessageW(hwnd, EM_SETLIMITTEXT, 0x7FFF_FFFE, 0) };
    set_text(hwnd, text);
    hwnd
}

/// A drop-down list (editable when `typed` is true) with these items.
pub fn combo(parent: HWND, id: u16, items: &[&str], typed: bool) -> HWND {
    let style = WS_VSCROLL | CBS_AUTOHSCROLL | if typed { CBS_DROPDOWN } else { CBS_DROPDOWNLIST };
    let hwnd = child("COMBOBOX", "", WS_TABSTOP | style, parent, id);
    unsafe { SendMessageW(hwnd, CB_SETMINVISIBLE, 20, 0) };
    set_items(hwnd, items);
    hwnd
}

pub fn set_items(hwnd: HWND, items: &[&str]) {
    unsafe { SendMessageW(hwnd, CB_RESETCONTENT, 0, 0) };
    for item in items {
        let text = wide(item);
        unsafe { SendMessageW(hwnd, CB_ADDSTRING, 0, text.as_ptr() as LPARAM) };
    }
}

pub fn selection(hwnd: HWND) -> Option<usize> {
    let index = unsafe { SendMessageW(hwnd, CB_GETCURSEL, 0, 0) };
    if index < 0 { None } else { Some(index as usize) }
}

pub fn select(hwnd: HWND, index: Option<usize>) {
    unsafe { SendMessageW(hwnd, CB_SETCURSEL, index.map(|index| index as usize).unwrap_or(usize::MAX), 0) };
}

pub fn checked(hwnd: HWND) -> bool {
    unsafe { SendMessageW(hwnd, BM_GETCHECK, 0, 0) == 1 }
}

pub fn set_checked(hwnd: HWND, value: bool) {
    unsafe { SendMessageW(hwnd, BM_SETCHECK, value as usize, 0) };
}

pub fn set_text(hwnd: HWND, text: &str) {
    let text = wide_crlf(text);
    unsafe { SetWindowTextW(hwnd, text.as_ptr()) };
}

pub fn text(hwnd: HWND) -> String {
    let length = unsafe { GetWindowTextLengthW(hwnd) };
    let mut buffer = vec![0u16; length as usize + 1];
    let copied = unsafe { GetWindowTextW(hwnd, buffer.as_mut_ptr(), buffer.len() as i32) };
    String::from_utf16_lossy(&buffer[..copied.max(0) as usize]).replace("\r\n", "\n")
}

pub fn class_name(hwnd: HWND) -> String {
    let mut buffer = [0u16; 64];
    let copied = unsafe { GetClassNameW(hwnd, buffer.as_mut_ptr(), buffer.len() as i32) };
    String::from_utf16_lossy(&buffer[..copied.max(0) as usize])
}

/// Add text at the end of an edit control and scroll to it.
pub fn append(hwnd: HWND, text: &str) {
    let length = unsafe { GetWindowTextLengthW(hwnd) };
    // Keep the log from growing without end.
    if length > 600_000 {
        let kept = self::text(hwnd);
        let cut = kept.char_indices().nth(kept.chars().count() / 2).map(|(at, _)| at).unwrap_or(0);
        let tail = &kept[cut..];
        set_text(hwnd, tail.splitn(2, '\n').nth(1).unwrap_or(tail));
    }
    let end = unsafe { GetWindowTextLengthW(hwnd) } as usize;
    let text = wide_crlf(text);
    unsafe {
        SendMessageW(hwnd, EM_SETSEL, end, end as isize);
        SendMessageW(hwnd, EM_REPLACESEL, 0, text.as_ptr() as LPARAM);
        SendMessageW(hwnd, EM_SCROLLCARET, 0, 0);
    }
}

pub fn place(hwnd: HWND, x: i32, y: i32, width: i32, height: i32) {
    unsafe { MoveWindow(hwnd, x, y, width.max(0), height.max(0), 1) };
}

pub fn show(hwnd: HWND, visible: bool) {
    unsafe { ShowWindow(hwnd, if visible { SW_SHOW } else { SW_HIDE }) };
}

pub fn enable(hwnd: HWND, enabled: bool) {
    unsafe { EnableWindow(hwnd, enabled as BOOL) };
}

pub fn redraw(hwnd: HWND) {
    unsafe { InvalidateRect(hwnd, null(), 0) };
}

pub fn client_size(hwnd: HWND) -> (i32, i32) {
    let mut rect = RECT::default();
    unsafe { GetClientRect(hwnd, &mut rect) };
    (rect.right - rect.left, rect.bottom - rect.top)
}

pub fn dpi(hwnd: HWND) -> u32 {
    match unsafe { GetDpiForWindow(hwnd) } {
        0 => 96,
        value => value,
    }
}

pub fn set_font(hwnd: HWND, font: HFONT) {
    unsafe { SendMessageW(hwnd, WM_SETFONT, font as WPARAM, 1) };
}

/// A font `points` high at this DPI.
pub fn font(face: &str, points: f32, bold: bool, dpi: u32) -> HFONT {
    let face = wide(face);
    let height = -(points * dpi as f32 / 72.0).round() as i32;
    unsafe { CreateFontW(height, 0, 0, 0, if bold { FW_BOLD } else { FW_NORMAL }, 0, 0, 0, DEFAULT_CHARSET, 0, 0, CLEARTYPE_QUALITY, 0, face.as_ptr()) }
}

pub fn key_down(key: i32) -> bool {
    unsafe { GetKeyState(key) < 0 }
}

pub fn set_cursor(shape: usize) {
    unsafe { SetCursor(LoadCursorW(0, shape as *const u16)) };
}

fn message(owner: HWND, text: &str, kind: u32) -> i32 {
    let (text, caption) = (wide(text), wide("PolyKara"));
    unsafe { MessageBoxW(owner, text.as_ptr(), caption.as_ptr(), kind) }
}

pub fn info(owner: HWND, text: &str) {
    message(owner, text, MB_OK | MB_ICONINFORMATION);
}

pub fn warn(owner: HWND, text: &str) {
    message(owner, text, MB_OK | MB_ICONWARNING);
}

pub fn confirm(owner: HWND, text: &str) -> bool {
    message(owner, text, MB_YESNO | MB_ICONQUESTION) == IDYES
}

/// Yes, No, or None for Cancel.
pub fn confirm_or_cancel(owner: HWND, text: &str) -> Option<bool> {
    match message(owner, text, MB_YESNOCANCEL | MB_ICONQUESTION) {
        IDYES => Some(true),
        IDNO => Some(false),
        _ => None,
    }
}

/// Choose files to open. `filters` are (name, "*.mp4;*.mkv") pairs.
pub fn open_files(owner: HWND, title: &str, folder: Option<&Path>, filters: &[(&str, &str)], many: bool) -> Vec<PathBuf> {
    let mut filter: Vec<u16> = Vec::new();
    for (name, pattern) in filters {
        filter.extend(name.encode_utf16());
        filter.push(0);
        filter.extend(pattern.encode_utf16());
        filter.push(0);
    }
    filter.push(0);
    let mut buffer = vec![0u16; 65536];
    let title = wide(title);
    let folder = folder.map(path_wide);
    let mut dialog = OPENFILENAMEW {
        lStructSize: std::mem::size_of::<OPENFILENAMEW>() as u32,
        hwndOwner: owner,
        hInstance: 0,
        lpstrFilter: filter.as_ptr(),
        lpstrCustomFilter: null_mut(),
        nMaxCustFilter: 0,
        nFilterIndex: 1,
        lpstrFile: buffer.as_mut_ptr(),
        nMaxFile: buffer.len() as u32,
        lpstrFileTitle: null_mut(),
        nMaxFileTitle: 0,
        lpstrInitialDir: folder.as_ref().map(|folder| folder.as_ptr()).unwrap_or(null()),
        lpstrTitle: title.as_ptr(),
        Flags: OFN_EXPLORER | OFN_FILEMUSTEXIST | OFN_PATHMUSTEXIST | OFN_HIDEREADONLY | OFN_NOCHANGEDIR | if many { OFN_ALLOWMULTISELECT } else { 0 },
        nFileOffset: 0,
        nFileExtension: 0,
        lpstrDefExt: null(),
        lCustData: 0,
        lpfnHook: null(),
        lpTemplateName: null(),
        pvReserved: null_mut(),
        dwReserved: 0,
        FlagsEx: 0,
    };
    if unsafe { GetOpenFileNameW(&mut dialog) } == 0 {
        return Vec::new();
    }
    // One file: its full path. Several: the folder, then each name, each ending in 0.
    let parts: Vec<String> = buffer.split(|&unit| unit == 0).take_while(|part| !part.is_empty()).map(String::from_utf16_lossy).collect();
    match parts.len() {
        0 => Vec::new(),
        1 => vec![PathBuf::from(&parts[0])],
        _ => parts[1..].iter().map(|name| Path::new(&parts[0]).join(name)).collect(),
    }
}

/// Pick a colour, starting from `start`.
pub fn choose_colour(owner: HWND, start: COLORREF) -> Option<COLORREF> {
    let mut custom = [0xFFFFFFu32; 16];
    let mut dialog = CHOOSECOLORW {
        lStructSize: std::mem::size_of::<CHOOSECOLORW>() as u32,
        hwndOwner: owner,
        hInstance: 0,
        rgbResult: start,
        lpCustColors: custom.as_mut_ptr(),
        Flags: CC_RGBINIT | CC_FULLOPEN,
        lCustData: 0,
        lpfnHook: null(),
        lpTemplateName: null(),
    };
    if unsafe { ChooseColorW(&mut dialog) } == 0 { None } else { Some(dialog.rgbResult) }
}

/// Open a file or folder with the program Windows uses for it.
pub fn open_path(path: &Path) {
    let (operation, file) = (wide("open"), path_wide(path));
    unsafe { ShellExecuteW(0, operation.as_ptr(), file.as_ptr(), null(), null(), SW_SHOWNORMAL) };
}

/// Drawing into an off-screen bitmap that is copied to the window in one go, so nothing flickers.
pub struct Canvas {
    target: HDC,
    pub dc: HDC,
    bitmap: HBITMAP,
    old: HGDIOBJ,
    pub width: i32,
    pub height: i32,
}

impl Canvas {
    pub fn new(target: HDC, width: i32, height: i32) -> Canvas {
        let (width, height) = (width.max(1), height.max(1));
        unsafe {
            let dc = CreateCompatibleDC(target);
            let bitmap = CreateCompatibleBitmap(target, width, height);
            let old = SelectObject(dc, bitmap);
            SetBkMode(dc, TRANSPARENT);
            Canvas { target, dc, bitmap, old, width, height }
        }
    }

    pub fn fill(&self, left: i32, top: i32, right: i32, bottom: i32, colour: COLORREF) {
        let rect = RECT { left, top, right, bottom };
        unsafe {
            SetDCBrushColor(self.dc, colour);
            FillRect(self.dc, &rect, GetStockObject(DC_BRUSH));
        }
    }

    /// A rectangle with an outline (and a fill unless `fill` is None).
    pub fn rect(&self, left: i32, top: i32, right: i32, bottom: i32, fill: Option<COLORREF>, outline: COLORREF) {
        unsafe {
            let old_brush = match fill {
                Some(colour) => {
                    SetDCBrushColor(self.dc, colour);
                    SelectObject(self.dc, GetStockObject(DC_BRUSH))
                }
                None => SelectObject(self.dc, GetStockObject(NULL_BRUSH)),
            };
            SetDCPenColor(self.dc, outline);
            let old_pen = SelectObject(self.dc, GetStockObject(DC_PEN));
            Rectangle(self.dc, left, top, right.max(left + 1), bottom);
            SelectObject(self.dc, old_brush);
            SelectObject(self.dc, old_pen);
        }
    }

    pub fn dotted_rect(&self, left: i32, top: i32, right: i32, bottom: i32, outline: COLORREF) {
        unsafe {
            let pen = CreatePen(PS_DOT, 1, outline);
            let old_pen = SelectObject(self.dc, pen);
            let old_brush = SelectObject(self.dc, GetStockObject(NULL_BRUSH));
            Rectangle(self.dc, left, top, right.max(left + 1), bottom);
            SelectObject(self.dc, old_brush);
            SelectObject(self.dc, old_pen);
            DeleteObject(pen);
        }
    }

    pub fn line(&self, x1: i32, y1: i32, x2: i32, y2: i32, colour: COLORREF) {
        unsafe {
            SetDCPenColor(self.dc, colour);
            let old = SelectObject(self.dc, GetStockObject(DC_PEN));
            MoveToEx(self.dc, x1, y1, null_mut());
            LineTo(self.dc, x2, y2);
            SelectObject(self.dc, old);
        }
    }

    pub fn use_font(&self, font: HFONT) {
        unsafe { SelectObject(self.dc, font) };
    }

    pub fn text(&self, x: i32, y: i32, text: &str, colour: COLORREF) {
        let units: Vec<u16> = text.encode_utf16().collect();
        unsafe {
            SetTextColor(self.dc, colour);
            TextOutW(self.dc, x, y, units.as_ptr(), units.len() as i32);
        }
    }

    /// Text inside a box, one line, cut off with … when it does not fit.
    pub fn text_in(&self, left: i32, top: i32, right: i32, bottom: i32, text: &str, colour: COLORREF, centred: bool) {
        let mut units: Vec<u16> = text.encode_utf16().collect();
        let mut rect = RECT { left, top, right, bottom };
        units.push(0);
        unsafe {
            SetTextColor(self.dc, colour);
            DrawTextW(self.dc, units.as_ptr(), -1, &mut rect, DT_SINGLELINE | DT_VCENTER | DT_NOPREFIX | DT_END_ELLIPSIS | if centred { DT_CENTER } else { 0 });
        }
    }

    pub fn measure(&self, text: &str) -> (i32, i32) {
        let units: Vec<u16> = text.encode_utf16().collect();
        let mut size = SIZE::default();
        unsafe { GetTextExtentPoint32W(self.dc, units.as_ptr(), units.len() as i32, &mut size) };
        (size.cx, size.cy)
    }

    pub fn clip(&self, left: i32, top: i32, right: i32, bottom: i32) -> i32 {
        unsafe {
            let saved = SaveDC(self.dc);
            IntersectClipRect(self.dc, left, top, right, bottom);
            saved
        }
    }

    pub fn unclip(&self, saved: i32) {
        unsafe { RestoreDC(self.dc, saved) };
    }
}

impl Drop for Canvas {
    fn drop(&mut self) {
        unsafe {
            BitBlt(self.target, 0, 0, self.width, self.height, self.dc, 0, 0, SRCCOPY);
            SelectObject(self.dc, self.old);
            DeleteObject(self.bitmap);
            DeleteDC(self.dc);
        }
    }
}

/// Paint a window: `draw` gets a canvas the size of the window.
pub fn paint(hwnd: HWND, draw: impl FnOnce(&Canvas)) {
    let mut paint = PAINTSTRUCT { hdc: 0, fErase: 0, rcPaint: RECT::default(), fRestore: 0, fIncUpdate: 0, rgbReserved: [0; 32] };
    let hdc = unsafe { BeginPaint(hwnd, &mut paint) };
    let (width, height) = client_size(hwnd);
    {
        let canvas = Canvas::new(hdc, width, height);
        draw(&canvas);
    }
    unsafe { EndPaint(hwnd, &paint) };
}

/// Measure text in a font without painting (for layout outside WM_PAINT).
pub fn measure_with(hwnd: HWND, font: HFONT, text: &str) -> (i32, i32) {
    unsafe {
        let hdc = GetDC(hwnd);
        let old = SelectObject(hdc, font);
        let units: Vec<u16> = text.encode_utf16().collect();
        let mut size = SIZE::default();
        GetTextExtentPoint32W(hdc, units.as_ptr(), units.len() as i32, &mut size);
        SelectObject(hdc, old);
        ReleaseDC(hwnd, hdc);
        (size.cx, size.cy)
    }
}

pub fn scroll_info(hwnd: HWND, bar: i32) -> SCROLLINFO {
    let mut info = SCROLLINFO { cbSize: std::mem::size_of::<SCROLLINFO>() as u32, fMask: SIF_ALL, nMin: 0, nMax: 0, nPage: 0, nPos: 0, nTrackPos: 0 };
    unsafe { GetScrollInfo(hwnd, bar, &mut info) };
    info
}

/// Set a scroll bar's range (0..total), visible part and position; returns the position used.
pub fn set_scroll(hwnd: HWND, bar: i32, total: i32, page: i32, position: i32) -> i32 {
    let position = position.min((total - page).max(0)).max(0);
    let info = SCROLLINFO { cbSize: std::mem::size_of::<SCROLLINFO>() as u32, fMask: SIF_RANGE | SIF_PAGE | SIF_POS, nMin: 0, nMax: total.max(1) - 1, nPage: page.max(0) as u32, nPos: position, nTrackPos: 0 };
    unsafe { SetScrollInfo(hwnd, bar, &info, 1) };
    position
}

/// Where a scroll bar message moves the position to.
pub fn scrolled(hwnd: HWND, bar: i32, wparam: WPARAM, line: i32) -> i32 {
    let info = scroll_info(hwnd, bar);
    let page = info.nPage as i32;
    match loword(wparam) {
        SB_LINEUP => info.nPos - line,
        SB_LINEDOWN => info.nPos + line,
        SB_PAGEUP => info.nPos - page,
        SB_PAGEDOWN => info.nPos + page,
        SB_THUMBTRACK => info.nTrackPos,
        SB_TOP => 0,
        SB_BOTTOM => info.nMax,
        _ => info.nPos,
    }
}

pub fn tooltip(owner: HWND) -> HWND {
    let hwnd = create(WS_EX_TOPMOST, "tooltips_class32", "", WS_POPUP | TTS_ALWAYSTIP | TTS_NOPREFIX, owner, 0);
    unsafe { SendMessageW(hwnd, TTM_SETMAXTIPWIDTH, 0, 480) };
    hwnd
}

pub fn add_tip(tooltip: HWND, owner: HWND, control: HWND, text: &str) {
    if text.is_empty() {
        return;
    }
    let mut text = wide(text);
    let info = TTTOOLINFOW {
        cbSize: std::mem::size_of::<TTTOOLINFOW>() as u32,
        uFlags: TTF_IDISHWND | TTF_SUBCLASS,
        hwnd: owner,
        uId: control as usize,
        rect: RECT::default(),
        hinst: 0,
        lpszText: text.as_mut_ptr(),
        lParam: 0,
        lpReserved: null_mut(),
    };
    unsafe { SendMessageW(tooltip, TTM_ADDTOOLW, 0, &info as *const TTTOOLINFOW as LPARAM) };
}

pub fn init_controls() {
    let init = INITCOMMONCONTROLSEX { dwSize: std::mem::size_of::<INITCOMMONCONTROLSEX>() as u32, dwICC: ICC_LISTVIEW_CLASSES | ICC_TAB_CLASSES | ICC_STANDARD_CLASSES };
    unsafe {
        InitCommonControlsEx(&init);
        SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    }
}

/// Tab control helpers.
pub fn add_tab(hwnd: HWND, index: usize, text: &str) {
    let mut text = wide(text);
    let item = TCITEMW { mask: TCIF_TEXT, dwState: 0, dwStateMask: 0, pszText: text.as_mut_ptr(), cchTextMax: 0, iImage: 0, lParam: 0 };
    unsafe { SendMessageW(hwnd, TCM_INSERTITEMW, index, &item as *const TCITEMW as LPARAM) };
}

pub fn current_tab(hwnd: HWND) -> usize {
    unsafe { SendMessageW(hwnd, TCM_GETCURSEL, 0, 0).max(0) as usize }
}

pub fn set_tab(hwnd: HWND, index: usize) {
    unsafe { SendMessageW(hwnd, TCM_SETCURSEL, index, 0) };
}

/// The area inside a tab control below its tabs, in the tab control's own coordinates.
pub fn tab_area(hwnd: HWND) -> RECT {
    let (width, height) = client_size(hwnd);
    let mut rect = RECT { left: 0, top: 0, right: width, bottom: height };
    unsafe { SendMessageW(hwnd, TCM_ADJUSTRECT, 0, &mut rect as *mut RECT as LPARAM) };
    rect
}

/// List view (report style) helpers.
pub fn list_columns(hwnd: HWND, columns: &[(&str, i32)]) {
    unsafe { SendMessageW(hwnd, LVM_SETEXTENDEDLISTVIEWSTYLE, LVS_EX_FULLROWSELECT | LVS_EX_DOUBLEBUFFER, (LVS_EX_FULLROWSELECT | LVS_EX_DOUBLEBUFFER) as isize) };
    for (index, (name, width)) in columns.iter().enumerate() {
        let mut text = wide(name);
        let column = LVCOLUMNW { mask: LVCF_TEXT | LVCF_WIDTH, fmt: 0, cx: *width, pszText: text.as_mut_ptr(), cchTextMax: 0, iSubItem: index as i32, iImage: 0, iOrder: 0, cxMin: 0, cxDefault: 0, cxIdeal: 0 };
        unsafe { SendMessageW(hwnd, LVM_INSERTCOLUMNW, index, &column as *const LVCOLUMNW as LPARAM) };
    }
}

pub fn list_width(hwnd: HWND, column: usize, width: i32) {
    unsafe { SendMessageW(hwnd, LVM_SETCOLUMNWIDTH, column, width as isize) };
}

fn list_item(row: usize, column: usize, text: &mut Vec<u16>) -> LVITEMW {
    LVITEMW { mask: LVIF_TEXT, iItem: row as i32, iSubItem: column as i32, state: 0, stateMask: 0, pszText: text.as_mut_ptr(), cchTextMax: 0, iImage: 0, lParam: 0, iIndent: 0, iGroupId: 0, cColumns: 0, puColumns: null_mut(), piColFmt: null_mut(), iGroup: 0 }
}

pub fn list_fill(hwnd: HWND, rows: &[Vec<String>]) {
    unsafe {
        SendMessageW(hwnd, WM_SETREDRAW, 0, 0);
        SendMessageW(hwnd, LVM_DELETEALLITEMS, 0, 0);
    }
    for (row, cells) in rows.iter().enumerate() {
        for (column, cell) in cells.iter().enumerate() {
            let mut text = wide(cell);
            let item = list_item(row, column, &mut text);
            let message = if column == 0 { LVM_INSERTITEMW } else { LVM_SETITEMTEXTW };
            unsafe { SendMessageW(hwnd, message, row, &item as *const LVITEMW as LPARAM) };
        }
    }
    unsafe {
        SendMessageW(hwnd, WM_SETREDRAW, 1, 0);
        InvalidateRect(hwnd, null(), 1);
    }
}

pub fn list_set(hwnd: HWND, row: usize, column: usize, text: &str) {
    let mut text = wide(text);
    let item = list_item(row, column, &mut text);
    unsafe { SendMessageW(hwnd, LVM_SETITEMTEXTW, row, &item as *const LVITEMW as LPARAM) };
}

pub fn list_selected(hwnd: HWND) -> Vec<usize> {
    let mut found = Vec::new();
    let mut at: isize = -1;
    loop {
        at = unsafe { SendMessageW(hwnd, LVM_GETNEXTITEM, at as usize, LVNI_SELECTED as isize) };
        if at < 0 {
            return found;
        }
        found.push(at as usize);
    }
}

/// Select exactly these rows (an empty list clears the selection).
pub fn list_select(hwnd: HWND, rows: &[usize]) {
    let mut none = Vec::new();
    let mut clear = list_item(0, 0, &mut none);
    clear.mask = LVIF_STATE;
    clear.stateMask = LVIS_SELECTED | LVIS_FOCUSED;
    unsafe { SendMessageW(hwnd, LVM_SETITEMSTATE, usize::MAX, &clear as *const LVITEMW as LPARAM) };
    for (number, row) in rows.iter().enumerate() {
        let mut item = list_item(*row, 0, &mut none);
        item.mask = LVIF_STATE;
        item.state = LVIS_SELECTED | if number == 0 { LVIS_FOCUSED } else { 0 };
        item.stateMask = LVIS_SELECTED | LVIS_FOCUSED;
        unsafe {
            SendMessageW(hwnd, LVM_SETITEMSTATE, *row, &item as *const LVITEMW as LPARAM);
            if number == 0 {
                SendMessageW(hwnd, LVM_ENSUREVISIBLE, *row, 0);
            }
        }
    }
}
