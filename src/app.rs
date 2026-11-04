//! The egui user interface.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, channel};
use std::thread;

use eframe::egui::{
    self, Align, Button, Color32, CornerRadius, FontId, Frame, Layout, Margin, ProgressBar,
    RichText, Stroke, TextEdit, ThemePreference, Ui, Vec2,
};

use crate::split::{Mode, PageGroup, Pdf, Progress, SplitError, output_name, write_groups};

const ACCENT: Color32 = Color32::from_rgb(37, 99, 235);
const ACCENT_HOVER: Color32 = Color32::from_rgb(59, 130, 246);
const ERROR_LIGHT: Color32 = Color32::from_rgb(185, 28, 28);
const ERROR_DARK: Color32 = Color32::from_rgb(248, 113, 113);
const SUCCESS_LIGHT: Color32 = Color32::from_rgb(21, 128, 61);
const SUCCESS_DARK: Color32 = Color32::from_rgb(74, 222, 128);

struct LoadedPdf {
    path: PathBuf,
    pdf: Arc<Pdf>,
    size: u64,
}

enum Status {
    Idle(String),
    Working { done: usize, total: usize },
    Done { count: usize, dir: PathBuf },
    Failed(String),
}

enum WorkerMsg {
    Progress(Progress),
    Finished(Result<Vec<PathBuf>, SplitError>),
}

struct Job {
    rx: Receiver<WorkerMsg>,
    cancel: Arc<AtomicBool>,
    out_dir: PathBuf,
}

struct PasswordPrompt {
    path: PathBuf,
    password: String,
    error: Option<String>,
    focus: bool,
}

pub struct SplitterApp {
    file: Option<LoadedPdf>,
    mode: Mode,
    /// Separate input per mode so switching modes doesn't lose what was typed.
    inputs: [String; 5],
    out_dir: String,
    prefix: String,
    overwrite: bool,
    theme: ThemePreference,
    status: Status,
    job: Option<Job>,
    password: Option<PasswordPrompt>,
    error_dialog: Option<String>,
    last_output: Option<PathBuf>,
}

impl SplitterApp {
    pub fn new(cc: &eframe::CreationContext<'_>, initial: Option<PathBuf>) -> Self {
        configure_style(&cc.egui_ctx);
        let mut app = SplitterApp {
            file: None,
            mode: Mode::EveryPage,
            inputs: [
                String::new(),
                "2".into(),
                "2".into(),
                String::new(),
                String::new(),
            ],
            out_dir: String::new(),
            prefix: String::new(),
            overwrite: false,
            theme: ThemePreference::System,
            status: Status::Idle(format!("v{}", env!("CARGO_PKG_VERSION"))),
            job: None,
            password: None,
            error_dialog: None,
            last_output: None,
        };
        if let Some(path) = initial {
            app.load(path, None);
        }
        app
    }

    fn busy(&self) -> bool {
        self.job.is_some()
    }

    fn input(&self) -> &str {
        &self.inputs[mode_index(self.mode)]
    }

    fn load(&mut self, path: PathBuf, password: Option<&str>) {
        if self.busy() {
            return;
        }
        let path = std::fs::canonicalize(&path).map(strip_unc).unwrap_or(path);
        match Pdf::open(&path, password) {
            Ok(pdf) => {
                let size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
                self.file = Some(LoadedPdf {
                    path,
                    pdf: Arc::new(pdf),
                    size,
                });
                self.password = None;
                self.last_output = None;
                self.status = Status::Idle("Ready".into());
            }
            Err(SplitError::PasswordRequired) => {
                self.password = Some(PasswordPrompt {
                    path,
                    password: String::new(),
                    error: None,
                    focus: true,
                });
            }
            Err(SplitError::WrongPassword) => {
                self.password = Some(PasswordPrompt {
                    path,
                    password: String::new(),
                    error: Some("Wrong password, please try again.".into()),
                    focus: true,
                });
            }
            Err(err) => self.error_dialog = Some(err.to_string()),
        }
    }

    fn groups(&self) -> Option<Result<Vec<PageGroup>, SplitError>> {
        let file = self.file.as_ref()?;
        Some(self.mode.groups(file.pdf.page_count(), self.input()))
    }

