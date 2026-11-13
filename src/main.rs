#![cfg_attr(windows, windows_subsystem = "windows")]

mod install;
mod runner;
mod update;

use eframe::egui::{self, Color32, CornerRadius, RichText, Stroke};
use runner::{Job, Msg, Runner};
use update::Release;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender};
use std::sync::Arc;

const ACCENT: Color32 = Color32::from_rgb(0x25, 0x63, 0xEB);
const OK: Color32 = Color32::from_rgb(0x16, 0xA3, 0x4A);
const ERR: Color32 = Color32::from_rgb(0xDC, 0x26, 0x26);

#[derive(Serialize, Deserialize, Clone)]
#[serde(default)]
struct Settings {
    custom_output: bool,
    output_dir: Option<PathBuf>,
    overwrite: bool,
    keep_data_uris: bool,
    use_plugins: bool,
    ext_hint: String,
    mime_hint: String,
    charset_hint: String,
    use_docintel: bool,
    docintel_endpoint: String,
    use_cu: bool,
    cu_endpoint: String,
    cu_analyzer: String,
    cu_file_types: String,
    #[serde(skip)]
    azure_key: String,
    jobs: usize,
    command_override: String,
    open_folder_when_done: bool,
    check_on_startup: bool,
    desktop_shortcut: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            custom_output: false,
            output_dir: None,
            overwrite: false,
            keep_data_uris: false,
            use_plugins: false,
            ext_hint: String::new(),
            mime_hint: String::new(),
            charset_hint: String::new(),
            use_docintel: false,
            docintel_endpoint: String::new(),
            use_cu: false,
            cu_endpoint: String::new(),
            cu_analyzer: String::new(),
            cu_file_types: String::new(),
            azure_key: String::new(),
            jobs: 3,
            command_override: String::new(),
            open_folder_when_done: false,
            check_on_startup: true,
            desktop_shortcut: true,
        }
    }
}

#[derive(Clone, PartialEq)]
enum Status {
    Pending,
    Queued,
    Running,
    Done(PathBuf),
    Failed(String),
    Cancelled,
}

struct Item {
    path: PathBuf,
    /// Set for web pages; `path` is unused then.
    url: Option<String>,
    status: Status,
}

impl Item {
    fn title(&self) -> String {
        match &self.url {
            Some(u) => u.clone(),
            None => self.path.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default(),
        }
    }
    fn subtitle(&self) -> String {
        match &self.url {
            Some(_) => "Web page".into(),
            None => self.path.parent().map(|p| p.display().to_string()).unwrap_or_default(),
        }
    }
}

/// Accepts "example.com/page" or "https://example.com/page"; returns None if it isn't a web address.
fn normalize_url(raw: &str) -> Option<String> {
    let t = raw.trim().trim_matches('"').trim();
    if t.is_empty() || t.contains(char::is_whitespace) {
        return None;
    }
    let lower = t.to_lowercase();
    let u = if lower.starts_with("http://") || lower.starts_with("https://") { t.to_string() } else { format!("https://{t}") };
    let host = u.split("://").nth(1)?.split(['/', '?', '#']).next()?;
    (host.contains('.') || host.starts_with("localhost")).then_some(u)
}

