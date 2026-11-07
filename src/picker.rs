//! The setup screen's model list and the GPU memory meter beside it.
use super::setup::bytes;
use super::*;
use plamo_translate::model_catalog::{
    self, Alternative, Backend, Blocker, GpuMeter, MemoryChange, Model, Recommendation,
};
use std::collections::HashSet;
use theme::{
    ACCENT, ACCENT_SOFT, BORDER, BORDER_STRONG, INPUT, SURFACE, SURFACE_RAISED, TEXT, TEXT_MUTED,
    TEXT_SECONDARY, WARNING,
};

/// Width of the meter column beside the list.
const METER_WIDTH: f32 = 320.0;
/// The picker never grows past this, however wide the window.
const PICKER_MAX_WIDTH: f32 = 1040.0;
/// Below this content width the meter sits above the list instead of beside it.
/// The list needs about 660 for its longest row, plus the meter and the gap.
const SIDE_BY_SIDE_MIN_WIDTH: f32 = 1000.0;
const BAR_HEIGHT: f32 = 12.0;
const RADIO_RADIUS: f32 = 7.0;
/// Room for the widest size label ("0.8B") before the description starts.
const SIZE_COLUMN: f32 = 56.0;

/// What a row's status column says, and whether the row can be chosen.
#[derive(Clone, Debug, PartialEq)]
pub(super) enum RowStatus {
    Blocked(Blocker),
    /// In use. Carries the quantization when it is not the one this row installs.
    InUse(Option<String>),
    Saved,
    Recommended,
    Plain,
}

impl RowStatus {
    pub(super) fn selectable(&self) -> bool {
        !matches!(self, Self::Blocked(_))
    }

    pub(super) fn text(&self) -> Option<String> {
        match self {
            Self::Blocked(blocker) => Some(blocker_text(*blocker)),
            Self::InUse(None) => Some("使用中".into()),
            Self::InUse(Some(quantization)) => Some(format!("使用中（{quantization}）")),
            Self::Saved => Some("保存済み".into()),
            Self::Recommended => Some("おすすめ".into()),
            Self::Plain => None,
        }
    }

    fn color(&self) -> Color32 {
        match self {
            Self::Blocked(_) => WARNING,
            Self::InUse(_) => TEXT,
            Self::Recommended => ACCENT,
            Self::Saved | Self::Plain => TEXT_SECONDARY,
        }
    }
}

/// The reason a row cannot be chosen, with how much is missing.
fn blocker_text(blocker: Blocker) -> String {
    // Round up, so a small shortfall never reads as "0 MB".
    let missing = |bytes_missing: u64| bytes(bytes_missing.next_multiple_of(1_048_576));
    match blocker {
        Blocker::NeedsGpu => "GPUが必要です".into(),
        Blocker::GpuMemory(short) => format!("GPUメモリが足りません（あと{}）", missing(short)),
        Blocker::Memory(short) => format!("メモリが足りません（あと{}）", missing(short)),
        Blocker::Disk(short) => format!("空き容量が足りません（あと{}）", missing(short)),
        Blocker::Unavailable => "このPCでは使えません".into(),
    }
}

fn same_size(a: f32, b: f32) -> bool {
    (a - b).abs() < 0.01
}

fn blurb(parameters_b: f32) -> &'static str {
    if parameters_b < 1.0 {
        "いちばん軽い"
    } else if parameters_b <= 2.0 {
        "軽くて自然"
    } else if parameters_b <= 8.0 {
        "自然な日本語"
    } else {
        "いちばん自然・大きなGPU向け"
    }
}

pub(super) struct PickerRow {
    /// The model this row installs: the size's Q4_K_M file.
    pub id: String,
    pub size: String,
    pub parameters_b: f32,
    pub blurb: &'static str,
    pub bytes: u64,
    pub status: RowStatus,
    /// Shown on hover when the row cannot be chosen.
    pub details: String,
}

/// One row per model size, in catalog order.
pub(super) struct Picker {
    pub rows: Vec<PickerRow>,
    recommended: usize,
}

