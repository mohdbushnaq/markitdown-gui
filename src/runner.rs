use eframe::egui;
use std::collections::VecDeque;
use std::io::Read;
use std::path::{Path, PathBuf};
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
    AppLatest(Result<crate::update::Release, String>),
    /// install / uninstall / app update finished: (ok, message, exe path)
    TaskDone(bool, String, Option<PathBuf>),
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
    pub is_url: bool,
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

    /// Runs markitdown once: `input` -> `output`.
    fn run_once(&self, input: &std::ffi::OsStr, output: &Path, args: &[String], env: &[(String, String)], cancel: &AtomicBool) -> Result<(), String> {
        let mut cmd = self.command();
        cmd.args(args).arg("-o").arg(output).arg(input).stdout(Stdio::null()).stderr(Stdio::piped());
        for (k, v) in env {
            cmd.env(k, v);
        }
        let mut child = cmd.spawn().map_err(|e| format!("Failed to start markitdown: {e}"))?;
        let mut stderr = child.stderr.take();
        let reader = std::thread::spawn(move || {
            let mut buf = Vec::new();
            if let Some(e) = stderr.as_mut() {
                let _ = e.read_to_end(&mut buf);
            }
            String::from_utf8_lossy(&buf).into_owned()
        });
        let status = wait_child(&mut child, cancel, None)?;
        let err = reader.join().unwrap_or_default();
        if status.success() && output.exists() {
            Ok(())
        } else if err.trim().is_empty() {
            Err(format!("markitdown exited with {status}"))
        } else {
            Err(err.trim().to_string())
        }
    }

    fn convert(&self, job: &Job, env: &[(String, String)], cancel: &AtomicBool) -> Result<PathBuf, String> {
        let direct = self.run_once(&job.input, &job.output, &job.args, env, cancel);
        if direct.as_ref().err().map(|e| e == CANCELLED).unwrap_or(false) {
            return Err(CANCELLED.into());
        }
        if !job.is_url {
            return direct.map(|_| job.output.clone());
        }
        if direct.is_ok() && has_content(&job.output) {
            return Ok(job.output.clone());
        }
        // Web page came back empty (built by JavaScript) or was blocked:
        // load it in a hidden browser and convert the finished page instead.
        let url = job.input.to_string_lossy().into_owned();
        let rendered = render_page(&url, cancel);
        if rendered.as_ref().err().map(|e| e == CANCELLED).unwrap_or(false) {
            let _ = std::fs::remove_file(&job.output);
            return Err(CANCELLED.into());
        }
        let result = rendered.and_then(|html| {
            let tmp = std::env::temp_dir().join(format!("markitdown-gui-{}-{}.html", std::process::id(), job.index));
            std::fs::write(&tmp, with_base_href(&html, &url)).map_err(|e| e.to_string())?;
            let mut args: Vec<String> = job.args.iter().filter(|a| !a.is_empty()).cloned().collect();
            strip_hint_args(&mut args);
            let r = self.run_once(tmp.as_os_str(), &job.output, &args, env, cancel);
            let _ = std::fs::remove_file(&tmp);
            r
        });
        match result {
            Ok(()) if has_content(&job.output) => Ok(job.output.clone()),
            Ok(()) => {
                let _ = std::fs::remove_file(&job.output);
                Err("The page has no readable text, even after loading it in a browser. It may need a sign-in, or block automated access.".into())
            }
            Err(e) if e == CANCELLED => Err(e),
            Err(e) => {
                let _ = std::fs::remove_file(&job.output);
                match direct {
                    Err(d) => Err(format!("{d}\n\nBrowser fallback: {e}")),
                    Ok(()) => Err(format!("The page came back empty (it is probably built with JavaScript), and loading it in a browser failed: {e}")),
                }
            }
        }
    }
}

/// True when a converted file has real text in it.
fn has_content(p: &Path) -> bool {
    std::fs::read_to_string(p).map(|t| t.chars().filter(|c| !c.is_whitespace()).count() >= 20).unwrap_or(false)
}

/// Input hints (-x/-m/-c) describe the original URL, not the saved HTML file.
fn strip_hint_args(args: &mut Vec<String>) {
    let mut out = Vec::new();
    let mut skip = false;
    for a in args.drain(..) {
        if skip {
            skip = false;
            continue;
        }
        if a == "-x" || a == "-m" || a == "-c" {
            skip = true;
            continue;
        }
        out.push(a);
    }
    *args = out;
}

