//! Checking for new MarkItDown releases and installing/upgrading it with pip.

use crate::runner::{hidden, Msg, Runner};
use eframe::egui;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::mpsc::Sender;

pub const REPO: &str = "microsoft/markitdown";
const GH_API: &str = "https://api.github.com/repos/microsoft/markitdown/releases/latest";
const PYPI_API: &str = "https://pypi.org/pypi/markitdown/json";
pub const PYTHON_DOWNLOAD: &str = "https://www.python.org/downloads/windows/";

#[derive(Clone)]
pub struct Release {
    pub version: String,
    pub url: String,
}

/// "markitdown 0.1.7", "v0.1.7" -> [0, 1, 7]
pub fn version_tuple(s: &str) -> Vec<u64> {
    let Some(start) = s.find(|c: char| c.is_ascii_digit()) else { return vec![] };
    let mut out = Vec::new();
    let mut cur = String::new();
    for c in s[start..].chars() {
        if c.is_ascii_digit() {
            cur.push(c);
        } else if c == '.' && !cur.is_empty() {
            out.push(cur.parse().unwrap_or(0));
            cur.clear();
        } else {
            break;
        }
    }
    if !cur.is_empty() {
        out.push(cur.parse().unwrap_or(0));
    }
    out
}

pub fn is_newer(latest: &str, installed: &str) -> bool {
    let (mut a, mut b) = (version_tuple(latest), version_tuple(installed));
    let n = a.len().max(b.len());
    a.resize(n, 0);
    b.resize(n, 0);
    !a.is_empty() && a > b
}

/// GET a URL with Windows' built-in curl.exe, falling back to Python's urllib.
fn http_get(url: &str, python: Option<&str>) -> Result<String, String> {
    let curl = hidden("curl")
        .args(["-sSfL", "--max-time", "20", "-H", "Accept: application/json", "-H", "User-Agent: markitdown-gui", url])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output();
    let mut err = match curl {
        Ok(o) if o.status.success() => return Ok(String::from_utf8_lossy(&o.stdout).into_owned()),
        Ok(o) => String::from_utf8_lossy(&o.stderr).trim().to_string(),
        Err(e) => e.to_string(),
    };
    if let Some(py) = python {
        let script = "import sys,urllib.request as u;r=u.Request(sys.argv[1],headers={'User-Agent':'markitdown-gui','Accept':'application/json'});sys.stdout.write(u.urlopen(r,timeout=20).read().decode('utf-8'))";
        match hidden(py).args(["-c", script, url]).stdout(Stdio::piped()).stderr(Stdio::piped()).output() {
            Ok(o) if o.status.success() => return Ok(String::from_utf8_lossy(&o.stdout).into_owned()),
            Ok(o) => err = String::from_utf8_lossy(&o.stderr).lines().last().unwrap_or("").to_string(),
            Err(e) => err = e.to_string(),
        }
    }
    Err(if err.is_empty() { "network request failed".into() } else { err })
}

/// Latest release of microsoft/markitdown on GitHub, falling back to PyPI.
pub fn latest_release(python: Option<&str>) -> Result<Release, String> {
    let gh = http_get(GH_API, python).and_then(|body| {
        let v: serde_json::Value = serde_json::from_str(&body).map_err(|e| e.to_string())?;
        let tag = v["tag_name"].as_str().ok_or("no tag_name in GitHub response")?;
        Ok(Release { version: tag.trim_start_matches('v').to_string(), url: v["html_url"].as_str().unwrap_or("").to_string() })
    });
    if gh.is_ok() {
        return gh;
    }
    let pypi = http_get(PYPI_API, python).and_then(|body| {
        let v: serde_json::Value = serde_json::from_str(&body).map_err(|e| e.to_string())?;
        let ver = v["info"]["version"].as_str().ok_or("no version in PyPI response")?;
        Ok(Release { version: ver.to_string(), url: format!("https://github.com/{REPO}/releases") })
    });
    pypi.map_err(|e| format!("Could not reach GitHub or PyPI: {e}"))
}

fn works(py: &str, code: &str) -> bool {
    hidden(py).args(["-c", code]).stdout(Stdio::null()).stderr(Stdio::null()).status().map(|s| s.success()).unwrap_or(false)
}