/// "https://en.wikipedia.org/wiki/Markdown" -> "en.wikipedia.org - Markdown"
fn url_stem(url: &str) -> String {
    let rest = url.split("://").nth(1).unwrap_or(url);
    let rest = rest.split(['?', '#']).next().unwrap_or(rest);
    let mut parts = rest.split('/').filter(|p| !p.is_empty());
    let host = parts.next().unwrap_or("page").trim_start_matches("www.").to_string();
    let last = parts.last().map(|seg| {
        let seg = percent_decode(seg);
        match seg.rsplit_once('.') {
            Some((stem, ext)) if ext.len() <= 5 && !stem.is_empty() => stem.to_string(),
            _ => seg,
        }
    });
    let name = match last {
        Some(l) => format!("{host} - {l}"),
        None => host,
    };
    let clean: String = name.chars().map(|c| if r#"\/:*?"<>|"#.contains(c) || c.is_control() { '_' } else { c }).collect();
    clean.chars().take(90).collect::<String>().trim().trim_end_matches('.').to_string()
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            let hex = |c: u8| (c as char).to_digit(16);
            if let (Some(h), Some(l)) = (hex(b[i + 1]), hex(b[i + 2])) {
                out.push((h * 16 + l) as u8);
                i += 3;
                continue;
            }
        }
        out.push(if b[i] == b'+' { b' ' } else { b[i] });
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn downloads_dir() -> PathBuf {
    let home = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME")).map(PathBuf::from).unwrap_or_default();
    home.join("Downloads")
}

#[derive(Clone, Copy, PartialEq)]
enum Task {
    Pip,
    Install,
    Uninstall,
    AppUpdate,
}

enum Detect {
    Searching,
    Found(Runner),
    Missing(String),
}

struct App {
    s: Settings,
    items: Vec<Item>,
    detect: Detect,
    tx: Sender<Msg>,
    rx: Receiver<Msg>,
    cancel: Arc<AtomicBool>,
    active: usize,
    show_options: bool,
    plugins_text: Option<String>,
    last_out_dir: Option<PathBuf>,
    latest: Option<Release>,
    update_status: String,
    checking: bool,
    checked_once: bool,
    task_running: bool,
    log: Vec<String>,
    show_log: bool,
    auto_setup_tried: bool,
    url_input: String,
    url_error: bool,
    task: Task,
    task_result: Option<(bool, String)>,
    app_latest: Option<Release>,
    app_status: String,
    installed_copy: bool,
    after_task: Option<AfterTask>,
    ctx: egui::Context,
}

enum AfterTask {
    Relaunch(PathBuf),
    Quit,
}

impl App {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        setup_style(&cc.egui_ctx);
        let s: Settings = cc
            .storage
            .and_then(|st| eframe::get_value(st, eframe::APP_KEY))
            .unwrap_or_default();
        let (tx, rx) = std::sync::mpsc::channel();
        let mut app = Self {
            s,
            items: Vec::new(),
            detect: Detect::Searching,
            tx,
            rx,
            cancel: Arc::new(AtomicBool::new(false)),
            active: 0,
            show_options: false,
            plugins_text: None,
            last_out_dir: None,
            latest: None,
            update_status: String::new(),
            checking: false,
            checked_once: false,
            task_running: false,
            log: Vec::new(),
            show_log: false,
            auto_setup_tried: false,
            url_input: String::new(),
            url_error: false,
            task: Task::Pip,
            task_result: None,
            app_latest: None,
            app_status: String::new(),
            installed_copy: install::is_installed_copy(),
            after_task: None,
            ctx: cc.egui_ctx.clone(),
        };
        // Files passed on the command line (e.g. "Open with" or drag onto the .exe).
        for a in std::env::args_os().skip(1) {
            if !a.to_string_lossy().starts_with("--") {
                app.add_path(PathBuf::from(a));
            }
        }
        app.redetect(&cc.egui_ctx);
        app
    }

    fn redetect(&mut self, ctx: &egui::Context) {
        self.detect = Detect::Searching;
        let tx = self.tx.clone();
        let ov = self.s.command_override.clone();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(Msg::Detected(Runner::detect(&ov)));
            ctx.request_repaint();
        });
    }

    fn runner(&self) -> Option<Runner> {
        match &self.detect {
            Detect::Found(r) => Some(r.clone()),
            _ => None,
        }
    }

    fn check_updates(&mut self, ctx: &egui::Context) {
        if self.checking {
            return;
        }
        self.checking = true;
        self.checked_once = true;
        self.update_status = "Latest release: checking...".into();
        let (runner, tx, ctx) = (self.runner(), self.tx.clone(), ctx.clone());
        std::thread::spawn(move || {
            let py = update::find_python(runner.as_ref());
            let _ = tx.send(Msg::Latest(update::latest_release(py.as_deref())));
            ctx.request_repaint();
            let _ = tx.send(Msg::AppLatest(update::latest_app_release(py.as_deref())));
            ctx.request_repaint();
        });
    }

    fn run_pip(&mut self, ctx: &egui::Context) {
        if self.task_running || self.busy() {
            return;
        }
        self.task = Task::Pip;
        self.task_result = None;
        self.task_running = true;
        self.show_log = true;
        self.log.clear();
        update::pip_install(self.runner(), self.tx.clone(), ctx.clone());
    }

    /// Runs install / uninstall / app update in the background with the log window.
    fn run_task(&mut self, task: Task) {
        if self.task_running || self.busy() {
            return;
        }
        self.task = task;
        self.task_result = None;
        self.task_running = true;
        self.show_log = true;
        self.log.clear();
        let (tx, ctx) = (self.tx.clone(), self.ctx.clone());
        let desktop = self.s.desktop_shortcut;
        let version = self.app_latest.as_ref().map(|r| r.version.clone()).unwrap_or_default();
        std::thread::spawn(move || {
            let log = |s: String| {
                let _ = tx.send(Msg::Log(s));
                ctx.request_repaint();
            };
            let msg = match task {
                Task::Install => match install::install(desktop, &log) {
                    Ok(p) => Msg::TaskDone(true, "Installed for this Windows user.".into(), Some(p)),
                    Err(e) => Msg::TaskDone(false, e, None),
                },
                Task::Uninstall => {
                    log("Removing shortcuts and the uninstall entry...".into());
                    match install::uninstall() {
                        Ok(()) => Msg::TaskDone(true, "Uninstalled. The app will now close.".into(), None),
                        Err(e) => Msg::TaskDone(false, e, None),
                    }
                }
                Task::AppUpdate => match update::update_app(&version, &log) {
                    Ok(p) => Msg::TaskDone(true, "Updated. Restarting...".into(), Some(p)),
                    Err(e) => Msg::TaskDone(false, e, None),
                },
                Task::Pip => return,
            };
            let _ = tx.send(msg);
            ctx.request_repaint();
        });
    }

    fn app_update_available(&self) -> Option<&Release> {
        self.app_latest.as_ref().filter(|l| update::is_newer(&l.version, update::APP_VERSION))
    }

    fn update_available(&self) -> Option<&Release> {
        match (&self.detect, &self.latest) {
            (Detect::Found(r), Some(l)) if update::is_newer(&l.version, &r.version) => Some(l),
            _ => None,
        }
    }

    fn refresh_update_status(&mut self) {
        let Some(l) = &self.latest else { return };
        self.update_status = match &self.detect {
            Detect::Found(r) if update::is_newer(&l.version, &r.version) => format!("Latest release: {} (update available)", l.version),
            Detect::Found(_) => format!("Latest release: {} (up to date)", l.version),
            _ => format!("Latest release: {}", l.version),
        };
    }

    fn current_release(&self) -> String {
        match &self.detect {
            Detect::Found(r) => r.version.trim_start_matches("markitdown").trim().to_string(),
            Detect::Searching => "checking...".into(),
            Detect::Missing(_) => "not installed".into(),
        }
    }

    fn add_path(&mut self, p: PathBuf) {
        if p.is_dir() {
            if let Ok(rd) = std::fs::read_dir(&p) {
                let mut v: Vec<PathBuf> = rd.flatten().map(|e| e.path()).filter(|p| p.is_file()).collect();
                v.sort();
                for f in v {
                    self.add_file(f);
                }
            }
        } else if p.is_file() {
            self.add_file(p);
        }
    }

    fn add_file(&mut self, p: PathBuf) {
        if !self.items.iter().any(|i| i.path == p) {
            self.items.push(Item { path: p, url: None, status: Status::Pending });
        }
    }

    /// Adds the URL typed in the URL field. Returns false if it isn't a valid web address.
    fn add_url_input(&mut self) -> bool {
        let Some(u) = normalize_url(&self.url_input) else {
            self.url_error = !self.url_input.trim().is_empty();
            return false;
        };
        if !self.items.iter().any(|i| i.url.as_deref() == Some(u.as_str())) {
            self.items.push(Item { path: PathBuf::new(), url: Some(u), status: Status::Pending });
        }
        self.url_input.clear();
        self.url_error = false;
        true
    }

    fn url_field(&mut self, ui: &mut egui::Ui, width: f32) {
        let busy = self.busy();
        ui.add_enabled_ui(!busy, |ui| {
            ui.horizontal(|ui| {
                let r = ui.add(egui::TextEdit::singleline(&mut self.url_input).hint_text("Paste a web page URL, e.g. https://example.com/article").desired_width(width));
                if r.changed() {
                    self.url_error = false;
                }
                let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                if ui.button("Add URL").clicked() || enter {
                    self.add_url_input();
                }
            });
        });
        let target = match (&self.s.custom_output, &self.s.output_dir) {
            (true, Some(d)) => d.display().to_string(),
            _ => downloads_dir().display().to_string(),
        };
        if self.url_error {
            ui.label(RichText::new("That doesn't look like a web address.").small().color(ERR));
        } else {
            ui.label(RichText::new(format!("Web pages are saved to {}", elide(&target, 60))).small().weak()).on_hover_text(target);
        }
    }

    fn busy(&self) -> bool {
        self.active > 0
    }

    fn output_for(&self, item: &Item, reserved: &mut HashSet<PathBuf>) -> PathBuf {
        let input = item.path.as_path();
        let dir = match (&self.s.custom_output, &self.s.output_dir, &item.url) {
            (true, Some(d), _) => d.clone(),
            (_, _, Some(_)) => downloads_dir(),
            _ => input.parent().map(Path::to_path_buf).unwrap_or_default(),
        };
        let stem = match &item.url {
            Some(u) => url_stem(u),
            None => input.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "output".into()),
        };
        let stem = if stem.is_empty() { "page".to_string() } else { stem };
        let mut out = dir.join(format!("{stem}.md"));
        let mut n = 1;
        while out == input || reserved.contains(&out) || (!self.s.overwrite && out.exists()) {
            out = dir.join(format!("{stem} ({n}).md"));
            n += 1;
        }
        reserved.insert(out.clone());
        out
    }

    fn start(&mut self, ctx: &egui::Context) {
        let Detect::Found(runner) = &self.detect else { return };
        let runner = runner.clone();
        if self.s.custom_output && self.s.output_dir.is_none() {
            self.pick_output_dir();
            if self.s.output_dir.is_none() {
                return;
            }
        }
        let args = self.option_args();
        let mut reserved = HashSet::new();
        let mut jobs = Vec::new();
        for i in 0..self.items.len() {
            if matches!(self.items[i].status, Status::Done(_) | Status::Running | Status::Queued) {
                continue;
            }
            let output = self.output_for(&self.items[i], &mut reserved);
            if let Some(d) = output.parent() {
                let _ = std::fs::create_dir_all(d);
            }
            self.items[i].status = Status::Queued;
            let input = match &self.items[i].url {
                Some(u) => std::ffi::OsString::from(u),
                None => self.items[i].path.clone().into_os_string(),
            };
            let is_url = self.items[i].url.is_some();
            jobs.push(Job { index: i, input, output, args: args.clone(), is_url });
        }
        if jobs.is_empty() {
            return;
        }
        self.last_out_dir = jobs[0].output.parent().map(Path::to_path_buf);
        self.cancel = Arc::new(AtomicBool::new(false));
        let env = if self.s.azure_key.trim().is_empty() { vec![] } else { vec![("AZURE_API_KEY".to_string(), self.s.azure_key.trim().to_string())] };
        self.active = runner::spawn_workers(runner, jobs, env, self.s.jobs.max(1), self.tx.clone(), self.cancel.clone(), ctx.clone());
    }

    fn option_args(&self) -> Vec<String> {
        let s = &self.s;
        let mut a = Vec::new();
        let mut opt = |flag: &str, v: &str| {
            if !v.trim().is_empty() {
                a.push(flag.to_string());
                a.push(v.trim().to_string());
            }
        };
        opt("-x", &s.ext_hint);
        opt("-m", &s.mime_hint);
        opt("-c", &s.charset_hint);
        if s.use_docintel {
            opt("-e", &s.docintel_endpoint);
        }
        if s.use_cu {
            opt("--cu-endpoint", &s.cu_endpoint);
            opt("--cu-analyzer", &s.cu_analyzer);
            opt("--cu-file-types", &s.cu_file_types);
        }
        if s.use_docintel {
            a.push("-d".into());
        }
        if s.use_cu {
            a.push("--use-cu".into());
        }
        if s.use_plugins {
            a.push("-p".into());
        }
        if s.keep_data_uris {
            a.push("--keep-data-uris".into());
        }
        a
    }

    fn pick_files(&mut self) {
        if let Some(files) = rfd::FileDialog::new().set_title("Add files").pick_files() {
            for f in files {
                self.add_file(f);
            }
        }
    }

    fn pick_output_dir(&mut self) {
        let mut d = rfd::FileDialog::new().set_title("Output folder");
        if let Some(cur) = &self.s.output_dir {
            d = d.set_directory(cur);
        }
        if let Some(dir) = d.pick_folder() {
            self.s.output_dir = Some(dir);
            self.s.custom_output = true;
        }
    }

    fn poll(&mut self) {
        while let Ok(m) = self.rx.try_recv() {
            match m {
                Msg::Detected(r) => {
                    self.detect = match r {
                        Ok(r) => Detect::Found(r),
                        Err(e) => Detect::Missing(e),
                    };
                    // First run (or broken install): set everything up automatically, once per session.
                    if matches!(self.detect, Detect::Missing(_)) && !self.auto_setup_tried {
                        self.auto_setup_tried = true;
                        self.run_pip(&self.ctx_clone());
                    }
                    self.refresh_update_status();
                    if self.s.check_on_startup && !self.checked_once {
                        self.check_updates(&self.ctx_clone());
                    }
                }
                Msg::Latest(r) => {
                    self.checking = false;
                    match r {
                        Ok(rel) => {
                            self.latest = Some(rel);
                            self.refresh_update_status();
                        }
                        Err(e) => self.update_status = format!("Latest release: unavailable ({e})"),
                    }
                }
                Msg::Log(l) => self.log.push(l),
                Msg::AppLatest(r) => {
                    self.app_status = match &r {
                        Ok(l) if update::is_newer(&l.version, update::APP_VERSION) => format!("Latest version: {} (update available)", l.version),
                        Ok(l) => format!("Latest version: {} (up to date)", l.version),
                        Err(e) => format!("Latest version: unavailable ({e})"),
                    };
                    self.app_latest = r.ok();
                }
                Msg::TaskDone(ok, text, path) => {
                    self.task_running = false;
                    self.task_result = Some((ok, text));
                    if ok {
                        match (self.task, path) {
                            (Task::AppUpdate, Some(p)) => self.after_task = Some(AfterTask::Relaunch(p)),
                            (Task::Uninstall, _) => self.after_task = Some(AfterTask::Quit),
                            _ => {}
                        }
                    }
                }
                Msg::PipDone => {
                    self.task_running = false;
                    let ctx = self.ctx_clone();
                    self.redetect(&ctx);
                }
                Msg::Started(i) => self.set(i, Status::Running),
                Msg::Finished(i, r) => self.set(
                    i,
                    match r {
                        Ok(p) => Status::Done(p),
                        Err(e) if e == runner::CANCELLED => Status::Cancelled,
                        Err(e) => Status::Failed(e),
                    },
                ),
                Msg::WorkerDone => {
                    self.active = self.active.saturating_sub(1);
                    if self.active == 0 {
                        for it in &mut self.items {
                            if it.status == Status::Queued {
                                it.status = Status::Cancelled;
                            }
                        }
                        if self.s.open_folder_when_done && !self.cancel.load(Ordering::Relaxed) {
                            if let Some(d) = &self.last_out_dir {
                                let _ = open::that_detached(d);
                            }
                        }
                    }
                }
                Msg::Plugins(t) => self.plugins_text = Some(t),
            }
        }
    }

    fn ctx_clone(&self) -> egui::Context {
        self.ctx.clone()
    }

    fn set(&mut self, i: usize, st: Status) {
        if let Some(it) = self.items.get_mut(i) {
            it.status = st;
        }
    }

    fn counts(&self) -> (usize, usize, usize) {
        let mut d = 0;
        let mut f = 0;
        for it in &self.items {
            match it.status {
                Status::Done(_) => d += 1,
                Status::Failed(_) => f += 1,
                _ => {}
            }
        }
        (d, f, self.items.len())
    }
}

