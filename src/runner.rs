use eframe::egui;
use std::collections::VecDeque;
use std::io::Read;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};
use std::time::Duration;

pub const CANCELLED: &str = "\u{0}cancelled";

pub enum Msg {
    Detected(Result<Runner, String>),
    Started(usize),
    Finished(usize, Result<PathBuf, String>),
    WorkerDone,
    Plugins(String),
    Latest(Result<crate::update::Release, String>),
    Log(String),
    PipDone,
}

/// A Command with no console window and UTF-8 Python I/O.
pub fn hidden(program: &str) -> Command {
    let mut c = Command::new(program);
    c.env("PYTHONIOENCODING", "utf-8").env("PYTHONUTF8", "1").stdin(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        c.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    c
}

pub struct Job {
    pub index: usize,
    /// A file path or a web URL.
    pub input: std::ffi::OsString,
    pub output: PathBuf,
    pub args: Vec<String>,
}

/// How to invoke markitdown: either the markitdown executable or `python -m markitdown`.
#[derive(Clone)]
pub struct Runner {
    program: String,
    pre: Vec<String>,
    pub version: String,
}

impl Runner {
    pub fn describe(&self) -> String {
        let mut s = self.program.clone();
        for a in &self.pre {
            s.push(' ');
            s.push_str(a);
        }
        s
    }

    fn command(&self) -> Command {
        let mut c = hidden(&self.program);
        c.args(&self.pre);
        c
    }

    pub fn program(&self) -> &str {
        &self.program
    }

    /// Set when markitdown is run as `python -m markitdown`.
    pub fn python_program(&self) -> Option<String> {
        (!self.pre.is_empty()).then(|| self.program.clone())
    }

    fn try_candidate(program: String, pre: Vec<String>) -> Option<Runner> {
        let mut r = Runner { program, pre, version: String::new() };
        let out = r.command().arg("--version").stdout(Stdio::piped()).stderr(Stdio::piped()).output().ok()?;
        if !out.status.success() {
            return None;
        }
        let v = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if !v.to_lowercase().contains("markitdown") && !v.chars().any(|c| c.is_ascii_digit()) {
            return None;
        }
        r.version = if v.to_lowercase().starts_with("markitdown") { v } else { format!("markitdown {v}") };
        Some(r)
    }

    pub fn detect(override_cmd: &str) -> Result<Runner, String> {
        let m = || vec!["-m".to_string(), "markitdown".to_string()];
        let ov = override_cmd.trim().trim_matches('"');
        if !ov.is_empty() {
            let file = std::path::Path::new(ov).file_name().map(|f| f.to_string_lossy().to_lowercase()).unwrap_or_default();
            let pre = if file.starts_with("python") || file == "py" || file == "py.exe" { m() } else { vec![] };
            return Self::try_candidate(ov.to_string(), pre).ok_or_else(|| format!("\"{ov}\" did not respond to --version."));
        }
        let mut cands: Vec<(String, Vec<String>)> = Vec::new();
        // The app's own Python (set up on first run) comes first.
        if let Some(py) = crate::update::private_python() {
            cands.push((py, m()));
        }
        cands.push(("markitdown".into(), vec![]));
        // pip often installs scripts into folders that are not on PATH.
        for exe in user_script_dirs() {
            cands.push((exe.display().to_string(), vec![]));
        }
        for py in ["python", "py", "python3"] {
            cands.push((py.into(), m()));
        }
        for (p, pre) in cands {
            if let Some(r) = Self::try_candidate(p, pre) {
                return Ok(r);
            }
        }
        Err("Could not find markitdown on PATH or via Python. Install it, or set the command under Options.".into())
    }

    pub fn list_plugins(&self) -> String {
        match self.command().arg("--list-plugins").output() {
            Ok(o) => {
                let mut s = String::from_utf8_lossy(&o.stdout).trim().to_string();
                let e = String::from_utf8_lossy(&o.stderr).trim().to_string();
                if !e.is_empty() {
                    s = format!("{s}\n{e}");
                }
                if s.trim().is_empty() { "No plugins installed.".into() } else { s }
            }
            Err(e) => e.to_string(),
        }
    }

    fn convert(&self, job: &Job, env: &[(String, String)], cancel: &AtomicBool) -> Result<PathBuf, String> {
        let mut cmd = self.command();
        cmd.args(&job.args).arg("-o").arg(&job.output).arg(&job.input).stdout(Stdio::null()).stderr(Stdio::piped());
        for (k, v) in env {
            cmd.env(k, v);
        }
        let mut child = cmd.spawn().map_err(|e| format!("Failed to start markitdown: {e}"))?;
        let mut stderr = child.stderr.take();
        let reader = std::thread::spawn(move || {
            let mut s = String::new();
            if let Some(e) = stderr.as_mut() {
                let mut buf = Vec::new();
                let _ = e.read_to_end(&mut buf);
                s = String::from_utf8_lossy(&buf).into_owned();
            }
            s
        });
        let status = loop {
            if cancel.load(Ordering::Relaxed) {
                let _ = child.kill();
                let _ = child.wait();
                return Err(CANCELLED.into());
            }
            match child.try_wait() {
                Ok(Some(st)) => break st,
                Ok(None) => std::thread::sleep(Duration::from_millis(60)),
                Err(e) => return Err(e.to_string()),
            }
        };
        let err = reader.join().unwrap_or_default();
        if status.success() && job.output.exists() {
            Ok(job.output.clone())
        } else if err.trim().is_empty() {
            Err(format!("markitdown exited with {status}"))
        } else {
            Err(err.trim().to_string())
        }
    }
}

fn user_script_dirs() -> Vec<PathBuf> {
    let mut out = Vec::new();
    #[cfg(windows)]
    {
        let mut roots = Vec::new();
        if let Ok(a) = std::env::var("APPDATA") {
            roots.push(PathBuf::from(a).join("Python"));
        }
        if let Ok(l) = std::env::var("LOCALAPPDATA") {
            roots.push(PathBuf::from(l).join("Programs").join("Python"));
        }
        for root in roots {
            if let Ok(rd) = std::fs::read_dir(&root) {
                let mut dirs: Vec<PathBuf> = rd.flatten().map(|e| e.path()).collect();
                dirs.sort();
                dirs.reverse(); // newest Python first
                for d in dirs {
                    let exe = d.join("Scripts").join("markitdown.exe");
                    if exe.is_file() {
                        out.push(exe);
                    }
                }
            }
        }
    }
    #[cfg(not(windows))]
    if let Ok(h) = std::env::var("HOME") {
        let p = PathBuf::from(h).join(".local/bin/markitdown");
        if p.is_file() {
            out.push(p);
        }
    }
    out
}

/// Starts up to `n` worker threads. Returns how many were started.
pub fn spawn_workers(runner: Runner, jobs: Vec<Job>, env: Vec<(String, String)>, n: usize, tx: Sender<Msg>, cancel: Arc<AtomicBool>, ctx: egui::Context) -> usize {
    let n = n.min(jobs.len()).max(1);
    let queue = Arc::new(Mutex::new(jobs.into_iter().collect::<VecDeque<_>>()));
    let env = Arc::new(env);
    for _ in 0..n {
        let (runner, queue, env, tx, cancel, ctx) = (runner.clone(), queue.clone(), env.clone(), tx.clone(), cancel.clone(), ctx.clone());
        std::thread::spawn(move || {
            loop {
                if cancel.load(Ordering::Relaxed) {
                    break;
                }
                let Some(job) = queue.lock().unwrap().pop_front() else { break };
                let _ = tx.send(Msg::Started(job.index));
                ctx.request_repaint();
                let r = runner.convert(&job, &env, &cancel);
                let _ = tx.send(Msg::Finished(job.index, r));
                ctx.request_repaint();
            }
            let _ = tx.send(Msg::WorkerDone);
            ctx.request_repaint();
        });
    }
    n
}
