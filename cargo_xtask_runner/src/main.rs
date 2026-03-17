// xtask-runner — run `cargo xtask-runner` from inside any Rust project.
// Tab 1: xtask — discovers tasks via `cargo xtask --list` (target|task_id|description)
// Tab 2: cargo — standard cargo commands with auto-discovered workspace packages

#![windows_subsystem = "windows"]

use eframe::egui;
use std::{
    io::{BufRead, BufReader},
    path::PathBuf,
    process::{Child, Command, Stdio},
    sync::{Arc, Mutex},
    thread,
};

#[cfg(windows)]
fn no_window_command(program: &str) -> Command {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x08000000;
    let mut cmd = Command::new(program);
    cmd.creation_flags(CREATE_NO_WINDOW);
    cmd
}

#[cfg(not(windows))]
fn no_window_command(program: &str) -> Command {
    Command::new(program)
}

// ─── Theme ────────────────────────────────────────────────────────────────────

const BG_DARK: egui::Color32 = egui::Color32::from_rgb(15, 17, 21);
const BG_PANEL: egui::Color32 = egui::Color32::from_rgb(22, 25, 31);
const BG_TASK: egui::Color32 = egui::Color32::from_rgb(18, 21, 27);
const BG_CONSOLE: egui::Color32 = egui::Color32::from_rgb(10, 12, 15);
const ACCENT: egui::Color32 = egui::Color32::from_rgb(82, 196, 130);
const ACCENT_DIM: egui::Color32 = egui::Color32::from_rgb(45, 110, 72);
const TEXT_NORMAL: egui::Color32 = egui::Color32::from_rgb(210, 215, 220);
const TEXT_DIM: egui::Color32 = egui::Color32::from_rgb(120, 130, 145);

const COL_DEFAULT: egui::Color32 = egui::Color32::from_rgb(210, 215, 220);
const COL_ERROR: egui::Color32 = egui::Color32::from_rgb(230, 80, 70);
const COL_SUCCESS: egui::Color32 = egui::Color32::from_rgb(82, 196, 130);
const COL_SECTION: egui::Color32 = egui::Color32::from_rgb(82, 196, 130);
const COL_WARNING: egui::Color32 = egui::Color32::from_rgb(255, 180, 50);
const COL_DIM: egui::Color32 = egui::Color32::from_rgb(120, 130, 145);

fn setup_visuals(ctx: &egui::Context) {
    let mut v = egui::Visuals::dark();
    v.window_fill = BG_PANEL;
    v.panel_fill = BG_DARK;
    v.faint_bg_color = BG_PANEL;
    v.extreme_bg_color = BG_CONSOLE;
    v.override_text_color = Some(TEXT_NORMAL);
    v.widgets.noninteractive.bg_fill = BG_PANEL;
    v.widgets.inactive.bg_fill = BG_PANEL;
    v.widgets.hovered.bg_fill = egui::Color32::from_rgb(35, 40, 50);
    v.widgets.active.bg_fill = ACCENT_DIM;
    v.widgets.noninteractive.fg_stroke = egui::Stroke::new(1.0, TEXT_DIM);
    v.widgets.inactive.fg_stroke = egui::Stroke::new(1.0, TEXT_NORMAL);
    v.widgets.hovered.fg_stroke = egui::Stroke::new(1.0, ACCENT);
    v.widgets.active.fg_stroke = egui::Stroke::new(1.0, ACCENT);
    v.selection.bg_fill = ACCENT_DIM;
    v.selection.stroke = egui::Stroke::new(1.0, ACCENT);
    v.window_stroke = egui::Stroke::new(1.0, egui::Color32::from_rgb(45, 50, 60));
    ctx.set_visuals(v);
}

fn panel_frame() -> egui::Frame {
    egui::Frame::none()
        .inner_margin(egui::Margin::same(12.0))
        .rounding(egui::Rounding::same(6.0))
        .fill(BG_PANEL)
}

fn task_frame() -> egui::Frame {
    egui::Frame::none()
        .inner_margin(egui::Margin::symmetric(10.0, 7.0))
        .rounding(egui::Rounding::same(4.0))
        .fill(BG_TASK)
}

// ─── Active tab ───────────────────────────────────────────────────────────────

#[derive(PartialEq, Clone, Copy)]
enum ActiveTab {
    Xtask,
    Cargo,
}

// ─── Cargo commands ───────────────────────────────────────────────────────────

/// A hardcoded cargo command shown in the Cargo tab.
#[derive(Clone)]
struct CargoCmd {
    id: &'static str,              // short key used in sentinels
    label: &'static str,           // display name
    desc: &'static str,            // one-line description
    args: &'static [&'static str], // cargo args (before optional -p <pkg>)
    scope: CmdScope,
}

#[derive(Clone, PartialEq)]
enum CmdScope {
    /// Always workspace-wide; -p is never appended
    Workspace,
    /// Runs per-package when a package is selected, workspace-wide otherwise
    Package,
    /// Only shown / enabled when a specific package is selected
    PackageOnly,
}

/// The fixed list of cargo commands in logical order.
fn cargo_commands() -> Vec<CargoCmd> {
    vec![
        CargoCmd {
            id: "check",
            label: "check",
            desc: "Fast type-check without producing binaries",
            args: &["check"],
            scope: CmdScope::Package,
        },
        CargoCmd {
            id: "build",
            label: "build",
            desc: "Compile in debug mode",
            args: &["build"],
            scope: CmdScope::Package,
        },
        CargoCmd {
            id: "build-r",
            label: "build --release",
            desc: "Compile optimised release binary",
            args: &["build", "--release"],
            scope: CmdScope::Package,
        },
        CargoCmd {
            id: "test",
            label: "test",
            desc: "Run all tests",
            args: &["test"],
            scope: CmdScope::Package,
        },
        CargoCmd {
            id: "clippy",
            label: "clippy",
            desc: "Run Clippy lints",
            args: &["clippy", "--", "--D", "warnings"],
            scope: CmdScope::Package,
        },
        CargoCmd {
            id: "fmt",
            label: "fmt",
            desc: "Format all code (workspace-wide)",
            args: &["fmt", "--all"],
            scope: CmdScope::Workspace,
        },
        CargoCmd {
            id: "doc",
            label: "doc",
            desc: "Build documentation",
            args: &["doc"],
            scope: CmdScope::Package,
        },
        CargoCmd {
            id: "run",
            label: "run",
            desc: "Run the binary of the selected package",
            args: &["run"],
            scope: CmdScope::PackageOnly,
        },
        CargoCmd {
            id: "clean",
            label: "clean",
            desc: "Remove build artefacts (workspace-wide)",
            args: &["clean"],
            scope: CmdScope::Workspace,
        },
        CargoCmd {
            id: "update",
            label: "update",
            desc: "Update dependencies in Cargo.lock",
            args: &["update"],
            scope: CmdScope::Workspace,
        },
        CargoCmd {
            id: "publish",
            label: "publish",
            desc: "Publish package to crates.io",
            args: &["publish"],
            scope: CmdScope::PackageOnly,
        },
    ]
}

