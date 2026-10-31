//! Tools tab: the maintenance actions that are not about one selected song, kept
//! off the Songs toolbar so it does not get crowded. Output from the ones that run
//! a command appears in the log on the Songs tab, so those switch back to it.

use crate::app::Shared;
use crate::win::{self, ffi::*};

const ID_T_VIDEOS: u16 = 150;
const ID_T_DATA: u16 = 151;
const ID_T_YTDLP: u16 = 152;
const ID_T_CHECK: u16 = 153;
const ID_T_SELFTEST: u16 = 154;

/// (id, button label, description shown beside it)
const BUTTONS: &[(u16, &str, &str)] = &[
    (ID_T_VIDEOS, "Open videos folder", "Open the folder with the finished MP4 videos."),
    (ID_T_DATA, "Open PolyKara folder", "Open the PolyKara folder: songs.csv, settings and the work files."),
    (ID_T_YTDLP, "Update yt-dlp", "Update the bundled yt-dlp when YouTube downloads start failing."),
    (ID_T_CHECK, "Check tools", "Check that ffmpeg, yt-dlp, whisper and the vocal separator are present."),
    (ID_T_SELFTEST, "Self-test", "Render a test frame with the bundled tools and report anything missing."),
];

/// What the main window should do after a Tools button.
pub enum ToolsOutcome {
    /// Not a Tools control.
    Ignored,
    /// Handled with nothing further to do.
    Handled,
    /// Started a run; its output is in the Songs-tab log, so switch there.
    Ran,
}

pub struct ToolsPage {
    controls: Vec<HWND>,
    buttons: Vec<(HWND, HWND)>,
}

impl ToolsPage {
    pub fn create(shared: &mut Shared) -> ToolsPage {
        let main = shared.main;
        let heading = shared.styled(win::label(main, "Tools and maintenance"));
        win::set_font(heading, shared.bold);
        let mut controls = vec![heading];
        let mut buttons = Vec::new();
        for (id, text, hint) in BUTTONS {
            let button = shared.styled(win::button(main, *id, text));
            let label = shared.styled(win::label(main, hint));
            controls.push(button);
            controls.push(label);
            buttons.push((button, label));
        }
        ToolsPage { controls, buttons }
    }

    pub fn show(&self, visible: bool) {
        for control in &self.controls {
            win::show(*control, visible);
        }
    }

    pub fn layout(&self, shared: &Shared, x: i32, y: i32, width: i32, _height: i32) {
        let px = |value: i32| shared.px(value);
        if let Some(heading) = self.controls.first() {
            win::place(*heading, x, y + px(6), width, px(22));
        }
        let mut top = y + px(40);
        for (button, label) in &self.buttons {
            win::place(*button, x, top, px(200), px(28));
            win::place(*label, x + px(212), top + px(5), (width - px(224)).max(px(100)), px(22));
            top += px(40);
        }
    }

    pub fn command(&mut self, id: u16, code: u32, shared: &mut Shared) -> ToolsOutcome {
        if code != BN_CLICKED {
            return ToolsOutcome::Ignored;
        }
        match id {
            ID_T_VIDEOS => {
                if let Some(hello) = &shared.hello {
                    std::fs::create_dir_all(&hello.folders.output).ok();
                    win::open_path(&hello.folders.output);
                }
                ToolsOutcome::Handled
            }
            ID_T_DATA => {
                if let Some(hello) = &shared.hello {
                    win::open_path(&hello.home);
                }
                ToolsOutcome::Handled
            }
            ID_T_YTDLP => {
                let program = shared.tool("yt-dlp");
                let mut command = std::process::Command::new(&program);
                command.arg("-U");
                polykara::tools::quiet(&mut command);
                shared.start_command(command, format!("{} -U", program.display()));
                ToolsOutcome::Ran
            }
            ID_T_CHECK => {
                shared.start(vec!["check".into()]);
                ToolsOutcome::Ran
            }
            ID_T_SELFTEST => {
                shared.start(vec!["selftest".into()]);
                ToolsOutcome::Ran
            }
            _ => ToolsOutcome::Ignored,
        }
    }
}
