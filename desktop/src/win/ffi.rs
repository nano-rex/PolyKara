//! The parts of the Windows API the app uses, declared here instead of taken from a crate.
//!
//! Layouts follow the Windows SDK headers for 64-bit Windows. On other systems
//! the declarations still type-check (nothing is linked), so `cargo check`
//! works anywhere; the program itself only runs on Windows.
#![allow(non_snake_case, non_camel_case_types, clippy::upper_case_acronyms, dead_code)]

use std::ffi::c_void;

pub type HWND = isize;
pub type HANDLE = isize;
pub type HDC = isize;
pub type HGDIOBJ = isize;
pub type HFONT = isize;
pub type HBRUSH = isize;
pub type HBITMAP = isize;
pub type HCURSOR = isize;
pub type HINSTANCE = isize;
pub type HMENU = isize;
pub type WPARAM = usize;
pub type LPARAM = isize;
pub type LRESULT = isize;
pub type BOOL = i32;
pub type COLORREF = u32;
pub type WNDPROC = Option<unsafe extern "system" fn(HWND, u32, WPARAM, LPARAM) -> LRESULT>;
pub type SUBCLASSPROC = Option<unsafe extern "system" fn(HWND, u32, WPARAM, LPARAM, usize, usize) -> LRESULT>;

#[repr(C)]
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub struct POINT {
    pub x: i32,
    pub y: i32,
}

