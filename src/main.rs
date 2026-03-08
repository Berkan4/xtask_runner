// xtask-runner — drop next to Cargo.toml.
// Reads `cargo xtask --list` which now outputs: target|task_id|description
// Shows a target dropdown; task list updates per selected target.

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

const BG_DARK:     egui::Color32 = egui::Color32::from_rgb(15,  17,  21);
const BG_PANEL:    egui::Color32 = egui::Color32::from_rgb(22,  25,  31);
const BG_TASK:     egui::Color32 = egui::Color32::from_rgb(18,  21,  27);
const BG_CONSOLE:  egui::Color32 = egui::Color32::from_rgb(10,  12,  15);
const ACCENT:      egui::Color32 = egui::Color32::from_rgb(82,  196, 130);
const ACCENT_DIM:  egui::Color32 = egui::Color32::from_rgb(45,  110, 72);
const TEXT_NORMAL: egui::Color32 = egui::Color32::from_rgb(210, 215, 220);
const TEXT_DIM:    egui::Color32 = egui::Color32::from_rgb(120, 130, 145);

const COL_DEFAULT: egui::Color32 = egui::Color32::from_rgb(210, 215, 220);
const COL_ERROR:   egui::Color32 = egui::Color32::from_rgb(230, 80,  70);
const COL_SUCCESS: egui::Color32 = egui::Color32::from_rgb(82,  196, 130);
const COL_SECTION: egui::Color32 = egui::Color32::from_rgb(82,  196, 130);
const COL_WARNING: egui::Color32 = egui::Color32::from_rgb(255, 180, 50);
const COL_DIM:     egui::Color32 = egui::Color32::from_rgb(120, 130, 145);

fn setup_visuals(ctx: &egui::Context) {
    let mut v = egui::Visuals::dark();
    v.window_fill                      = BG_PANEL;
    v.panel_fill                       = BG_DARK;
    v.faint_bg_color                   = BG_PANEL;
    v.extreme_bg_color                 = BG_CONSOLE;
    v.override_text_color              = Some(TEXT_NORMAL);
    v.widgets.noninteractive.bg_fill   = BG_PANEL;
    v.widgets.inactive.bg_fill         = BG_PANEL;
    v.widgets.hovered.bg_fill          = egui::Color32::from_rgb(35, 40, 50);
    v.widgets.active.bg_fill           = ACCENT_DIM;
    v.widgets.noninteractive.fg_stroke = egui::Stroke::new(1.0, TEXT_DIM);
    v.widgets.inactive.fg_stroke       = egui::Stroke::new(1.0, TEXT_NORMAL);
    v.widgets.hovered.fg_stroke        = egui::Stroke::new(1.0, ACCENT);
    v.widgets.active.fg_stroke         = egui::Stroke::new(1.0, ACCENT);
    v.selection.bg_fill                = ACCENT_DIM;
    v.selection.stroke                 = egui::Stroke::new(1.0, ACCENT);
    v.window_stroke                    = egui::Stroke::new(1.0, egui::Color32::from_rgb(45, 50, 60));
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
            TaskStatus::Idle    => "idle",
            TaskStatus::Running => "running",
            TaskStatus::Done    => "done",
            TaskStatus::Failed  => "failed",
        }
    }
    fn icon(&self) -> &str {
        match self {
            TaskStatus::Idle    => "○",
            TaskStatus::Running => "○",
            TaskStatus::Done    => "○",
            TaskStatus::Failed  => "○",
        }
    }
    fn color(&self) -> egui::Color32 {
        match self {
            TaskStatus::Idle    => TEXT_DIM,
            TaskStatus::Running => COL_WARNING,
            TaskStatus::Done    => ACCENT,
            TaskStatus::Failed  => COL_ERROR,
        }
    }
}

// ─── Data model ───────────────────────────────────────────────────────────────

/// One entry as parsed from `cargo xtask --list`
#[derive(Clone)]
struct TaskEntry {
    target: String,
    id:     String,
    desc:   String,
}

/// Runtime state for one task row in the UI
struct TaskRow {
    id:        String,
    desc:      String,
    checked:   bool,
    status:    TaskStatus,
    is_global: bool,
}

impl TaskRow {
    fn from_entry(e: &TaskEntry) -> Self {
        Self {
            id:        e.id.clone(),
            desc:      e.desc.clone(),
            checked:   true,
            status:    TaskStatus::Idle,
            is_global: e.target == "workspace",
        }
    }
}