// ─── Task status ──────────────────────────────────────────────────────────────

#[derive(Clone, PartialEq)]
enum TaskStatus {
    Idle,
    Running,
    Done,
    Failed,
}

impl TaskStatus {
    fn label(&self) -> &str {
        match self {
            TaskStatus::Idle => "idle",
            TaskStatus::Running => "running",
            TaskStatus::Done => "done",
            TaskStatus::Failed => "failed",
        }
    }
    fn icon(&self) -> &str {
        "○"
    }
    fn color(&self) -> egui::Color32 {
        match self {
            TaskStatus::Idle => TEXT_DIM,
            TaskStatus::Running => COL_WARNING,
            TaskStatus::Done => ACCENT,
            TaskStatus::Failed => COL_ERROR,
        }
    }
}

// ─── Data model ───────────────────────────────────────────────────────────────

/// One entry as parsed from `cargo xtask --list`
#[derive(Clone)]
struct TaskEntry {
    target: String,
    id: String,
    desc: String,
}

/// Runtime state for one xtask row in the UI
struct TaskRow {
    id: String,
    desc: String,
    checked: bool,
    status: TaskStatus,
    is_global: bool,
}

impl TaskRow {
    fn from_entry(e: &TaskEntry) -> Self {
        Self {
            id: e.id.clone(),
            desc: e.desc.clone(),
            checked: true,
            status: TaskStatus::Idle,
            is_global: e.target == "workspace",
        }
    }
}

/// Runtime state for one cargo command row in the UI
struct CargoRow {
    cmd: CargoCmd,
    checked: bool,
    status: TaskStatus,
}

// ─── Shared state ─────────────────────────────────────────────────────────────

type Log = Arc<Mutex<Vec<String>>>;
type IsRunning = Arc<Mutex<bool>>;
type CurrentChild = Arc<Mutex<Option<Child>>>;

struct AppState {
    // ── xtask ──
    all_entries: Vec<TaskEntry>,
    target_names: Vec<String>,
    selected_target: usize,
    tasks: Vec<TaskRow>,
    checked_state: std::collections::HashMap<String, std::collections::HashMap<String, bool>>,
    load_error: Option<String>,

    // ── cargo tab ──
    packages: Vec<String>, // discovered from workspace (empty = single-crate project)
    selected_package: usize, // index into packages; 0 = "workspace / all"
    cargo_rows: Vec<CargoRow>,

    // ── shared ──
    log: Log,
    is_running: IsRunning,
    current_child: CurrentChild,
    project_root: PathBuf,
}

impl AppState {
    fn new(root_result: Result<PathBuf, String>) -> Self {
        let (project_root, initial_error) = match root_result {
            Ok(p) => (p, None),
            Err(e) => (
                std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
                Some(e),
            ),
        };

        let cargo_rows = cargo_commands()
            .into_iter()
            .map(|cmd| CargoRow {
                cmd,
                checked: true,
                status: TaskStatus::Idle,
            })
            .collect();

        let mut s = Self {
            all_entries: Vec::new(),
            target_names: Vec::new(),
            selected_target: 0,
            tasks: Vec::new(),
            checked_state: std::collections::HashMap::new(),
            load_error: initial_error,
            packages: Vec::new(),
            selected_package: 0,
            cargo_rows,
            log: Arc::new(Mutex::new(Vec::new())),
            is_running: Arc::new(Mutex::new(false)),
            current_child: Arc::new(Mutex::new(None)),
            project_root,
        };

        if s.load_error.is_none() {
            s.reload();
        }
        s.reload_packages();
        s
    }

    // ── xtask ─────────────────────────────────────────────────────────────────

    fn reload(&mut self) {
        self.all_entries.clear();
        self.target_names.clear();
        self.tasks.clear();
        self.load_error = None;

        let output = no_window_command("cargo")
            .args(["xtask", "--list"])
            .current_dir(&self.project_root)
            .output();

        match output {
            Err(e) => {
                self.load_error = Some(format!(
                    "Failed to run `cargo xtask --list`: {e}\n\
                     Make sure `cargo` is in PATH and this exe is next to Cargo.toml."
                ));
                return;
            }
            Ok(out) => {
                let stdout = String::from_utf8_lossy(&out.stdout);
                for line in stdout.lines() {
                    let parts: Vec<&str> = line.splitn(3, '|').collect();
                    if parts.len() == 3 {
                        let target = parts[0].trim().to_string();
                        let id = parts[1].trim().to_string();
                        let desc = parts[2].trim().to_string();
                        if !self.target_names.contains(&target) {
                            self.target_names.push(target.clone());
                        }
                        self.all_entries.push(TaskEntry { target, id, desc });
                    }
                }
            }
        }

        if self.target_names.is_empty() {
            self.load_error = Some(
                "`cargo xtask --list` returned no entries.\n\
                 Expected format: target|task_id|description"
                    .to_string(),
            );
            return;
        }

        self.selected_target = self.selected_target.min(self.target_names.len() - 1);
        self.rebuild_task_rows();
    }

    fn rebuild_task_rows(&mut self) {
        let target = match self.target_names.get(self.selected_target) {
            Some(t) => t.clone(),
            None => return,
        };
        self.tasks = self
            .all_entries
            .iter()
            .filter(|e| e.target == target)
            .map(|e| {
                let mut row = TaskRow::from_entry(e);
                if let Some(tm) = self.checked_state.get(&target) {
                    if let Some(&c) = tm.get(&row.id) {
                        row.checked = c;
                    }
                }
                row
            })
            .collect();
    }

    fn current_target(&self) -> Option<&str> {
        self.target_names
            .get(self.selected_target)
            .map(|s| s.as_str())
    }

    fn checked_ids(&self) -> Vec<String> {
        self.tasks
            .iter()
            .filter(|t| t.checked)
            .map(|t| t.id.clone())
            .collect()
    }

    fn any_checked(&self) -> bool {
        self.tasks.iter().any(|t| t.checked)
    }

    // ── cargo tab ─────────────────────────────────────────────────────────────