impl Picker {
    pub(super) fn new(
        recommendation: &Recommendation,
        in_use: Option<&Model>,
        saved: &HashSet<String>,
    ) -> Self {
        let sizes: Vec<&Alternative> = recommendation
            .alternatives
            .iter()
            .filter(|a| a.model.quantization == "Q4_K_M")
            .collect();
        // The recommendation can be another quantization of a size whose Q4_K_M
        // does not fit. Start from the largest size a row can actually choose.
        let recommended = sizes
            .iter()
            .rposition(|a| {
                a.blocker.is_none() && a.model.parameters_b <= recommendation.selected.parameters_b
            })
            .unwrap_or(0);
        let rows = sizes
            .iter()
            .enumerate()
            .map(|(index, alternative)| {
                let model = &alternative.model;
                let status = if let Some(blocker) = alternative.blocker {
                    RowStatus::Blocked(blocker)
                } else if let Some(current) =
                    in_use.filter(|m| same_size(m.parameters_b, model.parameters_b))
                {
                    RowStatus::InUse((current.id != model.id).then(|| current.quantization.clone()))
                } else if saved.contains(&model.id) {
                    RowStatus::Saved
                } else if index == recommended {
                    RowStatus::Recommended
                } else {
                    RowStatus::Plain
                };
                PickerRow {
                    id: model.id.clone(),
                    size: format!("{}B", model.parameters_b),
                    parameters_b: model.parameters_b,
                    blurb: blurb(model.parameters_b),
                    bytes: model.bytes,
                    status,
                    details: alternative
                        .blocker
                        .map(|_| alternative.reasons.join("\n"))
                        .unwrap_or_default(),
                }
            })
            .collect();
        Self { rows, recommended }
    }

    /// The model the screen starts on: the one in use, whatever its quantization,
    /// while its size can be chosen. Otherwise the recommended size.
    pub(super) fn default_choice<'a>(&'a self, in_use: Option<&'a Model>) -> &'a str {
        in_use
            .filter(|m| {
                self.rows
                    .iter()
                    .any(|r| same_size(r.parameters_b, m.parameters_b) && r.status.selectable())
            })
            .map(|m| m.id.as_str())
            .or_else(|| self.rows.get(self.recommended).map(|r| r.id.as_str()))
            .unwrap_or_default()
    }

    /// The row a model belongs to: its size, in any quantization.
    pub(super) fn row_of(&self, model: &Model) -> Option<usize> {
        self.rows
            .iter()
            .position(|r| same_size(r.parameters_b, model.parameters_b))
    }
}

/// Everything the meter shows, so its words can be checked without drawing.
pub(super) struct MeterView {
    pub name: String,
    /// The OS budget, not a measurement of the whole card.
    pub estimated: bool,
    pub meter: GpuMeter,
    /// The GPU is too full for even the smallest model, so the selection runs on the CPU.
    pub runs_on_cpu: bool,
    /// The selected model is loaded, but the server is not ready yet.
    pub loading: bool,
}

impl MeterView {
    pub(super) fn used_line(&self) -> String {
        format!(
            "使用中 {} / {}",
            bytes(self.meter.used),
            bytes(self.meter.total)
        )
    }

    pub(super) fn model_line(&self) -> String {
        if self.runs_on_cpu {
            return "GPUの空きが足りず、CPUで実行します".into();
        }
        match self.meter.change {
            MemoryChange::Loaded if self.loading => "読み込み中".into(),
            MemoryChange::Loaded => "読み込み済み".into(),
            MemoryChange::Adds(extra) => format!("このモデルで +{}", bytes(extra)),
            MemoryChange::Frees(freed) => format!("このモデルで -{}", bytes(freed)),
        }
    }

    pub(super) fn shortfall_line(&self) -> Option<String> {
        self.exceeds().then(|| {
            format!(
                "あと{}足りません",
                bytes(self.meter.missing.next_multiple_of(1_048_576))
            )
        })
    }

    pub(super) fn exceeds(&self) -> bool {
        !self.runs_on_cpu && !self.meter.fits()
    }

    /// Whether the bar draws a segment for the selected model.
    fn draws_estimate(&self) -> bool {
        !self.runs_on_cpu && matches!(self.meter.change, MemoryChange::Adds(extra) if extra > 0)
    }

    /// The lines under the bar. A colour key only where the bar draws that colour.
    fn items(&self) -> Vec<MeterItem> {
        let estimate = if self.exceeds() { WARNING } else { ACCENT };
        let mut items = vec![
            MeterItem {
                key: Some(TEXT_MUTED),
                text: self.used_line(),
                color: TEXT_SECONDARY,
            },
            MeterItem {
                key: self.draws_estimate().then_some(estimate),
                text: self.model_line(),
                color: if self.exceeds() {
                    WARNING
                } else {
                    TEXT_SECONDARY
                },
            },
        ];
        if let Some(text) = self.shortfall_line() {
            items.push(MeterItem {
                key: None,
                text,
                color: WARNING,
            });
        }
        items
    }
}

