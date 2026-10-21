//! Audio playback with ffplay, following the playback clock it reports.
//!
//! ffplay prints its audio clock about every 30 ms; the position between two
//! reports is extrapolated, so it is accurate to a few milliseconds even though
//! ffplay starts at the nearest seek point rather than exactly where asked.
//! With a slower speed, ffplay's clock runs in played time and is converted
//! back to song time.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Instant;

use crate::tools::quiet;

#[derive(Default)]
struct Clock {
    /// The first clock value ffplay reported, in seconds.
    first: Option<f64>,
    /// The latest clock value and when it was read.
    latest: Option<(f64, Instant)>,
}

pub struct Player {
    ffplay: PathBuf,
    child: Option<Child>,
    /// Which run the clock belongs to, so a stopped ffplay's last lines are ignored.
    run: Arc<Mutex<(u64, Clock)>>,
    runs: u64,
    requested_ms: i64,
    speed: f64,
}

/// The clock value of an ffplay status line such as "  25.56 M-A:  0.000 fd=   0".
pub fn status_clock(line: &str) -> Option<f64> {
    let line = line.trim_start();
    let (number, rest) = line.split_once(char::is_whitespace)?;
    if !rest.trim_start().starts_with("M-") || !number.contains('.') {
        return None;
    }
    let value: f64 = number.parse().ok()?;
    value.is_finite().then_some(value)
}

/// Song position in ms from ffplay's first and latest clock, `elapsed` seconds after the latest.
pub fn song_position(first: f64, latest: f64, elapsed: f64, speed: f64) -> i64 {
    // A stalled clock (paused output, end of file) must not run away.
    let elapsed = elapsed.min(0.25);
    ((first + (latest - first + elapsed) * speed) * 1000.0).round() as i64
}

impl Player {
    pub fn new(ffplay: PathBuf) -> Self {
        Player { ffplay, child: None, run: Arc::new(Mutex::new((0, Clock::default()))), runs: 0, requested_ms: 0, speed: 1.0 }
    }

    pub fn playing(&mut self) -> bool {
        match &mut self.child {
            Some(child) => matches!(child.try_wait(), Ok(None)),
            None => false,
        }
    }

    pub fn play(&mut self, path: &Path, at_ms: i64, speed: f64) -> std::io::Result<()> {
        self.stop();
        self.runs += 1;
        self.requested_ms = at_ms.max(0);
        self.speed = speed;
        *self.run.lock().unwrap() = (self.runs, Clock::default());
        let mut command = Command::new(&self.ffplay);
        command.args(["-nodisp", "-autoexit", "-vn", "-sn", "-hide_banner", "-loglevel", "error", "-stats", "-ss"]);
        command.arg(format!("{:.3}", self.requested_ms as f64 / 1000.0));
        if (speed - 1.0).abs() > 0.001 {
            command.arg("-af").arg(format!("atempo={speed}"));
        }
        command.arg(path).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::piped());
        quiet(&mut command);
        let mut child = command.spawn()?;
        let mut stderr = child.stderr.take().expect("stderr is piped");
        let (run, number) = (self.run.clone(), self.runs);
        thread::spawn(move || {
            let mut buffer = Vec::new();
            let mut chunk = [0u8; 256];
            while let Ok(count) = stderr.read(&mut chunk) {
                if count == 0 {
                    break;
                }
                buffer.extend_from_slice(&chunk[..count]);
                // Status lines end in \r; keep only the unfinished tail.
                let last = match buffer.iter().rposition(|&byte| byte == b'\r' || byte == b'\n') {
                    Some(last) => last,
                    None => continue,
                };
                let complete = String::from_utf8_lossy(&buffer[..last]).into_owned();
                buffer.drain(..=last);
                if let Some(value) = complete.split(|c| c == '\r' || c == '\n').rev().find_map(status_clock) {
                    let mut guard = run.lock().unwrap();
                    if guard.0 != number {
                        break;
                    }
                    guard.1.first.get_or_insert(value);
                    guard.1.latest = Some((value, Instant::now()));
                }
            }
        });
        self.child = Some(child);
        Ok(())
    }

    /// Song position in milliseconds.
    pub fn position(&self) -> i64 {
        let guard = self.run.lock().unwrap();
        match (guard.1.first, guard.1.latest) {
            (Some(first), Some((latest, seen))) => song_position(first, latest, seen.elapsed().as_secs_f64(), self.speed),
            _ => self.requested_ms,
        }
    }

    pub fn stop(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

impl Drop for Player {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_lines_are_recognised() {
        assert_eq!(status_clock("  25.56 M-A:  0.000 fd=   0 aq=   17KB"), Some(25.56));
        assert_eq!(status_clock("    nan M-A:    nan fd=   0"), None);
        assert_eq!(status_clock("[mp3 @ 0x55] Estimating duration"), None);
    }

    #[test]
    fn position_follows_the_reported_clock() {
        assert_eq!(song_position(9.5, 12.0, 0.1, 1.0), 12100);
        // Half speed: ffplay's clock runs in played time from where it started.
        assert_eq!(song_position(9.5, 12.0, 0.1, 0.5), 9500 + (2500 + 100) / 2);
        // A stalled clock stops a quarter second on.
        assert_eq!(song_position(0.0, 1.0, 5.0, 1.0), 1250);
    }
}
