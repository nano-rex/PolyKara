//! Local web server for the karaoke studio and the music player
//! (polykara/server.py ported). Binds to localhost only; media is served only
//! for known songs.csv ids and scanned library tracks, never arbitrary paths.

use std::collections::HashMap;
use std::fs::File;
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::config::{self, paths};
use crate::csv::Row;
use crate::json::Value;
use crate::library;
use crate::manifest;
use crate::project;
use crate::tools;

const PEAK_RATE: usize = 100;
const PAGES: &[(&str, &str)] = &[("/", "studio.html"), ("/studio", "studio.html"), ("/player", "player.html"), ("/lyrics", "desklyric.html")];

#[derive(Clone)]
struct Job {
    state: String,
    log: Vec<String>,
    started: f64,
}

impl Job {
    fn to_json(&self) -> Value {
        Value::object(vec![
            ("state", Value::from(self.state.as_str())),
            ("log", Value::Array(self.log.iter().map(|line| Value::from(line.as_str())).collect())),
            ("started", Value::Number(self.started)),
        ])
    }
}

struct Studio {
    jobs: Mutex<HashMap<String, Job>>,
    library: Mutex<Vec<Value>>,
}

impl Studio {
    fn new() -> Studio {
        Studio { jobs: Mutex::new(HashMap::new()), library: Mutex::new(Vec::new()) }
    }

    fn tracks(&self) -> Vec<Value> {
        let mut library = self.library.lock().unwrap();
        if library.is_empty() {
            *library = library::scan(&paths().root, false);
        }
        library.clone()
    }

    fn track(&self, tid: &str) -> Option<Value> {
        self.tracks().into_iter().find(|item| item.get("id").str() == tid)
    }

    fn refresh(&self) -> Vec<Value> {
        let tracks = library::scan(&paths().root, true);
        *self.library.lock().unwrap() = tracks.clone();
        tracks
    }

    fn mark_lyrics(&self, tid: &str) {
        let mut library = self.library.lock().unwrap();
        for item in library.iter_mut() {
            if item.get("id").str() == tid {
                if let Value::Object(pairs) = item {
                    match pairs.iter_mut().find(|(key, _)| key == "lyrics") {
                        Some(pair) => pair.1 = Value::Bool(true),
                        None => pairs.push(("lyrics".into(), Value::Bool(true))),
                    }
                }
            }
        }
    }

    fn job(&self, sid: &str) -> Option<Job> {
        self.jobs.lock().unwrap().get(sid).cloned()
    }
}

// ------------------------------------------------------------------ requests

struct Request {
    method: String,
    path: String,
    query: String,
    headers: HashMap<String, String>,
    body: Vec<u8>,
}

fn read_request(stream: &mut TcpStream) -> std::io::Result<Request> {
    let mut data = Vec::new();
    let mut buffer = [0u8; 8192];
    let header_end = loop {
        if let Some(index) = data.windows(4).position(|window| window == b"\r\n\r\n") {
            break index;
        }
        if data.len() > 1024 * 1024 {
            return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "request too large"));
        }
        let count = stream.read(&mut buffer)?;
        if count == 0 {
            return Err(std::io::Error::new(std::io::ErrorKind::UnexpectedEof, "empty request"));
        }
        data.extend_from_slice(&buffer[..count]);
    };
    let head = String::from_utf8_lossy(&data[..header_end]).into_owned();
    let mut lines = head.split("\r\n");
    let mut request_line = lines.next().unwrap_or_default().split_whitespace();
    let method = request_line.next().unwrap_or_default().to_string();
    let target = request_line.next().unwrap_or("/").to_string();
    let mut headers = HashMap::new();
    for line in lines {
        if let Some((name, value)) = line.split_once(':') {
            headers.insert(name.trim().to_lowercase(), value.trim().to_string());
        }
    }
    let length: usize = headers.get("content-length").and_then(|value| value.parse().ok()).unwrap_or(0);
    if length > 64 * 1024 * 1024 {
        return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "request too large"));
    }
    let mut body = data[header_end + 4..].to_vec();
    while body.len() < length {
        let count = stream.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        body.extend_from_slice(&buffer[..count]);
    }
    body.truncate(length);
    let (path, query) = target.split_once('?').unwrap_or((target.as_str(), ""));
    Ok(Request { method, path: percent_decode(path), query: query.to_string(), headers, body })
}

fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'%' if index + 2 < bytes.len() && bytes[index + 1].is_ascii_hexdigit() && bytes[index + 2].is_ascii_hexdigit() => {
                match u8::from_str_radix(&text[index + 1..index + 3], 16) {
                    Ok(value) => {
                        out.push(value);
                        index += 3;
                    }
                    Err(_) => {
                        out.push(bytes[index]);
                        index += 1;
                    }
                }
            }
            b'+' => {
                out.push(b' ');
                index += 1;
            }
            other => {
                out.push(other);
                index += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn percent_encode(text: &str) -> String {
    let mut out = String::new();
    for byte in text.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => out.push(byte as char),
            other => out.push_str(&format!("%{other:02X}")),
        }
    }
    out
}

fn param(query: &str, key: &str) -> Option<String> {
    for pair in query.split('&') {
        let (name, value) = pair.split_once('=').unwrap_or((pair, ""));
        if percent_decode(name) == key {
            return Some(percent_decode(value));
        }
    }
    None
}

// ----------------------------------------------------------------- responses

fn reason(status: u16) -> &'static str {
    match status {
        200 => "OK",
        206 => "Partial Content",
        400 => "Bad Request",
        404 => "Not Found",
        405 => "Method Not Allowed",
        416 => "Range Not Satisfiable",
        _ => "Internal Server Error",
    }
}

fn write_headers(stream: &mut TcpStream, status: u16, headers: &[(&str, String)], length: usize) {
    let mut head = format!("HTTP/1.1 {status} {}\r\n", reason(status));
    for (name, value) in headers {
        head.push_str(&format!("{name}: {value}\r\n"));
    }
    head.push_str(&format!("Content-Length: {length}\r\nConnection: close\r\n\r\n"));
    let _ = stream.write_all(head.as_bytes());
}

fn send_json(stream: &mut TcpStream, status: u16, value: &Value, head: bool) {
    let body = value.dump().into_bytes();
    write_headers(stream, status, &[("Content-Type", "application/json; charset=utf-8".into()), ("Cache-Control", "no-store".into())], body.len());
    if !head {
        let _ = stream.write_all(&body);
    }
}

fn fail(stream: &mut TcpStream, status: u16, message: &str) {
    send_json(stream, status, &Value::object(vec![("error", Value::from(message))]), false);
}

fn send_text(stream: &mut TcpStream, text: &str, filename: &str, content_type: &str, head: bool) {
    let body = text.as_bytes();
    write_headers(
        stream,
        200,
        &[
            ("Content-Type", format!("{content_type}; charset=utf-8")),
            ("Content-Disposition", format!("attachment; filename*=UTF-8''{}", percent_encode(filename))),
        ],
        body.len(),
    );
    if !head {
        let _ = stream.write_all(body);
    }
}

fn mime(path: &Path) -> &'static str {
    match path.extension().map(|extension| extension.to_string_lossy().to_lowercase()).unwrap_or_default().as_str() {
        "html" => "text/html; charset=utf-8",
        "js" => "text/javascript",
        "css" => "text/css",
        "json" => "application/json",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "svg" => "image/svg+xml",
        "woff2" => "font/woff2",
        "mp3" => "audio/mpeg",
        "m4a" | "aac" => "audio/mp4",
        "flac" => "audio/flac",
        "ogg" | "opus" => "audio/ogg",
        "wav" => "audio/wav",
        "mp4" => "video/mp4",
        "mkv" => "video/x-matroska",
        "webm" => "video/webm",
        _ => "application/octet-stream",
    }
}