struct MeterItem {
    key: Option<Color32>,
    text: String,
    color: Color32,
}

impl App {
    fn meter_view(&self) -> Option<MeterView> {
        let recommendation = self.setup.recommendation.as_ref()?;
        let plan = recommendation.gpu.as_ref()?;
        let selected = self.setup_model()?;
        let loaded = self.gpu_resident()
            && self
                .setup
                .in_use
                .as_ref()
                .is_some_and(|m| m.id == selected.id);
        Some(MeterView {
            name: plan.name.clone(),
            estimated: plan.telemetry != "nvidia-smi",
            meter: model_catalog::gpu_meter(
                plan.total_bytes,
                plan.free_bytes,
                plan.reserve_bytes,
                self.held_gpu_bytes(),
                model_catalog::inference_bytes(&selected),
                loaded,
            ),
            runs_on_cpu: recommendation.backend == Backend::Cpu,
            loading: !self.backend_ready,
        })
    }

    /// The model list, with the GPU memory beside it when the window is wide and
    /// above it when narrow.
    pub(super) fn picker_ui(&mut self, ui: &mut egui::Ui) {
        let Some(recommendation) = &self.setup.recommendation else {
            return;
        };
        let picker = Picker::new(
            recommendation,
            self.setup.in_use.as_ref(),
            &self.setup.saved,
        );
        let selected_row = self.setup_model().and_then(|m| picker.row_of(&m));
        let meter = self.meter_view();
        let force_cpu = self.setup.force_cpu;
        let editable = !self.setup.installing;
        let mut clicked = None;
        let wide = ui.available_width() >= SIDE_BY_SIDE_MIN_WIDTH;
        let width = ui.available_width().min(PICKER_MAX_WIDTH);
        let side = |ui: &mut egui::Ui| match &meter {
            Some(view) => meter_ui(ui, view, wide),
            None => cpu_note(ui, force_cpu, wide),
        };
        ui.scope(|ui| {
            ui.set_max_width(width);
            if wide {
                ui.horizontal_top(|ui| {
                    ui.spacing_mut().item_spacing.x = theme::SPACE_4;
                    ui.vertical(|ui| {
                        ui.set_width(width - METER_WIDTH - theme::SPACE_4);
                        clicked = rows_ui(ui, &picker, selected_row, editable);
                    });
                    ui.vertical(|ui| {
                        ui.set_width(METER_WIDTH);
                        side(ui);
                    });
                });
            } else {
                side(ui);
                ui.add_space(theme::SPACE_3);
                clicked = rows_ui(ui, &picker, selected_row, editable);
            }
        });
        if let Some(id) = clicked {
            self.setup.selected_id = id;
            self.setup.cancelled = false;
        }
    }
}

fn one_line(text: &str, color: Color32, width: f32) -> egui::text::LayoutJob {
    let mut job = egui::text::LayoutJob::single_section(
        text.to_owned(),
        egui::TextFormat::simple(egui::FontId::proportional(theme::SIZE_LABEL), color),
    );
    job.wrap = egui::text::TextWrapping::truncate_at_width(width);
    job
}

fn rows_ui(
    ui: &mut egui::Ui,
    picker: &Picker,
    selected: Option<usize>,
    editable: bool,
) -> Option<String> {
    let widest = picker
        .rows
        .iter()
        .filter_map(|r| r.status.text())
        .map(|text| {
            ui.painter()
                .layout_job(one_line(&text, TEXT, f32::INFINITY))
                .size()
                .x
        })
        .fold(0.0, f32::max);
    let mut clicked = None;
    ui.add_enabled_ui(editable, |ui| {
        ui.spacing_mut().item_spacing.y = theme::SPACE_1;
        for (index, row) in picker.rows.iter().enumerate() {
            if paint_row(ui, row, selected == Some(index), widest).clicked() {
                clicked = Some(row.id.clone());
            }
        }
    });
    clicked
}

