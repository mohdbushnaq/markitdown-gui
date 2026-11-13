//! Optional per-user installation (no admin rights): copies the app to
//! %LOCALAPPDATA%\Programs\MarkItDown GUI, adds Start menu / desktop shortcuts
//! and an entry in Windows "Installed apps" so it can be uninstalled normally.

use crate::runner::hidden;
use std::path::{Path, PathBuf};

pub const APP_NAME: &str = "MarkItDown GUI";
const EXE_NAME: &str = "MarkItDown GUI.exe";
const UNINSTALL_KEY: &str = r"HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\MarkItDownGUI";
pub const RUNTIME_DIR: &str = "MarkItDown GUI runtime";
const SETTINGS_FILE: &str = "MarkItDown GUI.settings.ron";

pub fn install_dir() -> Option<PathBuf> {
    std::env::var_os("LOCALAPPDATA").map(|l| PathBuf::from(l).join("Programs").join(APP_NAME))
}

pub fn installed_exe() -> Option<PathBuf> {
    install_dir().map(|d| d.join(EXE_NAME)).filter(|p| p.is_file())
}

fn same_path(a: &Path, b: &Path) -> bool {
    let n = |p: &Path| p.canonicalize().unwrap_or_else(|_| p.to_path_buf()).to_string_lossy().to_lowercase();
    n(a) == n(b)
}

/// True when this process is the installed copy.
pub fn is_installed_copy() -> bool {
    match (std::env::current_exe().ok().and_then(|e| e.parent().map(Path::to_path_buf)), install_dir()) {
        (Some(here), Some(dir)) => same_path(&here, &dir),
        _ => false,
    }
}

/// PowerShell single-quoted string literal.
fn ps_str(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''"))
}

fn run_ps(script: &str) -> Result<(), String> {
    let file = std::env::temp_dir().join(format!("markitdown-gui-{}.ps1", std::process::id()));
    // UTF-8 with BOM so Windows PowerShell 5 reads non-ASCII paths correctly.
    let mut bytes = vec![0xEF, 0xBB, 0xBF];
    bytes.extend_from_slice(script.as_bytes());
    std::fs::write(&file, bytes).map_err(|e| e.to_string())?;
    let out = hidden("powershell")
        .args(["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-File"])
        .arg(&file)
        .output()
        .map_err(|e| format!("Could not start PowerShell: {e}"));
    let _ = std::fs::remove_file(&file);
    let out = out?;
    if out.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).lines().find(|l| !l.trim().is_empty()).unwrap_or("PowerShell failed").to_string())
    }
}

fn copy_dir(src: &Path, dst: &Path) -> std::io::Result<u64> {
    std::fs::create_dir_all(dst)?;
    let mut n = 0;
    for e in std::fs::read_dir(src)? {
        let e = e?;
        let to = dst.join(e.file_name());
        if e.file_type()?.is_dir() {
            n += copy_dir(&e.path(), &to)?;
        } else {
            std::fs::copy(e.path(), &to)?;
            n += 1;
        }
    }
    Ok(n)
}

fn dir_size_kb(p: &Path) -> u64 {
    fn walk(p: &Path) -> u64 {
        std::fs::read_dir(p).map(|rd| rd.flatten().map(|e| match e.file_type() {
            Ok(t) if t.is_dir() => walk(&e.path()),
            _ => e.metadata().map(|m| m.len()).unwrap_or(0),
        }).sum()).unwrap_or(0)
    }
    walk(p) / 1024
}

