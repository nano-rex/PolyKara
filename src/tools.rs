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

/// What PolyKara says it is when it asks web services for lyrics.
pub const USER_AGENT: &str = "PolyKara/1.0 (+https://github.com/nano-rex/PolyKara)";

/// A web request through curl, which Windows 10 and 11 include: (HTTP status, body).
pub fn http(url: &str, headers: &[(&str, &str)], body: Option<&[u8]>, timeout_seconds: u32) -> Result<(u16, Vec<u8>), String> {
    use std::io::Write;
    const MARK: &[u8] = b"\n__POLYKARA_STATUS__";
    let mut command = Command::new(crate::config::tool("curl"));
    command.args(["-sS", "-L", "--compressed", "--max-time"]).arg(timeout_seconds.to_string()).args(["-A", USER_AGENT, "-w", "\n__POLYKARA_STATUS__%{http_code}"]);
    for (name, value) in headers {
        command.arg("-H").arg(format!("{name}: {value}"));
    }
    if body.is_some() {
        command.args(["-X", "POST", "--data-binary", "@-"]);
    }
    command.arg(url).stdin(if body.is_some() { Stdio::piped() } else { Stdio::null() }).stdout(Stdio::piped()).stderr(Stdio::piped());
    quiet(&mut command);
    let mut child = command.spawn().map_err(|error| format!("curl could not be started ({error})"))?;
    if let Some(body) = body {
        if let Some(mut stdin) = child.stdin.take() {
            stdin.write_all(body).map_err(|error| error.to_string())?;
        }
    }
    let output = child.wait_with_output().map_err(|error| error.to_string())?;
    let data = output.stdout;
    let at = data.windows(MARK.len()).rposition(|window| window == MARK).ok_or_else(|| {
        let problem = String::from_utf8_lossy(&output.stderr).trim().to_string();
        if problem.is_empty() { "no answer".to_string() } else { problem }
    })?;
    let status = String::from_utf8_lossy(&data[at + MARK.len()..]).trim().parse::<u16>().unwrap_or(0);
    if status == 0 {
        let problem = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(if problem.is_empty() { "no answer".into() } else { problem });
    }
    Ok((status, data[..at].to_vec()))
}

/// A web request that must succeed (2xx); the error says "HTTP 404" and so on.
pub fn fetch(url: &str, headers: &[(&str, &str)]) -> Result<Vec<u8>, HttpError> {
    match http(url, headers, None, 15) {
        Ok((status, body)) if (200..300).contains(&status) => Ok(body),
        Ok((status, _)) => Err(HttpError::Status(status)),
        Err(problem) => Err(HttpError::Network(problem)),
    }
}

#[derive(Debug)]
pub enum HttpError {
    Status(u16),
    Network(String),
}

impl std::fmt::Display for HttpError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            HttpError::Status(status) => write!(formatter, "HTTP Error {status}"),
            HttpError::Network(problem) => write!(formatter, "{problem}"),
        }
    }
}

/// Percent-encoding for a URL query value (spaces as +), as Python's urlencode does.
pub fn quote_plus(text: &str) -> String {
    let mut out = String::new();
    for byte in text.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(byte as char),
            b' ' => out.push('+'),
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

/// Percent-encoding for a URL path part (nothing is left unescaped but letters, digits and -_.~).
pub fn quote(text: &str) -> String {
    quote_plus(text).replace('+', "%20")
}

pub fn urlencode(pairs: &[(&str, String)]) -> String {
    pairs.iter().map(|(key, value)| format!("{}={}", quote_plus(key), quote_plus(value))).collect::<Vec<_>>().join("&")
}

#[cfg(windows)]
mod system {
    #[repr(C)]
    struct MemoryStatus {
        length: u32,
        load: u32,
        total_physical: u64,
        available_physical: u64,
        total_page: u64,
        available_page: u64,
        total_virtual: u64,
        available_virtual: u64,
        extended: u64,
    }

    #[link(name = "kernel32")]
    extern "system" {
        fn GlobalMemoryStatusEx(status: *mut MemoryStatus) -> i32;
        fn GetDiskFreeSpaceExW(folder: *const u16, available: *mut u64, total: *mut u64, free: *mut u64) -> i32;
    }

    pub fn total_memory() -> u64 {
        let mut status = MemoryStatus { length: std::mem::size_of::<MemoryStatus>() as u32, load: 0, total_physical: 0, available_physical: 0, total_page: 0, available_page: 0, total_virtual: 0, available_virtual: 0, extended: 0 };
        if unsafe { GlobalMemoryStatusEx(&mut status) } != 0 { status.total_physical } else { 0 }
    }

    pub fn free_bytes(path: &std::path::Path) -> Option<u64> {
        let wide: Vec<u16> = path.to_string_lossy().encode_utf16().chain(std::iter::once(0)).collect();
        let (mut available, mut total, mut free) = (0u64, 0u64, 0u64);
        (unsafe { GetDiskFreeSpaceExW(wide.as_ptr(), &mut available, &mut total, &mut free) } != 0).then(|| available)
    }
}

#[cfg(not(windows))]
mod system {
    pub fn total_memory() -> u64 {
        std::fs::read_to_string("/proc/meminfo")
            .ok()
            .and_then(|text| text.lines().find(|line| line.starts_with("MemTotal:")).and_then(|line| line.split_whitespace().nth(1)).and_then(|value| value.parse::<u64>().ok()))
            .map(|kib| kib * 1024)
            .unwrap_or(0)
    }

    pub fn free_bytes(_path: &std::path::Path) -> Option<u64> {
        None
    }
}

pub fn total_memory() -> u64 {
    system::total_memory()
}

pub fn free_bytes(path: &Path) -> Option<u64> {
    system::free_bytes(path)
}