// ─── Shared state ─────────────────────────────────────────────────────────────

type Log          = Arc<Mutex<Vec<String>>>;
type IsRunning    = Arc<Mutex<bool>>;
type CurrentChild = Arc<Mutex<Option<Child>>>;

struct AppState {
    /// All entries parsed from --list, kept for reference
    all_entries:      Vec<TaskEntry>,
    /// Unique target names in the order they appeared
    target_names:     Vec<String>,
    /// Currently selected index into target_names
    selected_target:  usize,
    /// Task rows for the currently selected target
    tasks:            Vec<TaskRow>,
    checked_state:    std::collections::HashMap<String, std::collections::HashMap<String, bool>>,
    log:              Log,
    is_running:       IsRunning,
    current_child:    CurrentChild,
    project_root:     PathBuf,
    load_error:       Option<String>,
}

impl AppState {
    fn new(root_result: Result<PathBuf, String>) -> Self {
        let (project_root, initial_error) = match root_result {
            Ok(p)  => (p, None),
            Err(e) => (std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")), Some(e)),
        };
        let mut s = Self {
            all_entries:     Vec::new(),
            target_names:    Vec::new(),
            selected_target: 0,
            tasks:           Vec::new(),
            log:             Arc::new(Mutex::new(Vec::new())),
            checked_state:   std::collections::HashMap::new(),
            is_running:      Arc::new(Mutex::new(false)),
            current_child:   Arc::new(Mutex::new(None)),
            project_root,
            load_error:      initial_error,
        };
        if s.load_error.is_none() {
            s.reload();
        }
        s
    }

    /// Run `cargo xtask --list`, parse the three-column output, rebuild state.
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
                    // Expected format: "target|task_id|description"
                    let parts: Vec<&str> = line.splitn(3, '|').collect();
                    if parts.len() == 3 {
                        let target = parts[0].trim().to_string();
                        let id     = parts[1].trim().to_string();
                        let desc   = parts[2].trim().to_string();

                        // Track unique targets in order
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
                 Expected format: target|task_id|description".to_string()
            );
            return;
        }

        // Clamp selection and populate tasks for it
        self.selected_target = self.selected_target.min(self.target_names.len() - 1);
        self.rebuild_task_rows();
    }

    /// Rebuild the task rows from the currently selected target.
    fn rebuild_task_rows(&mut self) {
        let target = match self.target_names.get(self.selected_target) {
            Some(t) => t.clone(),
            None    => return,
        };
        self.tasks = self.all_entries.iter()
            .filter(|e| e.target == target)
            .map(|e| {
                let mut row = TaskRow::from_entry(e);
                if let Some(target_map) = self.checked_state.get(&target) {
                    if let Some(&checked) = target_map.get(&row.id) {
                        row.checked = checked;
                    }
                }
                row
            })
            .collect();
    }

    fn current_target(&self) -> Option<&str> {
        self.target_names.get(self.selected_target).map(|s| s.as_str())
    }

    fn checked_ids(&self) -> Vec<String> {
        self.tasks.iter().filter(|t| t.checked).map(|t| t.id.clone()).collect()
    }

    fn any_checked(&self) -> bool {
        self.tasks.iter().any(|t| t.checked)
    }

    fn currently_running(&self) -> bool {
        *self.is_running.lock().unwrap()
    }

    fn stop(&mut self) {
        if let Some(mut child) = self.current_child.lock().unwrap().take() {
            let _ = child.kill();
        }
        *self.is_running.lock().unwrap() = false;
        self.log.lock().unwrap().push("■  Stopped by user.".to_string());
        for t in &mut self.tasks {
            if t.status == TaskStatus::Running {
                t.status = TaskStatus::Failed;
            }
        }
    }
}

// ─── App ──────────────────────────────────────────────────────────────────────

struct XtaskRunner {
    state: AppState,
    header_image: egui::TextureHandle,
}