    /// Discover workspace members by parsing `cargo metadata --no-deps --format-version 1`.
    /// Falls back gracefully to an empty list (single-crate project).
    fn reload_packages(&mut self) {
        self.packages.clear();

        let cargo_toml_path = self.project_root.join("Cargo.toml");
        let content = match std::fs::read_to_string(&cargo_toml_path) {
            Ok(c) => c,
            Err(_) => return,
        };

        // Look for a [workspace] members = [...] array.
        // Falls back to empty (single-crate project — no package selector needed).
        if let Some(ws_start) = content.find("[workspace]") {
            let slice = &content[ws_start..];
            if let Some(members_start) = slice.find("members") {
                let slice = &slice[members_start..];
                if let Some(arr_start) = slice.find('[') {
                    if let Some(arr_end) = slice.find(']') {
                        let arr = &slice[arr_start + 1..arr_end];
                        for entry in arr.split(',') {
                            let name = entry.trim().trim_matches('"').trim_matches('\'').trim();
                            if name.is_empty() {
                                continue;
                            }
                            // The entry is a path like "my-crate" or "crates/my-crate".
                            // Read that crate's Cargo.toml to get the real package name.
                            let member_toml = self.project_root.join(name).join("Cargo.toml");
                            if let Ok(member_content) = std::fs::read_to_string(&member_toml) {
                                if let Some(name_line) = member_content
                                    .lines()
                                    .find(|l| l.trim_start().starts_with("name") && l.contains('='))
                                {
                                    if let Some(pkg_name) = name_line.splitn(2, '=').nth(1) {
                                        let pkg_name =
                                            pkg_name.trim().trim_matches('"').to_string();
                                        if !pkg_name.is_empty() {
                                            self.packages.push(pkg_name);
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    /// The package name to pass as `-p <pkg>`, or None for workspace-wide.
    fn current_package(&self) -> Option<&str> {
        // index 0 = "All / Workspace"
        if self.selected_package == 0 || self.packages.is_empty() {
            None
        } else {
            self.packages
                .get(self.selected_package - 1)
                .map(|s| s.as_str())
        }
    }

    fn checked_cargo_ids(&self) -> Vec<String> {
        self.cargo_rows
            .iter()
            .filter(|r| r.checked && self.cargo_cmd_enabled(&r.cmd))
            .map(|r| r.cmd.id.to_string())
            .collect()
    }

    fn any_cargo_checked(&self) -> bool {
        self.cargo_rows
            .iter()
            .any(|r| r.checked && self.cargo_cmd_enabled(&r.cmd))
    }

    /// A PackageOnly command is disabled when no specific package is selected.
    fn cargo_cmd_enabled(&self, cmd: &CargoCmd) -> bool {
        if cmd.scope == CmdScope::PackageOnly {
            self.current_package().is_some()
        } else {
            true
        }
    }

    // ── shared ────────────────────────────────────────────────────────────────

    fn currently_running(&self) -> bool {
        *self.is_running.lock().unwrap()
    }

    fn stop(&mut self) {
        if let Some(child) = self.current_child.lock().unwrap().as_ref() {
            let pid = child.id();
            #[cfg(windows)]
            {
                use std::os::windows::process::CommandExt;
                const CREATE_NO_WINDOW: u32 = 0x08000000;
                let _ = Command::new("taskkill")
                    .args(["/PID", &pid.to_string(), "/T", "/F"])
                    .creation_flags(CREATE_NO_WINDOW)
                    .spawn();
            }
            #[cfg(not(windows))]
            {
                let _ = Command::new("kill").args(["-9", &pid.to_string()]).spawn();
            }
        }
        *self.is_running.lock().unwrap() = false;
        self.log
            .lock()
            .unwrap()
            .push("■  Stopped by user.".to_string());
        for t in &mut self.tasks {
            if t.status == TaskStatus::Running {
                t.status = TaskStatus::Failed;
            }
        }
        for r in &mut self.cargo_rows {
            if r.status == TaskStatus::Running {
                r.status = TaskStatus::Failed;
            }
        }
    }
}

// ─── App ──────────────────────────────────────────────────────────────────────

struct XtaskRunner {
    state: AppState,
    active_tab: ActiveTab,
    header_image: egui::TextureHandle,
}

impl XtaskRunner {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        Self {
            state: AppState::new(find_project_root()),
            active_tab: ActiveTab::Xtask,
            header_image: load_header_image(&cc.egui_ctx),
        }
    }

    // ── run xtask pipeline ────────────────────────────────────────────────────

    fn run_tasks(&mut self, ids: Vec<String>) {
        if ids.is_empty() || self.state.currently_running() {
            return;
        }

        let target = match self.state.current_target() {
            Some(t) => t.to_string(),
            None => return,
        };

        for task in &mut self.state.tasks {
            task.status = TaskStatus::Idle;
        }
        self.state.log.lock().unwrap().clear();

        let log = Arc::clone(&self.state.log);
        let is_running = Arc::clone(&self.state.is_running);
        let current_child = Arc::clone(&self.state.current_child);
        let project_root = self.state.project_root.clone();

        let id_is_global: std::collections::HashMap<String, bool> = self
            .state
            .tasks
            .iter()
            .map(|t| (t.id.clone(), t.is_global))
            .collect();

        *is_running.lock().unwrap() = true;

        thread::spawn(move || {
            for id in &ids {
                if !*is_running.lock().unwrap() {
                    break;
                }

                let global = id_is_global.get(id.as_str()).copied().unwrap_or(false);
                let tgt_label = if global {
                    String::new()
                } else {
                    format!(" {target}")
                };

                log.lock().unwrap().push(format!("@@START:{id}"));
                log.lock().unwrap().push(format!(
                    "\n── cargo xtask {id}{tgt_label} {}\n",
                    "─".repeat(38usize.saturating_sub(id.len() + tgt_label.len()))
                ));

                let mut cmd_args = vec!["xtask", id.as_str()];
                if !global {
                    cmd_args.push(target.as_str());
                }

                let success = spawn_and_wait(
                    no_window_command("cargo")
                        .args(&cmd_args)
                        .current_dir(&project_root),
                    &log,
                    &is_running,
                    &current_child,
                );

                if !*is_running.lock().unwrap() {
                    break;
                }
                log.lock().unwrap().push(if success {
                    format!("@@DONE:{id}")
                } else {
                    format!("@@FAIL:{id}")
                });
                if !success {
                    log.lock()
                        .unwrap()
                        .push(format!("❌ Task `{id}` failed — pipeline stopped."));
                    break;
                }
            }
            *is_running.lock().unwrap() = false;
        });
    }

    // ── run cargo pipeline ────────────────────────────────────────────────────

    fn run_cargo_cmds(&mut self, ids: Vec<String>) {
        if ids.is_empty() || self.state.currently_running() {
            return;
        }

        let package = self.state.current_package().map(|s| s.to_string());
        let all_cmds = cargo_commands();

        // Build a lookup id -> CargoCmd
        let cmd_map: std::collections::HashMap<&str, &CargoCmd> =
            all_cmds.iter().map(|c| (c.id, c)).collect();

        for row in &mut self.state.cargo_rows {
            row.status = TaskStatus::Idle;
        }
        self.state.log.lock().unwrap().clear();

        let log = Arc::clone(&self.state.log);
        let is_running = Arc::clone(&self.state.is_running);
        let current_child = Arc::clone(&self.state.current_child);
        let project_root = self.state.project_root.clone();

        // Collect the full command specs we need to run
        let specs: Vec<(String, Vec<String>)> = ids
            .iter()
            .filter_map(|id| {
                let cmd = cmd_map.get(id.as_str())?;
                let mut args: Vec<String> = cmd.args.iter().map(|s| s.to_string()).collect();
                if cmd.scope != CmdScope::Workspace {
                    if let Some(ref pkg) = package {
                        args.push("-p".to_string());
                        args.push(pkg.clone());
                    }
                }
                Some((id.clone(), args))
            })
            .collect();

        *is_running.lock().unwrap() = true;

        thread::spawn(move || {
            for (id, args) in &specs {
                if !*is_running.lock().unwrap() {
                    break;
                }

                let display = format!("cargo {}", args.join(" "));
                log.lock().unwrap().push(format!("@@START:{id}"));
                log.lock().unwrap().push(format!(
                    "\n── {display} {}\n",
                    "─".repeat(50usize.saturating_sub(display.len()))
                ));

                let success = spawn_and_wait(
                    no_window_command("cargo")
                        .args(args.as_slice())
                        .current_dir(&project_root),
                    &log,
                    &is_running,
                    &current_child,
                );

                if !*is_running.lock().unwrap() {
                    break;
                }
                log.lock().unwrap().push(if success {
                    format!("@@DONE:{id}")
                } else {
                    format!("@@FAIL:{id}")
                });
                if !success {
                    log.lock()
                        .unwrap()
                        .push(format!("❌ `{display}` failed — pipeline stopped."));
                    break;
                }
            }
            *is_running.lock().unwrap() = false;
        });
    }
}

// ─── Shared spawn helper ──────────────────────────────────────────────────────

fn spawn_and_wait(
    cmd: &mut Command,
    log: &Log,
    is_running: &IsRunning,
    current_child: &CurrentChild,
) -> bool {
    let child = cmd.stdout(Stdio::piped()).stderr(Stdio::piped()).spawn();

    match child {
        Err(e) => {
            log.lock().unwrap().push(format!("❌ Failed to spawn: {e}"));
            false
        }
        Ok(mut child) => {
            if let Some(stdout) = child.stdout.take() {
                let log2 = Arc::clone(log);
                thread::spawn(move || {
                    for line in BufReader::new(stdout).lines().flatten() {
                        log2.lock().unwrap().push(line);
                    }
                });
            }
            if let Some(stderr) = child.stderr.take() {
                let log2 = Arc::clone(log);
                thread::spawn(move || {
                    for line in BufReader::new(stderr).lines().flatten() {
                        log2.lock().unwrap().push(format!("@@STDERR:{line}"));
                    }
                });
            }

            *current_child.lock().unwrap() = Some(child);

            let exit_status = loop {
                if !*is_running.lock().unwrap() {
                    break None;
                }
                {
                    let mut guard = current_child.lock().unwrap();
                    if let Some(c) = guard.as_mut() {
                        if let Ok(Some(status)) = c.try_wait() {
                            break Some(status);
                        }
                    } else {
                        break None;
                    }
                }
                std::thread::sleep(std::time::Duration::from_millis(50));
            };

            current_child.lock().unwrap().take();
            exit_status.map_or(false, |s| s.success())
        }
    }
}

// ─── Console coloring ─────────────────────────────────────────────────────────

fn line_color(line: &str) -> egui::Color32 {
    let s = line.strip_prefix("@@STDERR:").unwrap_or(line);
    if s.starts_with("error[") || s.starts_with("error: ") || line.starts_with("❌") {
        COL_ERROR
    } else if s.starts_with("warning[") || s.starts_with("warning: ") || line.starts_with("⚠") {
        COL_WARNING
    } else if s.starts_with("   Finished")
        || s.starts_with("    Finished")
        || line.starts_with("✅")
        || s.starts_with("test result: ok")
    {
        COL_SUCCESS
    } else if s.starts_with("   Compiling")
        || s.starts_with("    Compiling")
        || s.starts_with("   Downloading")
        || s.starts_with("    Downloading")
        || s.starts_with("   Updating")
        || s.starts_with("    Updating")
        || s.starts_with("   Running")
        || s.starts_with("    Running")
        || s.starts_with("   Fetching")
        || s.starts_with("    Fetching")
    {
        COL_DIM
    } else if line.starts_with("──") || line.starts_with("■") || line.starts_with("🚀") {
        COL_SECTION
    } else if s.starts_with("note:") || s.starts_with("help:") || s.starts_with("  -->") {
        COL_DIM
    } else {
        COL_DEFAULT
    }
}

fn display_line(line: &str) -> &str {
    line.strip_prefix("@@STDERR:").unwrap_or(line)
}

// ─── eframe::App ─────────────────────────────────────────────────────────────

impl eframe::App for XtaskRunner {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        setup_visuals(ctx);

        // ── Process sentinel log lines → update task statuses ──────────────
        {
            let mut log = self.state.log.lock().unwrap();
            let sentinels: Vec<String> = log
                .iter()
                .filter(|l| {
                    l.starts_with("@@START:")
                        || l.starts_with("@@DONE:")
                        || l.starts_with("@@FAIL:")
                })
                .cloned()
                .collect();

            for s in &sentinels {
                if let Some(id) = s.strip_prefix("@@START:") {
                    if let Some(t) = self.state.tasks.iter_mut().find(|t| t.id == id) {
                        t.status = TaskStatus::Running;
                    }
                    if let Some(r) = self.state.cargo_rows.iter_mut().find(|r| r.cmd.id == id) {
                        r.status = TaskStatus::Running;
                    }
                } else if let Some(id) = s.strip_prefix("@@DONE:") {
                    if let Some(t) = self.state.tasks.iter_mut().find(|t| t.id == id) {
                        t.status = TaskStatus::Done;
                    }
                    if let Some(r) = self.state.cargo_rows.iter_mut().find(|r| r.cmd.id == id) {
                        r.status = TaskStatus::Done;
                    }
                } else if let Some(id) = s.strip_prefix("@@FAIL:") {
                    if let Some(t) = self.state.tasks.iter_mut().find(|t| t.id == id) {
                        t.status = TaskStatus::Failed;
                    }
                    if let Some(r) = self.state.cargo_rows.iter_mut().find(|r| r.cmd.id == id) {
                        r.status = TaskStatus::Failed;
                    }
                }
            }
            log.retain(|l| {
                !l.starts_with("@@START:") && !l.starts_with("@@DONE:") && !l.starts_with("@@FAIL:")
            });
        }

        if self.state.currently_running() {
            ctx.request_repaint();
        }

        // ── Header ────────────────────────────────────────────────────────
        egui::TopBottomPanel::top("header")
            .frame(
                egui::Frame::none()
                    .fill(BG_DARK)
                    .inner_margin(egui::Margin::same(10.0)),
            )
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.add_space(4.0);
                    let size = egui::vec2(44.0, 44.0);
                    ui.add(egui::Image::new(&self.header_image).fit_to_exact_size(size));
                    ui.add_space(8.0);
                    ui.label(
                        egui::RichText::new("xtask runner")
                            .strong()
                            .size(40.0)
                            .color(ACCENT),
                    );
                    ui.add_space(12.0);
                    ui.vertical(|ui| {
                        ui.label(
                            egui::RichText::new(
                                "Run cargo xtask pipelines and standard cargo commands",
                            )
                            .size(11.0)
                            .color(TEXT_DIM),
                        );
                        ui.label(
                            egui::RichText::new(
                                "from a graphical interface with live console output.",
                            )
                            .size(11.0)
                            .color(TEXT_DIM),
                        );
                    });
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(
                            egui::RichText::new(format!(
                                "📁  {}",
                                self.state.project_root.display()
                            ))
                            .size(11.0)
                            .color(TEXT_DIM),
                        );
                    });
                });
            });

        // ── Deferred actions ──────────────────────────────────────────────
        let mut single_run: Option<String> = None;
        let mut do_run_selected = false;
        let mut do_stop = false;
        let mut new_target: Option<usize> = None;
        let mut new_package: Option<usize> = None;
        let mut single_cargo_run: Option<String> = None;
        let mut do_run_cargo = false;

        // ── Left panel ────────────────────────────────────────────────────
        egui::SidePanel::left("tasks_panel")
            .resizable(true)
            .default_width(360.0)
            .min_width(320.0)
            .frame(egui::Frame::none().fill(BG_DARK).inner_margin(egui::Margin::same(0.0)))
            .show(ctx, |ui| {
                ui.add_space(8.0);

                // ── Tab bar ───────────────────────────────────────────────
                panel_frame().show(ui, |ui| {
                    ui.horizontal(|ui| {
                        let tab_btn = |ui: &mut egui::Ui, label: &str, tab: ActiveTab, active: ActiveTab| -> bool {
                            let selected = tab == active;
                            let color = if selected { ACCENT } else { TEXT_DIM };
                            let fill  = if selected { ACCENT_DIM } else { egui::Color32::TRANSPARENT };
                            ui.add(
                                egui::Button::new(egui::RichText::new(label).color(color).size(13.0).strong())
                                    .fill(fill)
                                    .stroke(egui::Stroke::new(1.0, if selected { ACCENT } else { TEXT_DIM }))
                                    .min_size(egui::vec2(100.0, 28.0)),
                            ).clicked()
                        };

                        if tab_btn(ui, "⚙  xtask", ActiveTab::Xtask, self.active_tab) {
                            self.active_tab = ActiveTab::Xtask;
                        }
                        ui.add_space(6.0);
                        if tab_btn(ui, "📦  cargo", ActiveTab::Cargo, self.active_tab) {
                            self.active_tab = ActiveTab::Cargo;
                        }
                    });
                });

                ui.add_space(8.0);

                let running = self.state.currently_running();

                match self.active_tab {
                    // ══════════════════════════════════════════════════════
                    // XTASK TAB
                    // ══════════════════════════════════════════════════════
                    ActiveTab::Xtask => {
                        if let Some(err) = self.state.load_error.clone() {
                            ui.add_space(8.0);
                            egui::Frame::none()
                                .fill(egui::Color32::from_rgb(40, 18, 18))
                                .rounding(egui::Rounding::same(8.0))
                                .inner_margin(egui::Margin::same(16.0))
                                .stroke(egui::Stroke::new(1.0, COL_ERROR))
                                .show(ui, |ui| {
                                    ui.label(
                                        egui::RichText::new("\u{26a0}  xtask not available")
                                            .strong().size(14.0).color(COL_ERROR),
                                    );
                                    ui.add_space(8.0);
                                    for line in err.lines() {
                                        let color = if line.starts_with('\u{2022}') { ACCENT }
                                        else if line.trim().starts_with("cargo xtask") { TEXT_NORMAL }
                                        else { TEXT_DIM };
                                        ui.label(
                                            egui::RichText::new(line).size(12.0).color(color).monospace(),
                                        );
                                    }
                                    ui.add_space(12.0);
                                    ui.separator();
                                    ui.add_space(8.0);
                                    ui.horizontal(|ui| {
                                        if ui.add(
                                            egui::Button::new(
                                                egui::RichText::new("\u{21ba}  Retry")
                                                    .color(egui::Color32::BLACK).strong().size(12.0),
                                            )
                                                .fill(ACCENT)
                                                .min_size(egui::vec2(90.0, 26.0)),
                                        ).clicked() {
                                            self.state.load_error = None;
                                            self.state.reload();
                                        }
                                        ui.add_space(8.0);
                                        ui.label(
                                            egui::RichText::new("Switch to the Cargo tab to use standard commands.")
                                                .size(11.0).color(TEXT_DIM),
                                        );
                                    });
                                });
                            return;
                        }

                        // Target selector
                        panel_frame().show(ui, |ui| {
                            ui.horizontal(|ui| {
                                ui.label(egui::RichText::new("Target").color(TEXT_DIM).size(12.0));
                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                    let n   = self.state.target_names.len();
                                    let cur = self.state.selected_target;
                                    if ui.add(
                                        egui::Button::new(egui::RichText::new("▶").size(11.0).color(TEXT_DIM))
                                            .fill(egui::Color32::TRANSPARENT)
                                            .stroke(egui::Stroke::new(1.0, TEXT_DIM))
                                            .min_size(egui::vec2(22.0, 18.0)),
                                    ).clicked() && n > 0 {
                                        new_target = Some((cur + 1) % n);
                                    }
                                    ui.add_space(2.0);
                                    if ui.add(
                                        egui::Button::new(egui::RichText::new("◀").size(11.0).color(TEXT_DIM))
                                            .fill(egui::Color32::TRANSPARENT)
                                            .stroke(egui::Stroke::new(1.0, TEXT_DIM))
                                            .min_size(egui::vec2(22.0, 18.0)),
                                    ).clicked() && n > 0 {
                                        new_target = Some((cur + n - 1) % n);
                                    }
                                });
                            });
                            ui.add_space(6.0);

                            let current_name = self.state.target_names
                                .get(self.state.selected_target).cloned().unwrap_or_default();
                            let display_name = if current_name == "workspace" {
                                "Workspace".to_string()
                            } else {
                                current_name.clone()
                            };

                            egui::ComboBox::from_id_source("target_combo")
                                .width(ui.available_width() - 4.0)
                                .selected_text(egui::RichText::new(&display_name).color(TEXT_NORMAL).size(13.0))
                                .show_ui(ui, |ui| {
                                    for (i, name) in self.state.target_names.iter().enumerate() {
                                        let selected = i == self.state.selected_target;
                                        let (label_str, label_color) = if name == "workspace" {
                                            ("Workspace", if selected { ACCENT } else { COL_WARNING })
                                        } else {
                                            (name.as_str(), if selected { ACCENT } else { TEXT_NORMAL })
                                        };
                                        let label = egui::RichText::new(label_str).color(label_color).size(13.0);
                                        if ui.selectable_label(selected, label).clicked() {
                                            new_target = Some(i);
                                        }
                                    }
                                });
                        });

                        ui.add_space(8.0);

                        // Task list
                        panel_frame().show(ui, |ui| {
                            ui.horizontal(|ui| {
                                ui.label(egui::RichText::new("Tasks").color(TEXT_DIM).size(12.0));
                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                    if ui.add(egui::Button::new(egui::RichText::new("None").color(TEXT_DIM).size(11.0))
                                        .fill(egui::Color32::TRANSPARENT).stroke(egui::Stroke::NONE))
                                        .clicked() {
                                        for t in &mut self.state.tasks { t.checked = false; }
                                    }
                                    ui.label(egui::RichText::new("·").color(TEXT_DIM).size(11.0));
                                    if ui.add(egui::Button::new(egui::RichText::new("All").color(TEXT_DIM).size(11.0))
                                        .fill(egui::Color32::TRANSPARENT).stroke(egui::Stroke::NONE))
                                        .clicked() {
                                        for t in &mut self.state.tasks { t.checked = true; }
                                    }
                                });
                            });
                            ui.add_space(6.0);

                            egui::ScrollArea::vertical()
                                .id_source("task_scroll")
                                .max_height(ui.available_height() - 60.0)
                                .show(ui, |ui| {
                                    for task in &mut self.state.tasks {
                                        let sc   = task.status.color();
                                        let icon = task.status.icon();
                                        let mut run_btn_rect: Option<egui::Rect> = None;

                                        let card_resp = task_frame().show(ui, |ui| {
                                            ui.horizontal(|ui| {
                                                ui.vertical(|ui| {
                                                    ui.label(egui::RichText::new(icon).color(sc).size(13.0));
                                                    ui.add_enabled_ui(!running, |ui| {
                                                        ui.checkbox(&mut task.checked, "");
                                                    });
                                                });
                                                ui.with_layout(egui::Layout::top_down(egui::Align::LEFT).with_main_wrap(false), |ui| {
                                                    ui.set_max_width(ui.available_width() - 68.0);
                                                    ui.horizontal(|ui| {
                                                        ui.label(egui::RichText::new(&task.id).strong().size(13.0).color(TEXT_NORMAL));
                                                        ui.label(egui::RichText::new(task.status.label()).size(10.5).color(sc));
                                                    });
                                                    ui.add(egui::Label::new(egui::RichText::new(&task.desc).size(11.0).color(TEXT_DIM)).truncate(true));
                                                });
                                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                                    ui.add_enabled_ui(!running, |ui| {
                                                        let btn = egui::Button::new(egui::RichText::new("▶ Run").size(12.0).strong().color(egui::Color32::BLACK))
                                                            .fill(ACCENT).min_size(egui::vec2(60.0, 24.0));
                                                        let resp = ui.add(btn).on_hover_text(format!("Run `{}`", task.id));
                                                        run_btn_rect = Some(resp.rect);
                                                        if resp.clicked() { single_run = Some(task.id.clone()); }
                                                    });
                                                });
                                            });
                                        });

                                        if !running {
                                            let card_rect    = card_resp.response.rect;
                                            let run_btn_x    = run_btn_rect.map_or(card_rect.right(), |r| r.left());
                                            let checkbox_right = card_rect.min.x + 46.0;
                                            let safe_rect = egui::Rect::from_min_max(
                                                egui::pos2(checkbox_right, card_rect.min.y),
                                                egui::pos2(run_btn_x - 4.0, card_rect.max.y),
                                            );
                                            let click_resp = ui.interact(safe_rect, ui.id().with(&task.id), egui::Sense::click());
                                            if click_resp.clicked() { task.checked = !task.checked; }
                                        }
                                        ui.add_space(4.0);
                                    }
                                });

                            ui.add_space(4.0);
                            ui.separator();
                            ui.add_space(8.0);
                            ui.horizontal(|ui| {
                                let can_run = self.state.any_checked() && !running;
                                ui.add_enabled_ui(can_run, |ui| {
                                    if ui.add(egui::Button::new(
                                        egui::RichText::new("▶  Run selected").color(egui::Color32::BLACK).strong().size(13.0))
                                        .fill(ACCENT).min_size(egui::Vec2::new(140.0, 30.0)))
                                        .clicked() { do_run_selected = true; }
                                });
                                if running { show_running_stop(ui, &mut do_stop); }
                            });
                            ui.add_space(4.0);
                        });
                    }