    fn effective_prefix(&self) -> String {
        let p = self.prefix.trim();
        if !p.is_empty() {
            return p.to_string();
        }
        self.file
            .as_ref()
            .and_then(|f| f.path.file_stem())
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "document".into())
    }

    fn effective_out_dir(&self) -> Option<PathBuf> {
        let custom = self.out_dir.trim();
        if !custom.is_empty() {
            return Some(PathBuf::from(custom));
        }
        let file = self.file.as_ref()?;
        let stem = file.path.file_stem()?.to_string_lossy();
        Some(file.path.parent()?.join(format!("{stem}_split")))
    }

    fn browse_file(&mut self) {
        let mut dialog = rfd::FileDialog::new()
            .set_title("Choose a PDF")
            .add_filter("PDF files", &["pdf", "PDF"])
            .add_filter("All files", &["*"]);
        if let Some(dir) = self.file.as_ref().and_then(|f| f.path.parent()) {
            dialog = dialog.set_directory(dir);
        }
        if let Some(path) = dialog.pick_file() {
            self.load(path, None);
        }
    }

    fn browse_output(&mut self) {
        let mut dialog = rfd::FileDialog::new().set_title("Choose output folder");
        if let Some(dir) = self.effective_out_dir() {
            let existing = dir.ancestors().find(|d| d.is_dir()).map(Path::to_path_buf);
            if let Some(d) = existing {
                dialog = dialog.set_directory(d);
            }
        }
        if let Some(dir) = dialog.pick_folder() {
            self.out_dir = dir.display().to_string();
        }
    }

    fn start_split(&mut self, ctx: &egui::Context) {
        if self.busy() {
            return;
        }
        let (Some(file), Some(out_dir)) = (self.file.as_ref(), self.effective_out_dir()) else {
            return;
        };
        let groups = match self.mode.groups(file.pdf.page_count(), self.input()) {
            Ok(g) => g,
            Err(err) => {
                self.error_dialog = Some(err.to_string());
                return;
            }
        };
        let pdf = Arc::clone(&file.pdf);
        let prefix = self.effective_prefix();
        let overwrite = self.overwrite;
        let cancel = Arc::new(AtomicBool::new(false));
        let (tx, rx) = channel();
        let ctx = ctx.clone();
        let worker_cancel = Arc::clone(&cancel);
        let worker_dir = out_dir.clone();

        self.status = Status::Working {
            done: 0,
            total: groups.len(),
        };
        thread::spawn(move || {
            let progress_tx = tx.clone();
            let progress_ctx = ctx.clone();
            let result = write_groups(
                &pdf,
                &groups,
                &worker_dir,
                &prefix,
                overwrite,
                &worker_cancel,
                |p| {
                    let _ = progress_tx.send(WorkerMsg::Progress(p));
                    progress_ctx.request_repaint();
                },
            );
            let _ = tx.send(WorkerMsg::Finished(result));
            ctx.request_repaint();
        });
        self.job = Some(Job {
            rx,
            cancel,
            out_dir,
        });
    }

    fn poll_job(&mut self) {
        let Some(job) = &self.job else { return };
        let mut finished = None;
        loop {
            let msg = match job.rx.try_recv() {
                Ok(msg) => msg,
                Err(std::sync::mpsc::TryRecvError::Empty) => break,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    // The worker exited; if it died without reporting back, say so.
                    if finished.is_none() {
                        finished = Some(Err(SplitError::Pdf("unexpected internal error".into())));
                    }
                    break;
                }
            };
            match msg {
                WorkerMsg::Progress(p) => {
                    self.status = Status::Working {
                        done: p.files_done,
                        total: p.files_total,
                    }
                }
                WorkerMsg::Finished(result) => finished = Some(result),
            }
        }
        if let Some(result) = finished {
            let job = self.job.take().expect("job exists");
            match result {
                Ok(files) => {
                    self.last_output = Some(job.out_dir.clone());
                    self.status = Status::Done {
                        count: files.len(),
                        dir: job.out_dir,
                    };
                }
                Err(SplitError::Cancelled) => self.status = Status::Idle("Cancelled".into()),
                Err(err) => {
                    self.status = Status::Failed("Splitting failed".into());
                    self.error_dialog = Some(err.to_string());
                }
            }
        }
    }

    fn handle_dropped_files(&mut self, ctx: &egui::Context) {
        let dropped: Vec<PathBuf> = ctx.input(|i| {
            i.raw
                .dropped_files
                .iter()
                .map(|f| f.path().to_path_buf())
                .filter(|p| !p.as_os_str().is_empty())
                .collect()
        });
        if dropped.is_empty() {
            return;
        }
        match dropped.into_iter().find(|p| is_pdf(p)) {
            Some(path) => self.load(path, None),
            None => self.error_dialog = Some("Please drop a PDF file.".into()),
        }
    }

    // ------------------------------------------------------------------ UI

    fn header(&mut self, ui: &mut Ui) {
        ui.horizontal(|ui| {
            pdf_glyph(ui, 34.0);
            ui.add_space(6.0);
            ui.vertical(|ui| {
                ui.label(RichText::new("PDF Splitter").size(26.0).strong());
                ui.label(
                    RichText::new("Split PDFs into pages, chunks or custom ranges")
                        .color(ui.visuals().weak_text_color()),
                );
            });
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let before = self.theme;
                for (pref, label) in [
                    (ThemePreference::Dark, "Dark"),
                    (ThemePreference::Light, "Light"),
                    (ThemePreference::System, "System"),
                ] {
                    ui.selectable_value(&mut self.theme, pref, label);
                }
                if before != self.theme {
                    ui.ctx().set_theme(self.theme);
                }
            });
        });
    }

    fn file_card(&mut self, ui: &mut Ui) {
        let hovering = ui.ctx().input(|i| !i.raw.hovered_files.is_empty());
        let stroke = if hovering {
            Stroke::new(2.0, ACCENT)
        } else {
            Stroke::new(1.0, ui.visuals().widgets.noninteractive.bg_stroke.color)
        };
        let response = card(ui, stroke, |ui| {
            ui.horizontal(|ui| {
                ui.set_min_height(64.0);
                page_glyph(ui, self.file.as_ref().map(|f| f.pdf.page_count()));
                ui.add_space(8.0);
                ui.vertical(|ui| {
                    ui.add_space(6.0);
                    match &self.file {
                        _ if hovering => {
                            ui.label(
                                RichText::new("Drop to open")
                                    .size(17.0)
                                    .strong()
                                    .color(ACCENT),
                            );
                            ui.label(
                                RichText::new("Release the mouse button to load the PDF").weak(),
                            );
                        }
                        Some(file) => {
                            let name = file.path.file_name().unwrap_or_default().to_string_lossy();
                            ui.label(RichText::new(name).size(17.0).strong());
                            let pages = file.pdf.page_count();
                            let folder = shorten(
                                &file
                                    .path
                                    .parent()
                                    .unwrap_or(Path::new(""))
                                    .display()
                                    .to_string(),
                                52,
                            );
                            ui.label(
                                RichText::new(format!(
                                    "{pages} page{}  •  {}  •  {folder}",
                                    plural(pages),
                                    human_size(file.size)
                                ))
                                .weak(),
                            )
                            .on_hover_text(file.path.display().to_string());
                        }
                        None => {
                            ui.label(RichText::new("No file selected").size(17.0).strong());
                            ui.label(
                                RichText::new(
                                    "Drop a PDF anywhere on this window, or click Browse",
                                )
                                .weak(),
                            );
                        }
                    }
                });
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let browse = ui.add_enabled(
                        !self.busy(),
                        Button::new("Browse…").min_size(Vec2::new(110.0, 34.0)),
                    );
                    if browse.clicked() {
                        self.browse_file();
                    }
                });
            });
        });
        if response
            .response
            .interact(egui::Sense::click())
            .double_clicked()
            && !self.busy()
        {
            self.browse_file();
        }
    }

    fn options_card(&mut self, ui: &mut Ui) {
        let stroke = Stroke::new(1.0, ui.visuals().widgets.noninteractive.bg_stroke.color);
        card(ui, stroke, |ui| {
            ui.add_enabled_ui(!self.busy(), |ui| {
                egui::Grid::new("options")
                    .num_columns(2)
                    .spacing([16.0, 12.0])
                    .min_col_width(120.0)
                    .show(ui, |ui| {
                        ui.label(RichText::new("Split mode").strong());
                        ui.horizontal_wrapped(|ui| {
                            for mode in Mode::ALL {
                                ui.selectable_value(&mut self.mode, mode, mode.title());
                            }
                        });
                        ui.end_row();

                        ui.label("");
                        ui.label(RichText::new(self.mode.description()).weak());
                        ui.end_row();

                        match self.mode {
                            Mode::EveryPage => {}
                            Mode::EveryN | Mode::EqualParts => {
                                let label = if self.mode == Mode::EveryN {
                                    "Pages per file"
                                } else {
                                    "Number of files"
                                };
                                ui.label(RichText::new(label).strong());
                                let idx = mode_index(self.mode);
                                ui.horizontal(|ui| {
                                    let mut n: usize = self.inputs[idx].trim().parse().unwrap_or(1);
                                    let max = self
                                        .file
                                        .as_ref()
                                        .map_or(9999, |f| f.pdf.page_count().max(1));
                                    if ui
                                        .add(egui::DragValue::new(&mut n).range(1..=max).speed(0.1))
                                        .changed()
                                    {
                                        self.inputs[idx] = n.to_string();
                                    }
                                });
                                ui.end_row();
                            }
                            Mode::Ranges | Mode::Extract => {
                                ui.label(RichText::new("Pages").strong());
                                let idx = mode_index(self.mode);
                                ui.add(
                                    TextEdit::singleline(&mut self.inputs[idx])
                                        .hint_text("e.g. 1-3, 5, 8-last")
                                        .desired_width(f32::INFINITY),
                                );
                                ui.end_row();
                            }
                        }

                        ui.label(RichText::new("Output folder").strong());
                        ui.horizontal(|ui| {
                            let hint = "Default: a “<name>_split” folder next to the PDF";
                            ui.add(
                                TextEdit::singleline(&mut self.out_dir)
                                    .hint_text(hint)
                                    .desired_width((ui.available_width() - 100.0).max(160.0)),
                            );
                            if ui
                                .add(Button::new("Choose…").min_size(Vec2::new(90.0, 0.0)))
                                .clicked()
                            {
                                self.browse_output();
                            }
                        });
                        ui.end_row();

                        ui.label(RichText::new("File name prefix").strong());
                        ui.add(
                            TextEdit::singleline(&mut self.prefix)
                                .hint_text("Defaults to the PDF name")
                                .desired_width(f32::INFINITY),
                        );
                        ui.end_row();

                        ui.label("");
                        ui.checkbox(&mut self.overwrite, "Overwrite existing files");
                        ui.end_row();
                    });
            });

            ui.add_space(8.0);
            self.preview(ui);
        });
    }

    fn preview(&self, ui: &mut Ui) {
        let dark = ui.visuals().dark_mode;
        match self.groups() {
            None => {
                ui.label(RichText::new("Open a PDF to see which files will be created.").weak());
            }
            Some(Err(err)) => {
                ui.label(RichText::new(format!("⚠  {err}")).color(if dark {
                    ERROR_DARK
                } else {
                    ERROR_LIGHT
                }));
            }
            Some(Ok(groups)) => {
                let count = self.file.as_ref().map_or(0, |f| f.pdf.page_count());
                let prefix = self.effective_prefix();
                let n = groups.len();
                let names: Vec<String> = groups
                    .iter()
                    .take(3)
                    .map(|g| output_name(&prefix, g, count))
                    .collect();
                let more = if n > 3 {
                    format!(", … (+{} more)", n - 3)
                } else {
                    String::new()
                };
                let color = if dark { ACCENT_HOVER } else { ACCENT };
                ui.label(
                    RichText::new(format!(
                        "→  {n} file{} will be created:  {}{more}",
                        plural(n),
                        names.join(", ")
                    ))
                    .color(color),
                );
            }
        }
    }

    fn footer(&mut self, ui: &mut Ui) {
        let dark = ui.visuals().dark_mode;
        ui.horizontal(|ui| {
            let buttons_width = 300.0;
            ui.vertical(|ui| {
                ui.set_width((ui.available_width() - buttons_width).max(150.0));
                let fraction = match self.status {
                    Status::Working { done, total } => done as f32 / total.max(1) as f32,
                    Status::Done { .. } => 1.0,
                    _ => 0.0,
                };
                ui.add_space(6.0);
                let fill = if fraction > 0.0 {
                    ACCENT
                } else {
                    Color32::TRANSPARENT
                };
                ui.add(ProgressBar::new(fraction).desired_height(8.0).fill(fill));
                ui.add_space(2.0);
                let text = match &self.status {
                    Status::Idle(s) => RichText::new(s).weak(),
                    Status::Working { done, total } => RichText::new(format!(
                        "Writing file {} of {total}…",
                        (done + 1).min(*total)
                    ))
                    .weak(),
                    Status::Done { count, dir } => RichText::new(format!(
                        "✔  Created {count} file{} in “{}”",
                        plural(*count),
                        dir.file_name().unwrap_or_default().to_string_lossy()
                    ))
                    .color(if dark { SUCCESS_DARK } else { SUCCESS_LIGHT }),
                    Status::Failed(s) => {
                        RichText::new(s).color(if dark { ERROR_DARK } else { ERROR_LIGHT })
                    }
                };
                ui.label(text);
            });

            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if self.busy() {
                    let cancel = ui.add(
                        Button::new(RichText::new("Cancel").size(15.0).strong())
                            .min_size(Vec2::new(150.0, 42.0)),
                    );
                    if cancel.clicked()
                        && let Some(job) = &self.job
                    {
                        job.cancel.store(true, Ordering::Relaxed);
                    }
                } else {
                    let ready = matches!(self.groups(), Some(Ok(_)));
                    let split = ui.add_enabled(
                        ready,
                        Button::new(
                            RichText::new("Split PDF")
                                .size(15.0)
                                .strong()
                                .color(Color32::WHITE),
                        )
                        .fill(ACCENT)
                        .min_size(Vec2::new(150.0, 42.0)),
                    );
                    if split.clicked() {
                        self.start_split(ui.ctx());
                    }
                }
                let can_open = self.last_output.is_some() && !self.busy();
                if ui
                    .add_enabled(
                        can_open,
                        Button::new("Open folder").min_size(Vec2::new(120.0, 42.0)),
                    )
                    .clicked()
                    && let Some(dir) = &self.last_output
                {
                    open_folder(dir);
                }
            });
        });
    }

    fn dialogs(&mut self, ctx: &egui::Context) {
        if let Some(prompt) = &mut self.password {
            let mut submit = None;
            let mut cancel = false;
            egui::Modal::new(egui::Id::unique("password")).show(ctx, |ui| {
                ui.set_width(340.0);
                ui.label(RichText::new("Password required").size(18.0).strong());
                ui.add_space(4.0);
                ui.label(format!(
                    "“{}” is password protected.",
                    prompt
                        .path
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                ));
                ui.add_space(8.0);
                let edit = ui.add(
                    TextEdit::singleline(&mut prompt.password)
                        .password(true)
                        .hint_text("Password")
                        .desired_width(f32::INFINITY),
                );
                if prompt.focus {
                    edit.request_focus();
                    prompt.focus = false;
                }
                if let Some(err) = &prompt.error {
                    ui.label(RichText::new(err).color(if ui.visuals().dark_mode {
                        ERROR_DARK
                    } else {
                        ERROR_LIGHT
                    }));
                }
                let enter = edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                ui.add_space(8.0);
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let open = Button::new(RichText::new("Open").color(Color32::WHITE))
                        .fill(ACCENT)
                        .min_size(Vec2::new(80.0, 30.0));
                    if ui.add(open).clicked() || enter {
                        submit = Some((prompt.path.clone(), prompt.password.clone()));
                    }
                    if ui
                        .add(Button::new("Cancel").min_size(Vec2::new(80.0, 30.0)))
                        .clicked()
                    {
                        cancel = true;
                    }
                });
                if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                    cancel = true;
                }
            });
            if cancel {
                self.password = None;
            } else if let Some((path, pw)) = submit {
                self.password = None;
                self.load(path, Some(&pw));
            }
        }

        if let Some(message) = &self.error_dialog {
            let mut close = false;
            egui::Modal::new(egui::Id::unique("error")).show(ctx, |ui| {
                ui.set_width(360.0);
                ui.label(RichText::new("Something went wrong").size(18.0).strong());
                ui.add_space(6.0);
                ui.label(message);
                ui.add_space(10.0);
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if ui
                        .add(Button::new("OK").min_size(Vec2::new(80.0, 30.0)))
                        .clicked()
                        || ui.input(|i| {
                            i.key_pressed(egui::Key::Enter) || i.key_pressed(egui::Key::Escape)
                        })
                    {
                        close = true;
                    }
                });
            });
            if close {
                self.error_dialog = None;
            }
        }
    }
}

