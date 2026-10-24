//! The engine (`polykara-cli engine`) and the command line it belongs to.
//!
//! The engine answers one JSON request per line. Requests are sent without
//! waiting; each answer comes back through `poll`, paired with what the app
//! wanted it for, so the window never waits on the engine.

use std::collections::{HashMap, VecDeque};
use std::ffi::OsString;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{channel, Receiver};
use std::sync::{Arc, Mutex};
use std::thread;

use crate::json::Value;
use crate::tools::{quiet, Wake};

/// How to start the PolyKara command line: polykara-cli next to this program.
#[derive(Clone, Debug)]
pub struct Cli {
    pub program: PathBuf,
    pub prefix: Vec<OsString>,
}

impl Cli {
    pub fn find() -> Option<Cli> {
        if let Some(program) = std::env::var_os("POLYKARA_CLI") {
            return Some(Cli { program: program.into(), prefix: Vec::new() });
        }
        let exe = std::env::current_exe().ok()?;
        let folder = exe.parent()?.to_path_buf();
        let name = if cfg!(windows) { "polykara-cli.exe" } else { "polykara-cli" };
        let program = folder.join(name);
        if program.is_file() {
            return Some(Cli { program, prefix: Vec::new() });
        }
        None
    }

    pub fn command<I, S>(&self, args: I) -> Command
    where
        I: IntoIterator<Item = S>,
        S: Into<OsString>,
    {
        let mut command = Command::new(&self.program);
        command.args(&self.prefix);
        command.args(args.into_iter().map(Into::into));
        quiet(&mut command);
        command
    }

    pub fn shown(&self, args: &[String]) -> String {
        let mut parts: Vec<String> = vec![self.program.to_string_lossy().into_owned()];
        parts.extend(self.prefix.iter().map(|part| part.to_string_lossy().into_owned()));
        parts.extend(args.iter().cloned());
        parts.join(" ")
    }
}

struct Answer {
    /// None when the engine stopped.
    id: Option<u64>,
    result: Result<Value, String>,
}

pub struct Engine<W> {
    child: Option<Child>,
    stdin: Option<ChildStdin>,
    answers: Receiver<Answer>,
    pending: HashMap<u64, W>,
    failed: VecDeque<(W, String)>,
    next: u64,
    stderr: Arc<Mutex<VecDeque<String>>>,
    pub failure: Option<String>,
}

impl<W> Engine<W> {
    pub fn start(cli: &Cli, home: Option<&Path>, wake: Wake) -> Engine<W> {
        let (sender, answers) = channel();
        let stderr = Arc::new(Mutex::new(VecDeque::new()));
        let mut engine = Engine { child: None, stdin: None, answers, pending: HashMap::new(), failed: VecDeque::new(), next: 1, stderr: stderr.clone(), failure: None };
        let mut command = cli.command(["engine"]);
        if let Some(home) = home {
            command.env("POLYKARA_HOME", home);
        }
        command.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
        let mut child = match command.spawn() {
            Ok(child) => child,
            Err(error) => {
                engine.failure = Some(format!("The PolyKara engine ({}) could not be started: {error}", cli.program.display()));
                return engine;
            }
        };
        let stdout = child.stdout.take().expect("stdout is piped");
        let errors = child.stderr.take().expect("stderr is piped");
        engine.stdin = child.stdin.take();
        engine.child = Some(child);
        let repaint = wake;
        thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let line = match line {
                    Ok(line) => line,
                    Err(_) => break,
                };
                let answer = match Value::parse(&line) {
                    Ok(value) => Answer {
                        id: value.get("id").as_i64().map(|id| id as u64),
                        result: if value.get("ok").as_bool() == Some(true) {
                            Ok(value.get("result").clone())
                        } else {
                            Err(value.get("error").as_str().unwrap_or("the engine gave no reason").to_string())
                        },
                    },
                    Err(_) => continue,
                };
                if answer.id.is_none() {
                    continue;
                }
                if sender.send(answer).is_err() {
                    return;
                }
                repaint();
            }
            let _ = sender.send(Answer { id: None, result: Err(String::new()) });
            repaint();
        });
        // POLYKARA_ENGINE_LOG names a file that gets everything the engine prints (for support and tests).
        let mut log = std::env::var_os("POLYKARA_ENGINE_LOG").and_then(|path| std::fs::OpenOptions::new().create(true).append(true).open(path).ok());
        thread::spawn(move || {
            for line in BufReader::new(errors).lines() {
                let line = match line {
                    Ok(line) => line,
                    Err(_) => break,
                };
                if let Some(file) = log.as_mut() {
                    let _ = writeln!(file, "{line}");
                }
                let mut tail = stderr.lock().unwrap();
                tail.push_back(line);
                if tail.len() > 40 {
                    tail.pop_front();
                }
            }
        });
        engine
    }

    /// Send a request; its answer comes back from `poll` together with `want`.
    pub fn ask(&mut self, cmd: &str, args: Value, want: W) {
        if let Some(failure) = &self.failure {
            self.failed.push_back((want, failure.clone()));
            return;
        }
        let id = self.next;
        self.next += 1;
        let line = Value::object(vec![("id", Value::from(id as i64)), ("cmd", Value::from(cmd)), ("args", args)]).dump() + "\n";
        let sent = self.stdin.as_mut().map(|stdin| stdin.write_all(line.as_bytes()).and_then(|_| stdin.flush()));
        if matches!(sent, Some(Ok(()))) {
            self.pending.insert(id, want);
        } else {
            self.failed.push_back((want, "The PolyKara engine is not running.".into()));
        }
    }

    pub fn waiting(&self) -> bool {
        !self.pending.is_empty()
    }

    pub fn poll(&mut self) -> Vec<(W, Result<Value, String>)> {
        let mut done: Vec<(W, Result<Value, String>)> = self.failed.drain(..).map(|(want, error)| (want, Err(error))).collect();
        while let Ok(answer) = self.answers.try_recv() {
            match answer.id {
                Some(id) => {
                    if let Some(want) = self.pending.remove(&id) {
                        done.push((want, answer.result));
                    }
                }
                None => {
                    let tail: Vec<String> = self.stderr.lock().unwrap().iter().cloned().collect();
                    let failure = format!("The PolyKara engine stopped.\n{}", tail.join("\n"));
                    for (_, want) in self.pending.drain() {
                        done.push((want, Err(failure.clone())));
                    }
                    self.failure = Some(failure);
                    self.stdin = None;
                }
            }
        }
        done
    }
}

impl<W> Drop for Engine<W> {
    fn drop(&mut self) {
        // Closing stdin ends the engine; kill it if it is busy with a long request.
        self.stdin = None;
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}