                    // ══════════════════════════════════════════════════════
                    // CARGO TAB
                    // ══════════════════════════════════════════════════════
                    ActiveTab::Cargo => {
                        // Package selector
                        panel_frame().show(ui, |ui| {
                            ui.label(egui::RichText::new("Package").color(TEXT_DIM).size(12.0));
                            ui.add_space(6.0);

                            // Build display list: "Workspace (all)" + individual packages
                            let pkg_count = self.state.packages.len();
                            let display_sel = if self.state.selected_package == 0 || pkg_count == 0 {
                                "Workspace (all)".to_string()
                            } else {
                                self.state.packages
                                    .get(self.state.selected_package - 1)
                                    .cloned()
                                    .unwrap_or_default()
                            };

                            egui::ComboBox::from_id_source("package_combo")
                                .width(ui.available_width() - 4.0)
                                .selected_text(egui::RichText::new(&display_sel).color(TEXT_NORMAL).size(13.0))
                                .show_ui(ui, |ui| {
                                    let sel = self.state.selected_package == 0;
                                    let label = egui::RichText::new("Workspace (all)")
                                        .color(if sel { ACCENT } else { COL_WARNING }).size(13.0);
                                    if ui.selectable_label(sel, label).clicked() {
                                        new_package = Some(0);
                                    }
                                    for (i, name) in self.state.packages.iter().enumerate() {
                                        let idx      = i + 1;
                                        let selected = self.state.selected_package == idx;
                                        let label    = egui::RichText::new(name)
                                            .color(if selected { ACCENT } else { TEXT_NORMAL }).size(13.0);
                                        if ui.selectable_label(selected, label).clicked() {
                                            new_package = Some(idx);
                                        }
                                    }
                                });

                            if self.state.packages.is_empty() {
                                ui.add_space(4.0);
                                ui.label(egui::RichText::new("Single-crate project — workspace discovery not applicable.")
                                    .size(10.0).color(TEXT_DIM));
                            }
                        });

                        ui.add_space(8.0);

                        // Command list
                        panel_frame().show(ui, |ui| {
                            ui.horizontal(|ui| {
                                ui.label(egui::RichText::new("Commands").color(TEXT_DIM).size(12.0));
                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                    if ui.add(egui::Button::new(egui::RichText::new("None").color(TEXT_DIM).size(11.0))
                                        .fill(egui::Color32::TRANSPARENT).stroke(egui::Stroke::NONE))
                                        .clicked() {
                                        for r in &mut self.state.cargo_rows { r.checked = false; }
                                    }
                                    ui.label(egui::RichText::new("·").color(TEXT_DIM).size(11.0));
                                    if ui.add(egui::Button::new(egui::RichText::new("All").color(TEXT_DIM).size(11.0))
                                        .fill(egui::Color32::TRANSPARENT).stroke(egui::Stroke::NONE))
                                        .clicked() {
                                        for r in &mut self.state.cargo_rows { r.checked = true; }
                                    }
                                });
                            });
                            ui.add_space(6.0);

                            egui::ScrollArea::vertical()
                                .id_source("cargo_scroll")
                                .max_height(ui.available_height() - 60.0)
                                .show(ui, |ui| {
                                    for row in &mut self.state.cargo_rows {
                                        let has_package = self.state.selected_package > 0 && !self.state.packages.is_empty();
                                        let enabled = row.cmd.scope != CmdScope::PackageOnly || has_package;
                                        let sc      = if enabled { row.status.color() } else { TEXT_DIM };
                                        let icon    = row.status.icon();
                                        let mut run_btn_rect: Option<egui::Rect> = None;

                                        let card_resp = task_frame().show(ui, |ui| {
                                            ui.horizontal(|ui| {
                                                ui.vertical(|ui| {
                                                    ui.label(egui::RichText::new(icon).color(sc).size(13.0));
                                                    ui.add_enabled_ui(!running && enabled, |ui| {
                                                        ui.checkbox(&mut row.checked, "");
                                                    });
                                                });
                                                ui.with_layout(egui::Layout::top_down(egui::Align::LEFT).with_main_wrap(false), |ui| {
                                                    ui.set_max_width(ui.available_width() - 68.0);
                                                    ui.horizontal(|ui| {
                                                        ui.label(egui::RichText::new(row.cmd.label).strong().size(13.0)
                                                            .color(if enabled { TEXT_NORMAL } else { TEXT_DIM }));
                                                        ui.label(egui::RichText::new(row.status.label()).size(10.5).color(sc));
                                                    });
                                                    let desc_suffix = if !enabled && row.cmd.scope == CmdScope::PackageOnly {
                                                        " (select a package first)"
                                                    } else { "" };
                                                    ui.add(egui::Label::new(
                                                        egui::RichText::new(format!("{}{}", row.cmd.desc, desc_suffix))
                                                            .size(11.0).color(TEXT_DIM))
                                                        .truncate(true));
                                                });
                                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                                    ui.add_enabled_ui(!running && enabled, |ui| {
                                                        let btn = egui::Button::new(
                                                            egui::RichText::new("▶ Run").size(12.0).strong().color(egui::Color32::BLACK))
                                                            .fill(ACCENT).min_size(egui::vec2(60.0, 24.0));
                                                        let resp = ui.add(btn).on_hover_text(format!("Run `cargo {}`", row.cmd.label));
                                                        run_btn_rect = Some(resp.rect);
                                                        if resp.clicked() { single_cargo_run = Some(row.cmd.id.to_string()); }
                                                    });
                                                });
                                            });
                                        });

                                        if !running && enabled {
                                            let card_rect      = card_resp.response.rect;
                                            let run_btn_x      = run_btn_rect.map_or(card_rect.right(), |r| r.left());
                                            let checkbox_right = card_rect.min.x + 46.0;
                                            let safe_rect = egui::Rect::from_min_max(
                                                egui::pos2(checkbox_right, card_rect.min.y),
                                                egui::pos2(run_btn_x - 4.0, card_rect.max.y),
                                            );
                                            let click_resp = ui.interact(safe_rect, ui.id().with(row.cmd.id), egui::Sense::click());
                                            if click_resp.clicked() { row.checked = !row.checked; }
                                        }
                                        ui.add_space(4.0);
                                    }
                                });

                            ui.add_space(4.0);
                            ui.separator();
                            ui.add_space(8.0);
                            ui.horizontal(|ui| {
                                let can_run = self.state.any_cargo_checked() && !running;
                                ui.add_enabled_ui(can_run, |ui| {
                                    if ui.add(egui::Button::new(
                                        egui::RichText::new("▶  Run selected").color(egui::Color32::BLACK).strong().size(13.0))
                                        .fill(ACCENT).min_size(egui::Vec2::new(140.0, 30.0)))
                                        .clicked() { do_run_cargo = true; }
                                });
                                if running { show_running_stop(ui, &mut do_stop); }
                            });
                            ui.add_space(4.0);
                        });
                    }
                }
            });

        // ── Apply deferred actions ────────────────────────────────────────
        if let Some(idx) = new_target {
            if idx != self.state.selected_target {
                let leaving = self
                    .state
                    .target_names
                    .get(self.state.selected_target)
                    .cloned()
                    .unwrap_or_default();
                let saved: std::collections::HashMap<String, bool> = self
                    .state
                    .tasks
                    .iter()
                    .map(|t| (t.id.clone(), t.checked))
                    .collect();
                self.state.checked_state.insert(leaving, saved);
                self.state.selected_target = idx;
                self.state.rebuild_task_rows();
                self.state.log.lock().unwrap().clear();
            }
        }
        if let Some(idx) = new_package {
            self.state.selected_package = idx;
            // Reset cargo row statuses when switching package
            for r in &mut self.state.cargo_rows {
                r.status = TaskStatus::Idle;
            }
        }
        if do_stop {
            self.state.stop();
        } else if let Some(id) = single_run {
            self.run_tasks(vec![id]);
        } else if do_run_selected {
            let ids = self.state.checked_ids();
            self.run_tasks(ids);
        } else if let Some(id) = single_cargo_run {
            self.run_cargo_cmds(vec![id]);
        } else if do_run_cargo {
            let ids = self.state.checked_cargo_ids();
            self.run_cargo_cmds(ids);
        }

        // ── Console panel ─────────────────────────────────────────────────
        egui::CentralPanel::default()
            .frame(egui::Frame::central_panel(&ctx.style()).fill(BG_DARK))
            .show(ctx, |ui| {
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    ui.add_space(4.0);
                    ui.label(egui::RichText::new("Console").color(TEXT_DIM).size(12.0));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui
                            .add(
                                egui::Button::new(
                                    egui::RichText::new("Clear").color(TEXT_DIM).size(11.0),
                                )
                                .fill(egui::Color32::TRANSPARENT)
                                .stroke(egui::Stroke::new(1.0, TEXT_DIM)),
                            )
                            .clicked()
                        {
                            self.state.log.lock().unwrap().clear();
                        }
                    });
                });
                ui.add_space(4.0);

                let available = ui.available_size();
                egui::Frame::none()
                    .fill(BG_CONSOLE)
                    .inner_margin(egui::Margin::same(10.0))
                    .rounding(egui::Rounding::same(6.0))
                    .stroke(egui::Stroke::new(1.0, egui::Color32::from_rgb(35, 40, 50)))
                    .show(ui, |ui| {
                        egui::ScrollArea::vertical()
                            .max_height(available.y - 20.0)
                            .auto_shrink([false; 2])
                            .stick_to_bottom(true)
                            .show(ui, |ui| {
                                ui.set_min_width(available.x - 40.0);
                                let log = self.state.log.lock().unwrap();
                                if log.is_empty() {
                                    ui.label(
                                        egui::RichText::new("No output yet.")
                                            .monospace()
                                            .color(TEXT_DIM)
                                            .size(12.0),
                                    );
                                }
                                for line in log.iter() {
                                    ui.label(
                                        egui::RichText::new(display_line(line))
                                            .monospace()
                                            .size(12.0)
                                            .color(line_color(line)),
                                    );
                                }
                            });
                    });
            });
    }
}