/// Installs the running exe for the current user. Returns the installed exe path.
pub fn install(desktop_shortcut: bool, log: &dyn Fn(String)) -> Result<PathBuf, String> {
    if !cfg!(windows) {
        return Err("Installing is only available on Windows.".into());
    }
    let src_exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let src_dir = src_exe.parent().map(Path::to_path_buf).unwrap_or_default();
    let dir = install_dir().ok_or("LOCALAPPDATA is not set")?;
    let target = dir.join(EXE_NAME);
    std::fs::create_dir_all(&dir).map_err(|e| format!("Could not create {}: {e}", dir.display()))?;

    log(format!("Copying the app to {}", dir.display()));
    if !same_path(&src_exe, &target) {
        std::fs::copy(&src_exe, &target).map_err(|e| format!("Could not copy the app (is the installed copy running?): {e}"))?;
    }

    // Bring the private Python runtime along so nothing is downloaded again.
    let src_rt = src_dir.join(RUNTIME_DIR);
    let dst_rt = dir.join(RUNTIME_DIR);
    if src_rt.is_dir() && !dst_rt.exists() && !same_path(&src_dir, &dir) {
        log("Moving the MarkItDown runtime (no download needed)...".into());
        if std::fs::rename(&src_rt, &dst_rt).is_err() {
            log("Different drive, copying the runtime instead. This can take a minute...".into());
            match copy_dir(&src_rt, &dst_rt) {
                Ok(n) => log(format!("Copied {n} files.")),
                Err(e) => log(format!("Could not copy the runtime ({e}). The installed app will set it up again on first start.")),
            }
        }
    }
    let src_set = src_dir.join(SETTINGS_FILE);
    let dst_set = dir.join(SETTINGS_FILE);
    if src_set.is_file() && !dst_set.exists() {
        let _ = std::fs::copy(&src_set, &dst_set);
    }

    log("Creating shortcuts and the uninstall entry...".into());
    let t = ps_str(&target.to_string_lossy());
    let d = ps_str(&dir.to_string_lossy());
    let version = env!("CARGO_PKG_VERSION");
    let size = dir_size_kb(&dir);
    let mut script = format!(
        r#"$ErrorActionPreference = 'Stop'
$sh = New-Object -ComObject WScript.Shell
function New-Link($path) {{
  $l = $sh.CreateShortcut($path); $l.TargetPath = {t}; $l.WorkingDirectory = {d}; $l.IconLocation = {t} + ',0'
  $l.Description = 'Convert documents and web pages to Markdown'; $l.Save()
}}
New-Link (Join-Path ([Environment]::GetFolderPath('Programs')) '{APP_NAME}.lnk')
$k = {key}
New-Item -Path $k -Force | Out-Null
$v = @{{ DisplayName = '{APP_NAME}'; DisplayVersion = '{version}'; Publisher = 'Mohammed Bushnaq'; DisplayIcon = {t};
  InstallLocation = {d}; UninstallString = '"' + {t} + '" --uninstall'; QuietUninstallString = '"' + {t} + '" --uninstall --quiet';
  URLInfoAbout = 'https://github.com/mohdbushnaq/markitdown-gui' }}
foreach ($n in $v.Keys) {{ New-ItemProperty -Path $k -Name $n -Value $v[$n] -PropertyType String -Force | Out-Null }}
New-ItemProperty -Path $k -Name NoModify -Value 1 -PropertyType DWord -Force | Out-Null
New-ItemProperty -Path $k -Name NoRepair -Value 1 -PropertyType DWord -Force | Out-Null
New-ItemProperty -Path $k -Name EstimatedSize -Value {size} -PropertyType DWord -Force | Out-Null
"#,
        key = ps_str(UNINSTALL_KEY)
    );
    if desktop_shortcut {
        script.push_str(&format!("New-Link (Join-Path ([Environment]::GetFolderPath('Desktop')) '{APP_NAME}.lnk')\n"));
    }
    run_ps(&script)?;
    log("Installed. You can now start MarkItDown GUI from the Start menu.".into());
    Ok(target)
}

/// Removes shortcuts and the uninstall entry, then deletes the install folder after this process exits.
pub fn uninstall() -> Result<(), String> {
    if !cfg!(windows) {
        return Err("Uninstalling is only available on Windows.".into());
    }
    let dir = install_dir().ok_or("LOCALAPPDATA is not set")?;
    let script = format!(
        r#"$ErrorActionPreference = 'SilentlyContinue'
foreach ($f in @([Environment]::GetFolderPath('Programs'), [Environment]::GetFolderPath('Desktop'))) {{
  Remove-Item -LiteralPath (Join-Path $f '{APP_NAME}.lnk') -Force
}}
Remove-Item -Path {key} -Recurse -Force
"#,
        key = ps_str(UNINSTALL_KEY)
    );
    run_ps(&script)?;
    let _ = &dir;
    // The exe can't delete its own folder while running: let cmd do it a moment after we exit.
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let cmd = format!("/c ping -n 3 127.0.0.1 >nul & rmdir /s /q \"{}\"", dir.display());
        let _ = std::process::Command::new("cmd").raw_arg(cmd).current_dir(std::env::temp_dir()).creation_flags(0x0800_0000 | 0x0000_0008).spawn();
    }
    Ok(())
}

/// Handles `--uninstall` from Windows Settings. Returns true if the app should exit.
pub fn handle_cli() -> bool {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if !args.iter().any(|a| a == "--uninstall") {
        return false;
    }
    let quiet = args.iter().any(|a| a == "--quiet");
    if !quiet {
        let r = rfd::MessageDialog::new()
            .set_title("Uninstall MarkItDown GUI")
            .set_description("Remove MarkItDown GUI, its shortcuts, settings and its private MarkItDown runtime from this PC?\n\nPortable copies elsewhere are not affected, and your converted files are kept.")
            .set_buttons(rfd::MessageButtons::YesNo)
            .set_level(rfd::MessageLevel::Warning)
            .show();
        if r != rfd::MessageDialogResult::Yes {
            return true;
        }
    }
    let res = uninstall();
    if !quiet {
        let (lvl, msg) = match res {
            Ok(()) => (rfd::MessageLevel::Info, "MarkItDown GUI was uninstalled.".to_string()),
            Err(e) => (rfd::MessageLevel::Error, format!("Uninstall failed: {e}")),
        };
        rfd::MessageDialog::new().set_title("MarkItDown GUI").set_description(msg).set_level(lvl).show();
    }
    true
}

/// Keeps the version shown in Windows "Installed apps" current after an in-app update.
pub fn refresh_registered_version() {
    if cfg!(windows) && is_installed_copy() {
        std::thread::spawn(|| {
            let _ = run_ps(&format!("Set-ItemProperty -Path {} -Name DisplayVersion -Value '{}' -ErrorAction SilentlyContinue", ps_str(UNINSTALL_KEY), env!("CARGO_PKG_VERSION")));
        });
    }
}

/// Deletes the leftover `.old` exe from a previous self-update.
pub fn cleanup_old_exe() {
    if let Ok(exe) = std::env::current_exe() {
        let _ = std::fs::remove_file(exe.with_extension("exe.old"));
    }
}