impl XtaskRunner {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        Self {
            state:        AppState::new(find_project_root()),
            header_image: load_header_image(&cc.egui_ctx),
        }
    }

    /// Spawn tasks sequentially in a background thread.
    /// Each task is invoked as: cargo xtask <task_id> <target>
    fn run_tasks(&mut self, ids: Vec<String>) {
        if ids.is_empty() || self.state.currently_running() { return; }

        let target = match self.state.current_target() {
            Some(t) => t.to_string(),
            None    => return,
        };

        for task in &mut self.state.tasks {
            task.status = TaskStatus::Idle;
        }
        self.state.log.lock().unwrap().clear();

        let log           = Arc::clone(&self.state.log);
        let is_running    = Arc::clone(&self.state.is_running);
        let current_child = Arc::clone(&self.state.current_child);
        let project_root  = self.state.project_root.clone();

        let id_is_global: std::collections::HashMap<String, bool> = self.state.tasks.iter()
            .map(|t| (t.id.clone(), t.is_global))
            .collect();

        *is_running.lock().unwrap() = true;

        thread::spawn(move || {
            for id in &ids {
                // Check stop at the start of each iteration so pressing
                // Stop aborts before the next task begins.
                if !*is_running.lock().unwrap() { break; }

                let global = id_is_global.get(id.as_str()).copied().unwrap_or(false);
                let tgt_label = if global { String::new() } else { format!(" {target}") };
                log.lock().unwrap().push(format!("@@START:{id}"));
                log.lock().unwrap().push(format!(
                    "\n── cargo xtask {id}{tgt_label} {}\n",
                    "─".repeat(38usize.saturating_sub(id.len() + tgt_label.len()))
                ));

                let mut cmd_args = vec!["xtask", id.as_str()];
                if !global { cmd_args.push(target.as_str()); }

                let child = no_window_command("cargo")
                    .args(&cmd_args)
                    .current_dir(&project_root)
                    .stdout(Stdio::piped())
                    .stderr(Stdio::piped())
                    .spawn();

                let mut success = false;

                match child {
                    Err(e) => {
                        log.lock().unwrap().push(format!("❌ Failed to spawn: {e}"));
                    }
                    Ok(mut child) => {
                        if let Some(stdout) = child.stdout.take() {
                            let log2 = Arc::clone(&log);
                            thread::spawn(move || {
                                for line in BufReader::new(stdout).lines().flatten() {
                                    log2.lock().unwrap().push(line);
                                }
                            });
                        }
                        if let Some(stderr) = child.stderr.take() {
                            let log2 = Arc::clone(&log);
                            thread::spawn(move || {
                                for line in BufReader::new(stderr).lines().flatten() {
                                    log2.lock().unwrap().push(format!("@@STDERR:{line}"));
                                }
                            });
                        }

                        *current_child.lock().unwrap() = Some(child);

                        let exit_status = {
                            let mut guard = current_child.lock().unwrap();
                            guard.as_mut().and_then(|c| c.wait().ok())
                        };
                        *current_child.lock().unwrap() = None;

                        if let Some(s) = exit_status {
                            success = s.success();
                        }
                    }
                }

                if !*is_running.lock().unwrap() { break; }

                log.lock().unwrap().push(
                    if success { format!("@@DONE:{id}") } else { format!("@@FAIL:{id}") }
                );

                if !success {
                    log.lock().unwrap().push(
                        format!("❌ Task `{id}` failed — pipeline stopped.")
                    );
                    break;
                }
            }

            *is_running.lock().unwrap() = false;
        });
    }
}

// ─── Console coloring ─────────────────────────────────────────────────────────

