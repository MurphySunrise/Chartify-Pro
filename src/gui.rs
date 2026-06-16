use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::mpsc::{self, Receiver};
use std::thread;

use anyhow::{Context, Result, bail};
use eframe::egui;
use rfd::FileDialog;

use super::{Args, Structure, process};

const DEFAULT_WINDOW_WIDTH: f32 = 760.0;
const DEFAULT_WINDOW_HEIGHT: f32 = 900.0;
const MIN_WINDOW_WIDTH: f32 = 760.0;
const MIN_WINDOW_HEIGHT: f32 = 720.0;

pub(super) fn launch() -> Result<()> {
    let app_icon = eframe::icon_data::from_png_bytes(include_bytes!("../assets/chartify.png"))
        .context("Unable to load the embedded Chartify icon")?;
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([DEFAULT_WINDOW_WIDTH, DEFAULT_WINDOW_HEIGHT])
            .with_min_inner_size([MIN_WINDOW_WIDTH, MIN_WINDOW_HEIGHT])
            .with_icon(app_icon),
        ..Default::default()
    };
    eframe::run_native(
        "Chartify Pro",
        options,
        Box::new(|creation_context| Ok(Box::new(ChartifyApp::new(creation_context)))),
    )
    .map_err(|error| anyhow::anyhow!(error.to_string()))
}

struct ChartifyApp {
    input: Option<PathBuf>,
    ppt_template: Option<PathBuf>,
    structure: Structure,
    columns: Vec<String>,
    selected_data_cols: HashSet<String>,
    group_col: String,
    item_col: String,
    groups: Vec<String>,
    control_group: String,
    sample_size: String,
    status: String,
    running: bool,
    generated_ppt: Option<PathBuf>,
    receiver: Option<Receiver<WorkerResult>>,
}

enum WorkerResult {
    Complete {
        message: String,
        ppt_output: Option<PathBuf>,
    },
    Failed(String),
}

impl ChartifyApp {
    fn new(creation_context: &eframe::CreationContext<'_>) -> Self {
        let context = &creation_context.egui_ctx;
        context.set_zoom_factor(1.05);
        context.set_theme(egui::Theme::Light);
        context.send_viewport_cmd(egui::ViewportCommand::SetTheme(egui::SystemTheme::Light));

        let mut visuals = egui::Visuals::light();
        visuals.panel_fill = egui::Color32::WHITE;
        visuals.window_fill = egui::Color32::WHITE;
        visuals.faint_bg_color = egui::Color32::from_rgb(247, 249, 249);
        visuals.extreme_bg_color = egui::Color32::from_rgb(247, 249, 249);
        visuals.text_edit_bg_color = Some(egui::Color32::from_rgb(247, 249, 249));
        visuals.override_text_color = Some(egui::Color32::from_rgb(37, 48, 52));
        visuals.selection.bg_fill = egui::Color32::from_rgb(18, 126, 128);
        visuals.widgets.inactive.bg_fill = egui::Color32::from_rgb(247, 249, 249);
        visuals.widgets.inactive.bg_stroke =
            egui::Stroke::new(1.0, egui::Color32::from_rgb(211, 219, 221));
        visuals.widgets.hovered.bg_fill = egui::Color32::from_rgb(239, 245, 245);
        visuals.widgets.hovered.bg_stroke =
            egui::Stroke::new(1.0, egui::Color32::from_rgb(18, 126, 128));
        visuals.widgets.active.bg_fill = egui::Color32::from_rgb(226, 239, 239);
        visuals.widgets.active.bg_stroke =
            egui::Stroke::new(1.0, egui::Color32::from_rgb(18, 126, 128));
        context.set_visuals(visuals);
        context.global_style_mut(|style| {
            style.spacing.item_spacing = egui::vec2(10.0, 8.0);
            style.spacing.button_padding = egui::vec2(12.0, 7.0);
            style.spacing.interact_size.y = 32.0;
            style.text_styles.insert(
                egui::TextStyle::Heading,
                egui::FontId::new(25.0, egui::FontFamily::Proportional),
            );
            style.text_styles.insert(
                egui::TextStyle::Button,
                egui::FontId::new(14.0, egui::FontFamily::Proportional),
            );
        });
        Self {
            input: None,
            ppt_template: None,
            structure: Structure::Single,
            columns: Vec::new(),
            selected_data_cols: HashSet::new(),
            group_col: String::new(),
            item_col: String::new(),
            groups: Vec::new(),
            control_group: String::new(),
            sample_size: "10000".to_owned(),
            status: "Select a CSV file".to_owned(),
            running: false,
            generated_ppt: None,
            receiver: None,
        }
    }

