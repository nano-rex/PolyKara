//! One pipeline run at a time, with its output and the song it is working on.

use std::io::{BufRead, BufReader, Read};
use std::process::{Command, Stdio};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::thread;

use crate::tools::{stop_tree, Wake};

pub enum Event {
    Output(String),
    /// The pipeline started `step` on song `id` (its ">> step id" lines).
    Progress { step: String, id: String },
    Finished(Option<i32>),
}

/// A ">> step id" line, as the pipeline prints it when it starts on a song.
pub fn progress(line: &str) -> Option<(String, String)> {
    let rest = line.strip_prefix(">> ")?.trim_end();
    let (step, id) = rest.split_once(' ')?;
    let valid = !step.is_empty() && !step.contains(char::is_whitespace) && !id.is_empty() && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    valid.then(|| (step.to_string(), id.to_string()))
}

pub fn step_name(step: &str) -> String {
    match step {
        "download" => "downloading…".into(),
        "normalize" => "preparing audio…".into(),
        "align" => "timing words…".into(),
        "lyrics" => "making subtitles…".into(),
        "render" => "rendering…".into(),
        "verify" => "checking lyrics…".into(),
        "preview" => "previewing…".into(),
        "edit" => "editing…".into(),
        other => format!("{other}…"),
    }
}

#[derive(Default)]
pub struct Runner {
    pid: Option<u32>,
    events: Option<Receiver<Event>>,
}

fn pump(source: impl Read, sender: Sender<Event>, wake: Wake) {
    let mut reader = BufReader::new(source);
    let mut raw = Vec::new();
    while matches!(reader.read_until(b'\n', &mut raw), Ok(count) if count > 0) {
        let text = String::from_utf8_lossy(&raw).trim_end_matches(['\r', '\n']).to_string();
        raw.clear();
        // yt-dlp and ffmpeg redraw their progress with \r; keep the latest state of the line.
        let text = text.rsplit('\r').next().unwrap_or_default().to_string();
        let event = match progress(&text) {
            Some((step, id)) => Event::Progress { step, id },
            None => Event::Output(text),
        };
        if sender.send(event).is_err() {
            return;
        }
        wake();
    }
}

impl Runner {
    pub fn running(&self) -> bool {
        self.pid.is_some()
    }

    pub fn start(&mut self, mut command: Command, wake: &Wake) -> std::io::Result<()> {
        command.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
        let mut child = command.spawn()?;
        let (sender, events) = channel();
        let readers = [
            {
                let (stdout, sender, wake) = (child.stdout.take().expect("piped"), sender.clone(), wake.clone());
                thread::spawn(move || pump(stdout, sender, wake))
            },
            {
                let (stderr, sender, wake) = (child.stderr.take().expect("piped"), sender.clone(), wake.clone());
                thread::spawn(move || pump(stderr, sender, wake))
            },
        ];
        self.pid = Some(child.id());
        let wake = wake.clone();
        thread::spawn(move || {
            for reader in readers {
                let _ = reader.join();
            }
            let code = child.wait().ok().and_then(|status| status.code());
            let _ = sender.send(Event::Finished(code));
            wake();
        });
        self.events = Some(events);
        Ok(())
    }

    pub fn poll(&mut self) -> Vec<Event> {
        let mut found = Vec::new();
        if let Some(events) = &self.events {
            while let Ok(event) = events.try_recv() {
                let finished = matches!(event, Event::Finished(_));
                found.push(event);
                if finished {
                    self.pid = None;
                    self.events = None;
                    break;
                }
            }
        }
        found
    }

    pub fn stop(&self) {
        if let Some(pid) = self.pid {
            stop_tree(pid);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn progress_lines_are_recognised() {
        assert_eq!(progress(">> render my-song_2\n"), Some(("render".into(), "my-song_2".into())));
        assert_eq!(progress(">> render two words"), None);
        assert_eq!(progress("render song"), None);
        assert_eq!(step_name("align"), "timing words…");
        assert_eq!(step_name("mix"), "mix…");
    }
}