/// One whole-row button: size, description and download size on the left,
/// one status word at the right end.
fn paint_row(
    ui: &mut egui::Ui,
    row: &PickerRow,
    selected: bool,
    status_width: f32,
) -> egui::Response {
    let selectable = row.status.selectable();
    let (rect, mut response) = ui.allocate_exact_size(
        Vec2::new(ui.available_width(), theme::CONTROL_H_LARGE),
        if selectable {
            egui::Sense::click()
        } else {
            egui::Sense::hover()
        },
    );
    let (fill, stroke) = if selected {
        (ACCENT_SOFT, egui::Stroke::new(1.5_f32, ACCENT))
    } else if selectable && response.hovered() {
        (SURFACE_RAISED, egui::Stroke::new(1.0_f32, BORDER_STRONG))
    } else {
        (SURFACE, theme::hairline())
    };
    let painter = ui.painter();
    painter.rect(
        rect,
        egui::CornerRadius::same(theme::RADIUS_CONTROL),
        fill,
        stroke,
        egui::StrokeKind::Inside,
    );
    let middle = rect.center().y;
    let radio = egui::pos2(rect.left() + theme::SPACE_4 + RADIO_RADIUS, middle);
    painter.circle_stroke(
        radio,
        RADIO_RADIUS,
        egui::Stroke::new(
            if selected { 2.0_f32 } else { 1.5_f32 },
            if selected {
                ACCENT
            } else if selectable {
                BORDER_STRONG
            } else {
                BORDER
            },
        ),
    );
    if selected {
        painter.circle_filled(radio, RADIO_RADIUS / 2.0, ACCENT);
    }
    // A row that cannot be chosen is muted, but its reason stays at full strength.
    let (primary, secondary) = if selectable {
        (TEXT, TEXT_SECONDARY)
    } else {
        (TEXT_MUTED, TEXT_MUTED)
    };
    let name_x = radio.x + RADIO_RADIUS + theme::SPACE_3;
    painter.text(
        egui::pos2(name_x, middle),
        egui::Align2::LEFT_CENTER,
        &row.size,
        egui::FontId::proportional(theme::SIZE_BODY),
        primary,
    );
    let right = rect.right() - theme::SPACE_4;
    let status = row.status.text();
    if let Some(text) = &status {
        painter.text(
            egui::pos2(right, middle),
            egui::Align2::RIGHT_CENTER,
            text,
            egui::FontId::proportional(theme::SIZE_LABEL),
            row.status.color(),
        );
    }
    let size_right = if status_width > 0.0 {
        right - status_width - theme::SPACE_4
    } else {
        right
    };
    let size_text = painter.layout_job(one_line(&bytes(row.bytes), secondary, f32::INFINITY));
    let size_left = size_right - size_text.size().x;
    painter.galley(
        egui::pos2(size_left, middle - size_text.size().y / 2.0),
        size_text,
        secondary,
    );
    let blurb_x = name_x + SIZE_COLUMN;
    let blurb_text = painter.layout_job(one_line(
        row.blurb,
        secondary,
        (size_left - theme::SPACE_3 - blurb_x).max(0.0),
    ));
    painter.galley(
        egui::pos2(blurb_x, middle - blurb_text.size().y / 2.0),
        blurb_text,
        secondary,
    );
    theme::focus_ring(ui, &response);
    let label = format!(
        "{} {} {} {}",
        row.size,
        row.blurb,
        bytes(row.bytes),
        status.unwrap_or_default()
    );
    response.widget_info(|| {
        egui::WidgetInfo::selected(egui::WidgetType::RadioButton, selectable, selected, &label)
    });
    if !row.details.is_empty() {
        response = response.on_hover_text(&row.details);
    }
    response
}

/// Why there is no meter. Stacked, each sentence has its own line, so a narrow
/// column never breaks one in the middle.
fn cpu_note_lines(force_cpu: bool, stacked: bool) -> Vec<String> {
    let sentences = [
        if force_cpu {
            "GPUを使わない設定です。"
        } else {
            "GPUが見つかりません。"
        },
        "CPUで使える0.8Bだけを選べます。",
    ];
    if stacked {
        sentences.map(String::from).to_vec()
    } else {
        vec![sentences.concat()]
    }
}

fn cpu_note(ui: &mut egui::Ui, force_cpu: bool, stacked: bool) {
    theme::card().show(ui, |ui| {
        ui.set_min_width(ui.available_width());
        for line in cpu_note_lines(force_cpu, stacked) {
            ui.label(theme::label(line));
        }
    });
}