    fn select_input(&mut self) {
        let Some(path) = FileDialog::new().add_filter("CSV", &["csv"]).pick_file() else {
            return;
        };
        match read_headers(&path) {
            Ok(columns) => {
                self.columns = columns;
                self.input = Some(path);
                self.selected_data_cols.clear();
                self.group_col.clear();
                self.item_col.clear();
                self.groups.clear();
                self.control_group.clear();
                self.generated_ppt = None;
                self.status = "CSV loaded".to_owned();
            }
            Err(error) => self.status = format!("Error: {error:#}"),
        }
    }

    fn refresh_groups(&mut self) {
        let Some(input) = &self.input else {
            return;
        };
        if self.group_col.is_empty() {
            self.groups.clear();
            self.control_group.clear();
            return;
        }
        match read_unique_values(input, &self.group_col) {
            Ok(groups) => {
                self.groups = groups;
                if !self.groups.contains(&self.control_group) {
                    self.control_group = self.groups.first().cloned().unwrap_or_default();
                }
            }
            Err(error) => self.status = format!("Error: {error:#}"),
        }
    }

    fn start(&mut self) {
        match self.build_args() {
            Ok(args) => {
                let (sender, receiver) = mpsc::channel();
                self.receiver = Some(receiver);
                self.running = true;
                self.generated_ppt = None;
                self.status = "Processing...".to_owned();
                thread::spawn(move || {
                    let message = match process(&args) {
                        Ok(summary) => {
                            let output_name = summary
                                .ppt_output
                                .as_deref()
                                .map(file_name)
                                .unwrap_or_else(|| "unknown file".to_owned());
                            WorkerResult::Complete {
                                message: format!(
                                    "Complete: {} charts, {} ignored values. Saved as {}",
                                    summary.chart_count, summary.invalid_values, output_name
                                ),
                                ppt_output: summary.ppt_output,
                            }
                        }
                        Err(error) => WorkerResult::Failed(format!("{error:#}")),
                    };
                    let _ = sender.send(message);
                });
            }
            Err(error) => self.status = format!("Error: {error:#}"),
        }
    }

    fn build_args(&self) -> Result<Args> {
        let input = self.input.clone().context("Select a CSV file")?;
        let ppt_template = self
            .ppt_template
            .clone()
            .context("Select a PowerPoint template")?;
        if !ppt_template.is_file() {
            bail!("The selected PowerPoint template does not exist");
        }
        if self.group_col.is_empty() {
            bail!("Select a group column");
        }
        if self.control_group.is_empty() {
            bail!("Select a control group");
        }
        if self.structure == Structure::Single && self.item_col.is_empty() {
            bail!("Select an item column");
        }
        let mut data_cols = self
            .columns
            .iter()
            .filter(|column| self.selected_data_cols.contains(*column))
            .cloned()
            .collect::<Vec<_>>();
        if data_cols.is_empty() {
            bail!("Select at least one data column");
        }
        if self.structure == Structure::Single && data_cols.len() != 1 {
            bail!("Single structure requires exactly one data column");
        }
        data_cols.sort_by_key(|column| {
            self.columns
                .iter()
                .position(|candidate| candidate == column)
                .unwrap_or(usize::MAX)
        });
        let sample_size = self
            .sample_size
            .parse::<usize>()
            .context("Sample size must be a positive integer")?;
        if sample_size == 0 {
            bail!("Sample size must be greater than zero");
        }
        let parent = input.parent().unwrap_or_else(|| Path::new("."));
        let stem = input
            .file_stem()
            .and_then(|value| value.to_str())
            .unwrap_or("Chartify");
        let ppt_output = parent.join(format!("{stem}_Statistic_Report.pptx"));

        Ok(Args {
            input,
            output: None,
            structure: self.structure,
            group_col: self.group_col.clone(),
            item_col: (self.structure == Structure::Single).then(|| self.item_col.clone()),
            data_cols,
            control_group: self.control_group.clone(),
            charts_dir: None,
            sample_size,
            ppt_template: Some(ppt_template),
            ppt_output: Some(ppt_output),
        })
    }

    fn poll_worker(&mut self) {
        let message = self
            .receiver
            .as_ref()
            .and_then(|receiver| receiver.try_recv().ok());
        if let Some(message) = message {
            self.running = false;
            self.receiver = None;
            self.status = match message {
                WorkerResult::Complete {
                    message,
                    ppt_output,
                } => {
                    self.generated_ppt = ppt_output;
                    message
                }
                WorkerResult::Failed(text) => format!("Error: {text}"),
            };
        }
    }