/// Adds <base href> so relative links in the saved page still point at the site.
fn with_base_href(html: &str, url: &str) -> String {
    let lower = html.to_lowercase();
    if lower.contains("<base ") {
        return html.to_string();
    }
    let tag = format!("<base href=\"{}\">", url.replace('"', "%22"));
    match lower.find("<head>") {
        Some(i) => format!("{}{}{}", &html[..i + 6], tag, &html[i + 6..]),
        None => format!("{tag}{html}"),
    }
}

fn wait_child(child: &mut std::process::Child, cancel: &AtomicBool, timeout: Option<Duration>) -> Result<std::process::ExitStatus, String> {
    let start = std::time::Instant::now();
    loop {
        if cancel.load(Ordering::Relaxed) {
            let _ = child.kill();
            let _ = child.wait();
            return Err(CANCELLED.into());
        }
        if let Some(t) = timeout {
            if start.elapsed() > t {
                let _ = child.kill();
                let _ = child.wait();
                return Err("timed out".into());
            }
        }
        match child.try_wait() {
            Ok(Some(st)) => return Ok(st),
            Ok(None) => std::thread::sleep(Duration::from_millis(60)),
            Err(e) => return Err(e.to_string()),
        }
    }
}

/// Microsoft Edge (built into Windows 10/11) or Google Chrome.
/// Set MARKITDOWN_GUI_BROWSER to use a specific Chromium-based browser.
fn find_browser() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("MARKITDOWN_GUI_BROWSER") {
        return Some(PathBuf::from(p));
    }
    let mut c: Vec<PathBuf> = Vec::new();
    if cfg!(windows) {
        for (var, rel) in [
            ("ProgramFiles(x86)", r"Microsoft\Edge\Application\msedge.exe"),
            ("ProgramFiles", r"Microsoft\Edge\Application\msedge.exe"),
            ("LOCALAPPDATA", r"Microsoft\Edge\Application\msedge.exe"),
            ("ProgramFiles", r"Google\Chrome\Application\chrome.exe"),
            ("ProgramFiles(x86)", r"Google\Chrome\Application\chrome.exe"),
            ("LOCALAPPDATA", r"Google\Chrome\Application\chrome.exe"),
        ] {
            if let Some(base) = std::env::var_os(var) {
                c.push(PathBuf::from(base).join(rel));
            }
        }
    } else {
        for n in ["microsoft-edge", "google-chrome", "chromium", "chromium-browser"] {
            if let Some(path) = std::env::var_os("PATH") {
                c.extend(std::env::split_paths(&path).map(|d| d.join(n)));
            }
        }
    }
    c.into_iter().find(|p| p.is_file())
}

/// Loads the page in a hidden browser, lets its JavaScript run, and returns the finished HTML.
fn render_page(url: &str, cancel: &AtomicBool) -> Result<String, String> {
    let browser = find_browser().ok_or("Microsoft Edge or Google Chrome was not found")?;
    let profile = std::env::temp_dir().join(format!("markitdown-gui-browser-{}", std::process::id()));
    let mut child = hidden(&browser.to_string_lossy())
        .args(["--headless=new", "--disable-gpu", "--no-first-run", "--no-default-browser-check", "--disable-extensions", "--mute-audio", "--hide-scrollbars", "--window-size=1366,2000", "--virtual-time-budget=15000"])
        .arg(format!("--user-data-dir={}", profile.display()))
        .arg("--dump-dom")
        .arg(url)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("could not start {}: {e}", browser.display()))?;
    let mut out = child.stdout.take();
    let reader = std::thread::spawn(move || {
        let mut buf = Vec::new();
        if let Some(o) = out.as_mut() {
            let _ = o.read_to_end(&mut buf);
        }
        buf
    });
    let status = wait_child(&mut child, cancel, Some(Duration::from_secs(90)));
    let html = String::from_utf8_lossy(&reader.join().unwrap_or_default()).into_owned();
    let _ = std::fs::remove_dir_all(&profile);
    status?;
    if html.contains('<') && html.trim().len() > 50 {
        Ok(html)
    } else {
        Err("the browser returned no page".into())
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