/// pip console-script launchers end with "#!<path to python.exe>" before an embedded zip.
fn python_from_launcher(exe: &Path) -> Option<String> {
    let bytes = std::fs::read(exe).ok()?;
    let mut i = bytes.len().saturating_sub(2);
    loop {
        if bytes[i] == b'#' && bytes[i + 1] == b'!' {
            let rest = &bytes[i + 2..];
            let end = rest.iter().position(|&b| b == b'\n' || b == b'\r').unwrap_or(rest.len().min(1024));
            let line = String::from_utf8_lossy(&rest[..end]).trim().to_string();
            let path = if let Some(stripped) = line.strip_prefix('"') { stripped.split('"').next().unwrap_or("").to_string() } else { line.split(' ').next().unwrap_or("").to_string() };
            let lower = path.to_lowercase();
            if (lower.ends_with("python.exe") || lower.ends_with("pythonw.exe") || lower.ends_with("python3") || lower.ends_with("python")) && Path::new(&path).is_file() {
                return Some(path.replace("pythonw.exe", "python.exe"));
            }
        }
        if i == 0 {
            return None;
        }
        i -= 1;
    }
}

fn which(program: &str) -> Option<PathBuf> {
    let p = Path::new(program);
    if p.is_file() {
        return Some(p.to_path_buf());
    }
    let path = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path) {
        for name in [format!("{program}.exe"), program.to_string()] {
            let c = dir.join(&name);
            if c.is_file() {
                return Some(c);
            }
        }
    }
    None
}

/// The Python interpreter that owns markitdown (for pip), or any working Python.
pub fn find_python(runner: Option<&Runner>) -> Option<String> {
    if let Some(r) = runner {
        if let Some(py) = r.python_program() {
            return Some(py);
        }
        if let Some(py) = which(r.program()).and_then(|exe| python_from_launcher(&exe)) {
            return Some(py);
        }
        for py in ["python", "py", "python3"] {
            if works(py, "import markitdown") {
                return Some(py.into());
            }
        }
    }
    ["python", "py", "python3"].into_iter().find(|py| works(py, "import sys")).map(String::from).or_else(installed_python)
}

/// python.exe in the standard install folders (works before PATH is refreshed).
fn installed_python() -> Option<String> {
    // python.org's newer "Python install manager" layout.
    if let Ok(l) = std::env::var("LOCALAPPDATA") {
        let base = PathBuf::from(l).join("Python");
        let mut cands = vec![base.join("bin").join("python.exe")];
        if let Ok(rd) = std::fs::read_dir(&base) {
            let mut dirs: Vec<PathBuf> = rd.flatten().map(|e| e.path()).filter(|p| p.file_name().map(|n| n.to_string_lossy().starts_with("pythoncore-3")).unwrap_or(false)).collect();
            dirs.sort_by_key(|d| std::cmp::Reverse(version_tuple(&d.file_name().unwrap_or_default().to_string_lossy())));
            cands.extend(dirs.into_iter().map(|d| d.join("python.exe")));
        }
        for exe in cands {
            if exe.is_file() && works(&exe.to_string_lossy(), "import sys") {
                return Some(exe.to_string_lossy().into_owned());
            }
        }
    }
    let mut roots = Vec::new();
    if let Ok(l) = std::env::var("LOCALAPPDATA") {
        roots.push(PathBuf::from(l).join("Programs").join("Python"));
    }
    for v in ["ProgramFiles", "ProgramW6432"] {
        if let Ok(p) = std::env::var(v) {
            roots.push(PathBuf::from(p));
        }
    }
    for root in roots {
        let Ok(rd) = std::fs::read_dir(&root) else { continue };
        let mut dirs: Vec<PathBuf> = rd.flatten().map(|e| e.path()).filter(|p| p.file_name().map(|n| n.to_string_lossy().to_lowercase().starts_with("python3")).unwrap_or(false)).collect();
        dirs.sort_by_key(|d| std::cmp::Reverse(version_tuple(&d.file_name().unwrap_or_default().to_string_lossy().replace("Python3", "3."))));
        for d in dirs {
            let exe = d.join("python.exe");
            if exe.is_file() && works(&exe.to_string_lossy(), "import sys") {
                return Some(exe.to_string_lossy().into_owned());
            }
        }
    }
    None
}