/// The byte range a Range header asks for, or None when it has no usable range.
fn parse_range(header: &str, size: u64) -> Option<Option<(u64, u64)>> {
    let spec = header.trim().strip_prefix("bytes=")?;
    let (first, last) = spec.split_once('-')?;
    if first.is_empty() && last.is_empty() {
        return None;
    }
    if first.is_empty() {
        let suffix: u64 = last.parse().ok()?;
        if suffix == 0 || size == 0 {
            return Some(None);
        }
        return Some(Some((size.saturating_sub(suffix), size - 1)));
    }
    let start: u64 = first.parse().ok()?;
    let end = if last.is_empty() { size.saturating_sub(1) } else { last.parse::<u64>().ok()?.min(size.saturating_sub(1)) };
    if start >= size || start > end {
        return Some(None);
    }
    Some(Some((start, end)))
}

fn send_file(stream: &mut TcpStream, path: &Path, content_type: Option<&str>, range: Option<&str>, head: bool) {
    let metadata = match std::fs::metadata(path) {
        Ok(metadata) if metadata.is_file() => metadata,
        _ => return fail(stream, 404, "not found"),
    };
    let size = metadata.len();
    let mut start = 0u64;
    let mut end = size.saturating_sub(1);
    let mut status = 200;
    if let Some(Some((first, last))) = range.and_then(|header| parse_range(header, size)) {
        start = first;
        end = last;
        status = 206;
    } else if range.and_then(|header| parse_range(header, size)).is_some() {
        let headers = vec![("Content-Range", format!("bytes */{size}"))];
        write_headers(stream, 416, &headers, 0);
        return;
    }
    let length = if size == 0 { 0 } else { end - start + 1 };
    let mut headers = vec![("Content-Type", content_type.unwrap_or(mime(path)).to_string()), ("Accept-Ranges", "bytes".to_string())];
    if status == 206 {
        headers.push(("Content-Range", format!("bytes {start}-{end}/{size}")));
    }
    write_headers(stream, status, &headers, length as usize);
    if head || length == 0 {
        return;
    }
    let mut file = match File::open(path) {
        Ok(file) => file,
        Err(_) => return,
    };
    if file.seek(SeekFrom::Start(start)).is_err() {
        return;
    }
    let mut remaining = length;
    let mut buffer = [0u8; 65536];
    while remaining > 0 {
        let want = buffer.len().min(remaining as usize);
        match file.read(&mut buffer[..want]) {
            Ok(0) => break,
            Ok(count) => {
                if stream.write_all(&buffer[..count]).is_err() {
                    break;
                }
                remaining -= count as u64;
            }
            Err(_) => break,
        }
    }
}

// --------------------------------------------------------------------- songs

fn song_rows() -> Result<Vec<Row>, String> {
    manifest::rows(&[])
}

fn find_song(sid: &str) -> Result<Option<Row>, String> {
    Ok(song_rows()?.into_iter().find(|row| row.get("id").map(String::as_str) == Some(sid)))
}

fn valid_id(sid: &str) -> bool {
    !sid.is_empty() && sid.chars().all(|character| character.is_ascii_alphanumeric() || character == '_' || character == '-')
}

fn song_media(sid: &str) -> Option<PathBuf> {
    let folder = paths().work.join("raw");
    let mut found: Vec<PathBuf> = std::fs::read_dir(folder)
        .ok()?
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|path| path.file_stem().map(|stem| stem.to_string_lossy().as_ref() == sid).unwrap_or(false))
        .filter(|path| !matches!(path.extension().map(|extension| extension.to_string_lossy().to_lowercase()).unwrap_or_default().as_str(), "json" | "part" | "ytdl"))
        .collect();
    found.sort();
    found.into_iter().next()
}

fn song_summary(row: &Row) -> Value {
    let sid = row.get("id").cloned().unwrap_or_default();
    let field = |name: &str| Value::from(row.get(name).cloned().unwrap_or_default());
    let ass = paths().ass.join(format!("{sid}.edited.ass")).exists() || paths().ass.join(format!("{sid}.auto.ass")).exists();
    Value::object(vec![
        ("id", Value::from(sid.as_str())),
        ("title", field("title")),
        ("artist", field("artist")),
        ("language", field("language")),
        ("media", Value::Bool(song_media(&sid).is_some())),
        ("project", Value::Bool(project::project_path(&sid).exists())),
        ("ass", Value::Bool(ass)),
        ("mp4", Value::Bool(paths().output.join(format!("{sid}.mp4")).exists())),
    ])
}