fn line_color(line: &str) -> egui::Color32 {
    let s = line.strip_prefix("@@STDERR:").unwrap_or(line);
    if s.starts_with("error[") || s.starts_with("error: ") || line.starts_with("❌") {
        COL_ERROR
    } else if s.starts_with("warning[") || s.starts_with("warning: ") || line.starts_with("⚠") {
        COL_WARNING
    } else if s.starts_with("   Finished") || s.starts_with("    Finished")
           || line.starts_with("✅") || s.starts_with("test result: ok")
    {
        COL_SUCCESS
    } else if s.starts_with("   Compiling") || s.starts_with("    Compiling")
           || s.starts_with("   Downloading") || s.starts_with("    Downloading")
           || s.starts_with("   Updating") || s.starts_with("    Updating")
           || s.starts_with("   Running") || s.starts_with("    Running")
           || s.starts_with("   Fetching") || s.starts_with("    Fetching")
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

        // ── Process sentinel log lines → update task statuses ─────────────────
        {
            let mut log = self.state.log.lock().unwrap();
            let sentinels: Vec<String> = log.iter()
                .filter(|l| l.starts_with("@@START:") || l.starts_with("@@DONE:") || l.starts_with("@@FAIL:"))
                .cloned()
                .collect();

            for s in &sentinels {
                if let Some(id) = s.strip_prefix("@@START:") {
                    if let Some(t) = self.state.tasks.iter_mut().find(|t| t.id == id) {
                        t.status = TaskStatus::Running;
                    }
                } else if let Some(id) = s.strip_prefix("@@DONE:") {
                    if let Some(t) = self.state.tasks.iter_mut().find(|t| t.id == id) {
                        t.status = TaskStatus::Done;
                    }
                } else if let Some(id) = s.strip_prefix("@@FAIL:") {
                    if let Some(t) = self.state.tasks.iter_mut().find(|t| t.id == id) {
                        t.status = TaskStatus::Failed;
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

        // ── Header ────────────────────────────────────────────────────────────
        egui::TopBottomPanel::top("header")
            .frame(egui::Frame::none().fill(BG_DARK).inner_margin(egui::Margin::same(10.0)))
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.add_space(4.0);
                    let size = egui::vec2(44.0, 44.0); // adjust to taste
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
                        ui.label(egui::RichText::new("Drop next to a Cargo.toml. Discovers tasks via").size(11.0).color(TEXT_DIM));
                        ui.label(egui::RichText::new("cargo xtask --list").size(10.0).color(TEXT_NORMAL).monospace());
                        ui.label(egui::RichText::new("and lets you run them individually or as a pipeline.").size(11.0).color(TEXT_DIM));
                    });
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(
                            egui::RichText::new(format!("📁  {}", self.state.project_root.display()))
                                .size(11.0)
                                .color(TEXT_DIM),
                        );
                    });
                });
            });

        // ── Deferred actions ──────────────────────────────────────────────────
        let mut single_run: Option<String> = None;
        let mut do_run_selected = false;
        let mut do_stop = false;
        let mut new_target: Option<usize> = None;

        // ── Left panel ────────────────────────────────────────────────────────
        egui::SidePanel::left("tasks_panel")
            .resizable(true)
            .default_width(360.0)
            .min_width(360.0)
            .frame(egui::Frame::none().fill(BG_DARK).inner_margin(egui::Margin::same(0.0)))
            .show(ctx, |ui| {
                ui.add_space(8.0);

                if let Some(err) = self.state.load_error.clone() {
                    ui.add_space(16.0);
                    egui::Frame::none()
                        .fill(egui::Color32::from_rgb(40, 18, 18))
                        .rounding(egui::Rounding::same(8.0))
                        .inner_margin(egui::Margin::same(16.0))
                        .stroke(egui::Stroke::new(1.0, COL_ERROR))
                        .show(ui, |ui| {
                            ui.label(
                                egui::RichText::new("\u{26a0}  Could not start")
                                    .strong()
                                    .size(14.0)
                                    .color(COL_ERROR),
                            );
                            ui.add_space(8.0);
                            for line in err.clone().lines() {
                                let color = if line.starts_with('\u{2022}') {
                                    ACCENT
                                } else if line.trim().starts_with("cargo xtask") {
                                    TEXT_NORMAL
                                } else {
                                    TEXT_DIM
                                };
                                ui.label(
                                    egui::RichText::new(line)
                                        .size(12.0)
                                        .color(color)
                                        .monospace(),
                                );
                            }
                            ui.add_space(12.0);
                            ui.separator();
                            ui.add_space(8.0);
                            if ui.add(
                                egui::Button::new(
                                    egui::RichText::new("\u{21ba}  Retry").color(egui::Color32::BLACK).strong().size(12.0)
                                )
                                .fill(ACCENT)
                                .min_size(egui::vec2(90.0, 26.0)),
                            ).clicked() {
                                self.state.load_error = None;
                                self.state.reload();
                            }
                        });
                    return;
                }

                // ── Target selector ───────────────────────────────────────────
                panel_frame().show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(egui::RichText::new("Target").color(TEXT_DIM).size(12.0));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            let n = self.state.target_names.len();
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
                        .get(self.state.selected_target)
                        .cloned()
                        .unwrap_or_default();

                    let display_name = if current_name == "workspace" {
                        "Workspace".to_string()
                    } else {
                        current_name.clone()
                    };

                    egui::ComboBox::from_id_source("target_combo")
                        .width(ui.available_width() - 4.0)
                        .selected_text(
                            egui::RichText::new(&display_name).color(TEXT_NORMAL).size(13.0)
                        )
                        .show_ui(ui, |ui| {
                            for (i, name) in self.state.target_names.iter().enumerate() {
                                let selected = i == self.state.selected_target;
                                let (label_str, label_color) = if name == "workspace" {
                                    ("Workspace", if selected { ACCENT } else { COL_WARNING })
                                } else {
                                    (name.as_str(), if selected { ACCENT } else { TEXT_NORMAL })
                                };
                                let label = egui::RichText::new(label_str)
                                    .color(label_color)
                                    .size(13.0);
                                if ui.selectable_label(selected, label).clicked() {
                                    new_target = Some(i);
                                }
                            }
                        });
                });

                ui.add_space(8.0);

                // ── Task list ─────────────────────────────────────────────────
                panel_frame().show(ui, |ui| {
                    // All / None row
                    ui.horizontal(|ui| {
                        ui.label(egui::RichText::new("Tasks").color(TEXT_DIM).size(12.0));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.add(
                                egui::Button::new(egui::RichText::new("None").color(TEXT_DIM).size(11.0))
                                    .fill(egui::Color32::TRANSPARENT)
                                    .stroke(egui::Stroke::NONE),
                            ).clicked() {
                                for t in &mut self.state.tasks { t.checked = false; }
                            }
                            ui.label(egui::RichText::new("·").color(TEXT_DIM).size(11.0));
                            if ui.add(
                                egui::Button::new(egui::RichText::new("All").color(TEXT_DIM).size(11.0))
                                    .fill(egui::Color32::TRANSPARENT)
                                    .stroke(egui::Stroke::NONE),
                            ).clicked() {
                                for t in &mut self.state.tasks { t.checked = true; }
                            }
                        });
                    });

                    ui.add_space(6.0);

                    let running = self.state.currently_running();

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
                                            ui.set_max_width(ui.available_width() - 68.0); // 68 = Run button width + margin
                                            ui.horizontal(|ui| {
                                                ui.label(
                                                    egui::RichText::new(&task.id)
                                                        .strong()
                                                        .size(13.0)
                                                        .color(TEXT_NORMAL),
                                                );
                                                ui.label(
                                                    egui::RichText::new(task.status.label())
                                                        .size(10.5)
                                                        .color(sc),
                                                );
                                            });
                                            ui.add(
                                                egui::Label::new(
                                                    egui::RichText::new(&task.desc)
                                                        .size(11.0)
                                                        .color(TEXT_DIM),
                                                )
                                                    .truncate(true)
                                            );
                                        });

                                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                            ui.add_enabled_ui(!running, |ui| {
                                                let btn = egui::Button::new(
                                                    egui::RichText::new("▶ Run")
                                                        .size(12.0)
                                                        .strong()
                                                        .color(egui::Color32::BLACK),
                                                )
                                                .fill(ACCENT)
                                                .min_size(egui::vec2(60.0, 24.0));
                                                let resp = ui.add(btn)
                                                    .on_hover_text(format!("Run `{}`", task.id));
                                                run_btn_rect = Some(resp.rect);
                                                if resp.clicked() {
                                                    single_run = Some(task.id.clone());
                                                }
                                            });
                                        });
                                    });
                                });

                                // Click anywhere on card (except checkbox + run button) to toggle
                                if !running {
                                    let card_rect = card_resp.response.rect;
                                    let run_btn_x = run_btn_rect.map_or(card_rect.right(), |r| r.left());
                                    let checkbox_right = card_rect.min.x + 46.0; // icon(18) + checkbox(28)

                                    // Allocate a clickable sense over just the "safe" portion of the card:
                                    // from the left edge up to just before the Run button, excluding the checkbox.
                                    let safe_rect = egui::Rect::from_min_max(
                                        egui::pos2(checkbox_right, card_rect.min.y),
                                        egui::pos2(run_btn_x - 4.0, card_rect.max.y),
                                    );

                                    // interact_with_hovered uses egui's own hit-test + claim system,
                                    // so it won't fire when a widget below (Run selected) was clicked,
                                    // and it won't fire outside the scroll area's clip rect.
                                    let click_resp = ui.interact(safe_rect, ui.id().with(&task.id), egui::Sense::click());
                                    if click_resp.clicked() {
                                        task.checked = !task.checked;
                                    }
                                }

                                ui.add_space(4.0);
                            }
                        });

                    // Bottom bar
                    ui.add_space(4.0);
                    ui.separator();
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        let can_run = self.state.any_checked() && !running;
                        ui.add_enabled_ui(can_run, |ui| {
                            if ui.add(
                                egui::Button::new(
                                    egui::RichText::new("▶  Run selected")
                                        .color(egui::Color32::BLACK)
                                        .strong()
                                        .size(13.0),
                                )
                                .fill(ACCENT)
                                .min_size(egui::Vec2::new(140.0, 30.0)),
                            ).clicked() {
                                do_run_selected = true;
                            }
                        });

                        if running {
                            ui.add_space(8.0);
                            ui.label(egui::RichText::new("● running…").color(COL_WARNING).size(12.0));
                            ui.add_space(8.0);
                            if ui.add(
                                egui::Button::new(
                                    egui::RichText::new("■  Stop")
                                        .color(egui::Color32::WHITE)
                                        .strong()
                                        .size(13.0),
                                )
                                .fill(COL_ERROR)
                                .min_size(egui::Vec2::new(80.0, 30.0)),
                            ).clicked() {
                                do_stop = true;
                            }
                        }
                    });
                    ui.add_space(4.0);
                });
            });

        // ── Apply deferred actions ────────────────────────────────────────────
        if let Some(idx) = new_target {
            if idx != self.state.selected_target {
                // Flush checkboxes for the target we're LEAVING, before updating the index
                let leaving = self.state.target_names
                    .get(self.state.selected_target)
                    .cloned()
                    .unwrap_or_default();
                let saved: std::collections::HashMap<String, bool> = self.state.tasks.iter()
                    .map(|t| (t.id.clone(), t.checked))
                    .collect();
                self.state.checked_state.insert(leaving, saved);

                self.state.selected_target = idx;
                self.state.rebuild_task_rows();
                self.state.log.lock().unwrap().clear();
            }
        }
        if do_stop {
            self.state.stop();
        } else if let Some(id) = single_run {
            self.run_tasks(vec![id]);
        } else if do_run_selected {
            let ids = self.state.checked_ids();
            self.run_tasks(ids);
        }

        // ── Console panel ─────────────────────────────────────────────────────
        egui::CentralPanel::default()
            .frame(egui::Frame::central_panel(&ctx.style()).fill(BG_DARK))
            .show(ctx, |ui| {
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    ui.add_space(4.0);
                    ui.label(egui::RichText::new("Console").color(TEXT_DIM).size(12.0));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.add(
                            egui::Button::new(egui::RichText::new("Clear").color(TEXT_DIM).size(11.0))
                                .fill(egui::Color32::TRANSPARENT)
                                .stroke(egui::Stroke::new(1.0, TEXT_DIM)),
                        ).clicked() {
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

// ─── Find project root ────────────────────────────────────────────────────────

fn find_project_root() -> Result<PathBuf, String> {
    // First try: walk up from the current working directory (most reliable when
    // invoked as `cargo xtask-runner` from inside a project).
    if let Ok(cwd) = std::env::current_dir() {
        let mut dir = cwd.clone();
        loop {
            if dir.join("Cargo.toml").exists() {
                return Ok(dir);
            }
            match dir.parent() {
                Some(p) => dir = p.to_path_buf(),
                None    => break,
            }
        }
    }

    // Second try: walk up from the executable location (legacy double-click usage).
    if let Ok(exe) = std::env::current_exe() {
        let mut dir = exe.parent().map(|p| p.to_path_buf()).unwrap_or_default();
        loop {
            if dir.join("Cargo.toml").exists() {
                return Ok(dir);
            }
            match dir.parent() {
                Some(p) => dir = p.to_path_buf(),
                None    => break,
            }
        }
    }

    Err(
        "No Cargo.toml found.\n\
         \n\
         cargo-xtask-runner must be run from inside a Rust project.\n\
         \n\
         Usage:\n\
         \u{2022} cd into your project folder\n\
         \u{2022} run: cargo xtask-runner\n\
         \n\
         The project must also have an xtask runner that supports:\n\
         cargo xtask --list   (outputs: target|task_id|description)".to_string()
    )
}

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

    // If we were NOT started as the GUI instance, spawn it detached
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

        // Exit immediately so cargo returns control to terminal
        return Ok(());
    }

    let icon = load_icon();

    // Actual GUI
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