    fn open_generated_ppt(&mut self) {
        let Some(path) = self.generated_ppt.as_deref() else {
            return;
        };
        if let Err(error) = open_with_default_app(path) {
            self.status = format!("Error: {error:#}");
        }
    }
}

impl eframe::App for ChartifyApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.poll_worker();
        ui.painter()
            .rect_filled(ui.max_rect(), 0.0, egui::Color32::WHITE);
        if self.running {
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(100));
        }

        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                egui::Frame::new()
                    .fill(egui::Color32::WHITE)
                    .inner_margin(egui::Margin::symmetric(28, 22))
                    .show(ui, |ui| {
                        egui::Frame::new()
                            .fill(egui::Color32::WHITE)
                            .inner_margin(egui::Margin::symmetric(0, 8))
                            .show(ui, |ui| {
                                ui.horizontal(|ui| {
                                    ui.vertical(|ui| {
                                        ui.label(
                                            egui::RichText::new("Chartify Pro")
                                                .size(26.0)
                                                .strong()
                                                .color(egui::Color32::from_rgb(29, 41, 45)),
                                        );
                                        ui.label(
                                            egui::RichText::new(
                                                "Statistical comparison report builder",
                                            )
                                            .size(13.0)
                                            .color(egui::Color32::from_rgb(105, 119, 124)),
                                        );
                                    });
                                });
                            });

                        ui.add_space(12.0);
                        let accent_rect = ui
                            .allocate_exact_size(
                                egui::vec2(ui.available_width(), 3.0),
                                egui::Sense::hover(),
                            )
                            .0;
                        ui.painter().rect_filled(
                            accent_rect,
                            0.0,
                            egui::Color32::from_rgb(18, 126, 128),
                        );

                        ui.add_space(16.0);
                        section_heading(ui, "FILES", "CSV input and PowerPoint template");
                        egui::Frame::new()
                            .fill(egui::Color32::WHITE)
                            .stroke(egui::Stroke::new(
                                1.0,
                                egui::Color32::from_rgb(218, 224, 226),
                            ))
                            .corner_radius(6)
                            .inner_margin(egui::Margin::symmetric(14, 10))
                            .show(ui, |ui| {
                                if file_row(
                                    ui,
                                    "Source CSV",
                                    self.input.as_deref(),
                                    "Browse",
                                    !self.running,
                                ) {
                                    self.select_input();
                                }
                                ui.separator();
                                if file_row(
                                    ui,
                                    "PPT template",
                                    self.ppt_template.as_deref(),
                                    "Choose",
                                    !self.running,
                                ) && let Some(path) = FileDialog::new()
                                    .add_filter("PowerPoint", &["pptx"])
                                    .pick_file()
                                {
                                    self.ppt_template = Some(path);
                                }
                            });

                        ui.add_space(18.0);
                        section_heading(ui, "DATA SETUP", "Map CSV columns to the report");
                        ui.columns(2, |columns| {
                            columns[0].vertical(|ui| {
                                field_label(ui, "Table structure");
                                ui.horizontal(|ui| {
                                    segmented_value(
                                        ui,
                                        &mut self.structure,
                                        Structure::Single,
                                        "Single",
                                    );
                                    segmented_value(
                                        ui,
                                        &mut self.structure,
                                        Structure::Multiple,
                                        "Multiple",
                                    );
                                });
                                ui.add_space(8.0);

                                let previous_group = self.group_col.clone();
                                field_label(ui, "Group column");
                                combo_box(ui, "group_column", &mut self.group_col, &self.columns);
                                if previous_group != self.group_col {
                                    self.refresh_groups();
                                }

                                field_label(ui, "Control group");
                                combo_box(
                                    ui,
                                    "control_group",
                                    &mut self.control_group,
                                    &self.groups,
                                );

                                if self.structure == Structure::Single {
                                    field_label(ui, "Item column");
                                    combo_box(ui, "item_column", &mut self.item_col, &self.columns);
                                }

                                field_label(ui, "Maximum plotted values per group");
                                ui.add(
                                    egui::TextEdit::singleline(&mut self.sample_size)
                                        .desired_width(f32::INFINITY),
                                );
                            });

                            columns[1].vertical(|ui| {
                                ui.horizontal(|ui| {
                                    field_label(ui, "Data columns");
                                    ui.with_layout(
                                        egui::Layout::right_to_left(egui::Align::Center),
                                        |ui| {
                                            ui.label(
                                                egui::RichText::new(format!(
                                                    "{} selected",
                                                    self.selected_data_cols.len()
                                                ))
                                                .size(12.0)
                                                .color(egui::Color32::from_rgb(80, 104, 113)),
                                            );
                                        },
                                    );
                                });

                                egui::Frame::new()
                                    .fill(egui::Color32::from_rgb(250, 251, 251))
                                    .stroke(egui::Stroke::new(
                                        1.0,
                                        egui::Color32::from_rgb(211, 219, 221),
                                    ))
                                    .corner_radius(5)
                                    .inner_margin(egui::Margin::same(10))
                                    .show(ui, |ui| {
                                        egui::ScrollArea::vertical()
                                            .max_height(242.0)
                                            .auto_shrink([false, false])
                                            .show(ui, |ui| {
                                                if self.columns.is_empty() {
                                                    ui.add_space(70.0);
                                                    ui.vertical_centered(|ui| {
                                                        ui.label(
                                                            egui::RichText::new(
                                                                "Select a CSV file to view columns",
                                                            )
                                                            .color(egui::Color32::from_gray(120)),
                                                        );
                                                    });
                                                } else {
                                                    for column in &self.columns {
                                                        let mut checked = self
                                                            .selected_data_cols
                                                            .contains(column);
                                                        if ui
                                                            .checkbox(&mut checked, column)
                                                            .changed()
                                                        {
                                                            if checked {
                                                                if self.structure
                                                                    == Structure::Single
                                                                {
                                                                    self.selected_data_cols.clear();
                                                                }
                                                                self.selected_data_cols
                                                                    .insert(column.clone());
                                                            } else {
                                                                self.selected_data_cols
                                                                    .remove(column);
                                                            }
                                                        }
                                                    }
                                                }
                                            });
                                    });
                                ui.horizontal(|ui| {
                                    if self.structure == Structure::Multiple
                                        && ui.small_button("Select all").clicked()
                                    {
                                        self.selected_data_cols =
                                            self.columns.iter().cloned().collect();
                                    }
                                    if ui.small_button("Clear").clicked() {
                                        self.selected_data_cols.clear();
                                    }
                                });
                            });
                        });

                        ui.add_space(18.0);
                        ui.horizontal(|ui| {
                            let button = egui::Button::new(
                                egui::RichText::new(if self.running {
                                    "Generating report..."
                                } else {
                                    "Generate report"
                                })
                                .strong()
                                .color(egui::Color32::WHITE),
                            )
                            .fill(egui::Color32::from_rgb(18, 126, 128))
                            .stroke(egui::Stroke::NONE)
                            .corner_radius(5)
                            .min_size(egui::vec2(184.0, 40.0));
                            if ui.add_enabled(!self.running, button).clicked() {
                                self.start();
                            }
                            let open_button = egui::Button::new("Open PPT")
                                .corner_radius(5)
                                .min_size(egui::vec2(104.0, 40.0));
                            if ui
                                .add_enabled(
                                    !self.running && self.generated_ppt.is_some(),
                                    open_button,
                                )
                                .on_hover_text("Open the generated report")
                                .clicked()
                            {
                                self.open_generated_ppt();
                            }
                            if self.running {
                                ui.spinner();
                            }
                        });

                        ui.add_space(12.0);
                        status_bar(ui, &self.status, self.running);
                    });
            });
    }
}