fn row_json(row: &Row) -> Vec<(String, Value)> {
    row.iter().map(|(key, value)| (key.clone(), Value::from(value.as_str()))).collect()
}

/// Peak envelope at 100 values per second, 0-255, cached by file size and mtime.
fn peaks(media: &Path) -> Vec<u16> {
    let metadata = std::fs::metadata(media).ok();
    let size = metadata.as_ref().map(|meta| meta.len()).unwrap_or(0);
    let mtime = metadata
        .and_then(|meta| meta.modified().ok())
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map(|age| age.as_secs())
        .unwrap_or(0);
    let folder = paths().work.join("peaks");
    let cache = folder.join(format!("{}-{size}-{mtime}.json", media.file_stem().map(|stem| stem.to_string_lossy().into_owned()).unwrap_or_default()));
    if let Ok(text) = std::fs::read_to_string(&cache) {
        if let Ok(Value::Array(items)) = Value::parse(&text) {
            return items.iter().filter_map(|item| item.as_i64()).map(|value| value.max(0).min(255) as u16).collect();
        }
    }
    let values: Vec<u16> = tools::waveform(&config::tool("ffmpeg"), media).unwrap_or_default().into_iter().map(|value| value >> 7).collect();
    let _ = std::fs::create_dir_all(&folder);
    let json = Value::Array(values.iter().map(|value| Value::from(*value as i64)).collect());
    let _ = std::fs::write(&cache, json.dump());
    values
}

fn start_render(studio: &Arc<Studio>, sid: &str) -> Job {
    {
        let jobs = studio.jobs.lock().unwrap();
        if let Some(job) = jobs.get(sid) {
            if job.state == "running" {
                return job.clone();
            }
        }
    }
    let job = Job {
        state: "running".into(),
        log: Vec::new(),
        started: SystemTime::now().duration_since(UNIX_EPOCH).map(|age| age.as_secs_f64()).unwrap_or(0.0),
    };
    studio.jobs.lock().unwrap().insert(sid.to_string(), job.clone());
    let studio = studio.clone();
    let id = sid.to_string();
    std::thread::spawn(move || {
        let mut command = Command::new(std::env::current_exe().unwrap_or_else(|_| PathBuf::from("polykara-cli")));
        command.args(["render", id.as_str(), "--reprocess"]).stdout(Stdio::piped()).stderr(Stdio::piped());
        tools::quiet(&mut command);
        let mut child = match command.spawn() {
            Ok(child) => child,
            Err(problem) => {
                append(&studio, &id, format!("cannot start render: {problem}"));
                set_state(&studio, &id, "failed");
                return;
            }
        };
        if let Some(stderr) = child.stderr.take() {
            let studio = studio.clone();
            let id = id.clone();
            std::thread::spawn(move || {
                for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                    append(&studio, &id, line);
                }
            });
        }
        if let Some(stdout) = child.stdout.take() {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                append(&studio, &id, line);
            }
        }
        let ok = child.wait().map(|status| status.success()).unwrap_or(false);
        set_state(&studio, &id, if ok { "done" } else { "failed" });
    });
    job
}

fn append(studio: &Arc<Studio>, sid: &str, line: String) {
    let mut jobs = studio.jobs.lock().unwrap();
    if let Some(job) = jobs.get_mut(sid) {
        job.log.push(line);
        let extra = job.log.len().saturating_sub(200);
        if extra > 0 {
            job.log.drain(..extra);
        }
    }
}

fn set_state(studio: &Arc<Studio>, sid: &str, state: &str) {
    if let Some(job) = studio.jobs.lock().unwrap().get_mut(sid) {
        job.state = state.to_string();
    }
}

// ------------------------------------------------------------------- routing