impl eframe::App for SplitterApp {
    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.poll_job();
        self.handle_dropped_files(&ctx);

        let bg = ui.visuals().panel_fill;
        egui::CentralPanel::default()
            .frame(Frame::new().fill(bg).inner_margin(Margin::same(24)))
            .show(ui, |ui| {
                self.header(ui);
                ui.add_space(16.0);
                self.file_card(ui);
                ui.add_space(12.0);
                egui::ScrollArea::vertical()
                    .max_height((ui.available_height() - 70.0).max(120.0))
                    .auto_shrink([false, true])
                    .show(ui, |ui| self.options_card(ui));
                ui.add_space(12.0);
                ui.with_layout(Layout::bottom_up(Align::Min), |ui| self.footer(ui));
            });

        self.dialogs(&ctx);
    }
}

// ---------------------------------------------------------------- helpers

fn mode_index(mode: Mode) -> usize {
    Mode::ALL.iter().position(|m| *m == mode).unwrap_or(0)
}

fn card<R>(ui: &mut Ui, stroke: Stroke, add: impl FnOnce(&mut Ui) -> R) -> egui::InnerResponse<R> {
    let fill = if ui.visuals().dark_mode {
        Color32::from_gray(36)
    } else {
        Color32::WHITE
    };
    Frame::new()
        .fill(fill)
        .stroke(stroke)
        .corner_radius(CornerRadius::same(14))
        .inner_margin(Margin::same(18))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            add(ui)
        })
}