// ─── Small UI helpers ─────────────────────────────────────────────────────────

fn show_running_stop(ui: &mut egui::Ui, do_stop: &mut bool) {
    ui.add_space(8.0);
    ui.label(
        egui::RichText::new("● running…")
            .color(COL_WARNING)
            .size(12.0),
    );
    ui.add_space(8.0);
    if ui
        .add(
            egui::Button::new(
                egui::RichText::new("■  Stop")
                    .color(egui::Color32::WHITE)
                    .strong()
                    .size(13.0),
            )
            .fill(COL_ERROR)
            .min_size(egui::Vec2::new(80.0, 30.0)),
        )
        .clicked()
    {
        *do_stop = true;
    }
}

// ─── Find project root ────────────────────────────────────────────────────────

fn find_project_root() -> Result<PathBuf, String> {
    if let Ok(cwd) = std::env::current_dir() {
        let mut dir = cwd.clone();
        loop {
            if dir.join("Cargo.toml").exists() {
                return Ok(dir);
            }
            match dir.parent() {
                Some(p) => dir = p.to_path_buf(),
                None => break,
            }
        }
    }
    if let Ok(exe) = std::env::current_exe() {
        let mut dir = exe.parent().map(|p| p.to_path_buf()).unwrap_or_default();
        loop {
            if dir.join("Cargo.toml").exists() {
                return Ok(dir);
            }
            match dir.parent() {
                Some(p) => dir = p.to_path_buf(),
                None => break,
            }
        }
    }
    Err("No Cargo.toml found.\n\
         \n\
         cargo-xtask-runner must be run from inside a Rust project.\n\
         \n\
         Usage:\n\
         \u{2022} cd into your project folder\n\
         \u{2022} run: cargo xtask-runner\n\
         \n\
         The project must also have an xtask runner that supports:\n\
         cargo xtask --list   (outputs: target|task_id|description)"
        .to_string())
}