impl eframe::App for App {
    fn ui(&mut self, root: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = &root.ctx().clone();
        self.poll();
        if let Some(a) = self.after_task.take() {
            if let AfterTask::Relaunch(exe) = a {
                let _ = std::process::Command::new(exe).spawn();
            }
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }

        // Drag and drop
        let dropped: Vec<PathBuf> = ctx.input(|i| i.raw.dropped_files.iter().map(|f| f.path().to_path_buf()).collect());
        if !self.busy() {
            for p in dropped {
                self.add_path(p);
            }
        }
        let hovering = ctx.input(|i| !i.raw.hovered_files.is_empty());

        self.header(root);
        if self.show_options {
            self.options_panel(root);
        }
        self.footer(root);

        egui::CentralPanel::default().show(root, |ui| {
            if self.items.is_empty() {
                self.drop_zone(ui, hovering);
            } else {
                self.file_list(ui, hovering);
            }
        });

        if let Some(t) = self.plugins_text.clone() {
            let mut open = true;
            egui::Window::new("Installed plugins").open(&mut open).collapsible(false).resizable(true).default_width(420.0).show(ctx, |ui| {
                egui::ScrollArea::vertical().max_height(300.0).show(ui, |ui| {
                    ui.label(RichText::new(t).monospace());
                });
            });
            if !open {
                self.plugins_text = None;
            }
        }

        if self.show_log {
            let mut open = true;
            let running = self.task_running;
            egui::Window::new(match self.task {
                Task::Pip if matches!(self.detect, Detect::Found(_)) => "Update markitdown",
                Task::Pip => "Setting up MarkItDown",
                Task::Install => "Install MarkItDown GUI",
                Task::Uninstall => "Uninstall MarkItDown GUI",
                Task::AppUpdate => "Update MarkItDown GUI",
            }).id(egui::Id::unique("setup_log")).open(&mut open).collapsible(false).resizable(true).default_width(560.0).show(ctx, |ui| {
                egui::ScrollArea::vertical().max_height(320.0).stick_to_bottom(true).auto_shrink([false, true]).show(ui, |ui| {
                    for l in &self.log {
                        ui.label(RichText::new(l).monospace().small());
                    }
                });
                ui.separator();
                ui.horizontal(|ui| {
                    if running {
                        ui.spinner();
                        ui.label("Working...");
                    } else if self.task == Task::Pip {
                        match &self.detect {
                            Detect::Found(r) => ui.label(RichText::new(format!("Installed: {}", r.version)).color(OK)),
                            Detect::Searching => ui.spinner(),
                            Detect::Missing(_) => ui.label(RichText::new("markitdown is still not available").color(ERR)),
                        };
                        if ui.button("Close").clicked() {
                            self.show_log = false;
                        }
                    } else {
                        if let Some((ok, t)) = &self.task_result {
                            ui.label(RichText::new(t).color(if *ok { OK } else { ERR }));
                        }
                        if self.task == Task::Install && matches!(self.task_result, Some((true, _))) {
                            if let Some(exe) = install::installed_exe() {
                                if ui.add(egui::Button::new(RichText::new("Open installed app").color(Color32::WHITE)).fill(ACCENT)).on_hover_text("Starts the installed copy and closes this one").clicked() {
                                    self.after_task = Some(AfterTask::Relaunch(exe));
                                }
                            }
                        }
                        if ui.button("Close").clicked() {
                            self.show_log = false;
                        }
                    }
                });
            });
            if !open && !running {
                self.show_log = false;
            }
        }
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        eframe::set_value(storage, eframe::APP_KEY, &self.s);
    }
}