/// Small app logo drawn with the painter (no image assets needed).
fn pdf_glyph(ui: &mut Ui, size: f32) {
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(size), egui::Sense::hover());
    let p = ui.painter();
    p.rect_filled(rect, CornerRadius::same((size * 0.22) as u8), ACCENT);
    let w = rect.width();
    let top = egui::Rect::from_min_max(
        rect.min + Vec2::new(w * 0.25, w * 0.16),
        rect.min + Vec2::new(w * 0.75, w * 0.45),
    );
    let bottom = egui::Rect::from_min_max(
        rect.min + Vec2::new(w * 0.25, w * 0.55),
        rect.min + Vec2::new(w * 0.75, w * 0.84),
    );
    p.rect_filled(top, CornerRadius::same(2), Color32::WHITE);
    p.rect_filled(bottom, CornerRadius::same(2), Color32::WHITE);
    let y = rect.center().y;
    let mut x = rect.left() + w * 0.12;
    while x < rect.right() - w * 0.12 {
        p.line_segment(
            [egui::pos2(x, y), egui::pos2(x + w * 0.07, y)],
            Stroke::new(2.0, Color32::from_rgb(250, 204, 21)),
        );
        x += w * 0.13;
    }
}

/// Document icon with the page count, shown in the file card.
fn page_glyph(ui: &mut Ui, pages: Option<usize>) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(52.0, 66.0), egui::Sense::hover());
    let p = ui.painter();
    let dark = ui.visuals().dark_mode;
    let paper = if dark {
        Color32::from_gray(60)
    } else {
        Color32::from_gray(236)
    };
    let lines = if dark {
        Color32::from_gray(90)
    } else {
        Color32::from_gray(205)
    };
    p.rect_filled(rect, CornerRadius::same(6), paper);
    for i in 0..5 {
        let y = rect.top() + 12.0 + i as f32 * 7.0;
        let len = if i % 2 == 0 { 30.0 } else { 22.0 };
        p.line_segment(
            [
                egui::pos2(rect.left() + 10.0, y),
                egui::pos2(rect.left() + 10.0 + len, y),
            ],
            Stroke::new(2.0, lines),
        );
    }
    let badge = egui::Rect::from_min_max(
        egui::pos2(rect.left() + 4.0, rect.bottom() - 20.0),
        egui::pos2(rect.right() - 4.0, rect.bottom() - 4.0),
    );
    p.rect_filled(badge, CornerRadius::same(4), ACCENT);
    let text = pages.map_or("PDF".to_string(), |n| n.to_string());
    p.text(
        badge.center(),
        egui::Align2::CENTER_CENTER,
        text,
        FontId::proportional(11.0),
        Color32::WHITE,
    );
}

