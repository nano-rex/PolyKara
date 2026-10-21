//! PolyKara desktop app for Windows: the song list, the timing editor and the settings.
//!
//! The window uses the Windows API directly (no third-party crates); lyrics,
//! readings and settings files are handled by the Python engine
//! (`polykara-cli engine`), and every pipeline run is the same `polykara-cli`
//! command the terminal uses.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
#![allow(non_snake_case)]

mod app;
mod editor;
mod settings;
mod songs;
mod win;

fn main() {
    // A crash shows what went wrong instead of closing silently.
    std::panic::set_hook(Box::new(|info| {
        let text = format!("PolyKara stopped because of a problem:\n\n{info}\n\nPlease report this message.");
        let crash = std::env::temp_dir().join("polykara-crash.txt");
        let _ = std::fs::write(&crash, &text);
        win::warn(0, &text);
    }));
    let options = app::Options::from_args(std::env::args().skip(1));
    std::process::exit(app::run(options));
}