fn handle(stream: &mut TcpStream, studio: &Arc<Studio>, request: &Request) {
    let head = request.method == "HEAD";
    let path = request.path.as_str();
    if request.method != "GET" && request.method != "HEAD" && request.method != "PUT" && request.method != "POST" {
        return fail(stream, 405, "method not allowed");
    }
    if request.method == "PUT" {
        return handle_put(stream, request);
    }
    if request.method == "POST" {
        return handle_post(stream, studio, request);
    }
    if let Some((_, page)) = PAGES.iter().find(|(route, _)| *route == path) {
        return send_file(stream, &paths().root.join("web").join(*page), Some("text/html; charset=utf-8"), request.headers.get("range").map(String::as_str), head);
    }
    if let Some(rest) = path.strip_prefix("/static/") {
        let web = std::fs::canonicalize(paths().root.join("web")).unwrap_or_else(|_| paths().root.join("web"));
        let target = std::fs::canonicalize(web.join(rest)).unwrap_or_else(|_| web.join(rest));
        if !target.starts_with(&web) || !target.is_file() {
            return fail(stream, 404, "not found");
        }
        let kind = if target.extension().map(|extension| extension == "js").unwrap_or(false) { Some("text/javascript") } else { None };
        return send_file(stream, &target, kind, request.headers.get("range").map(String::as_str), head);
    }
    if path == "/api/config" {
        return send_json(stream, 200, &config::load_config().to_json(), head);
    }
    if path == "/api/songs" {
        return match song_rows() {
            Ok(rows) => send_json(stream, 200, &Value::Array(rows.iter().map(song_summary).collect()), head),
            Err(problem) => fail(stream, 500, &problem),
        };
    }
    if let Some(rest) = path.strip_prefix("/api/song/") {
        let mut parts = rest.split('/');
        let sid = parts.next().unwrap_or_default();
        let action = parts.next();
        let extra = parts.next();
        if !valid_id(sid) {
            return fail(stream, 404, "unknown song");
        }
        let row = match find_song(sid) {
            Ok(Some(row)) => row,
            Ok(None) => return fail(stream, 404, "unknown song"),
            Err(problem) => return fail(stream, 500, &problem),
        };
        let media = song_media(sid);
        match (action, extra) {
            (Some("peaks"), None) => {
                let values = media.as_deref().map(peaks).unwrap_or_default();
                let value = Value::object(vec![("rate", Value::from(PEAK_RATE as i64)), ("peaks", Value::Array(values.iter().map(|value| Value::from(*value as i64)).collect()))]);
                return send_json(stream, 200, &value, head);
            }
            (Some("export"), Some(format)) => {
                let entries = match project::load_entries(&row) {
                    Ok((entries, _)) => entries,
                    Err(problem) => return fail(stream, 400, &problem),
                };
                let format = format.to_lowercase();
                if format == "ass" {
                    return match crate::ass::ass(&row, &entries) {
                        Ok(text) => send_text(stream, &text, &format!("{sid}.ass"), "text/plain", head),
                        Err(problem) => fail(stream, 400, &problem),
                    };
                }
                return match project::export(&format, &entries) {
                    Some(text) => send_text(stream, &text, &format!("{sid}.{format}"), "text/plain", head),
                    None => fail(stream, 400, &format!("unknown export format {format}")),
                };
            }
            (None, None) => {
                let (entries, source) = match project::load_entries(&row) {
                    Ok(result) => result,
                    Err(problem) => return fail(stream, 400, &problem),
                };
                let mut pairs = row_json(&row);
                pairs.push(("source".into(), Value::from(source)));
                if let Value::Object(more) = project::to_json(&entries) {
                    pairs.extend(more);
                }
                let has_media = media.is_some();
                let video = media
                    .as_ref()
                    .map(|path| matches!(path.extension().map(|extension| extension.to_string_lossy().to_lowercase()).unwrap_or_default().as_str(), "mp4" | "mkv" | "webm" | "mov"))
                    .unwrap_or(false);
                pairs.push(("media".into(), Value::from(if has_media { format!("/media/song/{sid}") } else { String::new() })));
                pairs.push(("video".into(), Value::Bool(video)));
                pairs.push(("job".into(), studio.job(sid).map(|job| job.to_json()).unwrap_or(Value::Null)));
                return send_json(stream, 200, &Value::Object(pairs), head);
            }
            _ => return fail(stream, 404, "not found"),
        }
    }
    if let Some(sid) = path.strip_prefix("/media/song/") {
        let allowed = valid_id(sid) && find_song(sid).ok().flatten().is_some();
        let media = if allowed { song_media(sid) } else { None };
        return match media {
            Some(media) => send_file(stream, &media, None, request.headers.get("range").map(String::as_str), head),
            None => fail(stream, 404, "no media"),
        };
    }
    if path == "/api/library" {
        let tracks = if param(&request.query, "refresh").is_some() { studio.refresh() } else { studio.tracks() };
        let items: Vec<Value> = tracks
            .into_iter()
            .map(|item| match item {
                Value::Object(pairs) => Value::Object(pairs.into_iter().filter(|(key, _)| key != "size" && key != "mtime").collect()),
                other => other,
            })
            .collect();
        return send_json(stream, 200, &Value::Array(items), head);
    }
    if let Some(rest) = path.strip_prefix("/media/track/").or_else(|| path.strip_prefix("/api/track/")) {
        let is_media = path.starts_with("/media/");
        let mut parts = rest.split('/');
        let tid = parts.next().unwrap_or_default();
        let action = parts.next();
        if !tid.chars().all(|character| character.is_ascii_hexdigit()) || tid.len() != 16 {
            return fail(stream, 404, "unknown track");
        }
        if action.map(|value| value != "lyrics").unwrap_or(false) || parts.next().is_some() {
            return fail(stream, 404, "not found");
        }
        let item = match studio.track(tid) {
            Some(item) => item,
            None => return fail(stream, 404, "unknown track"),
        };
        if is_media {
            return send_file(stream, Path::new(item.get("path").str()), None, request.headers.get("range").map(String::as_str), head);
        }
        let online = param(&request.query, "online").map(|value| value != "0").unwrap_or(true);
        return send_json(stream, 200, &library::track_lyrics(&item, online), head);
    }
    if let Some(sid) = path.strip_prefix("/api/jobs/") {
        let job = studio.job(sid);
        let value = job.map(|job| job.to_json()).unwrap_or_else(|| Value::object(vec![("state", Value::from("idle")), ("log", Value::Array(Vec::new()))]));
        return send_json(stream, 200, &value, head);
    }
    if path == "/api/units" {
        let text = param(&request.query, "text").unwrap_or_default();
        let units = project::split_units(&text);
        return send_json(stream, 200, &Value::Array(units.iter().map(|unit| Value::from(unit.as_str())).collect()), head);
    }
    fail(stream, 404, "not found");
}