/// `stacked` puts each line under the last, for the narrow column beside the list.
fn meter_ui(ui: &mut egui::Ui, view: &MeterView, stacked: bool) {
    theme::card().show(ui, |ui| {
        ui.set_min_width(ui.available_width());
        ui.horizontal_wrapped(|ui| {
            ui.label(
                RichText::new("GPUメモリ")
                    .size(theme::SIZE_LABEL)
                    .color(TEXT)
                    .strong(),
            );
            ui.label(theme::caption(&view.name));
            if view.estimated {
                ui.label(theme::caption("目安")).on_hover_text(
                    "Windowsが示す利用可能量からの目安です。ほかのアプリの使用量は反映されないことがあります。",
                );
            }
        });
        meter_bar(ui, view);
        let items = view.items();
        if stacked {
            ui.vertical(|ui| items.iter().for_each(|item| legend(ui, item)));
        } else {
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing.x = theme::SPACE_6;
                items.iter().for_each(|item| legend(ui, item));
            });
        }
    });
}

/// A line under the bar, with its colour key kept together with its words.
fn legend(ui: &mut egui::Ui, item: &MeterItem) {
    let font = egui::FontId::proportional(theme::SIZE_LABEL);
    let mut job = egui::text::LayoutJob::default();
    if let Some(key) = item.key {
        job.append("■ ", 0.0, egui::TextFormat::simple(font.clone(), key));
    }
    job.append(&item.text, 0.0, egui::TextFormat::simple(font, item.color));
    ui.add(egui::Label::new(job).wrap_mode(egui::TextWrapMode::Extend));
}

/// Memory in use, then the selected model's estimate stacked after it. The
/// estimate turns to the warning colour when it does not fit.
fn meter_bar(ui: &mut egui::Ui, view: &MeterView) {
    let (rect, response) = ui.allocate_exact_size(
        Vec2::new(ui.available_width(), BAR_HEIGHT),
        egui::Sense::hover(),
    );
    let meter = &view.meter;
    let painter = ui.painter();
    painter.rect(
        rect,
        egui::CornerRadius::same((BAR_HEIGHT / 2.0) as u8),
        INPUT,
        egui::Stroke::new(1.0_f32, BORDER_STRONG),
        egui::StrokeKind::Inside,
    );
    let inner = rect.shrink(2.0);
    let at = |amount: u64| {
        inner.left() + inner.width() * (amount.min(meter.total) as f32 / meter.total.max(1) as f32)
    };
    let used_end = at(meter.used);
    let extra = match meter.change {
        MemoryChange::Adds(extra) if view.draws_estimate() => extra,
        _ => 0,
    };
    let estimate_end = at(meter.used.saturating_add(extra));
    segment(
        painter,
        inner,
        inner.left(),
        used_end,
        TEXT_MUTED,
        true,
        extra == 0,
    );
    if extra > 0 {
        let start = if used_end > inner.left() {
            used_end + 2.0
        } else {
            inner.left()
        };
        segment(
            painter,
            inner,
            start,
            estimate_end,
            if view.exceeds() { WARNING } else { ACCENT },
            used_end <= inner.left(),
            true,
        );
    }
    if meter.usable < meter.total {
        let limit = at(meter.usable);
        painter.line_segment(
            [
                egui::pos2(limit, rect.top() - 2.0),
                egui::pos2(limit, rect.bottom() + 2.0),
            ],
            egui::Stroke::new(1.0_f32, TEXT_SECONDARY),
        );
    }
    response.on_hover_text(format!(
        "画面表示などのため、{}は使わない前提で判定しています。",
        bytes(meter.total - meter.usable)
    ));
}

fn segment(
    painter: &egui::Painter,
    inner: egui::Rect,
    from: f32,
    to: f32,
    color: Color32,
    round_left: bool,
    round_right: bool,
) {
    if to - from < 1.0 {
        return;
    }
    let radius = (inner.height() / 2.0) as u8;
    let left = if round_left { radius } else { 0 };
    let right = if round_right { radius } else { 0 };
    painter.rect_filled(
        egui::Rect::from_min_max(
            egui::pos2(from, inner.top()),
            egui::pos2(to, inner.bottom()),
        ),
        egui::CornerRadius {
            nw: left,
            sw: left,
            ne: right,
            se: right,
        },
        color,
    );
}

#[cfg(test)]
mod tests;