// ─── Icon / image loading ─────────────────────────────────────────────────────

fn load_header_image(ctx: &egui::Context) -> egui::TextureHandle {
    let bytes = include_bytes!("../assets/icon.png");
    let image = image::load_from_memory(bytes).unwrap().into_rgba8();
    let (width, height) = image.dimensions();
    ctx.load_texture(
        "header_logo",
        egui::ColorImage::from_rgba_unmultiplied(
            [width as usize, height as usize],
            &image.into_raw(),
        ),
        egui::TextureOptions::LINEAR,
    )
}

fn load_icon() -> egui::IconData {
    let bytes = include_bytes!("../assets/icon.png");
    let image = image::load_from_memory(bytes).unwrap().into_rgba8();
    let (width, height) = image.dimensions();
    egui::IconData {
        rgba: image.into_raw(),
        width,
        height,
    }
}

// ─── Main ─────────────────────────────────────────────────────────────────────

fn main() -> eframe::Result<()> {
    let args: Vec<String> = std::env::args().collect();

    if !args.contains(&"--gui".to_string()) {
        use std::process::Command;

        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const DETACHED_PROCESS: u32 = 0x00000008;
            let exe = std::env::current_exe().unwrap();
            Command::new(exe)
                .arg("--gui")
                .creation_flags(DETACHED_PROCESS)
                .spawn()
                .expect("failed to spawn GUI");
        }

        #[cfg(not(windows))]
        {
            let exe = std::env::current_exe().unwrap();
            Command::new(exe)
                .arg("--gui")
                .spawn()
                .expect("failed to spawn GUI");
        }

        return Ok(());
    }

    let icon = load_icon();

    eframe::run_native(
        "xtask runner",
        eframe::NativeOptions {
            viewport: egui::ViewportBuilder::default()
                .with_title("xtask runner")
                .with_inner_size([980.0, 640.0])
                .with_min_inner_size([640.0, 420.0])
                .with_icon(icon),
            ..Default::default()
        },
        Box::new(|cc| Box::new(XtaskRunner::new(cc))),
    )
}