fn handle_put(stream: &mut TcpStream, request: &Request) {
    let sid = match request.path.strip_prefix("/api/song/") {
        Some(sid) if valid_id(sid) && !sid.contains('/') => sid,
        _ => return fail(stream, 404, "not found"),
    };
    let row = match find_song(sid) {
        Ok(Some(row)) => row,
        Ok(None) => return fail(stream, 404, "unknown song"),
        Err(problem) => return fail(stream, 500, &problem),
    };
    let data = match Value::parse(&String::from_utf8_lossy(&request.body)) {
        Ok(data) => data,
        Err(problem) => return fail(stream, 400, &format!("invalid project: {problem}")),
    };
    let entries = match project::from_json(&data) {
        Ok(entries) => entries,
        Err(problem) => return fail(stream, 400, &format!("invalid project: {problem}")),
    };
    match project::save_entries(&row, &entries) {
        Ok(edited) => {
            let root = paths().root.clone();
            let saved = edited.strip_prefix(&root).unwrap_or(&edited).display().to_string();
            let value = Value::object(vec![("saved", Value::from(saved)), ("lines", Value::from(entries.len() as i64))]);
            send_json(stream, 200, &value, false);
        }
        Err(problem) => fail(stream, 400, &format!("invalid project: {problem}")),
    }
}