/// Folder for the app's own Python: next to the .exe (portable), else %LOCALAPPDATA%.
fn runtime_roots() -> Vec<PathBuf> {
    let mut v = Vec::new();
    if let Some(dir) = std::env::current_exe().ok().and_then(|e| e.parent().map(Path::to_path_buf)) {
        v.push(dir.join("MarkItDown GUI runtime"));
    }
    if let Ok(l) = std::env::var("LOCALAPPDATA") {
        v.push(PathBuf::from(l).join("MarkItDown GUI").join("runtime"));
    }
    v
}

/// The app's private Python, if it has been set up.
pub fn private_python() -> Option<String> {
    runtime_roots().into_iter().map(|r| r.join("python").join(if cfg!(windows) { "python.exe" } else { "bin/python3" })).find(|p| p.is_file()).map(|p| p.to_string_lossy().into_owned())
}

fn writable_runtime_root() -> Option<PathBuf> {
    runtime_roots().into_iter().find(|r| {
        std::fs::create_dir_all(r).is_ok() && {
            let probe = r.join(".write-test");
            let ok = std::fs::write(&probe, b"ok").is_ok();
            let _ = std::fs::remove_file(&probe);
            ok
        }
    })
}

pub fn download(url: &str, to: &Path, log: &dyn Fn(String)) -> bool {
    log(format!("Downloading {url}"));
    let to_s = to.to_string_lossy().into_owned();
    let ok = hidden("curl").args(["-fsSL", "--retry", "2", "--max-time", "900", "-o", &to_s, url]).status().map(|s| s.success()).unwrap_or(false);
    if ok {
        return true;
    }
    // Fallback for PCs without curl.exe
    let ps = format!("$ProgressPreference='SilentlyContinue';Invoke-WebRequest -UseBasicParsing -Uri '{url}' -OutFile '{}'", to_s.replace('\'', "''"));
    hidden("powershell").args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-Command", &ps]).status().map(|s| s.success()).unwrap_or(false) && to.is_file()
}

/// Downloads the official embeddable Python from python.org into the app's own folder
/// and bootstraps pip. No installer, no admin rights, no PATH changes.
fn install_private_python(log: &dyn Fn(String)) -> Option<String> {
    if !cfg!(windows) {
        log("The private Python runtime is only available on Windows.".into());
        return None;
    }
    let root = writable_runtime_root()?;
    let dir = root.join("python");
    log(format!("Setting up a private Python in {}", root.display()));
    let zip = root.join("python-embed.zip");
    let mut got = false;
    for v in ["3.13.11", "3.13.10", "3.13.9", "3.13.8", "3.13.7", "3.12.10"] {
        if download(&format!("https://www.python.org/ftp/python/{v}/python-{v}-embed-amd64.zip"), &zip, log) {
            got = true;
            break;
        }
    }
    if !got {
        log("Could not download Python from python.org. Check the internet connection and try again.".into());
        return None;
    }
    let _ = std::fs::create_dir_all(&dir);
    log("Extracting...".into());
    let (zip_s, dir_s) = (zip.to_string_lossy().into_owned(), dir.to_string_lossy().into_owned());
    let mut ok = hidden("tar").args(["-xf", &zip_s, "-C", &dir_s]).status().map(|s| s.success()).unwrap_or(false);
    if !ok {
        let ps = format!("Expand-Archive -Force -LiteralPath '{}' -DestinationPath '{}'", zip_s.replace('\'', "''"), dir_s.replace('\'', "''"));
        ok = hidden("powershell").args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-Command", &ps]).status().map(|s| s.success()).unwrap_or(false);
    }
    let _ = std::fs::remove_file(&zip);
    let python = dir.join("python.exe");
    if !ok || !python.is_file() {
        log("Extracting Python failed.".into());
        return None;
    }
    // Enable site-packages in the embeddable distribution so pip packages are importable.
    if let Ok(rd) = std::fs::read_dir(&dir) {
        for e in rd.flatten() {
            let p = e.path();
            if p.extension().map(|x| x == "_pth").unwrap_or(false) {
                if let Ok(t) = std::fs::read_to_string(&p) {
                    let mut t = t.replace("#import site", "import site");
                    if !t.contains("Lib\\site-packages") {
                        t.push_str("\r\nLib\\site-packages\r\n");
                    }
                    let _ = std::fs::write(&p, t);
                }
            }
        }
    }
    let py = python.to_string_lossy().into_owned();
    log("Installing pip...".into());
    let getpip = root.join("get-pip.py");
    if !download("https://bootstrap.pypa.io/get-pip.py", &getpip, log) {
        log("Could not download get-pip.py.".into());
        return None;
    }
    let (ok, _) = run_streaming(&py, &[&getpip.to_string_lossy(), "--no-warn-script-location", "--disable-pip-version-check"], log);
    let _ = std::fs::remove_file(&getpip);
    if !ok {
        log("Installing pip failed.".into());
        return None;
    }
    Some(py)
}