impl App {
    fn header(&mut self, root: &mut egui::Ui) {
        let ctx = &root.ctx().clone();
        egui::Panel::top("top").frame(bar_frame(ctx)).show(root, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new("MarkItDown").size(18.0).strong());
                match &self.detect {
                    Detect::Searching => {
                        ui.spinner();
                    }
                    Detect::Found(r) => {
                        ui.label(RichText::new(&r.version).weak()).on_hover_text(r.describe());
                    }
                    Detect::Missing(_) => {}
                }
                if let Some(l) = self.update_available().cloned() {
                    let b = egui::Button::new(RichText::new(format!("Update to {}", l.version)).color(Color32::WHITE).small()).fill(ACCENT).corner_radius(10);
                    if ui.add_enabled(!self.busy() && !self.task_running, b).on_hover_text("Upgrade markitdown with pip").clicked() {
                        self.run_pip(ctx);
                    }
                }
                if cfg!(windows) {
                    if let Some(l) = self.app_update_available().cloned() {
                        let b = egui::Button::new(RichText::new(format!("App {} available", l.version)).color(Color32::WHITE).small()).fill(OK).corner_radius(10);
                        if ui.add_enabled(!self.busy() && !self.task_running, b).on_hover_text("Download and install the new MarkItDown GUI").clicked() {
                            self.run_task(Task::AppUpdate);
                        }
                    }
                }
                match &self.detect {
                    Detect::Searching | Detect::Found(_) => {}
                    Detect::Missing(_) => {
                        ui.label(RichText::new("markitdown not found").color(ERR));
                    }
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.selectable_label(self.show_options, "Options").clicked() {
                        self.show_options = !self.show_options;
                    }
                });
            });
            if let Detect::Missing(msg) = &self.detect {
                ui.add_space(4.0);
                ui.label(RichText::new(msg).small().weak());
                ui.horizontal(|ui| {
                    let b = egui::Button::new(RichText::new("Install markitdown").color(Color32::WHITE)).fill(ACCENT);
                    if ui.add_enabled(!self.task_running, b).on_hover_text("Runs: python -m pip install --upgrade markitdown[all]").clicked() {
                        self.run_pip(ctx);
                    }
                    if ui.button("Detect again").clicked() {
                        self.redetect(ctx);
                    }
                    ui.hyperlink_to("Get Python", update::PYTHON_DOWNLOAD);
                });
            }
        });
    }

    fn footer(&mut self, root: &mut egui::Ui) {
        let ctx = &root.ctx().clone();
        egui::Panel::bottom("bottom").frame(bar_frame(ctx)).show(root, |ui| {
            ui.add_enabled_ui(!self.busy(), |ui| {
                ui.horizontal(|ui| {
                    ui.label("Save to");
                    ui.radio_value(&mut self.s.custom_output, false, "Same folder as input");
                    ui.radio_value(&mut self.s.custom_output, true, "Folder:");
                    let label = self.s.output_dir.as_ref().map(|p| p.display().to_string()).unwrap_or_else(|| "Choose...".into());
                    let r = ui.add(egui::Button::new(elide(&label, 48)).min_size(egui::vec2(120.0, 0.0)));
                    if r.clicked() {
                        self.pick_output_dir();
                    }
                    r.on_hover_text(label);
                });
            });
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.add_enabled_ui(!self.busy(), |ui| {
                    if ui.button("Add files").clicked() {
                        self.pick_files();
                    }
                    if ui.button("Add folder").clicked() {
                        if let Some(d) = rfd::FileDialog::new().pick_folder() {
                            self.add_path(d);
                        }
                    }
                    if ui.add_enabled(!self.items.is_empty(), egui::Button::new("Clear")).clicked() {
                        self.items.clear();
                    }
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if self.busy() {
                        if ui.add(egui::Button::new(RichText::new("Stop").color(Color32::WHITE)).fill(ERR).min_size(egui::vec2(96.0, 30.0))).clicked() {
                            self.cancel.store(true, Ordering::Relaxed);
                        }
                        ui.spinner();
                    } else {
                        let pending = self.items.iter().any(|i| !matches!(i.status, Status::Done(_)));
                        let ready = pending && !self.task_running && matches!(self.detect, Detect::Found(_));
                        let btn = egui::Button::new(RichText::new("Convert").color(Color32::WHITE).strong()).fill(if ready { ACCENT } else { Color32::GRAY }).min_size(egui::vec2(96.0, 30.0));
                        if ui.add_enabled(ready, btn).clicked() {
                            self.start(ctx);
                        }
                    }
                    let (d, f, t) = self.counts();
                    if t > 0 {
                        let mut s = format!("{d}/{t} done");
                        if f > 0 {
                            s += &format!(", {f} failed");
                        }
                        ui.label(RichText::new(s).weak());
                    }
                });
            });
        });
    }

    fn drop_zone(&mut self, ui: &mut egui::Ui, hovering: bool) {
        let rect = ui.available_rect_before_wrap().shrink(8.0);
        let resp = ui.allocate_rect(rect, egui::Sense::click());
        let stroke_col = if hovering || resp.hovered() { ACCENT } else { ui.visuals().weak_text_color() };
        ui.painter().rect(rect, CornerRadius::same(14), if hovering { ACCENT.gamma_multiply(0.08) } else { Color32::TRANSPARENT }, Stroke::new(1.5, stroke_col), egui::StrokeKind::Inside);
        if resp.clicked() && !self.busy() {
            self.pick_files();
        }
        if resp.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }

        // Content is laid out on top of the clickable area; the URL field takes its own clicks.
        const CONTENT_H: f32 = 196.0;
        let inner = egui::Rect::from_center_size(rect.center(), egui::vec2((rect.width() - 40.0).min(520.0), CONTENT_H));
        ui.scope_builder(egui::UiBuilder::new().max_rect(inner).layout(egui::Layout::top_down(egui::Align::Center)), |ui| {
            ui.label(RichText::new("Drop files or folders here").size(18.0).color(ui.visuals().strong_text_color()));
            ui.label(RichText::new("or click to browse").size(13.0).weak());
            ui.label(RichText::new("PDF · Word · Excel · PowerPoint · HTML · images · audio · ZIP").size(12.0).weak());
            ui.add_space(16.0);
            // "OR" divider
            let (r, _) = ui.allocate_exact_size(egui::vec2(inner.width(), 18.0), egui::Sense::hover());
            let col = ui.visuals().weak_text_color().gamma_multiply(0.6);
            let y = r.center().y;
            ui.painter().line_segment([egui::pos2(r.left(), y), egui::pos2(r.center().x - 22.0, y)], Stroke::new(1.0, col));
            ui.painter().line_segment([egui::pos2(r.center().x + 22.0, y), egui::pos2(r.right(), y)], Stroke::new(1.0, col));
            ui.painter().text(r.center(), egui::Align2::CENTER_CENTER, "OR", egui::FontId::proportional(12.0), ui.visuals().weak_text_color());
            ui.add_space(14.0);
            let w = inner.width() - 90.0;
            self.url_field(ui, w);
        });
    }

    fn file_list(&mut self, ui: &mut egui::Ui, hovering: bool) {
        if hovering {
            ui.painter().rect_stroke(ui.max_rect(), CornerRadius::same(10), Stroke::new(1.5, ACCENT), egui::StrokeKind::Inside);
        }
        let busy = self.busy();
        let mut remove = None;
        let mut retry = None;
        ui.add_space(4.0);
        let w = (ui.available_width() - 90.0).max(200.0);
        self.url_field(ui, w);
        ui.add_space(4.0);
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            ui.add_space(4.0);
            for (i, it) in self.items.iter().enumerate() {
                egui::Frame::new().inner_margin(egui::Margin::symmetric(10, 7)).corner_radius(8).fill(ui.visuals().faint_bg_color).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        let name = it.title();
                        let dir = it.subtitle();
                        ui.vertical(|ui| {
                            ui.label(RichText::new(elide(&name, 64)).strong()).on_hover_text(&name);
                            ui.label(RichText::new(elide(&dir, 70)).small().weak()).on_hover_text(&dir);
                        });
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if !busy && ui.small_button("x").on_hover_text("Remove").clicked() {
                                remove = Some(i);
                            }
                            match &it.status {
                                Status::Pending => {
                                    ui.label(RichText::new("Ready").weak());
                                }
                                Status::Queued => {
                                    ui.label(RichText::new("Queued").weak());
                                }
                                Status::Running => {
                                    ui.spinner();
                                }
                                Status::Cancelled => {
                                    ui.label(RichText::new("Stopped").weak());
                                }
                                Status::Done(out) => {
                                    if ui.small_button("Folder").on_hover_text("Show in folder").clicked() {
                                        reveal(out);
                                    }
                                    if ui.small_button("Open").on_hover_text(out.display().to_string()).clicked() {
                                        let _ = open::that_detached(out);
                                    }
                                    ui.label(RichText::new("Done").color(OK));
                                }
                                Status::Failed(e) => {
                                    if !busy && ui.small_button("Retry").clicked() {
                                        retry = Some(i);
                                    }
                                    let short = e.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("Failed");
                                    ui.label(RichText::new(elide(short, 60)).color(ERR)).on_hover_ui(|ui| {
                                        ui.label(RichText::new(e).monospace().small());
                                    });
                                }
                            }
                        });
                    });
                });
                ui.add_space(4.0);
            }
        });
        if let Some(i) = remove {
            self.items.remove(i);
        }
        if let Some(i) = retry {
            self.items[i].status = Status::Pending;
        }
    }

    fn options_panel(&mut self, root: &mut egui::Ui) {
        let ctx = &root.ctx().clone();
        egui::Panel::right("opts").resizable(false).exact_size(300.0).frame(bar_frame(ctx)).show(root, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                ui.add_enabled_ui(!self.busy(), |ui| {
                    self.app_and_update_sections(ui);
                    section(ui, "Output");
                    ui.checkbox(&mut self.s.overwrite, "Overwrite existing .md files")
                        .on_hover_text("When off, a numbered name like \"file (1).md\" is used");
                    ui.checkbox(&mut self.s.keep_data_uris, "Keep embedded images (data URIs)")
                        .on_hover_text("--keep-data-uris: keep base64 images instead of truncating them");
                    ui.checkbox(&mut self.s.open_folder_when_done, "Open output folder when done");
                    ui.horizontal(|ui| {
                        ui.label("Parallel jobs");
                        ui.add(egui::Slider::new(&mut self.s.jobs, 1..=8));
                    });

                    section(ui, "Plugins");
                    ui.horizontal(|ui| {
                        ui.checkbox(&mut self.s.use_plugins, "Use 3rd-party plugins").on_hover_text("-p / --use-plugins");
                        if let Detect::Found(r) = &self.detect {
                            if ui.small_button("List").clicked() {
                                let (r, tx, ctx) = (r.clone(), self.tx.clone(), ui.ctx().clone());
                                std::thread::spawn(move || {
                                    let _ = tx.send(Msg::Plugins(r.list_plugins()));
                                    ctx.request_repaint();
                                });
                            }
                        }
                    });

                    section(ui, "Input hints");
                    field(ui, "Extension", &mut self.s.ext_hint, "e.g. pdf  (-x)");
                    field(ui, "MIME type", &mut self.s.mime_hint, "e.g. application/pdf  (-m)");
                    field(ui, "Charset", &mut self.s.charset_hint, "e.g. UTF-8  (-c)");

                    section(ui, "Azure Document Intelligence");
                    ui.checkbox(&mut self.s.use_docintel, "Use Document Intelligence (-d)");
                    if self.s.use_docintel {
                        field(ui, "Endpoint", &mut self.s.docintel_endpoint, "https://<name>.cognitiveservices.azure.com/");
                    }

                    section(ui, "Azure Content Understanding");
                    ui.checkbox(&mut self.s.use_cu, "Use Content Understanding (--use-cu)");
                    if self.s.use_cu {
                        field(ui, "Endpoint", &mut self.s.cu_endpoint, "--cu-endpoint");
                        field(ui, "Analyzer", &mut self.s.cu_analyzer, "auto if empty");
                        field(ui, "File types", &mut self.s.cu_file_types, "e.g. pdf,jpeg,mp4");
                    }
                    if self.s.use_docintel || self.s.use_cu {
                        ui.label("Azure API key (optional, not saved)");
                        ui.add(egui::TextEdit::singleline(&mut self.s.azure_key).password(true).hint_text("uses Azure login if empty").desired_width(f32::INFINITY));
                    }

                    section(ui, "markitdown command");
                    let r = ui.add(egui::TextEdit::singleline(&mut self.s.command_override).hint_text("auto-detect").desired_width(f32::INFINITY));
                    r.on_hover_text("Path to markitdown.exe or python.exe. Leave empty to search PATH.");
                    if ui.button("Detect again").clicked() {
                        self.redetect(ui.ctx());
                    }
                });
            });
        });
    }
}

