//! Child programs: ffmpeg for the waveform, the file browser, and stopping a run.

use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::Arc;

/// Called from a worker thread when it has something for the window.
pub type Wake = Arc<dyn Fn() + Send + Sync>;

/// Keep a console program from opening a window of its own on Windows.
pub fn quiet(command: &mut Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    #[cfg(not(windows))]
    let _ = command;
}

/// Peak level (0-32767) of every `bin_ms` of 16-bit mono samples at `rate` Hz.
pub fn peaks(samples: &[u8], rate: usize, bin_ms: usize) -> Vec<u16> {
    let size = (rate * bin_ms / 1000).max(1) * 2;
    samples
        .chunks(size)
        .map(|chunk| {
            chunk
                .chunks_exact(2)
                .map(|pair| i16::from_le_bytes([pair[0], pair[1]]).unsigned_abs().min(32767))
                .max()
                .unwrap_or(0)
        })
        .collect()
}

/// The peak of every 10 ms of the audio, decoded with ffmpeg.
pub fn waveform(ffmpeg: &Path, media: &Path) -> std::io::Result<Vec<u16>> {
    const RATE: usize = 8000;
    let mut command = Command::new(ffmpeg);
    command
        .args(["-hide_banner", "-loglevel", "error", "-i"])
        .arg(media)
        .args(["-vn", "-ac", "1", "-ar", "8000", "-f", "s16le", "-"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    quiet(&mut command);
    let mut child = command.spawn()?;
    let mut bytes = Vec::new();
    child.stdout.take().expect("stdout is piped").read_to_end(&mut bytes)?;
    child.wait()?;
    Ok(peaks(&bytes, RATE, 10))
}

/// Open a file or folder with the program the system uses for it.
pub fn open_path(path: &Path) {
    #[cfg(windows)]
    let mut command = {
        let mut command = Command::new("explorer");
        command.arg(path);
        command
    };
    #[cfg(target_os = "macos")]
    let mut command = {
        let mut command = Command::new("open");
        command.arg(path);
        command
    };
    #[cfg(all(not(windows), not(target_os = "macos")))]
    let mut command = {
        let mut command = Command::new("xdg-open");
        command.arg(path);
        command
    };
    let _ = command.spawn();
}

/// Stop a program and everything it started (ffmpeg and yt-dlp run as its children).
pub fn stop_tree(pid: u32) {
    #[cfg(windows)]
    let mut command = {
        let mut command = Command::new("taskkill");
        command.args(["/T", "/F", "/PID"]).arg(pid.to_string());
        command
    };
    #[cfg(not(windows))]
    let mut command = {
        // Development runs on other systems: the program itself is enough.
        let mut command = Command::new("kill");
        command.arg("-TERM").arg(pid.to_string());
        command
    };
    command.stdout(Stdio::null()).stderr(Stdio::null());
    quiet(&mut command);
    let _ = command.status();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn peaks_keep_the_loudest_sample_of_each_bin() {
        let mut samples = Vec::new();
        for value in [1i16, -300, 20, 5, i16::MIN, 7] {
            samples.extend_from_slice(&value.to_le_bytes());
        }
        // 2 samples per bin at 200 Hz and 10 ms bins.
        assert_eq!(peaks(&samples, 200, 10), vec![300, 20, 32767]);
        assert!(peaks(&[], 8000, 10).is_empty());
    }
}
