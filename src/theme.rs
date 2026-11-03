//! The app's colour theme. Dark mode is an [app] setting read once at start-up;
//! the window, lists, log and custom-drawn panes ask here for their colours so
//! light and dark stay in one place.

use std::sync::atomic::{AtomicBool, Ordering};

use crate::win::ffi::COLORREF;
use crate::win::rgb;

static DARK: AtomicBool = AtomicBool::new(false);

pub fn set_dark(on: bool) {
    DARK.store(on, Ordering::SeqCst);
}

pub fn dark() -> bool {
    DARK.load(Ordering::SeqCst)
}

/// The window background (behind the tabs and the settings form).
pub fn window_bg() -> COLORREF {
    rgb(0x20, 0x25, 0x2b)
}

/// The background of a text surface: the log, edit boxes, lists, the lyrics list.
pub fn surface_bg() -> COLORREF {
    rgb(0x16, 0x1a, 0x1e)
}

/// The main text colour.
pub fn text() -> COLORREF {
    rgb(0xdd, 0xe3, 0xe8)
}

/// Dimmer text for hints and row numbers.
pub fn hint_text() -> COLORREF {
    rgb(0x8a, 0x93, 0x9a)
}