impl App {
    fn app_and_update_sections(&mut self, ui: &mut egui::Ui) {
        let win = cfg!(windows);
        section(ui, "App");
        let mode = if self.installed_copy { "Installed" } else { "Portable" };
        ui.label(RichText::new(format!("MarkItDown GUI {}  ·  {mode}", update::APP_VERSION)).small().weak());
        if self.installed_copy {
            if ui.add_enabled(win && !self.task_running, egui::Button::new("Uninstall")).on_hover_text("Removes the app, its shortcuts, settings and private runtime from this PC").clicked() {
                let yes = rfd::MessageDialog::new()
                    .set_title("Uninstall MarkItDown GUI")
                    .set_description("Remove MarkItDown GUI, its shortcuts, settings and its private MarkItDown runtime from this PC? Your converted files are kept.")
                    .set_buttons(rfd::MessageButtons::YesNo)
                    .set_level(rfd::MessageLevel::Warning)
                    .show()
                    == rfd::MessageDialogResult::Yes;
                if yes {
                    self.run_task(Task::Uninstall);
                }
            }
        } else {
            ui.checkbox(&mut self.s.desktop_shortcut, "Create a desktop shortcut");
            ui.horizontal(|ui| {
                let b = egui::Button::new(RichText::new("Install on this PC").color(Color32::WHITE)).fill(ACCENT);
                let hover = if win { "Copies the app to your user Programs folder, adds a Start menu shortcut and an entry in Windows Installed apps. No admin rights needed." } else { "Only available on Windows" };
                if ui.add_enabled(win && !self.task_running, b).on_hover_text(hover).clicked() {
                    self.run_task(Task::Install);
                }
                if let Some(exe) = install::installed_exe() {
                    if ui.button("Open installed").on_hover_text("An installed copy exists. Start it and close this one.").clicked() {
                        self.after_task = Some(AfterTask::Relaunch(exe));
                    }
                }
            });
            ui.label(RichText::new("Or keep using it portable, nothing else needed.").small().weak());
        }

        section(ui, "Updates");
        ui.checkbox(&mut self.s.check_on_startup, "Check for updates on startup");
        ui.horizontal(|ui| {
            if ui.add_enabled(!self.checking, egui::Button::new("Check for updates")).clicked() {
                self.check_updates(ui.ctx());
            }
            if self.checking {
                ui.spinner();
            }
        });
        ui.add_space(4.0);
        ui.label(RichText::new("MarkItDown GUI (this app)").small().strong());
        ui.label(RichText::new(format!("Current version: {}", update::APP_VERSION)).small().weak());
        if !self.app_status.is_empty() {
            ui.label(RichText::new(&self.app_status).small().weak());
        }
        ui.horizontal(|ui| {
            if self.app_update_available().is_some() && ui.add_enabled(win && !self.task_running, egui::Button::new("Update app")).clicked() {
                self.run_task(Task::AppUpdate);
            }
            if let Some(l) = &self.app_latest {
                if !l.url.is_empty() {
                    ui.hyperlink_to(RichText::new("Release notes").small(), &l.url);
                }
            }
        });
        ui.add_space(4.0);
        ui.label(RichText::new("MarkItDown (converter)").small().strong());
        ui.label(RichText::new(format!("Current release: {}", self.current_release())).small().weak());
        if !self.update_status.is_empty() {
            ui.label(RichText::new(&self.update_status).small().weak());
        }
        ui.horizontal(|ui| {
            let label = if matches!(self.detect, Detect::Found(_)) { "Upgrade now" } else { "Install" };
            if ui.add_enabled(!self.task_running, egui::Button::new(label)).on_hover_text("python -m pip install --upgrade markitdown[all]").clicked() {
                self.run_pip(ui.ctx());
            }
            if let Some(l) = &self.latest {
                if !l.url.is_empty() {
                    ui.hyperlink_to(RichText::new("Release notes").small(), &l.url);
                }
            }
        });
    }
}