fn configure_style(ctx: &egui::Context) {
    ctx.all_styles_mut(|style| {
        style.spacing.item_spacing = Vec2::new(8.0, 8.0);
        style.spacing.button_padding = Vec2::new(12.0, 6.0);
        style.spacing.interact_size.y = 30.0;
        for font in style.text_styles.values_mut() {
            font.size *= 1.1;
        }
        let v = &mut style.visuals;
        v.selection.bg_fill = ACCENT;
        v.selection.stroke = Stroke::new(1.0, Color32::WHITE);
        v.hyperlink_color = ACCENT_HOVER;
        for w in [
            &mut v.widgets.inactive,
            &mut v.widgets.hovered,
            &mut v.widgets.active,
            &mut v.widgets.open,
            &mut v.widgets.noninteractive,
        ] {
            w.corner_radius = CornerRadius::same(8);
        }
    });
    ctx.style_mut_of(egui::Theme::Light, |s| {
        s.visuals.widgets.inactive.bg_stroke = Stroke::new(1.0, Color32::from_gray(210));
        s.visuals.panel_fill = Color32::from_rgb(243, 244, 246);
        s.visuals.extreme_bg_color = Color32::from_rgb(249, 250, 251);
    });
    ctx.style_mut_of(egui::Theme::Dark, |s| {
        s.visuals.widgets.inactive.bg_stroke = Stroke::new(1.0, Color32::from_gray(64));
        s.visuals.panel_fill = Color32::from_gray(24);
        s.visuals.extreme_bg_color = Color32::from_gray(20);
    });
}