fn combo_box(ui: &mut egui::Ui, id: &str, selected: &mut String, options: &[String]) {
    egui::ComboBox::from_id_salt(id)
        .width(ui.available_width())
        .selected_text(if selected.is_empty() {
            "Select...".to_owned()
        } else {
            selected.clone()
        })
        .show_ui(ui, |ui| {
            for option in options {
                ui.selectable_value(selected, option.clone(), option);
            }
        });
}

fn file_row(
    ui: &mut egui::Ui,
    label: &str,
    path: Option<&Path>,
    button_text: &str,
    enabled: bool,
) -> bool {
    let mut clicked = false;
    ui.horizontal(|ui| {
        ui.add_sized(
            [112.0, 24.0],
            egui::Label::new(
                egui::RichText::new(label)
                    .strong()
                    .color(egui::Color32::from_rgb(52, 70, 77)),
            ),
        );
        let available = (ui.available_width() - 92.0).max(120.0);
        ui.add_sized(
            [available, 24.0],
            egui::Label::new(
                egui::RichText::new(path_text(path))
                    .monospace()
                    .size(12.0)
                    .color(egui::Color32::from_rgb(80, 96, 102)),
            )
            .truncate(),
        )
        .on_hover_text(path_text(path));
        if ui
            .add_enabled(
                enabled,
                egui::Button::new(button_text).min_size([78.0, 28.0].into()),
            )
            .clicked()
        {
            clicked = true;
        }
    });
    clicked
}