fn handle_post(stream: &mut TcpStream, studio: &Arc<Studio>, request: &Request) {
    if let Some(rest) = request.path.strip_prefix("/api/song/") {
        if let Some(sid) = rest.strip_suffix("/render") {
            if !valid_id(sid) {
                return fail(stream, 404, "unknown song");
            }
            return match find_song(sid) {
                Ok(Some(_)) => send_json(stream, 200, &start_render(studio, sid).to_json(), false),
                Ok(None) => fail(stream, 404, "unknown song"),
                Err(problem) => fail(stream, 500, &problem),
            };
        }
    }
    if let Some(rest) = request.path.strip_prefix("/api/track/") {
        if let Some(tid) = rest.strip_suffix("/lyrics") {
            if !tid.chars().all(|character| character.is_ascii_hexdigit()) || tid.len() != 16 {
                return fail(stream, 404, "unknown track");
            }
            let item = match studio.track(tid) {
                Some(item) => item,
                None => return fail(stream, 404, "unknown track"),
            };
            let suffix = format!(".{}", request.headers.get("x-lyric-format").map(|value| value.trim_start_matches('.')).unwrap_or("lrc"));
            if let Err(problem) = library::store_lyrics(tid, &suffix, &request.body) {
                return fail(stream, 500, &problem);
            }
            studio.mark_lyrics(tid);
            return send_json(stream, 200, &library::track_lyrics(&item, false), false);
        }
    }
    fail(stream, 404, "not found");
}

// ---------------------------------------------------------------------- serve

fn open_browser(url: &str) {
    let url = url.to_string();
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(500));
        #[cfg(target_os = "windows")]
        let _ = Command::new("cmd").args(["/c", "start", "", url.as_str()]).spawn();
        #[cfg(target_os = "macos")]
        let _ = Command::new("open").arg(url.as_str()).spawn();
        #[cfg(all(unix, not(target_os = "macos")))]
        let _ = Command::new("xdg-open").arg(url.as_str()).spawn();
    });
}

/// Serve the studio or player on localhost until the process is stopped.
pub fn serve(host: &str, port: u16, page: &str, open_browser_page: bool) -> Result<(), String> {
    let listener = TcpListener::bind((host, port)).map_err(|problem| format!("cannot listen on {host}:{port}: {problem}"))?;
    let port = listener.local_addr().map(|address| address.port()).unwrap_or(port);
    let url = format!("http://{host}:{port}{page}");
    let label = if page == "/player" { "player" } else { "studio" };
    println!("PolyKara {label}: {url}  (Ctrl+C to stop)");
    if open_browser_page {
        open_browser(&url);
    }
    let studio = Arc::new(Studio::new());
    for connection in listener.incoming() {
        let stream = match connection {
            Ok(stream) => stream,
            Err(_) => continue,
        };
        let studio = studio.clone();
        std::thread::spawn(move || {
            let mut stream = stream;
            let _ = stream.set_nodelay(true);
            let request = match read_request(&mut stream) {
                Ok(request) => request,
                Err(_) => return,
            };
            handle(&mut stream, &studio, &request);
            let _ = stream.flush();
            let _ = stream.shutdown(std::net::Shutdown::Both);
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ranges_are_parsed_like_http_clients_expect() {
        assert_eq!(parse_range("bytes=0-9", 100), Some(Some((0, 9))));
        assert_eq!(parse_range("bytes=10-", 100), Some(Some((10, 99))));
        assert_eq!(parse_range("bytes=-10", 100), Some(Some((90, 99))));
        assert_eq!(parse_range("bytes=200-300", 100), Some(None));
        assert_eq!(parse_range("items=0-9", 100), None);
    }

    #[test]
    fn query_values_are_decoded() {
        assert_eq!(param("text=%E6%99%B4%E5%A4%A9&x=1", "text").as_deref(), Some("晴天"));
        assert_eq!(param("a+b=c+d", "a b").as_deref(), Some("c d"));
        assert_eq!(param("online=0", "online").as_deref(), Some("0"));
    }

    #[test]
    fn song_ids_are_restricted() {
        assert!(valid_id("abc-123_XY"));
        assert!(!valid_id(""));
        assert!(!valid_id("../etc"));
    }

    #[test]
    fn mime_types_follow_the_extension() {
        assert_eq!(mime(Path::new("a.js")), "text/javascript");
        assert_eq!(mime(Path::new("a.mp4")), "video/mp4");
    }
}