/// Runs `python -m pip install --upgrade markitdown[all]`, streaming output as Msg::Log.
pub fn pip_install(runner: Option<Runner>, tx: Sender<Msg>, ctx: egui::Context) {
    std::thread::spawn(move || {
        let log = |s: String| {
            let _ = tx.send(Msg::Log(s));
            ctx.request_repaint();
        };
        // Updating an existing install: use the Python that owns it.
        // First-time setup: use the app's own private Python (downloaded if needed),
        // so nothing depends on a system Python, PATH or admin rights.
        let python = match runner.as_ref() {
            Some(r) => find_python(Some(r)),
            None => private_python().or_else(|| if cfg!(windows) { install_private_python(&log) } else { find_python(None) }),
        };
        let Some(python) = python else {
            log(format!("Setup could not finish. You can also install Python yourself from {PYTHON_DOWNLOAD} and press Install again."));
            let _ = tx.send(Msg::PipDone);
            ctx.request_repaint();
            return;
        };
        let run = |extra: &[&str]| -> (bool, String) {
            let mut args = vec!["-m", "pip", "install", "--upgrade", "--disable-pip-version-check", "--no-warn-script-location", "markitdown[all]"];
            args.extend_from_slice(extra);
            log(format!("> \"{python}\" {}", args.join(" ")));
            run_streaming(&python, &args, &log)
        };
        let (mut ok, mut out) = run(&[]);
        if !ok && out.contains("No module named pip") {
            log("pip is missing, bootstrapping it with ensurepip...".into());
            run_streaming(&python, &["-m", "ensurepip", "--upgrade"], &log);
            (ok, out) = run(&[]);
        }
        let lower = out.to_lowercase();
        if !ok && (lower.contains("--user") || lower.contains("access is denied") || lower.contains("permission denied")) {
            log("No write access to the Python folder, retrying as a per-user install...".into());
            (ok, _) = run(&["--user"]);
        }
        if ok {
            log("Checking that markitdown starts...".into());
            match hidden(&python).args(["-m", "markitdown", "--version"]).stdout(Stdio::piped()).stderr(Stdio::piped()).output() {
                Ok(o) if o.status.success() => log(String::from_utf8_lossy(&o.stdout).trim().to_string()),
                Ok(o) => {
                    ok = false;
                    let err = String::from_utf8_lossy(&o.stderr);
                    log(err.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("markitdown failed to start").to_string());
                    if err.contains("DLL") {
                        log("A Windows runtime library is missing. Install the Microsoft Visual C++ Redistributable from https://aka.ms/vs/17/release/vc_redist.x64.exe and press Install again.".into());
                    }
                }
                Err(e) => log(e.to_string()),
            }
        }
        log(if ok { "Finished. MarkItDown is ready.".into() } else { "Setup did not complete (see above).".into() });
        let _ = tx.send(Msg::PipDone);
        ctx.request_repaint();
    });
}