fn section_heading(ui: &mut egui::Ui, title: &str, subtitle: &str) {
    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new(title)
                .size(12.0)
                .strong()
                .color(egui::Color32::from_rgb(18, 126, 128)),
        );
        ui.label(
            egui::RichText::new(subtitle)
                .size(12.0)
                .color(egui::Color32::from_rgb(102, 116, 122)),
        );
    });
    ui.add_space(5.0);
}

fn field_label(ui: &mut egui::Ui, text: &str) {
    ui.label(
        egui::RichText::new(text)
            .size(12.0)
            .strong()
            .color(egui::Color32::from_rgb(62, 77, 83)),
    );
}

fn segmented_value(ui: &mut egui::Ui, current: &mut Structure, value: Structure, label: &str) {
    let selected = *current == value;
    let button = egui::Button::new(egui::RichText::new(label).color(if selected {
        egui::Color32::WHITE
    } else {
        egui::Color32::from_rgb(55, 72, 79)
    }))
    .fill(if selected {
        egui::Color32::from_rgb(18, 126, 128)
    } else {
        egui::Color32::from_rgb(236, 240, 241)
    })
    .stroke(egui::Stroke::new(
        1.0,
        egui::Color32::from_rgb(190, 202, 206),
    ))
    .corner_radius(4)
    .min_size(egui::vec2(104.0, 32.0));
    if ui.add(button).clicked() {
        *current = value;
    }
}

fn status_bar(ui: &mut egui::Ui, status: &str, running: bool) {
    let (background, accent) = if status.starts_with("Error:") {
        (
            egui::Color32::from_rgb(255, 240, 238),
            egui::Color32::from_rgb(188, 58, 46),
        )
    } else if status.starts_with("Complete:") {
        (
            egui::Color32::from_rgb(235, 247, 241),
            egui::Color32::from_rgb(42, 126, 83),
        )
    } else if running {
        (
            egui::Color32::from_rgb(236, 246, 247),
            egui::Color32::from_rgb(18, 126, 128),
        )
    } else {
        (
            egui::Color32::from_rgb(243, 245, 246),
            egui::Color32::from_rgb(85, 101, 107),
        )
    };
    egui::Frame::new()
        .fill(background)
        .corner_radius(4)
        .inner_margin(egui::Margin::symmetric(12, 9))
        .show(ui, |ui| {
            ui.label(egui::RichText::new(status).color(accent).strong());
        });
}

fn read_headers(path: &Path) -> Result<Vec<String>> {
    let mut reader = csv::Reader::from_path(path)
        .with_context(|| format!("Unable to open {}", path.display()))?;
    Ok(reader.headers()?.iter().map(str::to_owned).collect())
}

fn read_unique_values(path: &Path, column: &str) -> Result<Vec<String>> {
    let mut reader = csv::Reader::from_path(path)
        .with_context(|| format!("Unable to open {}", path.display()))?;
    let headers = reader.headers()?.clone();
    let index = headers
        .iter()
        .position(|header| header == column)
        .with_context(|| format!("Column '{column}' was not found"))?;
    let mut values = Vec::new();
    let mut seen = HashSet::new();
    for record in reader.records() {
        let value = record?.get(index).unwrap_or("").trim().to_owned();
        if !value.is_empty() && seen.insert(value.clone()) {
            values.push(value);
        }
    }
    Ok(values)
}

fn path_text(path: Option<&Path>) -> String {
    path.map(file_name)
        .unwrap_or_else(|| "Not selected".to_owned())
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .and_then(|value| value.to_str())
        .map(str::to_owned)
        .unwrap_or_else(|| path.display().to_string())
}

fn open_with_default_app(path: &Path) -> Result<()> {
    if !path.is_file() {
        bail!("The generated PowerPoint file no longer exists");
    }

    #[cfg(target_os = "macos")]
    let mut command = {
        let mut command = Command::new("open");
        command.arg(path);
        command
    };

    #[cfg(target_os = "windows")]
    let mut command = {
        let mut command = Command::new("cmd");
        command.args(["/C", "start", ""]).arg(path);
        command
    };

    #[cfg(all(unix, not(target_os = "macos")))]
    let mut command = {
        let mut command = Command::new("xdg-open");
        command.arg(path);
        command
    };

    command
        .spawn()
        .with_context(|| format!("Unable to open {}", path.display()))?;
    Ok(())
}