fn is_pdf(path: &Path) -> bool {
    path.extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("pdf"))
}

fn plural(n: usize) -> &'static str {
    if n == 1 { "" } else { "s" }
}

fn human_size(bytes: u64) -> String {
    let kb = bytes as f64 / 1024.0;
    if kb >= 1024.0 {
        format!("{:.1} MB", kb / 1024.0)
    } else {
        format!("{kb:.0} KB")
    }
}

fn shorten(s: &str, max: usize) -> String {
    let chars: Vec<char> = s.chars().collect();
    if chars.len() <= max {
        s.to_string()
    } else {
        let tail: String = chars[chars.len() - (max - 1)..].iter().collect();
        format!("…{tail}")
    }
}

/// `canonicalize` on Windows returns `\\?\C:\...` paths; strip that prefix for display.
fn strip_unc(path: PathBuf) -> PathBuf {
    let s = path.to_string_lossy();
    match s.strip_prefix(r"\\?\") {
        Some(rest) if !rest.starts_with("UNC\\") => PathBuf::from(rest),
        _ => path,
    }
}

fn open_folder(path: &Path) {
    #[cfg(target_os = "windows")]
    let cmd = "explorer";
    #[cfg(target_os = "macos")]
    let cmd = "open";
    #[cfg(all(unix, not(target_os = "macos")))]
    let cmd = "xdg-open";
    let _ = std::process::Command::new(cmd).arg(path).spawn();
}