fn section(ui: &mut egui::Ui, t: &str) {
    ui.add_space(10.0);
    ui.label(RichText::new(t).strong());
    ui.add_space(2.0);
}

fn field(ui: &mut egui::Ui, label: &str, v: &mut String, hint: &str) {
    ui.label(RichText::new(label).small().weak());
    ui.add(egui::TextEdit::singleline(v).hint_text(hint).desired_width(f32::INFINITY));
}

fn bar_frame(ctx: &egui::Context) -> egui::Frame {
    egui::Frame::new().inner_margin(egui::Margin::symmetric(14, 10)).fill(ctx.global_style().visuals.panel_fill)
}

fn elide(s: &str, max: usize) -> String {
    let n = s.chars().count();
    if n <= max {
        return s.to_string();
    }
    let keep = max.saturating_sub(3) / 2;
    let head: String = s.chars().take(keep).collect();
    let tail: String = s.chars().skip(n - keep).collect();
    format!("{head}...{tail}")
}

fn reveal(p: &Path) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let _ = std::process::Command::new("explorer").raw_arg(format!("/select,\"{}\"", p.display())).spawn();
    }
    #[cfg(not(windows))]
    if let Some(d) = p.parent() {
        let _ = open::that_detached(d);
    }
}

fn setup_style(ctx: &egui::Context) {
    // Use Segoe UI on Windows: cleaner look and covers Arabic and other scripts.
    let mut fonts = egui::FontDefinitions::default();
    for (name, path) in [("segoe", r"C:\Windows\Fonts\segoeui.ttf"), ("segoe_sym", r"C:\Windows\Fonts\seguisym.ttf")] {
        if let Ok(bytes) = std::fs::read(path) {
            fonts.font_data.insert(name.into(), Arc::new(egui::FontData::from_owned(bytes)));
            let fam = fonts.families.entry(egui::FontFamily::Proportional).or_default();
            if name == "segoe" {
                fam.insert(0, name.into());
            } else {
                fam.push(name.into());
            }
        }
    }
    ctx.set_fonts(fonts);
    ctx.all_styles_mut(|style| {
        style.spacing.item_spacing = egui::vec2(8.0, 6.0);
        style.spacing.button_padding = egui::vec2(10.0, 5.0);
        for w in [&mut style.visuals.widgets.inactive, &mut style.visuals.widgets.hovered, &mut style.visuals.widgets.active] {
            w.corner_radius = CornerRadius::same(6);
        }
        style.visuals.selection.bg_fill = ACCENT;
    });
}

/// Keep settings next to the .exe when that folder is writable (portable mode).
fn portable_settings_path() -> Option<PathBuf> {
    let dir = std::env::current_exe().ok()?.parent()?.to_path_buf();
    let file = dir.join("MarkItDown GUI.settings.ron");
    std::fs::OpenOptions::new().create(true).append(true).open(&file).ok()?;
    Some(file)
}

fn main() -> eframe::Result {
    if install::handle_cli() {
        return Ok(());
    }
    install::cleanup_old_exe();
    install::refresh_registered_version();
    let opts = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("MarkItDown")
            .with_inner_size([820.0, 560.0])
            .with_min_inner_size([560.0, 380.0])
            .with_drag_and_drop(true)
            .with_icon(Arc::new(eframe::icon_data::from_png_bytes(include_bytes!("../assets/icon.png")).unwrap_or_default())),
        persistence_path: portable_settings_path(),
        ..Default::default()
    };
    eframe::run_native("MarkItDown GUI", opts, Box::new(|cc| Ok(Box::new(App::new(cc)))))
}