#[repr(C)]
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub struct RECT {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct SIZE {
    pub cx: i32,
    pub cy: i32,
}

#[repr(C)]
pub struct WNDCLASSEXW {
    pub cbSize: u32,
    pub style: u32,
    pub lpfnWndProc: WNDPROC,
    pub cbClsExtra: i32,
    pub cbWndExtra: i32,
    pub hInstance: HINSTANCE,
    pub hIcon: HANDLE,
    pub hCursor: HCURSOR,
    pub hbrBackground: HBRUSH,
    pub lpszMenuName: *const u16,
    pub lpszClassName: *const u16,
    pub hIconSm: HANDLE,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct MSG {
    pub hwnd: HWND,
    pub message: u32,
    pub wParam: WPARAM,
    pub lParam: LPARAM,
    pub time: u32,
    pub pt: POINT,
}

#[repr(C)]
pub struct PAINTSTRUCT {
    pub hdc: HDC,
    pub fErase: BOOL,
    pub rcPaint: RECT,
    pub fRestore: BOOL,
    pub fIncUpdate: BOOL,
    pub rgbReserved: [u8; 32],
}

#[repr(C)]
pub struct INITCOMMONCONTROLSEX {
    pub dwSize: u32,
    pub dwICC: u32,
}

#[repr(C)]
pub struct OPENFILENAMEW {
    pub lStructSize: u32,
    pub hwndOwner: HWND,
    pub hInstance: HINSTANCE,
    pub lpstrFilter: *const u16,
    pub lpstrCustomFilter: *mut u16,
    pub nMaxCustFilter: u32,
    pub nFilterIndex: u32,
    pub lpstrFile: *mut u16,
    pub nMaxFile: u32,
    pub lpstrFileTitle: *mut u16,
    pub nMaxFileTitle: u32,
    pub lpstrInitialDir: *const u16,
    pub lpstrTitle: *const u16,
    pub Flags: u32,
    pub nFileOffset: u16,
    pub nFileExtension: u16,
    pub lpstrDefExt: *const u16,
    pub lCustData: LPARAM,
    pub lpfnHook: *const c_void,
    pub lpTemplateName: *const u16,
    pub pvReserved: *mut c_void,
    pub dwReserved: u32,
    pub FlagsEx: u32,
}

#[repr(C)]
pub struct CHOOSECOLORW {
    pub lStructSize: u32,
    pub hwndOwner: HWND,
    pub hInstance: HWND,
    pub rgbResult: COLORREF,
    pub lpCustColors: *mut COLORREF,
    pub Flags: u32,
    pub lCustData: LPARAM,
    pub lpfnHook: *const c_void,
    pub lpTemplateName: *const u16,
}

#[repr(C)]
pub struct LVCOLUMNW {
    pub mask: u32,
    pub fmt: i32,
    pub cx: i32,
    pub pszText: *mut u16,
    pub cchTextMax: i32,
    pub iSubItem: i32,
    pub iImage: i32,
    pub iOrder: i32,
    pub cxMin: i32,
    pub cxDefault: i32,
    pub cxIdeal: i32,
}

#[repr(C)]
pub struct LVITEMW {
    pub mask: u32,
    pub iItem: i32,
    pub iSubItem: i32,
    pub state: u32,
    pub stateMask: u32,
    pub pszText: *mut u16,
    pub cchTextMax: i32,
    pub iImage: i32,
    pub lParam: LPARAM,
    pub iIndent: i32,
    pub iGroupId: i32,
    pub cColumns: u32,
    pub puColumns: *mut u32,
    pub piColFmt: *mut i32,
    pub iGroup: i32,
}

#[repr(C)]
pub struct NMHDR {
    pub hwndFrom: HWND,
    pub idFrom: usize,
    pub code: u32,
}

#[repr(C)]
pub struct TCITEMW {
    pub mask: u32,
    pub dwState: u32,
    pub dwStateMask: u32,
    pub pszText: *mut u16,
    pub cchTextMax: i32,
    pub iImage: i32,
    pub lParam: LPARAM,
}

#[repr(C)]
pub struct SCROLLINFO {
    pub cbSize: u32,
    pub fMask: u32,
    pub nMin: i32,
    pub nMax: i32,
    pub nPage: u32,
    pub nPos: i32,
    pub nTrackPos: i32,
}

#[repr(C)]
pub struct TTTOOLINFOW {
    pub cbSize: u32,
    pub uFlags: u32,
    pub hwnd: HWND,
    pub uId: usize,
    pub rect: RECT,
    pub hinst: HINSTANCE,
    pub lpszText: *mut u16,
    pub lParam: LPARAM,
    pub lpReserved: *mut c_void,
}

#[repr(C)]
pub struct MINMAXINFO {
    pub ptReserved: POINT,
    pub ptMaxSize: POINT,
    pub ptMaxPosition: POINT,
    pub ptMinTrackSize: POINT,
    pub ptMaxTrackSize: POINT,
}

// Window messages
pub const WM_CREATE: u32 = 0x0001;
pub const WM_DESTROY: u32 = 0x0002;
pub const WM_SIZE: u32 = 0x0005;
pub const WM_SETFOCUS: u32 = 0x0007;
pub const WM_SETREDRAW: u32 = 0x000B;
pub const WM_PAINT: u32 = 0x000F;
pub const WM_CLOSE: u32 = 0x0010;
pub const WM_ERASEBKGND: u32 = 0x0014;
pub const WM_SETCURSOR: u32 = 0x0020;
pub const WM_GETMINMAXINFO: u32 = 0x0024;
pub const WM_SETFONT: u32 = 0x0030;
pub const WM_NOTIFY: u32 = 0x004E;
pub const WM_KEYDOWN: u32 = 0x0100;
pub const WM_KEYUP: u32 = 0x0101;
pub const WM_COMMAND: u32 = 0x0111;
pub const WM_TIMER: u32 = 0x0113;
pub const WM_HSCROLL: u32 = 0x0114;
pub const WM_VSCROLL: u32 = 0x0115;
pub const WM_CTLCOLORSTATIC: u32 = 0x0138;
pub const WM_MOUSEMOVE: u32 = 0x0200;
pub const WM_LBUTTONDOWN: u32 = 0x0201;
pub const WM_LBUTTONUP: u32 = 0x0202;
pub const WM_LBUTTONDBLCLK: u32 = 0x0203;
pub const WM_MOUSEWHEEL: u32 = 0x020A;
pub const WM_DPICHANGED: u32 = 0x02E0;
pub const WM_APP: u32 = 0x8000;

// Window styles
pub const WS_OVERLAPPEDWINDOW: u32 = 0x00CF_0000;
pub const WS_POPUP: u32 = 0x8000_0000;
pub const WS_CHILD: u32 = 0x4000_0000;
pub const WS_VISIBLE: u32 = 0x1000_0000;
pub const WS_CLIPSIBLINGS: u32 = 0x0400_0000;
pub const WS_CLIPCHILDREN: u32 = 0x0200_0000;
pub const WS_CAPTION: u32 = 0x00C0_0000;
pub const WS_BORDER: u32 = 0x0080_0000;
pub const WS_VSCROLL: u32 = 0x0020_0000;
pub const WS_HSCROLL: u32 = 0x0010_0000;
pub const WS_SYSMENU: u32 = 0x0008_0000;
pub const WS_THICKFRAME: u32 = 0x0004_0000;
pub const WS_GROUP: u32 = 0x0002_0000;
pub const WS_TABSTOP: u32 = 0x0001_0000;
pub const WS_EX_DLGMODALFRAME: u32 = 0x0001;
pub const WS_EX_TOPMOST: u32 = 0x0008;
pub const WS_EX_CLIENTEDGE: u32 = 0x0200;
pub const WS_EX_CONTROLPARENT: u32 = 0x0001_0000;
pub const CW_USEDEFAULT: i32 = 0x8000_0000u32 as i32;
pub const CS_VREDRAW: u32 = 0x0001;
pub const CS_HREDRAW: u32 = 0x0002;
pub const CS_DBLCLKS: u32 = 0x0008;
pub const COLOR_WINDOW: i32 = 5;
pub const COLOR_BTNFACE: i32 = 15;

// Controls
pub const BS_PUSHBUTTON: u32 = 0x0000;
pub const BS_DEFPUSHBUTTON: u32 = 0x0001;
pub const BS_AUTOCHECKBOX: u32 = 0x0003;
pub const BS_GROUPBOX: u32 = 0x0007;
pub const BM_GETCHECK: u32 = 0x00F0;
pub const BM_SETCHECK: u32 = 0x00F1;
pub const BN_CLICKED: u32 = 0;
pub const ES_MULTILINE: u32 = 0x0004;
pub const ES_AUTOVSCROLL: u32 = 0x0040;
pub const ES_AUTOHSCROLL: u32 = 0x0080;
pub const ES_NOHIDESEL: u32 = 0x0100;
pub const ES_READONLY: u32 = 0x0800;
pub const ES_WANTRETURN: u32 = 0x1000;
pub const ES_NUMBER: u32 = 0x2000;
pub const EM_SETSEL: u32 = 0x00B1;
pub const EM_SCROLLCARET: u32 = 0x00B7;
pub const EM_REPLACESEL: u32 = 0x00C2;
pub const EM_SETLIMITTEXT: u32 = 0x00C5;
pub const EN_SETFOCUS: u32 = 0x0100;
pub const EN_CHANGE: u32 = 0x0300;
pub const SS_LEFT: u32 = 0x0000;
pub const SS_RIGHT: u32 = 0x0002;
pub const SS_NOPREFIX: u32 = 0x0080;
pub const SS_CENTERIMAGE: u32 = 0x0200;
pub const SS_ENDELLIPSIS: u32 = 0x4000;
pub const CBS_DROPDOWN: u32 = 0x0002;
pub const CBS_DROPDOWNLIST: u32 = 0x0003;
pub const CBS_AUTOHSCROLL: u32 = 0x0040;
pub const CB_GETCURSEL: u32 = 0x0147;
pub const CB_ADDSTRING: u32 = 0x0143;
pub const CB_RESETCONTENT: u32 = 0x014B;
pub const CB_SETCURSEL: u32 = 0x014E;
pub const CB_SETMINVISIBLE: u32 = 0x1701;
pub const CBN_SELCHANGE: u32 = 1;
pub const CBN_DROPDOWN: u32 = 7;

pub const LVS_REPORT: u32 = 0x0001;
pub const LVS_SHOWSELALWAYS: u32 = 0x0008;
pub const LVS_EX_FULLROWSELECT: usize = 0x0020;
pub const LVS_EX_DOUBLEBUFFER: usize = 0x0001_0000;
pub const LVM_GETITEMCOUNT: u32 = 0x1004;
pub const LVM_DELETEALLITEMS: u32 = 0x1009;
pub const LVM_GETNEXTITEM: u32 = 0x100C;
pub const LVM_ENSUREVISIBLE: u32 = 0x1013;
pub const LVM_SETCOLUMNWIDTH: u32 = 0x101E;
pub const LVM_SETITEMSTATE: u32 = 0x102B;
pub const LVM_SETEXTENDEDLISTVIEWSTYLE: u32 = 0x1036;
pub const LVM_INSERTITEMW: u32 = 0x104D;
pub const LVM_INSERTCOLUMNW: u32 = 0x1061;
pub const LVM_SETITEMTEXTW: u32 = 0x1074;
pub const LVCF_WIDTH: u32 = 0x0002;
pub const LVCF_TEXT: u32 = 0x0004;
pub const LVIF_TEXT: u32 = 0x0001;
pub const LVIF_STATE: u32 = 0x0008;
pub const LVIS_FOCUSED: u32 = 0x0001;
pub const LVIS_SELECTED: u32 = 0x0002;
pub const LVNI_SELECTED: usize = 0x0002;
pub const NM_DBLCLK: u32 = (-3i32) as u32;
pub const NM_RETURN: u32 = (-4i32) as u32;

pub const TCM_GETCURSEL: u32 = 0x130B;
pub const TCM_SETCURSEL: u32 = 0x130C;
pub const TCM_ADJUSTRECT: u32 = 0x1328;
pub const TCM_INSERTITEMW: u32 = 0x133E;
pub const TCIF_TEXT: u32 = 0x0001;
pub const TCN_SELCHANGE: u32 = (-551i32) as u32;

pub const TTM_SETMAXTIPWIDTH: u32 = 0x0418;
pub const TTM_ADDTOOLW: u32 = 0x0432;
pub const TTF_IDISHWND: u32 = 0x0001;
pub const TTF_SUBCLASS: u32 = 0x0010;
pub const TTS_ALWAYSTIP: u32 = 0x01;
pub const TTS_NOPREFIX: u32 = 0x02;

pub const ICC_LISTVIEW_CLASSES: u32 = 0x0001;
pub const ICC_TAB_CLASSES: u32 = 0x0008;
pub const ICC_STANDARD_CLASSES: u32 = 0x4000;

// Scroll bars
pub const SB_HORZ: i32 = 0;
pub const SB_VERT: i32 = 1;
pub const SB_LINEUP: u32 = 0;
pub const SB_LINEDOWN: u32 = 1;
pub const SB_PAGEUP: u32 = 2;
pub const SB_PAGEDOWN: u32 = 3;
pub const SB_THUMBTRACK: u32 = 5;
pub const SB_TOP: u32 = 6;
pub const SB_BOTTOM: u32 = 7;
pub const SIF_RANGE: u32 = 0x0001;
pub const SIF_PAGE: u32 = 0x0002;
pub const SIF_POS: u32 = 0x0004;
pub const SIF_TRACKPOS: u32 = 0x0010;
pub const SIF_ALL: u32 = 0x0017;

// Keys
pub const VK_BACK: u32 = 0x08;
pub const VK_TAB: u32 = 0x09;
pub const VK_RETURN: u32 = 0x0D;
pub const VK_SHIFT: i32 = 0x10;
pub const VK_CONTROL: i32 = 0x11;
pub const VK_ESCAPE: u32 = 0x1B;
pub const VK_SPACE: u32 = 0x20;
pub const VK_LEFT: u32 = 0x25;
pub const VK_UP: u32 = 0x26;
pub const VK_RIGHT: u32 = 0x27;
pub const VK_DOWN: u32 = 0x28;

// Message boxes and common dialogs
pub const MB_OK: u32 = 0x0000;
pub const MB_YESNOCANCEL: u32 = 0x0003;
pub const MB_YESNO: u32 = 0x0004;
pub const MB_ICONERROR: u32 = 0x0010;
pub const MB_ICONQUESTION: u32 = 0x0020;
pub const MB_ICONWARNING: u32 = 0x0030;
pub const MB_ICONINFORMATION: u32 = 0x0040;
pub const IDCANCEL: i32 = 2;
pub const IDYES: i32 = 6;
pub const IDNO: i32 = 7;
pub const OFN_NOCHANGEDIR: u32 = 0x0008;
pub const OFN_HIDEREADONLY: u32 = 0x0004;
pub const OFN_ALLOWMULTISELECT: u32 = 0x0200;
pub const OFN_PATHMUSTEXIST: u32 = 0x0800;
pub const OFN_FILEMUSTEXIST: u32 = 0x1000;
pub const OFN_EXPLORER: u32 = 0x0008_0000;
pub const CC_RGBINIT: u32 = 0x0001;
pub const CC_FULLOPEN: u32 = 0x0002;

// Showing and placing windows
pub const SW_HIDE: i32 = 0;
pub const SW_SHOWNORMAL: i32 = 1;
pub const SW_SHOW: i32 = 5;
pub const SWP_NOSIZE: u32 = 0x0001;
pub const SWP_NOZORDER: u32 = 0x0004;
pub const SWP_NOACTIVATE: u32 = 0x0010;
pub const IDC_ARROW: usize = 32512;
pub const IDC_IBEAM: usize = 32513;
pub const IDC_SIZEWE: usize = 32644;
pub const IDC_SIZEALL: usize = 32646;
pub const IDC_HAND: usize = 32649;
pub const DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2: isize = -4;

// Drawing
pub const TRANSPARENT: i32 = 1;
pub const SRCCOPY: u32 = 0x00CC_0020;
pub const DC_BRUSH: i32 = 18;
pub const DC_PEN: i32 = 19;
pub const NULL_BRUSH: i32 = 5;
pub const NULL_PEN: i32 = 8;
pub const PS_SOLID: i32 = 0;
pub const PS_DOT: i32 = 2;
pub const FW_NORMAL: i32 = 400;
pub const FW_BOLD: i32 = 700;
pub const DEFAULT_CHARSET: u32 = 1;
pub const CLEARTYPE_QUALITY: u32 = 5;
pub const DT_CENTER: u32 = 0x0001;
pub const DT_VCENTER: u32 = 0x0004;
pub const DT_SINGLELINE: u32 = 0x0020;
pub const DT_NOPREFIX: u32 = 0x0800;
pub const DT_END_ELLIPSIS: u32 = 0x8000;

#[cfg_attr(windows, link(name = "user32"))]
extern "system" {
    pub fn RegisterClassExW(class: *const WNDCLASSEXW) -> u16;
    pub fn CreateWindowExW(ex_style: u32, class: *const u16, name: *const u16, style: u32, x: i32, y: i32, width: i32, height: i32, parent: HWND, menu: HMENU, instance: HINSTANCE, param: *const c_void) -> HWND;
    pub fn DefWindowProcW(hwnd: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT;
    pub fn GetMessageW(msg: *mut MSG, hwnd: HWND, min: u32, max: u32) -> BOOL;
    pub fn TranslateMessage(msg: *const MSG) -> BOOL;
    pub fn DispatchMessageW(msg: *const MSG) -> LRESULT;
    pub fn IsDialogMessageW(hwnd: HWND, msg: *mut MSG) -> BOOL;
    pub fn PostMessageW(hwnd: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> BOOL;
    pub fn SendMessageW(hwnd: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT;
    pub fn PostQuitMessage(code: i32);
    pub fn ShowWindow(hwnd: HWND, command: i32) -> BOOL;
    pub fn DestroyWindow(hwnd: HWND) -> BOOL;
    pub fn IsWindow(hwnd: HWND) -> BOOL;
    pub fn IsWindowVisible(hwnd: HWND) -> BOOL;
    pub fn EnableWindow(hwnd: HWND, enable: BOOL) -> BOOL;
    pub fn MoveWindow(hwnd: HWND, x: i32, y: i32, width: i32, height: i32, repaint: BOOL) -> BOOL;
    pub fn SetWindowPos(hwnd: HWND, after: HWND, x: i32, y: i32, width: i32, height: i32, flags: u32) -> BOOL;
    pub fn GetClientRect(hwnd: HWND, rect: *mut RECT) -> BOOL;
    pub fn GetWindowRect(hwnd: HWND, rect: *mut RECT) -> BOOL;
    pub fn InvalidateRect(hwnd: HWND, rect: *const RECT, erase: BOOL) -> BOOL;
    pub fn BeginPaint(hwnd: HWND, paint: *mut PAINTSTRUCT) -> HDC;
    pub fn EndPaint(hwnd: HWND, paint: *const PAINTSTRUCT) -> BOOL;
    pub fn GetDC(hwnd: HWND) -> HDC;
    pub fn ReleaseDC(hwnd: HWND, hdc: HDC) -> i32;
    pub fn FillRect(hdc: HDC, rect: *const RECT, brush: HBRUSH) -> i32;
    pub fn DrawTextW(hdc: HDC, text: *const u16, count: i32, rect: *mut RECT, format: u32) -> i32;
    pub fn SetWindowTextW(hwnd: HWND, text: *const u16) -> BOOL;
    pub fn GetWindowTextW(hwnd: HWND, text: *mut u16, max: i32) -> i32;
    pub fn GetWindowTextLengthW(hwnd: HWND) -> i32;
    pub fn GetClassNameW(hwnd: HWND, name: *mut u16, max: i32) -> i32;
    pub fn MessageBoxW(hwnd: HWND, text: *const u16, caption: *const u16, kind: u32) -> i32;
    pub fn LoadCursorW(instance: HINSTANCE, name: *const u16) -> HCURSOR;
    pub fn SetCursor(cursor: HCURSOR) -> HCURSOR;
    pub fn SetTimer(hwnd: HWND, id: usize, elapse: u32, callback: *const c_void) -> usize;
    pub fn KillTimer(hwnd: HWND, id: usize) -> BOOL;
    pub fn SetFocus(hwnd: HWND) -> HWND;
    pub fn GetFocus() -> HWND;
    pub fn GetParent(hwnd: HWND) -> HWND;
    pub fn GetAncestor(hwnd: HWND, flags: u32) -> HWND;
    pub fn GetForegroundWindow() -> HWND;
    pub fn GetKeyState(key: i32) -> i16;
    pub fn SetCapture(hwnd: HWND) -> HWND;
    pub fn ReleaseCapture() -> BOOL;
    pub fn GetCapture() -> HWND;
    pub fn GetCursorPos(point: *mut POINT) -> BOOL;
    pub fn ScreenToClient(hwnd: HWND, point: *mut POINT) -> BOOL;
    pub fn SetScrollInfo(hwnd: HWND, bar: i32, info: *const SCROLLINFO, redraw: BOOL) -> i32;
    pub fn GetScrollInfo(hwnd: HWND, bar: i32, info: *mut SCROLLINFO) -> BOOL;
    pub fn GetDpiForWindow(hwnd: HWND) -> u32;
    pub fn SetProcessDpiAwarenessContext(context: isize) -> BOOL;
    pub fn GetSysColorBrush(index: i32) -> HBRUSH;
    pub fn GetSysColor(index: i32) -> COLORREF;
    pub fn SetForegroundWindow(hwnd: HWND) -> BOOL;
}

#[cfg_attr(windows, link(name = "gdi32"))]
extern "system" {
    pub fn CreateCompatibleDC(hdc: HDC) -> HDC;
    pub fn CreateCompatibleBitmap(hdc: HDC, width: i32, height: i32) -> HBITMAP;
    pub fn SelectObject(hdc: HDC, object: HGDIOBJ) -> HGDIOBJ;
    pub fn DeleteObject(object: HGDIOBJ) -> BOOL;
    pub fn DeleteDC(hdc: HDC) -> BOOL;
    pub fn BitBlt(hdc: HDC, x: i32, y: i32, width: i32, height: i32, source: HDC, sx: i32, sy: i32, rop: u32) -> BOOL;
    pub fn GetStockObject(index: i32) -> HGDIOBJ;
    pub fn SetDCBrushColor(hdc: HDC, colour: COLORREF) -> COLORREF;
    pub fn SetDCPenColor(hdc: HDC, colour: COLORREF) -> COLORREF;
    pub fn CreatePen(style: i32, width: i32, colour: COLORREF) -> HGDIOBJ;
    pub fn CreateSolidBrush(colour: COLORREF) -> HBRUSH;
    pub fn CreateFontW(height: i32, width: i32, escapement: i32, orientation: i32, weight: i32, italic: u32, underline: u32, strike: u32, charset: u32, precision: u32, clip: u32, quality: u32, pitch: u32, face: *const u16) -> HFONT;
    pub fn SetTextColor(hdc: HDC, colour: COLORREF) -> COLORREF;
    pub fn SetBkMode(hdc: HDC, mode: i32) -> i32;
    pub fn SetBkColor(hdc: HDC, colour: COLORREF) -> COLORREF;
    pub fn TextOutW(hdc: HDC, x: i32, y: i32, text: *const u16, count: i32) -> BOOL;
    pub fn GetTextExtentPoint32W(hdc: HDC, text: *const u16, count: i32, size: *mut SIZE) -> BOOL;
    pub fn MoveToEx(hdc: HDC, x: i32, y: i32, old: *mut POINT) -> BOOL;
    pub fn LineTo(hdc: HDC, x: i32, y: i32) -> BOOL;
    pub fn Rectangle(hdc: HDC, left: i32, top: i32, right: i32, bottom: i32) -> BOOL;
    pub fn Polygon(hdc: HDC, points: *const POINT, count: i32) -> BOOL;
    pub fn IntersectClipRect(hdc: HDC, left: i32, top: i32, right: i32, bottom: i32) -> i32;
    pub fn SaveDC(hdc: HDC) -> i32;
    pub fn RestoreDC(hdc: HDC, saved: i32) -> BOOL;
}

#[cfg_attr(windows, link(name = "comctl32"))]
extern "system" {
    pub fn InitCommonControlsEx(init: *const INITCOMMONCONTROLSEX) -> BOOL;
    pub fn SetWindowSubclass(hwnd: HWND, proc_: SUBCLASSPROC, id: usize, data: usize) -> BOOL;
    pub fn DefSubclassProc(hwnd: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT;
}

#[cfg_attr(windows, link(name = "comdlg32"))]
extern "system" {
    pub fn GetOpenFileNameW(dialog: *mut OPENFILENAMEW) -> BOOL;
    pub fn ChooseColorW(dialog: *mut CHOOSECOLORW) -> BOOL;
}

#[cfg_attr(windows, link(name = "shell32"))]
extern "system" {
    pub fn ShellExecuteW(hwnd: HWND, operation: *const u16, file: *const u16, parameters: *const u16, directory: *const u16, show: i32) -> HINSTANCE;
}

#[cfg_attr(windows, link(name = "kernel32"))]
extern "system" {
    pub fn GetModuleHandleW(name: *const u16) -> HINSTANCE;
}