fn run_streaming(program: &str, args: &[&str], log: &dyn Fn(String)) -> (bool, String) {
    let child = hidden(program).args(args).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn();
    let mut child = match child {
        Ok(c) => c,
        Err(e) => {
            log(format!("Failed to start {program}: {e}"));
            return (false, e.to_string());
        }
    };
    let (tx, rx) = std::sync::mpsc::channel::<String>();
    let mut handles = Vec::new();
    if let Some(o) = child.stdout.take() {
        let tx = tx.clone();
        handles.push(std::thread::spawn(move || BufReader::new(o).lines().map_while(Result::ok).for_each(|l| { let _ = tx.send(l); })));
    }
    if let Some(e) = child.stderr.take() {
        let tx = tx.clone();
        handles.push(std::thread::spawn(move || BufReader::new(e).lines().map_while(Result::ok).for_each(|l| { let _ = tx.send(l); })));
    }
    drop(tx);
    let mut all = String::new();
    for line in rx {
        all.push_str(&line);
        all.push('\n');
        log(line);
    }
    for h in handles {
        let _ = h.join();
    }
    let ok = child.wait().map(|s| s.success()).unwrap_or(false);
    (ok, all)
}

// ---------------------------------------------------------------------------
// Updates of MarkItDown GUI itself (from its GitHub releases)
// ---------------------------------------------------------------------------

pub const APP_REPO: &str = "mohdbushnaq/markitdown-gui";
pub const APP_VERSION: &str = env!("CARGO_PKG_VERSION");
const APP_ASSET: &str = "MarkItDown-GUI.exe";

/// The tag GitHub redirects /releases/latest to (works when the API is rate-limited).
fn latest_tag_via_redirect() -> Option<String> {
    let null = if cfg!(windows) { "NUL" } else { "/dev/null" };
    let url = format!("https://github.com/{APP_REPO}/releases/latest");
    let o = hidden("curl").args(["-sS", "--max-time", "20", "-o", null, "-w", "%{redirect_url}", &url]).stdout(Stdio::piped()).stderr(Stdio::null()).output().ok()?;
    let loc = String::from_utf8_lossy(&o.stdout).trim().to_string();
    loc.rsplit_once("/tag/").map(|(_, t)| t.to_string()).filter(|t| !t.is_empty())
}

/// Latest release of MarkItDown GUI.
pub fn latest_app_release(python: Option<&str>) -> Result<Release, String> {
    let api = format!("https://api.github.com/repos/{APP_REPO}/releases/latest");
    if let Ok(body) = http_get(&api, python) {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&body) {
            if let Some(tag) = v["tag_name"].as_str() {
                return Ok(Release { version: tag.trim_start_matches('v').to_string(), url: v["html_url"].as_str().unwrap_or("").to_string() });
            }
        }
    }
    latest_tag_via_redirect()
        .map(|tag| Release { version: tag.trim_start_matches('v').to_string(), url: format!("https://github.com/{APP_REPO}/releases/tag/{tag}") })
        .ok_or_else(|| "Could not reach GitHub to check for app updates.".into())
}

/// Downloads the given version and swaps it in for the running exe.
/// Windows lets a running exe be renamed, so: current -> .old, new -> current.
pub fn update_app(version: &str, log: &dyn Fn(String)) -> Result<PathBuf, String> {
    if !cfg!(windows) {
        return Err("App updates are only available on Windows.".into());
    }
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let new = exe.with_extension("exe.new");
    let old = exe.with_extension("exe.old");
    let url = format!("https://github.com/{APP_REPO}/releases/download/v{version}/{APP_ASSET}");
    let _ = std::fs::remove_file(&new);
    if !download(&url, &new, log) {
        return Err("Download failed. Check the internet connection and try again.".into());
    }
    let bytes = std::fs::read(&new).map_err(|e| e.to_string())?;
    if bytes.len() < 1_000_000 || !bytes.starts_with(b"MZ") {
        let _ = std::fs::remove_file(&new);
        return Err("The downloaded file is not a valid program.".into());
    }
    log("Replacing the app...".into());
    let _ = std::fs::remove_file(&old);
    std::fs::rename(&exe, &old).map_err(|e| format!("Could not replace the app (no write access to its folder?): {e}"))?;
    if let Err(e) = std::fs::rename(&new, &exe) {
        let _ = std::fs::rename(&old, &exe);
        return Err(format!("Could not replace the app: {e}"));
    }
    log(format!("Updated to {version}. Restarting..."));
    Ok(exe)
}
