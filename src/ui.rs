use super::theme::{
    self, ACCENT, ACCENT_SOFT, BORDER_STRONG, CANVAS, ERROR, INPUT, ON_ACCENT, SURFACE,
    SURFACE_RAISED, TEXT, TEXT_MUTED, TEXT_SECONDARY, WARNING,
};
use super::*;
use plamo_translate::credit;
use plamo_translate::journal::ParagraphState;

/// Height reserved for the page title and its actions.
const HEADER_HEIGHT: f32 = 56.0;
/// Height reserved for the live input/display controls.
const SOURCES_HEIGHT: f32 = 40.0;
/// Height reserved above the history list for its label.
const HISTORY_HEADING_HEIGHT: f32 = 28.0;

/// Measurements captured while drawing, so tests can assert real geometry
/// instead of re-deriving layout from constants.
#[cfg(test)]
#[derive(Default)]
pub(super) struct UiProbe {
    pub rail_width: f32,
    pub nav_heights: Vec<f32>,
    pub now_rect: Option<egui::Rect>,
    pub reading_font: f32,
}

fn paragraph_label(row: &Row) -> &'static str {
    match row.record.paragraph.as_ref().map(|p| p.state) {
        Some(ParagraphState::Open | ParagraphState::Closing) => "継続中の文章 · 訳文を更新中",
        Some(ParagraphState::Final) => "確定した文章",
        Some(ParagraphState::Interrupted) => "中断した文章 · 途中の訳文（未確定）",
        Some(ParagraphState::Failed) => "翻訳できなかった文章 · 未確定",
        None if row.pending => "翻訳中",
        None => "最新の対訳",
    }
}

fn history_row_visible(row: &Row, current: Option<u64>) -> bool {
    !row.is_provisional()
        && super::live_view::history_visible(
            row.record.id,
            row.pending,
            if row.is_confirmed() { current } else { None },
        )
}

/// A paragraph that is still being revised is never presented as final, and
/// never paired with a different source.
fn row_is_unfinished(row: &Row) -> bool {
    row.record.paragraph.as_ref().is_some_and(|p| {
        matches!(
            p.state,
            ParagraphState::Open
                | ParagraphState::Closing
                | ParagraphState::Interrupted
                | ParagraphState::Failed
        )
    })
}

#[derive(Clone, Copy)]
struct LiveLayout {
    latest_height: f32,
    status_height: f32,
    compact: bool,
}
impl LiveLayout {
    fn for_height(height: f32) -> Self {
        // Viewport points, not physical pixels or changing source/status text.
        // Grow with the window below 727 points without consuming the minimum
        // history viewport; the common DPI-scaled window gets a full paragraph.
        let compact = height < 840.0;
        Self {
            // Fixed for a given window size (content changes never resize it),
            // but it grows with the window so a short paragraph does not leave
            // a large empty box on a tall screen. The history list keeps at
            // least 110 points at the minimum window height.
            latest_height: (height * 0.36)
                .clamp(170.0, 300.0)
                .min((height - 434.0).max(150.0)),
            status_height: if compact { 56.0 } else { 88.0 },
            compact,
        }
    }
}

/// Allocate a hard outer rectangle; changing child content cannot move history.
fn live_region<R>(
    ui: &mut egui::Ui,
    id: &'static str,
    height: f32,
    contents: impl FnOnce(&mut egui::Ui) -> R,
) -> R {
    let (rect, _) = ui.allocate_exact_size(
        Vec2::new(ui.available_width(), height),
        egui::Sense::hover(),
    );
    let mut child = ui.new_child(
        egui::UiBuilder::new()
            .id_salt(id)
            .max_rect(rect)
            .layout(egui::Layout::top_down(egui::Align::Min)),
    );
    child.set_clip_rect(rect.intersect(ui.clip_rect()));
    contents(&mut child)
}

/// The page always fills the window; the caller decides where a single
/// prose column needs a readable measure.
fn page_column(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui)) {
    let width = ui.available_width();
    let height = ui.available_height();
    ui.allocate_ui_with_layout(
        Vec2::new(width, height),
        egui::Layout::top_down(egui::Align::Min),
        |ui| {
            ui.set_width(width);
            add(ui);
        },
    );
}

/// Left-aligned block for long prose inside a full-width surface. The block
/// follows the window until a line would exceed the readable measure.
fn measure_block<R>(
    ui: &mut egui::Ui,
    reading_size: f32,
    add: impl FnOnce(&mut egui::Ui) -> R,
) -> R {
    let width = ui.available_width().min(theme::prose_width(reading_size));
    let height = ui.available_height();
    ui.allocate_ui_with_layout(
        Vec2::new(width, height),
        egui::Layout::top_down(egui::Align::Min),
        |ui| {
            ui.set_width(width);
            add(ui)
        },
    )
    .inner
}

fn live_latest_panel(
    ui: &mut egui::Ui,
    current: Option<&Row>,
    draft: &str,
    mode: super::live_view::LatestMode,
    layout: LiveLayout,
    preferences: (f32, bool, bool),
) -> Option<String> {
    use super::live_view::LatestMode;
    let (font_size, show_original, auto_scroll) = preferences;
    // A fresh ASR draft is an addition to the accepted paragraph, never a
    // replacement for its coherent interim Japanese translation.
    let mode = if current.is_some() {
        LatestMode::Current
    } else {
        mode
    };
    let selected = if mode == LatestMode::Current {
        current
    } else {
        None
    };
    let translation = selected
        .map(|row| row.record.translation.as_str())
        .unwrap_or("");
    live_region(ui, "live-latest-panel", layout.latest_height, |ui| {
        theme::reading_surface().show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            measure_block(ui, font_size, |ui| {
                ui.spacing_mut().item_spacing.y = theme::SPACE_1;
                ui.add(
                    egui::Label::new(
                        RichText::new("進行中の文章／最新の対訳")
                            .size(12.0)
                            .strong()
                            .color(ACCENT),
                    )
                    .truncate(),
                );
                let state = match mode {
                    LatestMode::Draft => "認識途中 · 原文をつないで翻訳します".to_owned(),
                    LatestMode::Current => selected
                        .map(|row| {
                            format!(
                                "{} · {} · {}",
                                clock(row.record.start_ms),
                                row.record.kind,
                                paragraph_label(row)
                            )
                        })
                        .unwrap_or_default(),
                    LatestMode::Empty => "YouTube字幕またはPC音声の入力を待っています".to_owned(),
                };
                ui.add(
                    egui::Label::new(RichText::new(state).size(12.0).color(TEXT_MUTED)).truncate(),
                );
                let source = match mode {
                    LatestMode::Draft => draft.to_owned(),
                    LatestMode::Current if show_original => {
                        let accepted = selected.map(|row| row.record.source.as_str()).unwrap_or("");
                        if draft.trim().is_empty() {
                            accepted.to_owned()
                        } else {
                            format!("{accepted}\n\n追加で認識中の原文（未確定）:\n{draft}")
                        }
                    }
                    LatestMode::Current => {
                        "原文は非表示です。表示メニューから切り替えられます。".to_owned()
                    }
                    LatestMode::Empty => "原文はここに表示されます。".to_owned(),
                };
                let error = selected.and_then(|row| row.error.as_deref());
                let target = error.unwrap_or_else(|| {
                    if !translation.is_empty() {
                        translation
                    } else if mode == LatestMode::Draft {
                        "原文をつなぎながら訳文を更新します。"
                    } else if selected.is_some_and(|row| row.pending) {
                        "翻訳しています…"
                    } else {
                        "訳文はここに表示されます。"
                    }
                });
                let id = selected.map(|row| row.record.id).unwrap_or(0);
                let show_source = show_original || mode == LatestMode::Draft;
                let height = ui.available_height().max(24.0);
                ui.columns(if show_source { 2 } else { 1 }, |columns| {
                    if show_source {
                        live_text_column(
                            &mut columns[0],
                            ("latest-source", id, mode == LatestMode::Draft),
                            "原文",
                            &source,
                            (15.0, TEXT_MUTED),
                            auto_scroll
                                && (mode == LatestMode::Draft
                                    || selected
                                        .is_some_and(|row| row.pending || row.is_provisional())),
                            height,
                        );
                    }
                    live_text_column(
                        &mut columns[usize::from(show_source)],
                        ("latest-target", id, false),
                        if selected.is_some_and(Row::is_provisional) {
                            "訳文（継続して更新）"
                        } else {
                            "訳文"
                        },
                        target,
                        (font_size, if error.is_some() { ERROR } else { TEXT }),
                        auto_scroll
                            && selected.is_some_and(|row| row.pending || row.is_provisional()),
                        height,
                    );
                });
            });
        });
    });
    None
}

fn live_text_column(
    ui: &mut egui::Ui,
    id: (&'static str, u64, bool),
    label: &str,
    text: &str,
    style: (f32, Color32),
    follow: bool,
    height: f32,
) {
    let (size, color) = style;
    let label_height = ui
        .fonts_mut(|fonts| fonts.row_height(&egui::FontId::proportional(12.0)))
        .ceil();
    ui.label(RichText::new(label).size(12.0).color(TEXT_MUTED));
    // Use the same explicit pitch for wrapping and viewport sizing. A sticky
    // tail then lands on complete rows instead of clipping half the first row.
    let row_height = ui
        .fonts_mut(|fonts| fonts.row_height(&egui::FontId::proportional(size)))
        .ceil();
    let available = (height - label_height - theme::SPACE_1).max(row_height);
    let rows = ((available - 4.0) / row_height).floor().max(1.0);
    let height = rows * row_height + 4.0;
    egui::ScrollArea::vertical()
        .id_salt(id)
        .max_height(height)
        .min_scrolled_height(height)
        .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysVisible)
        .auto_shrink([false, false])
        .stick_to_bottom(follow)
        .show_viewport(ui, |ui, viewport| {
            // ScrollArea deliberately expands its default clip by a few points.
            // Prose aligned to a sticky tail must not reveal the previous row's
            // glyph edge through that margin.
            let visible = viewport.translate(ui.max_rect().min.to_vec2());
            ui.set_clip_rect(ui.clip_rect().intersect(visible));
            ui.spacing_mut().item_spacing.y = 0.0;
            ui.add_space(2.0);
            ui.add(
                egui::Label::new(
                    RichText::new(text)
                        .size(theme::reading_size(size))
                        .line_height(Some(row_height))
                        .color(color),
                )
                .wrap()
                .selectable(true),
            );
            ui.add_space(2.0);
        });
}

fn live_history_card(
    ui: &mut egui::Ui,
    row: &Row,
    font_size: f32,
    show_original: bool,
    read_requested: &mut Option<String>,
) -> egui::Rect {
    let unfinished = row_is_unfinished(row);
    egui::Frame::new()
        .inner_margin(egui::Margin::symmetric(0, 2))
        .show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(theme::caption(clock(row.record.start_ms)));
                let color = live_kind_color(&row.record.kind);
                egui::Frame::new()
                    .fill(SURFACE_RAISED)
                    .stroke(egui::Stroke::new(1.0_f32, color))
                    .corner_radius(4)
                    .inner_margin(egui::Margin::symmetric(6, 2))
                    .show(ui, |ui| {
                        ui.label(RichText::new(&row.record.kind).size(11.0).color(color));
                    });
                if unfinished {
                    // Unconfirmed text is labelled, not just tinted.
                    badge(
                        ui,
                        paragraph_label(row),
                        if row.error.is_some() { ERROR } else { WARNING },
                    );
                } else {
                    ui.label(
                        RichText::new(paragraph_label(row))
                            .size(12.0)
                            .color(TEXT_MUTED),
                    );
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui
                        .add_enabled(row.is_confirmed(), standard("読み上げ"))
                        .clicked()
                    {
                        *read_requested = Some(row.record.translation.clone());
                    }
                    if ui
                        .add_enabled(!row.record.translation.is_empty(), standard("コピー"))
                        .on_hover_text("PLaMoのクレジットを付けてコピーします")
                        .clicked()
                        && let Ok(mut clipboard) = arboard::Clipboard::new()
                    {
                        let _ = clipboard.set_text(credit::published(&row.record.translation));
                    }
                });
            });
            if let Some(error) = &row.error {
                ui.add(egui::Label::new(RichText::new(error).size(13.0).color(ERROR)).wrap());
            }
            ui.add_space(theme::SPACE_1);
            if !row.record.translation.is_empty() {
                ui.add(
                    egui::Label::new(theme::reading(&row.record.translation, font_size))
                        .wrap()
                        .selectable(true),
                );
            } else {
                ui.label(
                    RichText::new("訳文はありません。")
                        .size(15.0)
                        .color(TEXT_MUTED),
                );
            }
            if show_original {
                ui.add_space(theme::SPACE_1);
                ui.add(
                    egui::Label::new(
                        RichText::new(&row.record.source)
                            .size(14.0)
                            .color(TEXT_MUTED),
                    )
                    .wrap()
                    .selectable(true),
                );
            }
        })
        .response
        .rect
}

fn live_kind_color(kind: &str) -> Color32 {
    match kind {
        "YouTube" => Color32::from_rgb(226, 160, 118),
        "音声" => Color32::from_rgb(141, 180, 234),
        _ => Color32::from_rgb(186, 166, 224),
    }
}

pub(super) fn configure_theme(ctx: &egui::Context) {
    install_fonts(ctx);
    theme::apply(ctx);
}

fn card() -> egui::Frame {
    theme::card()
}
fn secondary(ui: &mut egui::Ui, text: impl Into<String>) {
    ui.label(theme::label(text));
}
pub(super) fn primary(label: &str) -> egui::Button<'_> {
    egui::Button::new(RichText::new(label).size(15.0).strong().color(ON_ACCENT))
        .fill(ACCENT)
        .min_size(Vec2::new(140.0, theme::CONTROL_H_LARGE))
}
fn standard(label: &str) -> egui::Button<'_> {
    egui::Button::new(RichText::new(label).size(14.0).color(TEXT))
        .min_size(Vec2::new(0.0, theme::CONTROL_H))
}
pub(super) fn toggle(ui: &mut egui::Ui, value: &mut bool, label: &str) -> egui::Response {
    ui.scope(|ui| {
        let checked = *value;
        let visuals = &mut ui.style_mut().visuals;
        for widget in [
            &mut visuals.widgets.inactive,
            &mut visuals.widgets.hovered,
            &mut visuals.widgets.active,
            &mut visuals.widgets.open,
        ] {
            widget.corner_radius = egui::CornerRadius::same(6);
            widget.bg_fill = if checked { ACCENT_SOFT } else { INPUT };
            widget.weak_bg_fill = widget.bg_fill;
            widget.bg_stroke =
                egui::Stroke::new(1.0_f32, if checked { ACCENT } else { BORDER_STRONG });
            widget.fg_stroke = egui::Stroke::new(2.0_f32, if checked { ACCENT } else { TEXT });
        }
        visuals.widgets.active.bg_stroke = egui::Stroke::new(2.0_f32, ACCENT);
        ui.checkbox(value, RichText::new(label).size(14.0).color(TEXT))
    })
    .inner
}

fn badge(ui: &mut egui::Ui, text: &str, color: Color32) {
    egui::Frame::new()
        .fill(ACCENT_SOFT)
        .stroke(egui::Stroke::new(1.0_f32, color))
        .corner_radius(4)
        .inner_margin(egui::Margin::symmetric(6, 2))
        .show(ui, |ui| {
            ui.label(RichText::new(text).size(11.0).color(color));
        });
}

/// Three explicit live-input modes. Captions and PC audio can run together,
/// but the choice is a single control instead of two independent checkboxes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LiveSourceMode {
    Off,
    Captions,
    Audio,
    Both,
}

impl LiveSourceMode {
    fn from_flags(captions: bool, audio: bool) -> Option<Self> {
        match (captions, audio) {
            (true, false) => Some(Self::Captions),
            (false, true) => Some(Self::Audio),
            (true, true) => Some(Self::Both),
            (false, false) => None,
        }
    }

    fn flags(self) -> (bool, bool) {
        match self {
            Self::Off => (false, false),
            Self::Captions => (true, false),
            Self::Audio => (false, true),
            Self::Both => (true, true),
        }
    }

    fn hint(self) -> &'static str {
        match self {
            Self::Off => "入力を止めます。保存済みの訳文はそのまま残ります",
            Self::Captions => "YouTubeの字幕だけを翻訳します",
            Self::Audio => "PCで再生中の音声だけを翻訳します",
            Self::Both => "字幕を優先し、字幕が止まっている間はPC音声を使います",
        }
    }
}

/// Horizontal segmented switch. Returns the newly selected index.
fn segmented(ui: &mut egui::Ui, labels: &[&str], selected: Option<usize>) -> Option<usize> {
    let mut clicked = None;
    egui::Frame::new()
        .fill(INPUT)
        .stroke(egui::Stroke::new(1.0_f32, BORDER_STRONG))
        .corner_radius(theme::RADIUS_CONTROL)
        .inner_margin(egui::Margin::same(2))
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            ui.horizontal(|ui| {
                for (index, label) in labels.iter().enumerate() {
                    let active = selected == Some(index);
                    let response = ui.add(
                        egui::Button::new(RichText::new(*label).size(13.0).color(if active {
                            ON_ACCENT
                        } else {
                            TEXT_SECONDARY
                        }))
                        .fill(if active { ACCENT } else { INPUT })
                        .stroke(egui::Stroke::NONE)
                        .min_size(Vec2::new(0.0, theme::CONTROL_H)),
                    );
                    theme::focus_ring(ui, &response);
                    if response.clicked() {
                        clicked = Some(index);
                    }
                }
            });
        });
    clicked
}

fn rail_item(ui: &mut egui::Ui, label: &str, selected: bool) -> egui::Response {
    let height = theme::CONTROL_H_LARGE + 8.0;
    let (rect, response) = ui.allocate_exact_size(
        Vec2::new(ui.available_width(), height),
        egui::Sense::click(),
    );
    let hovered = response.hovered();
    if selected {
        ui.painter().rect_filled(
            rect,
            egui::CornerRadius::same(theme::RADIUS_CONTROL),
            SURFACE_RAISED,
        );
        ui.painter().rect_filled(
            egui::Rect::from_min_size(
                egui::Pos2::new(rect.left(), rect.top() + 10.0),
                Vec2::new(3.0, rect.height() - 20.0),
            ),
            2.0,
            ACCENT,
        );
    } else if hovered {
        ui.painter().rect_filled(
            rect,
            egui::CornerRadius::same(theme::RADIUS_CONTROL),
            SURFACE,
        );
    }
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        label,
        egui::FontId::proportional(13.0),
        if selected { TEXT } else { TEXT_SECONDARY },
    );
    theme::focus_ring(ui, &response);
    response.widget_info(|| {
        egui::WidgetInfo::selected(egui::WidgetType::Button, true, selected, label)
    });
    response
}

fn parse_glossary(input: &str) -> std::result::Result<Vec<(String, String)>, String> {
    let mut entries = Vec::new();
    for (index, line) in input.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let Some((source, target)) = line.split_once('\t').or_else(|| line.split_once('=')) else {
            return Err(format!(
                "{}行目: 原語と訳語を = またはタブで区切ってください。",
                index + 1
            ));
        };
        if source.trim().is_empty() || target.trim().is_empty() {
            return Err(format!(
                "{}行目: 原語と訳語の両方を入力してください。",
                index + 1
            ));
        }
        entries.push((source.trim().to_owned(), target.trim().to_owned()));
    }
    Ok(entries)
}

impl App {
    /// Page title and supporting line. Actions sit on the same fixed row so
    /// changing content can never shift the reading area.
    fn page_header(
        &mut self,
        ui: &mut egui::Ui,
        title: &str,
        subtitle: &str,
        actions: impl FnOnce(&mut egui::Ui),
    ) {
        live_region(ui, "page-header", HEADER_HEIGHT, |ui| {
            let mut enter_mini = false;
            let mut open_subtitle = false;
            let backend_ready = self.backend_ready;
            ui.horizontal(|ui| {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    // Present on every page so the separate subtitle window and
                    // the collapsed mini view are always one click away.
                    if ui
                        .add(standard("字幕ウィンドウ"))
                        .on_hover_text(
                            "訳文を独立したウィンドウに表示します。ミニ表示と併用できます。",
                        )
                        .clicked()
                    {
                        open_subtitle = true;
                    }
                    if ui
                        .add_enabled(backend_ready, standard("ミニ表示"))
                        .on_hover_text("入力欄を畳み、訳文だけを小窓に表示します。Ctrl + Shift + M")
                        .clicked()
                    {
                        enter_mini = true;
                    }
                    actions(ui);
                    // The title takes whatever the actions leave, so a narrow
                    // window truncates it instead of overlapping the buttons.
                    ui.with_layout(egui::Layout::left_to_right(egui::Align::Min), |ui| {
                        ui.vertical(|ui| {
                            ui.spacing_mut().item_spacing.y = 2.0;
                            ui.add(egui::Label::new(theme::title(title)).truncate());
                            ui.add(
                                egui::Label::new(
                                    RichText::new(subtitle).size(12.0).color(TEXT_MUTED),
                                )
                                .truncate(),
                            );
                        });
                    });
                });
            });
            if open_subtitle {
                self.subtitle_window.open = true;
            }
            if enter_mini {
                self.enter_mini_view(ui.ctx());
            }
        });
    }

    /// Apply one of the three explicit input modes. Audio start is deferred
    /// through the existing preparation path when the engine is not ready.
    fn apply_live_sources(&mut self, mode: LiveSourceMode) {
        let (captions, audio) = mode.flags();
        self.set_caption_enabled(captions);
        let audio_active = self.audio_enabled || self.audio_start_requested;
        if audio && !audio_active {
            if self.backend_ready && self.asr_ready && self.audio_environment_ready() {
                self.toggle_audio();
            } else {
                self.audio_start_requested = true;
                self.prepare_audio();
            }
        } else if !audio && audio_active {
            self.audio_start_requested = false;
            if self.audio_enabled {
                self.stop_audio();
            }
        }
    }

    /// One fixed row for inputs, display options and the actions for the
    /// translation currently on screen.
    fn live_sources_row(&mut self, ui: &mut egui::Ui) {
        let current_text = self
            .document_visible_row()
            .filter(|row| row.is_provisional() || row.is_confirmed())
            .map(|row| row.record.translation.clone())
            .unwrap_or_default();
        let current_ready = self.document_visible_row().is_some_and(Row::is_confirmed);
        live_region(ui, "live-sources", SOURCES_HEIGHT, |ui| {
            let mut read_requested = None;
            ui.horizontal(|ui| {
                let caption_receiving = self.caption_enabled
                    && !self.caption_paused
                    && self
                        .last_caption
                        .is_some_and(|time| time.elapsed() < Duration::from_secs(8));
                let audio_wanted = self.audio_enabled || self.audio_start_requested;
                let current = LiveSourceMode::from_flags(self.caption_enabled, audio_wanted);
                let selected = match current {
                    None => 0,
                    Some(LiveSourceMode::Captions) => 1,
                    Some(LiveSourceMode::Audio) => 2,
                    Some(LiveSourceMode::Both) => 3,
                    Some(LiveSourceMode::Off) => 0,
                };
                let chosen = segmented(ui, &["停止", "字幕", "PC音声", "両方"], Some(selected));
                if let Some(index) = chosen {
                    let mode = match index {
                        1 => LiveSourceMode::Captions,
                        2 => LiveSourceMode::Audio,
                        3 => LiveSourceMode::Both,
                        _ => LiveSourceMode::Off,
                    };
                    if current != Some(mode) {
                        self.apply_live_sources(mode);
                    }
                }
                let state = if self.caption_enabled && caption_receiving {
                    ("字幕を受信中", ACCENT)
                } else if audio_wanted && self.audio_enabled && self.asr_ready {
                    ("音声を認識中", ACCENT)
                } else if self.caption_enabled || audio_wanted {
                    ("入力を待っています", TEXT_MUTED)
                } else {
                    ("停止中", TEXT_MUTED)
                };
                ui.add(egui::Label::new(RichText::new(state.0).size(12.0).color(state.1)))
                    .on_hover_text(current.map(LiveSourceMode::hint).unwrap_or("入力を選ぶと翻訳が始まります"));
                let was_enabled = self.settings.auto_read;
                let read_toggle = ui
                    .add_enabled_ui(self.readout.ready.is_some(), |ui| {
                        toggle(ui, &mut self.settings.auto_read, "自動読み上げ")
                    })
                    .inner;
                if was_enabled && !self.settings.auto_read {
                    self.stop_readout();
                }
                read_toggle.on_hover_text("新しい確定訳を読み上げます");

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.menu_button(RichText::new("その他").size(14.0), |ui| {
                        if ui.add(standard("新しく始める")).clicked() {
                            match Journal::new(&data_root().join("sessions")) {
                                Ok(journal) => {
                                    self.document_shutdown();
                                    self.journal = Some(journal);
                                    self.journal_error.clear();
                                    self.rows.clear();
                                    self.invalidate_timeline();
                                }
                                Err(error) => self.status = error.to_string(),
                            }
                            ui.close();
                        }
                        if ui
                            .add_enabled(!self.rows.is_empty(), standard("書き出し"))
                            .clicked()
                        {
                            self.export_history();
                            ui.close();
                        }
                    });
                    ui.menu_button(RichText::new("表示").size(14.0), |ui| {
                        toggle(ui, &mut self.settings.live_show_original, "原文を表示");
                        toggle(ui, &mut self.settings.auto_scroll, "末尾へ自動スクロール")
                            .on_hover_text(
                                "上へスクロールしている間は位置を保ちます。OFFからONに戻すと末尾へ移動します。",
                            );
                        ui.add_space(theme::SPACE_1);
                        ui.label(RichText::new("訳文の文字サイズ").size(12.0).color(TEXT_MUTED));
                        ui.add(
                            egui::Slider::new(
                                &mut self.settings.font_size,
                                theme::READING_MIN..=theme::READING_MAX,
                            )
                            .integer()
                            .suffix(" pt"),
                        );
                    });
                    if ui
                        .add_enabled(current_ready, standard("読み上げ"))
                        .clicked()
                    {
                        read_requested = Some(current_text.clone());
                    }
                    if ui
                        .add_enabled(!current_text.is_empty(), standard("コピー"))
                        .on_hover_text("PLaMoのクレジットを付けてコピーします")
                        .clicked()
                        && let Ok(mut clipboard) = arboard::Clipboard::new()
                    {
                        let _ = clipboard.set_text(credit::published(&current_text));
                    }
                });
            });
            if let Some(text) = read_requested.filter(|_| !ui.ctx().will_discard()) {
                self.read_translation(text);
            }
        });
    }

    fn live_status_region(&mut self, ui: &mut egui::Ui, layout: LiveLayout) {
        // Reserve status, notices and the stop control even while idle.
        live_region(ui, "live-status", layout.status_height, |ui| {
            if layout.compact {
                ui.spacing_mut().item_spacing.y = theme::SPACE_1;
            }
            let speaking = self.readout_busy();
            ui.horizontal(|ui| {
                let status = if speaking {
                    self.readout.status.clone()
                } else if self.live_busy.is_some() {
                    "翻訳中".to_owned()
                } else if !self.asr_partial.is_empty() {
                    "音声を認識中".to_owned()
                } else {
                    "最新の対訳を表示します".to_owned()
                };
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui
                        .add_enabled(speaking, standard("読み上げを停止"))
                        .clicked()
                    {
                        self.stop_readout();
                    }
                    ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                        ui.add(
                            egui::Label::new(
                                RichText::new(&status).size(13.0).color(if speaking {
                                    ACCENT
                                } else {
                                    TEXT_SECONDARY
                                }),
                            )
                            .truncate(),
                        );
                    });
                });
            });
            let saved = if !self.journal_error.is_empty() {
                "保存できていません"
            } else if self.journal.is_some() {
                "タイムラインへ自動保存"
            } else {
                "プレビュー · 保存されません"
            };
            let notice = if self.caption_enabled && self.audio_enabled {
                format!("{saved} · 字幕を優先し、字幕がない間は音声を使います")
            } else {
                saved.to_owned()
            };
            let errors = [&self.service_error, &self.journal_error]
                .into_iter()
                .filter(|s| !s.is_empty())
                .map(String::as_str)
                .collect::<Vec<_>>()
                .join(" / ");
            if layout.compact {
                let combined = if errors.is_empty() {
                    notice
                } else {
                    format!("{errors} / {notice}")
                };
                ui.add(
                    egui::Label::new(
                        RichText::new(&combined)
                            .size(12.0)
                            .color(if errors.is_empty() { TEXT_MUTED } else { ERROR }),
                    )
                    .truncate(),
                )
                .on_hover_text(combined);
            } else {
                ui.add(
                    egui::Label::new(RichText::new(notice).size(12.0).color(
                        if errors.is_empty() && self.journal.is_some() {
                            theme::SUCCESS
                        } else {
                            TEXT_MUTED
                        },
                    ))
                    .truncate(),
                );
                ui.add(egui::Label::new(RichText::new(&errors).size(12.0).color(ERROR)).truncate())
                    .on_hover_text(errors);
            }
        });
    }

    fn languages(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            egui::ComboBox::from_id_salt("source-language")
                .width(126.0)
                .selected_text(language_label(&self.settings.translation.source_lang))
                .show_ui(ui, |ui| {
                    ui.selectable_value(
                        &mut self.settings.translation.source_lang,
                        "auto".into(),
                        "自動判定",
                    );
                    for (code, label) in engine::LANGUAGES {
                        ui.selectable_value(
                            &mut self.settings.translation.source_lang,
                            (*code).into(),
                            *label,
                        );
                    }
                });
            if ui
                .add_enabled(
                    self.settings.translation.source_lang != "auto",
                    standard("交換"),
                )
                .clicked()
            {
                std::mem::swap(
                    &mut self.settings.translation.source_lang,
                    &mut self.settings.translation.target_lang,
                );
            }
            egui::ComboBox::from_id_salt("target-language")
                .width(126.0)
                .selected_text(language_label(&self.settings.translation.target_lang))
                .show_ui(ui, |ui| {
                    for (code, label) in engine::LANGUAGES {
                        ui.selectable_value(
                            &mut self.settings.translation.target_lang,
                            (*code).into(),
                            *label,
                        );
                    }
                });
            if self.settings.translation.target_lang == "ja" {
                egui::ComboBox::from_id_salt("form")
                    .width(108.0)
                    .selected_text(if self.settings.translation.style == "polite" {
                        "です・ます"
                    } else {
                        "常体"
                    })
                    .show_ui(ui, |ui| {
                        ui.selectable_value(
                            &mut self.settings.translation.style,
                            "plain".into(),
                            "常体",
                        );
                        ui.selectable_value(
                            &mut self.settings.translation.style,
                            "polite".into(),
                            "です・ます",
                        );
                    });
            }
        });
    }
    fn backend_notice(&mut self, ui: &mut egui::Ui) {
        if !self.backend_ready {
            ui.horizontal_wrapped(|ui| {
                secondary(
                    ui,
                    if self.backend_loading {
                        "翻訳モデルを読み込んでいます。"
                    } else {
                        "翻訳するにはモデルの準備が必要です。"
                    },
                );
                if ui.add(standard("モデルの準備")).clicked() {
                    self.open_setup();
                }
            });
        }
    }
    fn input_card(&mut self, ui: &mut egui::Ui, height: f32) {
        card().show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.label(
                    RichText::new(format!("原文 · {}字", self.input.chars().count()))
                        .size(14.0)
                        .strong(),
                );
                if ui.add(standard("貼り付け")).clicked() {
                    match arboard::Clipboard::new().and_then(|mut c| c.get_text()) {
                        Ok(text) => self.input = text,
                        Err(error) => self.status = error.to_string(),
                    }
                }
                if ui
                    .add_enabled(!self.input.is_empty(), standard("クリア"))
                    .clicked()
                {
                    self.input.clear();
                }
            });
            egui::ScrollArea::vertical()
                .id_salt("source-scroll")
                .max_height(height)
                .min_scrolled_height(height)
                .show(ui, |ui| {
                    ui.add_sized(
                        [ui.available_width(), height],
                        egui::TextEdit::multiline(&mut self.input)
                            .hint_text("翻訳したい文章を入力してください")
                            .font(egui::FontId::proportional(theme::SIZE_BODY))
                            .margin(12.0)
                            .desired_width(f32::INFINITY),
                    );
                });
        });
    }
    /// 下のステータス行に、各モデルが載っているかを小さく出す。
    fn model_dots_ui(&self, ui: &mut egui::Ui) {
        let ocr = match self.settings.ocr_backend.as_str() {
            "indra" => "Indra OCR",
            "paddle" => "PaddleOCR-VL",
            _ => "Windows OCR",
        };
        let asr = match self.settings.asr_backend.as_str() {
            "nemotron" => "Nemotron Speech",
            "qwen" => "Qwen3-ASR",
            _ => "Whisper",
        };
        let translation = format!("{} · {}", self.current_model_label(), self.connection_label());
        let items = [
            (
                "翻訳",
                self.backend_ready && self.service_error.is_empty(),
                translation,
            ),
            ("OCR", self.screen_ocr_ready(), ocr.to_owned()),
            ("STT", self.asr_ready, asr.to_owned()),
            ("TTS", self.readout.ready.is_some(), "読み上げ".to_owned()),
        ];
        for (name, ready, detail) in items {
            let hover = format!("{name}: {detail} · {}", if ready { "準備完了" } else { "未準備" });
            ui.label(RichText::new(name).size(11.0).color(TEXT_SECONDARY))
                .on_hover_text(hover.as_str());
            ui.label(
                RichText::new(if ready { "●" } else { "○" })
                    .size(9.0)
                    .color(if ready { ACCENT } else { TEXT_MUTED }),
            )
            .on_hover_text(hover);
            ui.add_space(6.0);
        }
    }
    fn output_card(&mut self, ui: &mut egui::Ui, height: f32) {
        card().show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new("訳文").size(14.0).strong().color(ACCENT));
                if ui
                    .add_enabled(!self.output.is_empty(), standard("コピー"))
                    .on_hover_text("PLaMoのクレジットを付けてコピーします")
                    .clicked()
                    && let Ok(mut clipboard) = arboard::Clipboard::new()
                {
                    let _ = clipboard.set_text(credit::published(&self.output));
                }
                if ui
                    .add_enabled(!self.output.is_empty(), standard("保存"))
                    .clicked()
                {
                    self.save_text();
                }
                if ui
                    .add_enabled(!self.output.is_empty(), standard("読み上げ"))
                    .clicked()
                {
                    self.read_translation(self.output.clone());
                }
                if self.readout_busy() && ui.add(standard("停止")).clicked() {
                    self.stop_readout();
                }
            });
            egui::ScrollArea::vertical()
                .id_salt("output-scroll")
                .max_height(height)
                .min_scrolled_height(height)
                .show(ui, |ui| {
                    // An immutable text buffer remains selectable, but never changes the translation.
                    let mut output = self.output.as_str();
                    ui.add_sized(
                        [ui.available_width(), height],
                        egui::TextEdit::multiline(&mut output)
                            .hint_text("訳文はここに表示されます")
                            .font(egui::FontId::proportional(theme::reading_size(
                                self.settings.font_size,
                            )))
                            .margin(12.0)
                            .desired_width(f32::INFINITY),
                    );
                });
        });
    }

    pub(super) fn text_ui(&mut self, ui: &mut egui::Ui) {
        page_column(ui, |ui| {
            let ready = self.backend_ready;
            let mut open_image = false;
            let translate_ready =
                self.backend_ready && self.manual_busy.is_none() && !self.input.trim().is_empty();
            let mut translate = false;
            self.page_header(
                ui,
                "文章翻訳",
                "文章・ファイル・画面の文字を翻訳します。",
                |ui| {
                    if ui
                        .add_enabled(translate_ready, primary("翻訳する"))
                        .on_hover_text("Ctrl + Enter")
                        .clicked()
                    {
                        translate = true;
                    }
                    if ui.add_enabled(ready, standard("画像から翻訳")).clicked() {
                        open_image = true;
                    }
                },
            );
            if translate {
                self.manual_translate();
            }
            if open_image {
                self.open_image();
            }
            ui.add_space(theme::SPACE_2);
            live_region(ui, "text-languages", 36.0, |ui| {
                self.languages(ui);
            });
            self.backend_notice(ui);
            ui.add_space(theme::SPACE_2);
            let ready =
                self.backend_ready && self.manual_busy.is_none() && !self.input.trim().is_empty();
            // The text area gets the remaining height after the card header
            // (40), the two card paddings (32) and the row gap (8).
            let available = (ui.available_height() - 8.0).max(240.0);
            let stacked = ui.available_width() < 820.0;
            let height = if stacked {
                (available / 2.0 - 96.0).max(140.0)
            } else {
                (available - 96.0).max(180.0)
            };
            if stacked {
                self.input_card(ui, height);
                ui.add_space(theme::SPACE_2);
                self.output_card(ui, height);
            } else {
                let total = ui.available_width();
                let input_width = total * 0.42;
                let output_width = total - input_width - theme::SPACE_3;
                ui.horizontal(|ui| {
                    ui.allocate_ui_with_layout(
                        Vec2::new(input_width, ui.available_height()),
                        egui::Layout::top_down(egui::Align::Min),
                        |ui| {
                            ui.set_width(input_width);
                            self.input_card(ui, height);
                        },
                    );
                    ui.add_space(theme::SPACE_3);
                    ui.allocate_ui_with_layout(
                        Vec2::new(output_width, ui.available_height()),
                        egui::Layout::top_down(egui::Align::Min),
                        |ui| {
                            ui.set_width(output_width);
                            self.output_card(ui, height);
                        },
                    );
                });
            }
            if ready && ui.input(|i| i.modifiers.ctrl && i.key_pressed(egui::Key::Enter)) {
                self.manual_translate();
            }
        });
    }

    pub(super) fn live_ui(&mut self, ui: &mut egui::Ui) {
        let layout = LiveLayout::for_height(ui.ctx().input(|input| input.content_rect().height()));
        page_column(ui, |ui| {
            self.page_header(
                ui,
                "ライブ翻訳",
                "字幕とPC音声を、通しの日本語で読みます。",
                |_| {},
            );
            ui.add_space(theme::SPACE_2);
            self.live_sources_row(ui);
            ui.add_space(theme::SPACE_2);
            self.live_status_region(ui, layout);
            ui.add_space(theme::SPACE_2);
            let current = self
                .document_visible_row()
                .filter(|row| row.is_provisional() || row.is_confirmed());
            let mode = super::live_view::latest_mode(
                current.is_none() && !self.asr_partial.trim().is_empty(),
                current.is_some(),
            );
            #[cfg(test)]
            let top_before = ui.cursor().top();
            let mut read_requested = live_latest_panel(
                ui,
                current,
                &self.asr_partial,
                mode,
                layout,
                (
                    self.settings.font_size,
                    self.settings.live_show_original,
                    self.settings.auto_scroll,
                ),
            );
            #[cfg(test)]
            {
                self.ui_probe.now_rect = Some(egui::Rect::from_min_max(
                    egui::Pos2::new(ui.min_rect().left(), top_before),
                    egui::Pos2::new(ui.min_rect().right(), ui.cursor().top()),
                ));
                self.ui_probe.reading_font = theme::reading_size(self.settings.font_size);
            }
            ui.add_space(theme::SPACE_2);
            let history_ids: Vec<u64> = self
                .rows
                .iter()
                .filter(|row| history_row_visible(row, self.current_live_id))
                .map(|row| row.record.id)
                .collect();
            live_region(ui, "live-history-heading", HISTORY_HEADING_HEIGHT, |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new("これまでの文章").size(12.0).color(TEXT_MUTED));
                    ui.label(
                        RichText::new(format!("{}件", history_ids.len()))
                            .size(12.0)
                            .color(TEXT_MUTED),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(
                            RichText::new(if self.settings.auto_scroll {
                                "新しい順に追従"
                            } else {
                                "読んでいる位置を保持"
                            })
                            .size(11.0)
                            .color(TEXT_MUTED),
                        );
                    });
                });
            });
            self.live_view.prepare_history(self.settings.auto_scroll);
            let area = egui::ScrollArea::vertical()
                .id_salt("live-confirmed-history")
                .auto_shrink([false, false])
                .stick_to_bottom(self.settings.auto_scroll);
            let output = area.show_viewport(ui, |ui, viewport| {
                let origin = ui.cursor().top();
                let mut geometry = Vec::with_capacity(history_ids.len());
                if history_ids.is_empty() {
                    ui.add_space(theme::SPACE_4);
                    ui.label(
                        RichText::new("まとまった文章がここに並びます。")
                            .size(16.0)
                            .color(TEXT_SECONDARY),
                    );
                    ui.add(
                        egui::Label::new(
                            RichText::new("YouTube字幕またはPC音声をオンにしてください。")
                                .size(13.0)
                                .color(TEXT_MUTED),
                        )
                        .wrap(),
                    );
                }
                for row in self
                    .rows
                    .iter()
                    .filter(|row| history_row_visible(row, self.current_live_id))
                {
                    let rect = ui
                        .push_id(("history-row", row.record.id), |ui| {
                            measure_block(ui, self.settings.font_size, |ui| {
                                let rect = live_history_card(
                                    ui,
                                    row,
                                    self.settings.font_size,
                                    self.settings.live_show_original,
                                    &mut read_requested,
                                );
                                ui.add_space(theme::SPACE_2);
                                ui.separator();
                                rect
                            })
                        })
                        .inner;
                    geometry.push(super::live_view::RowGeometry {
                        id: row.record.id,
                        top: rect.top() - origin,
                        bottom: rect.bottom() - origin,
                    });
                    ui.add_space(theme::SPACE_2);
                }
                (geometry, viewport)
            });
            let pointer = ui.input(|input| input.pointer.hover_pos());
            #[cfg(test)]
            {
                self.live_view.history_viewport_height = output
                    .inner_rect
                    .intersect(ui.clip_rect())
                    .height()
                    .max(0.0);
                self.live_view.history_order = output.inner.0.iter().map(|row| row.id).collect();
                self.live_view.history_geometry = output.inner.0.clone();
                self.live_view.history_offset = output.state.offset.y;
                self.live_view.history_rect = Some(output.inner_rect.intersect(ui.clip_rect()));
            }
            let hovering =
                pointer.is_some_and(|point| output.inner_rect.expand(18.0).contains(point));
            let user_scrolling = hovering
                && ui.input(|input| input.smooth_scroll_delta.y != 0.0 || input.pointer.any_down())
                || output.state.velocity().y.abs() > 0.1;
            if let Some(offset) = self.live_view.finish(
                &output.inner.0,
                output.state.offset.y,
                output.inner_rect.height(),
                output.content_size.y,
                user_scrolling,
            ) {
                let mut state = output.state;
                state.offset.y = offset;
                state.store(ui.ctx(), output.id);
                // Correct the sizing pass before presenting resized history content.
                // If another widget used the extra pass, the stored offset still
                // applies on the following repaint rather than accumulating drift.
                ui.ctx().request_discard("keep live history row anchored");
                ui.ctx().request_repaint();
            }
            if let Some(text) = read_requested.filter(|_| !ui.ctx().will_discard()) {
                self.read_translation(text);
            }
        });
    }

    pub(super) fn files_ui(&mut self, ui: &mut egui::Ui) {
        page_column(ui, |ui| {
            let ready = self.backend_ready && !self.file_busy;
            let has_rows = !self.rows.is_empty();
            let mut translate_file = false;
            let mut cancel_file = false;
            let mut export = false;
            self.page_header(
                ui,
                "ファイル・履歴",
                "テキスト・Markdown・SRT・WebVTTを翻訳します。字幕の時刻は維持されます。",
                |ui| {
                    if ui
                        .add_enabled(has_rows, standard("タイムラインを書き出す"))
                        .clicked()
                    {
                        export = true;
                    }
                    if ui
                        .add_enabled(ready, primary("ファイルを選んで翻訳"))
                        .clicked()
                    {
                        translate_file = true;
                    }
                },
            );
            if translate_file {
                self.translate_file();
            }
            if export {
                self.export_history();
            }
            ui.add_space(theme::SPACE_2);
            live_region(ui, "files-languages", 36.0, |ui| {
                self.languages(ui);
            });
            self.backend_notice(ui);
            if self.file_busy {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(
                        RichText::new("ファイルを翻訳しています")
                            .size(13.0)
                            .color(TEXT_SECONDARY),
                    );
                    if ui.add(standard("中止")).clicked() {
                        cancel_file = true;
                    }
                });
                if cancel_file {
                    self.file_cancel.store(true, Ordering::Relaxed);
                }
            }
            ui.add_space(theme::SPACE_2);
            ui.add_sized(
                [ui.available_width(), theme::CONTROL_H],
                egui::TextEdit::singleline(&mut self.search)
                    .hint_text("原文・訳文を検索")
                    .margin(8.0),
            );
            ui.add_space(theme::SPACE_2);
            egui::ScrollArea::vertical()
                .id_salt("file-history")
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    let search = self.search.to_lowercase();
                    let mut shown = 0usize;
                    for row in self.rows.iter().rev() {
                        if !search.is_empty()
                            && !row.record.source.to_lowercase().contains(&search)
                            && !row.record.translation.to_lowercase().contains(&search)
                        {
                            continue;
                        }
                        shown += 1;
                        ui.add_space(theme::SPACE_2);
                        ui.label(
                            RichText::new(format!(
                                "{} · {}",
                                row.record.created_at, row.record.kind
                            ))
                            .size(12.0)
                            .color(TEXT_MUTED),
                        );
                        ui.add(
                            egui::Label::new(
                                RichText::new(&row.record.translation)
                                    .size(theme::reading_size(self.settings.font_size - 2.0))
                                    .color(TEXT),
                            )
                            .wrap()
                            .selectable(true),
                        );
                        ui.add(
                            egui::Label::new(
                                RichText::new(&row.record.source)
                                    .size(14.0)
                                    .color(TEXT_MUTED),
                            )
                            .wrap()
                            .selectable(true),
                        );
                        ui.add_space(theme::SPACE_2);
                        ui.separator();
                    }
                    if self.rows.is_empty() {
                        ui.add_space(theme::SPACE_4);
                        ui.label(theme::body("翻訳履歴はここに表示されます。"));
                    } else if shown == 0 {
                        ui.add_space(theme::SPACE_4);
                        ui.label(
                            RichText::new("一致する履歴がありません。")
                                .size(15.0)
                                .color(TEXT_SECONDARY),
                        );
                    }
                });
        });
    }

    fn settings_translation_card(&mut self, ui: &mut egui::Ui) {
        ui.set_min_width(ui.available_width());
        ui.label(RichText::new("翻訳モデル").strong());
        ui.label(self.setup_summary());
        secondary(ui, self.connection_label());
        if ui.add(standard("モデルを変更")).clicked() {
            self.open_setup();
        }
    }

    fn settings_asr_card(&mut self, ui: &mut egui::Ui) {
        ui.set_min_width(ui.available_width());
        ui.label(RichText::new("音声認識").strong());
        egui::ComboBox::from_id_salt("asr-backend")
            .selected_text(match self.settings.asr_backend.as_str() {
                "nemotron" => "Nemotron Speech · 英語 / 連続認識",
                "qwen" => "Qwen3-ASR · 多言語",
                _ => "Whisper · 多言語 / CPU",
            })
            .show_ui(ui, |ui| {
                if ui
                    .selectable_value(
                        &mut self.settings.asr_backend,
                        "nemotron".into(),
                        "Nemotron Speech · 英語 / 連続認識",
                    )
                    .changed()
                {
                    self.settings.asr_model = "nvidia/nemotron-speech-streaming-en-0.6b".into();
                    self.settings.asr_endpoint = "http://127.0.0.1:17864".into();
                    self.settings.asr_device = "auto".into();
                }
                if ui
                    .selectable_value(
                        &mut self.settings.asr_backend,
                        "qwen".into(),
                        "Qwen3-ASR · 多言語",
                    )
                    .changed()
                {
                    self.settings.asr_model = "Qwen/Qwen3-ASR-0.6B-hf".into();
                    self.settings.asr_endpoint = "http://127.0.0.1:17861".into();
                    self.settings.asr_device = self
                        .setup
                        .recommendation
                        .as_ref()
                        .filter(|rec| rec.asr_backend == "qwen")
                        .map(|rec| rec.asr_device.clone())
                        .unwrap_or_else(|| "cpu".into());
                }
                if ui
                    .selectable_value(
                        &mut self.settings.asr_backend,
                        "whisper".into(),
                        "Whisper · 多言語 / CPU",
                    )
                    .changed()
                {
                    self.settings.asr_model = "base".into();
                    self.settings.asr_endpoint = "http://127.0.0.1:17861".into();
                    self.settings.asr_device = "cpu".into();
                }
            });
        secondary(
            ui,
            if self.asr_ready {
                "音声認識の準備完了"
            } else {
                "初回は必要なモデルを準備します"
            },
        );
        egui::CollapsingHeader::new("音声認識の詳細")
            .id_salt("asr-details")
            .default_open(false)
            .show(ui, |ui| {
                if self.settings.asr_backend == "nemotron" {
                    secondary(ui, "英語専用です。続きの原文をつないで訳文を更新し、まとまった文章を保存します。");
                    ui.horizontal_wrapped(|ui| {
                        secondary(ui, "認識モード");
                        ui.selectable_value(&mut self.settings.nemotron_context, 6, "精度優先 · 560ms");
                        ui.selectable_value(&mut self.settings.nemotron_context, 1, "速さ優先 · 160ms");
                    });
                    secondary(ui, "上の時間はモデルの音声処理単位です。表示までの時間には、文の区切りと翻訳の待ち時間も含まれます。");
                    ui.horizontal_wrapped(|ui| {
                        secondary(ui, "実行方式");
                        for (value, label) in [("auto", "自動"), ("vulkan", "GPU · Vulkan"), ("cpu", "CPU")] {
                            ui.selectable_value(&mut self.settings.asr_device, value.into(), label);
                        }
                    });
                } else {
                    ui.horizontal_wrapped(|ui| {
                        secondary(ui, "音声の区切り");
                        for (value, label) in [(2.0, "短め"), (3.0, "標準"), (4.0, "長め")] {
                            ui.selectable_value(&mut self.settings.audio_seconds, value, label);
                        }
                    });
                }
            });
        ui.add_space(theme::SPACE_1);
        if ui.add(standard("音声認識を準備")).clicked() {
            self.prepare_audio();
        }
    }

    fn settings_youtube_card(&mut self, ui: &mut egui::Ui) {
        ui.set_min_width(ui.available_width());
        ui.label(RichText::new("YouTube字幕の接続").strong());
        let receiving = self.caption_enabled
            && !self.caption_paused
            && self
                .last_caption
                .is_some_and(|time| time.elapsed() < Duration::from_secs(8));
        let (state, color) = if receiving {
            ("字幕を受信中", theme::SUCCESS)
        } else if self.caption_enabled {
            ("接続待ち", WARNING)
        } else {
            ("停止中", TEXT_MUTED)
        };
        ui.horizontal_wrapped(|ui| {
            badge(ui, state, color);
            if self.caption_enabled {
                if ui.add(standard("停止")).clicked() {
                    self.set_caption_enabled(false);
                }
            } else if ui.add(standard("字幕を受け取る")).clicked() {
                self.set_caption_enabled(true);
            }
        });
        ui.add_space(theme::SPACE_2);
        for (number, text) in [
            (
                1,
                "上の「字幕を受け取る」を押します。ライブ翻訳の入力切替で「YouTube字幕」を選んでも同じです。",
            ),
            (
                2,
                "Chromeは chrome://extensions、Edgeは edge://extensions を開き、開発者モードをオンにします。",
            ),
            (
                3,
                "「パッケージ化されていない拡張機能を読み込む」で、下の「字幕連携フォルダー」を選びます。",
            ),
            (
                4,
                "拡張機能の設定画面に接続キーを貼り付け、「保存」→「接続確認」を押します。",
            ),
            (
                5,
                "YouTubeで動画を開き、CCをオンにして英語などの字幕を選びます。表示中の字幕が約300msごとに届きます。",
            ),
        ] {
            ui.horizontal_top(|ui| {
                ui.add_sized(
                    [20.0, 18.0],
                    egui::Label::new(
                        RichText::new(format!("{number}"))
                            .size(12.0)
                            .strong()
                            .color(ACCENT),
                    ),
                );
                ui.add(
                    egui::Label::new(RichText::new(text).size(13.0).color(TEXT_SECONDARY)).wrap(),
                );
            });
            ui.add_space(theme::SPACE_1);
        }
        ui.label(RichText::new("接続キー").size(12.0).color(TEXT_MUTED));
        ui.horizontal_wrapped(|ui| {
            ui.add(
                egui::TextEdit::singleline(&mut self.settings.bridge_token)
                    .password(true)
                    .interactive(false)
                    .desired_width(220.0),
            );
            if ui.add(standard("接続キーをコピー")).clicked()
                && let Ok(mut clipboard) = arboard::Clipboard::new()
            {
                let _ = clipboard.set_text(self.settings.bridge_token.clone());
            }
            if ui.add(standard("字幕連携フォルダー")).clicked() {
                let mut command = Command::new("explorer.exe");
                command.arg(app_root().join("extension"));
                command_hidden(&mut command);
                let _ = command.spawn();
            }
        });
        secondary(
            ui,
            "字幕はこのPC内の 127.0.0.1:17862 にだけ送られ、外部には送信されません。",
        );
    }

    fn settings_glossary_section(&mut self, ui: &mut egui::Ui) {
        egui::CollapsingHeader::new("用語集と文脈")
            .id_salt("glossary")
            .default_open(false)
            .show(ui, |ui| {
                secondary(ui, "1行に「原語 = 訳語」を入力します。例: inference = 推論");
                ui.add_sized(
                    [ui.available_width(), 160.0],
                    egui::TextEdit::multiline(&mut self.glossary_input)
                        .desired_width(f32::INFINITY)
                        .margin(12.0),
                );
                if let Err(error) = parse_glossary(&self.glossary_input) {
                    ui.colored_label(ERROR, error);
                }
                ui.label(RichText::new("文脈（翻訳する文章の背景）").size(14.0));
                ui.add_sized(
                    [ui.available_width(), 88.0],
                    egui::TextEdit::multiline(&mut self.settings.translation.context)
                        .desired_width(f32::INFINITY)
                        .margin(12.0),
                );
            });
    }

    fn settings_shortcuts_section(&mut self, ui: &mut egui::Ui) {
        egui::CollapsingHeader::new("ショートカットと保存先")
            .id_salt("shortcuts")
            .default_open(false)
            .show(ui, |ui| {
                secondary(ui, "範囲翻訳: Ctrl + Alt + T　 音声翻訳: Ctrl + Alt + L");
                if ui.add(standard("保存フォルダーを開く")).clicked() {
                    self.open_data_folder();
                }
            });
    }

    fn settings_diagnostics_section(&mut self, ui: &mut egui::Ui) {
        egui::CollapsingHeader::new("詳細（トラブル時）")
            .id_salt("diagnostics")
            .default_open(false)
            .show(ui, |ui| {
                for (label, value) in [
                    ("翻訳モデルのファイル", &mut self.settings.model),
                    ("翻訳エンジン", &mut self.settings.llama_server),
                    ("翻訳サーバー", &mut self.settings.translation.endpoint),
                    ("音声認識モデル", &mut self.settings.asr_model),
                    ("音声認識の実行環境", &mut self.settings.python),
                    ("音声認識サーバー", &mut self.settings.asr_endpoint),
                ] {
                    ui.label(RichText::new(label).size(14.0));
                    ui.add(egui::TextEdit::singleline(value).desired_width(f32::INFINITY));
                }
                ui.label(
                    RichText::new(format!("保存先: {}", data_root().display()))
                        .size(12.0)
                        .color(TEXT_MUTED),
                );
                if ui.add(standard("翻訳エンジンを起動")).clicked() {
                    self.start_translation_server();
                }
            });
    }

    pub(super) fn settings_ui(&mut self, ui: &mut egui::Ui) {
        let previous_asr = (
            self.settings.asr_backend.clone(),
            self.settings.asr_model.clone(),
            self.settings.asr_endpoint.clone(),
            self.settings.asr_device.clone(),
            self.settings.nemotron_context,
        );
        page_column(ui, |ui| {
            self.page_header(ui, "設定", "翻訳と音声認識はこのPC内で処理します。", |_| {});
            ui.add_space(theme::SPACE_2);
            egui::ScrollArea::vertical()
                .id_salt("settings-scroll")
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    // Two independent columns above 1100 points so a wide
                    // window does not become one very long form.
                    if ui.available_width() >= 1100.0 {
                        ui.columns(2, |columns| {
                            columns[0].spacing_mut().item_spacing.y = theme::SPACE_3;
                            columns[1].spacing_mut().item_spacing.y = theme::SPACE_3;
                            card().show(&mut columns[0], |ui| self.settings_translation_card(ui));
                            card().show(&mut columns[0], |ui| self.readout_settings_ui(ui));
                            card().show(&mut columns[0], |ui| self.paragraph_tools_settings_ui(ui));
                            self.settings_glossary_section(&mut columns[0]);
                            card().show(&mut columns[1], |ui| self.settings_asr_card(ui));
                            card().show(&mut columns[1], |ui| self.screen_ocr_settings_ui(ui));
                            card().show(&mut columns[1], |ui| self.settings_youtube_card(ui));
                            self.settings_shortcuts_section(&mut columns[1]);
                            self.settings_diagnostics_section(&mut columns[1]);
                        });
                    } else {
                        card().show(ui, |ui| self.settings_translation_card(ui));
                        ui.add_space(theme::SPACE_2);
                        card().show(ui, |ui| self.readout_settings_ui(ui));
                        ui.add_space(theme::SPACE_2);
                        card().show(ui, |ui| self.settings_asr_card(ui));
                        ui.add_space(theme::SPACE_2);
                        card().show(ui, |ui| self.screen_ocr_settings_ui(ui));
                        ui.add_space(theme::SPACE_2);
                        card().show(ui, |ui| self.paragraph_tools_settings_ui(ui));
                        ui.add_space(theme::SPACE_2);
                        card().show(ui, |ui| self.settings_youtube_card(ui));
                        ui.add_space(theme::SPACE_2);
                        self.settings_glossary_section(ui);
                        self.settings_shortcuts_section(ui);
                        self.settings_diagnostics_section(ui);
                    }
                    ui.add_space(theme::SPACE_4);
                    if ui
                        .add_enabled(
                            parse_glossary(&self.glossary_input).is_ok(),
                            primary("設定を保存"),
                        )
                        .clicked()
                    {
                        match parse_glossary(&self.glossary_input) {
                            Ok(glossary) => {
                                self.settings.translation.glossary = glossary;
                                match self.persist_preferences() {
                                    Ok(()) => {
                                        self.settings_load_failed = false;
                                        self.status = "設定を保存しました。".into();
                                    }
                                    Err(error) => self.status = error.to_string(),
                                }
                            }
                            Err(error) => self.status = error,
                        }
                    }
                    ui.add_space(theme::SPACE_4);
                    if ui.add(standard("アプリを終了")).clicked() {
                        ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                    ui.add_space(theme::SPACE_4);
                });
        });
        if previous_asr
            != (
                self.settings.asr_backend.clone(),
                self.settings.asr_model.clone(),
                self.settings.asr_endpoint.clone(),
                self.settings.asr_device.clone(),
                self.settings.nemotron_context,
            )
        {
            if self.audio_enabled {
                self.stop_audio();
            }
            self.stop_asr();
            if previous_asr.0 != self.settings.asr_backend
                && let Err(error) = self.load_audio_installation()
            {
                self.service_error = error.to_string();
            }
            self.request_service_status();
        }
    }
    fn rail(&mut self, ctx: &egui::Context) {
        egui::SidePanel::left("rail")
            .exact_width(theme::RAIL_WIDTH)
            .resizable(false)
            .frame(
                egui::Frame::new()
                    .fill(SURFACE)
                    .inner_margin(egui::Margin::symmetric(8, 12)),
            )
            .show(ctx, |ui| {
                #[cfg(test)]
                {
                    self.ui_probe.rail_width = theme::RAIL_WIDTH;
                }
                ui.vertical_centered(|ui| {
                    ui.add(
                        egui::Label::new(
                            RichText::new("地元翻訳くん")
                                .size(11.0)
                                .strong()
                                .color(TEXT),
                        )
                        .wrap(),
                    );
                    ui.add(egui::Label::new(RichText::new(credit::CREDIT).size(7.5)).wrap());
                });
                ui.add_space(theme::SPACE_4);
                for (tab, label) in [
                    (Tab::Text, "文章"),
                    (Tab::Live, "ライブ"),
                    (Tab::Files, "履歴"),
                    (Tab::Settings, "設定"),
                ] {
                    let response = rail_item(ui, label, self.tab == tab);
                    #[cfg(test)]
                    {
                        self.ui_probe.nav_heights.push(response.rect.height());
                    }
                    if response.clicked() {
                        self.tab = tab;
                    }
                    ui.add_space(theme::SPACE_1);
                }
                // Pin the quick actions to the bottom so a shortage of height
                // can never push the last control outside the window.
                ui.with_layout(egui::Layout::bottom_up(egui::Align::Center), |ui| {
                    ui.add(
                        egui::Label::new(
                            RichText::new("翻訳はこのPC内で処理します")
                                .size(10.0)
                                .color(TEXT_MUTED),
                        )
                        .wrap(),
                    );
                    ui.add_space(theme::SPACE_2);
                    if !self.backend_ready
                        && ui
                            .add_sized([ui.available_width(), theme::CONTROL_H], standard("モデル"))
                            .on_hover_text("翻訳モデルの準備を開きます")
                            .clicked()
                    {
                        self.open_setup();
                    }
                    let audio_label = if self.audio_enabled {
                        "音声停止"
                    } else if self.audio_start_requested {
                        "開始取消"
                    } else {
                        "音声翻訳"
                    };
                    let can_toggle =
                        self.backend_ready || self.audio_enabled || self.audio_start_requested;
                    if ui
                        .add_enabled_ui(can_toggle, |ui| {
                            ui.add_sized(
                                [ui.available_width(), theme::CONTROL_H_LARGE],
                                standard(audio_label),
                            )
                        })
                        .inner
                        .clicked()
                    {
                        if self.audio_start_requested && !self.audio_enabled {
                            self.audio_start_requested = false;
                        } else if self.audio_enabled
                            || (self.audio_environment_ready()
                                && self.asr_ready
                                && self.backend_ready)
                        {
                            self.toggle_audio();
                        } else {
                            self.audio_start_requested = true;
                            self.prepare_audio();
                        }
                    }
                    ui.add_space(theme::SPACE_1);
                    let capture_ready = self.backend_ready && !self.capture_busy;
                    if ui
                        .add_enabled(
                            capture_ready,
                            egui::Button::new(RichText::new("範囲翻訳").size(13.0).color(TEXT))
                                .min_size(Vec2::new(ui.available_width(), theme::CONTROL_H_LARGE)),
                        )
                        .on_hover_text("画面の一部を選んで翻訳します。Ctrl + Alt + T")
                        .clicked()
                    {
                        self.capture(ctx);
                    }
                    if self.screens.len() > 1 {
                        let mut index = self.screen_index;
                        egui::ComboBox::from_id_salt("screen")
                            .width(ui.available_width())
                            .selected_text(format!("画面 {}", index + 1))
                            .show_ui(ui, |ui| {
                                for (position, screen) in self.screens.iter().enumerate() {
                                    ui.selectable_value(
                                        &mut index,
                                        position,
                                        format!("{}: {}", position + 1, screen.name),
                                    );
                                }
                            });
                        self.screen_index = index;
                    }
                    ui.add_space(theme::SPACE_2);
                    ui.separator();
                });
            });
    }

    pub(super) fn render_ui(&mut self, ctx: &egui::Context) {
        if self.close_after_restore.is_some() {
            egui::CentralPanel::default().show(ctx, |ui| {
                ui.label("終了しています…");
            });
            return;
        }
        if ctx.input_mut(|input| {
            input.consume_key(egui::Modifiers::CTRL | egui::Modifiers::SHIFT, egui::Key::M)
        }) {
            if self.window_mode.is_mini() {
                self.restore_normal_view(ctx);
            } else {
                self.enter_mini_view(ctx);
            }
        }
        if self.window_mode.is_mini() {
            self.mini_ui(ctx);
            if self.selection.is_none()
                && !egui::Popup::is_any_open(ctx)
                && ctx
                    .input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Escape))
            {
                self.restore_normal_view(ctx);
            }
            return;
        }
        egui::TopBottomPanel::bottom("status")
            .min_height(28.0)
            .frame(
                egui::Frame::new()
                    .fill(SURFACE)
                    .inner_margin(egui::Margin::symmetric(theme::GUTTER as i8, 4)),
            )
            .show(ctx, |ui| {
                ui.horizontal_wrapped(|ui| {
                    self.model_dots_ui(ui);
                    ui.label(
                        RichText::new(format!(
                            "{} · {}",
                            self.current_model_label(),
                            self.connection_label()
                        ))
                        .size(12.0)
                        .color(if self.backend_ready {
                            ACCENT
                        } else {
                            TEXT_MUTED
                        }),
                    );
                    if !self.status.is_empty() {
                        ui.label(RichText::new(&self.status).size(12.0).color(TEXT_SECONDARY));
                    }
                    if !self.readout.error.is_empty() {
                        ui.label(RichText::new(&self.readout.error).size(12.0).color(ERROR));
                    } else if !self.readout.status.is_empty() {
                        ui.label(
                            RichText::new(&self.readout.status)
                                .size(12.0)
                                .color(TEXT_SECONDARY),
                        );
                    }
                });
            });
        self.rail(ctx);
        egui::CentralPanel::default()
            .frame(
                egui::Frame::new()
                    .fill(CANVAS)
                    .inner_margin(egui::Margin::symmetric(
                        theme::GUTTER as i8,
                        theme::SPACE_4 as i8,
                    )),
            )
            .show(ctx, |ui| match self.tab {
                Tab::Text => self.text_ui(ui),
                Tab::Live => self.live_ui(ui),
                Tab::Files => self.files_ui(ui),
                Tab::Settings => self.settings_ui(ui),
                Tab::Setup => self.setup_ui(ui),
            });
    }
}
#[cfg(test)]
mod live_layout_tests {
    use super::*;
    fn row(id: u64, pending: bool, error: bool) -> Row {
        Row {
            record: Record {
                id,
                start_ms: 0,
                end_ms: 1000,
                source: "Long source text. ".repeat(300),
                translation: "Long translated text. ".repeat(300),
                kind: "音声".into(),
                video_id: String::new(),
                created_at: String::new(),
                paragraph: None,
            },
            pending,
            error: error.then(|| "Long error message. ".repeat(200)),
            generation: 0,
        }
    }
    #[test]
    fn paragraph_interim_stays_above_history_when_http_idle_and_a_new_draft_arrives() {
        use plamo_translate::journal::{ParagraphMeta, ParagraphState};
        let context = egui::Context::default();
        configure_theme(&context);
        let mut paragraph = row(7, false, false);
        paragraph.record.translation = "文章全体をつないだ継続中の日本語訳".into();
        paragraph.record.paragraph = Some(ParagraphMeta {
            revision: 4,
            state: ParagraphState::Open,
            close_reason: None,
        });
        assert!(!history_row_visible(&paragraph, None));
        let output = context.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(860.0, 620.0),
                )),
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    live_latest_panel(
                        ui,
                        Some(&paragraph),
                        "Additional unconfirmed draft",
                        crate::live_view::LatestMode::Draft,
                        LiveLayout::for_height(620.0),
                        (22.0, true, true),
                    );
                });
            },
        );
        fn text(shape: &egui::epaint::Shape) -> String {
            match shape {
                egui::epaint::Shape::Text(t) => t.galley.job.text.clone(),
                egui::epaint::Shape::Vec(shapes) => {
                    shapes.iter().map(text).collect::<Vec<_>>().join("\n")
                }
                _ => String::new(),
            }
        }
        let drawn = output
            .shapes
            .iter()
            .map(|shape| text(&shape.shape))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(drawn.contains("文章全体をつないだ継続中の日本語訳"));
        assert!(drawn.contains("継続中の文章"));
        assert!(drawn.contains("追加で認識中の原文（未確定）"));
        paragraph.record.paragraph.as_mut().unwrap().state = ParagraphState::Closing;
        assert!(!history_row_visible(&paragraph, None));
        paragraph.record.paragraph.as_mut().unwrap().state = ParagraphState::Final;
        assert!(history_row_visible(&paragraph, None));
        paragraph.record.paragraph.as_mut().unwrap().state = ParagraphState::Interrupted;
        assert!(history_row_visible(&paragraph, Some(7)));
        assert!(paragraph_label(&paragraph).contains("中断"));
        paragraph.record.paragraph.as_mut().unwrap().state = ParagraphState::Failed;
        assert!(history_row_visible(&paragraph, Some(7)));
        assert!(paragraph_label(&paragraph).contains("未確定"));
    }
    #[test]
    fn latest_panel_has_constant_outer_height_for_empty_draft_partial_and_completed_text() {
        use crate::live_view::LatestMode;
        for viewport_height in [620.0, 880.0] {
            let context = egui::Context::default();
            let mut heights = Vec::new();
            let layout = LiveLayout::for_height(viewport_height);
            for (mode, pending, error) in [
                (LatestMode::Empty, false, false),
                (LatestMode::Draft, false, false),
                (LatestMode::Current, true, false),
                (LatestMode::Current, false, false),
                (LatestMode::Current, false, true),
            ] {
                let row = row(9, pending, error);
                let input = egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        Vec2::new(900.0, viewport_height),
                    )),
                    ..Default::default()
                };
                let _ = context.run(input, |context| {
                    egui::CentralPanel::default().show(context, |ui| {
                        let before = ui.cursor().top();
                        live_latest_panel(
                            ui,
                            Some(&row),
                            &"Draft text. ".repeat(300),
                            mode,
                            layout,
                            (24.0, true, true),
                        );
                        heights.push(ui.cursor().top() - before);
                    });
                });
            }
            assert!(
                heights
                    .iter()
                    .all(|height| (*height - heights[0]).abs() < 0.1),
                "outer heights changed: {heights:?}"
            );
            assert!((heights[0] - layout.latest_height).abs() < 16.0);
        }
        assert_eq!(LiveLayout::for_height(800.0).latest_height, 288.0);
        assert_eq!(LiveLayout::for_height(800.0).status_height, 56.0);
    }
    #[test]
    fn live_japanese_has_complete_rows_at_minimum_and_dpi_scaled_window_heights() {
        use plamo_translate::journal::{ParagraphMeta, ParagraphState};
        fn collect<'a>(
            shape: &'a egui::epaint::Shape,
            clip: egui::Rect,
            out: &mut Vec<(egui::Rect, &'a egui::epaint::TextShape)>,
        ) {
            match shape {
                egui::epaint::Shape::Text(text) => out.push((clip, text)),
                egui::epaint::Shape::Vec(shapes) => {
                    for shape in shapes {
                        collect(shape, clip, out);
                    }
                }
                _ => {}
            }
        }
        for height in [620.0, 727.0, 800.0, 880.0] {
            for size in [18.0, 20.0, 24.0] {
                let context = egui::Context::default();
                configure_theme(&context);
                let mut app = App::inert();
                app.backend_ready = true;
                app.asr_ready = true;
                app.caption_enabled = true;
                app.settings.font_size = size;
                assert!(!app.settings.live_show_original);
                let japanese:String = "文章全体の文脈を保ちながら原文の続きを受け取り、一つの段落として訳文を更新します。".chars().cycle().take(600).collect();
                let mut active = row(2, false, false);
                active.record.translation = japanese.clone();
                active.record.paragraph = Some(ParagraphMeta {
                    revision: 1,
                    state: ParagraphState::Open,
                    close_reason: None,
                });
                app.rows = vec![row(1, false, false), active];
                app.current_live_id = Some(2);
                let mut output = None;
                for frame in 0..4 {
                    let mut input = egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(if height == 620.0 { 860.0 } else { 1182.0 }, height),
                        )),
                        time: Some(frame as f64 / 60.0),
                        ..Default::default()
                    };
                    input
                        .viewports
                        .entry(egui::ViewportId::ROOT)
                        .or_default()
                        .native_pixels_per_point = Some(1.25);
                    output = Some(context.run(input, |ctx| app.render_ui(ctx)));
                }
                let output = output.unwrap();
                let mut text_shapes = Vec::new();
                for shape in &output.shapes {
                    collect(&shape.shape, shape.clip_rect, &mut text_shapes);
                }
                let (clip, text) = text_shapes
                    .iter()
                    .find(|(_, text)| text.galley.job.text == japanese)
                    .expect("progress Japanese was hidden");
                let mut complete_rows = 0;
                for row in &text.galley.rows {
                    let bounds = row
                        .visuals
                        .mesh_bounds
                        .translate(text.pos.to_vec2() + row.pos.to_vec2());
                    if bounds.intersect(*clip).height() > 0.5 {
                        assert!(
                            bounds.top() >= clip.top() - 0.5
                                && bounds.bottom() <= clip.bottom() + 0.5,
                            "partial glyph row at height={height},font={size}: {bounds:?},clip={clip:?}"
                        );
                        complete_rows += 1;
                    }
                }
                eprintln!(
                    "live height={height} font={size}: {complete_rows} complete rows; history={}",
                    app.live_view.history_viewport_height
                );
                if height >= 727.0 {
                    assert!(
                        complete_rows >= 4,
                        "only {complete_rows} complete rows at {height}/{size}"
                    );
                }
                assert!(
                    app.live_view.history_viewport_height >= 100.0,
                    "history at {height}/{size}"
                );
                for (clip, text) in text_shapes
                    .iter()
                    .filter(|(_, text)| text.galley.job.text == "進行中の文章／最新の対訳")
                {
                    let bounds = text.galley.mesh_bounds.translate(text.pos.to_vec2());
                    assert!(
                        bounds.top() >= clip.top() - 0.5 && bounds.bottom() <= clip.bottom() + 0.5,
                        "progress toolbar clipped"
                    );
                }
            }
        }
    }
    #[test]
    fn minimum_live_window_keeps_more_than_one_hundred_pixels_of_history() {
        for (width, height) in [(860.0, 620.0), (1280.0, 800.0)] {
            let context = egui::Context::default();
            configure_theme(&context);
            let mut app = App::inert();
            app.backend_ready = true;
            app.asr_ready = true;
            app.audio_enabled = true;
            app.caption_enabled = true;
            app.rows = vec![
                row(1, false, false),
                row(2, false, false),
                row(3, false, false),
            ];
            app.current_live_id = Some(3);
            // Compact errors remain in the one reserved notice line, not hidden.
            app.service_error = "Long service error. ".repeat(100);
            for frame in 0..3 {
                let input = egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        Vec2::new(width, height),
                    )),
                    time: Some(frame as f64 / 60.0),
                    ..Default::default()
                };
                let _ = context.run(input, |context| app.render_ui(context));
            }
            eprintln!(
                "live history viewport {width}x{height}: {}",
                app.live_view.history_viewport_height
            );
            assert!(
                app.live_view.history_viewport_height > 100.0,
                "history at {width}x{height}: {}",
                app.live_view.history_viewport_height
            );
        }
    }
    #[test]
    fn chronological_history_stays_anchored_when_reading_and_returns_to_tail_on_enable() {
        let context = egui::Context::default();
        configure_theme(&context);
        let mut app = App::inert();
        app.backend_ready = true;
        app.settings.live_show_original = false;
        app.settings.auto_scroll = true;
        app.rows = (1..=6)
            .map(|id| {
                let mut row = row(id, false, false);
                row.record.translation = format!(
                    "Paragraph {id}. {}",
                    "A complete translated paragraph. ".repeat(16)
                );
                row
            })
            .collect();
        let mut frame = 0;
        let mut draw = |app: &mut App, events: Vec<egui::Event>| {
            frame += 1;
            let _ = context.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1280.0, 880.0),
                    )),
                    time: Some(frame as f64 * 0.05),
                    events,
                    ..Default::default()
                },
                |ctx| app.render_ui(ctx),
            );
        };
        for _ in 0..5 {
            draw(&mut app, vec![]);
        }
        assert_eq!(app.live_view.history_order, vec![1, 2, 3, 4, 5, 6]);
        let tail = app.live_view.history_offset;
        assert!(tail > 100.0);
        let pointer = app.live_view.history_rect.unwrap().center();
        draw(
            &mut app,
            vec![
                egui::Event::PointerMoved(pointer),
                egui::Event::MouseWheel {
                    unit: egui::MouseWheelUnit::Point,
                    delta: egui::vec2(0.0, 100.0),
                    modifiers: egui::Modifiers::default(),
                },
            ],
        );
        for _ in 0..25 {
            draw(&mut app, vec![]);
        }
        let reading_offset = app.live_view.history_offset;
        assert!(
            reading_offset < tail - 1.0,
            "wheel did not leave tail: {reading_offset} vs {tail}"
        );
        let mut appended = row(7, false, false);
        appended.record.translation = "Newly finalized paragraph. ".repeat(16);
        app.rows.push(appended);
        for _ in 0..3 {
            draw(&mut app, vec![]);
        }
        assert_eq!(app.live_view.history_order, vec![1, 2, 3, 4, 5, 6, 7]);
        assert!(
            (app.live_view.history_offset - reading_offset).abs() < 1.0,
            "append moved reading position"
        );
        let offset = app.live_view.history_offset;
        let anchor = *app
            .live_view
            .history_geometry
            .iter()
            .find(|row| row.bottom > offset)
            .unwrap();
        assert!(anchor.id > 1);
        let anchor_screen_top = anchor.top - offset;
        app.rows[0]
            .record
            .translation
            .push_str(&"Earlier paragraph grew. ".repeat(20));
        for _ in 0..3 {
            draw(&mut app, vec![]);
        }
        let updated_anchor = app
            .live_view
            .history_geometry
            .iter()
            .find(|row| row.id == anchor.id)
            .unwrap();
        assert!(
            (updated_anchor.top - app.live_view.history_offset - anchor_screen_top).abs() < 1.0,
            "height change moved reading anchor"
        );
        app.settings.auto_scroll = false;
        draw(&mut app, vec![]);
        let old_offset = app.live_view.history_offset;
        app.settings.auto_scroll = true;
        for _ in 0..3 {
            draw(&mut app, vec![]);
        }
        assert!(
            app.live_view.history_offset > old_offset + 50.0,
            "explicit enable failed to return to tail"
        );
        let tail = app.live_view.history_offset;
        let mut appended = row(8, false, false);
        appended.record.translation = "The following finalized paragraph. ".repeat(16);
        app.rows.push(appended);
        for _ in 0..3 {
            draw(&mut app, vec![]);
        }
        assert!(
            app.live_view.history_offset > tail + 50.0,
            "tail did not follow the next append"
        );
    }

    fn live_window(app: &mut App, width: f32, height: f32) -> egui::Context {
        let context = egui::Context::default();
        configure_theme(&context);
        for frame in 0..3 {
            let _ = context.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(width, height),
                    )),
                    time: Some(frame as f64 / 60.0),
                    ..Default::default()
                },
                |context| app.render_ui(context),
            );
        }
        context
    }

    #[test]
    fn minimum_window_keeps_32_point_targets_and_a_large_reading_font() {
        let mut app = App::inert();
        app.backend_ready = true;
        app.tab = Tab::Live;
        app.rows = vec![row(1, false, false)];
        app.current_live_id = Some(1);
        live_window(&mut app, 860.0, 620.0);
        assert_eq!(app.ui_probe.rail_width, theme::RAIL_WIDTH);
        assert!(app.ui_probe.nav_heights.len() >= 4);
        assert!(
            app.ui_probe.nav_heights.iter().all(|h| *h >= 32.0),
            "navigation targets under 32 points: {:?}",
            app.ui_probe.nav_heights
        );
        assert!(
            app.ui_probe.reading_font >= 20.0,
            "reading font {} is below 20 points",
            app.ui_probe.reading_font
        );
        assert!(
            app.live_view.history_viewport_height > 100.0,
            "history viewport {} at 860x620",
            app.live_view.history_viewport_height
        );
        let now = app.ui_probe.now_rect.unwrap();
        assert!(
            now.height() >= 150.0,
            "in-progress area is only {} points",
            now.height()
        );
    }

    #[test]
    fn growing_translation_never_moves_the_header_or_history_viewport() {
        let mut app = App::inert();
        app.backend_ready = true;
        app.tab = Tab::Live;
        let mut active = row(2, false, false);
        active.record.translation = "短い訳。".into();
        app.rows = vec![row(1, false, false), active];
        app.current_live_id = Some(2);
        live_window(&mut app, 1280.0, 800.0);
        let now_before = app.ui_probe.now_rect.unwrap();
        let history_before = app.live_view.history_rect.unwrap();
        app.rows[1].record.translation = "段落全体を更新する長い日本語の訳文です。".repeat(40);
        live_window(&mut app, 1280.0, 800.0);
        let now_after = app.ui_probe.now_rect.unwrap();
        let history_after = app.live_view.history_rect.unwrap();
        assert!((now_before.top() - now_after.top()).abs() < 0.5);
        assert!((now_before.height() - now_after.height()).abs() < 0.5);
        assert!((history_before.top() - history_after.top()).abs() < 0.5);
        assert!((history_before.height() - history_after.height()).abs() < 0.5);
    }

    #[test]
    fn unconfirmed_paragraph_is_labelled_apart_from_confirmed_history() {
        use plamo_translate::journal::{ParagraphMeta, ParagraphState};
        let mut app = App::inert();
        app.backend_ready = true;
        app.tab = Tab::Live;
        let mut confirmed = row(1, false, false);
        confirmed.record.paragraph = Some(ParagraphMeta {
            revision: 3,
            state: ParagraphState::Final,
            close_reason: None,
        });
        let mut open = row(2, false, false);
        open.record.paragraph = Some(ParagraphMeta {
            revision: 1,
            state: ParagraphState::Open,
            close_reason: None,
        });
        assert_eq!(paragraph_label(&confirmed), "確定した文章");
        assert!(paragraph_label(&open).contains("更新中"));
        app.rows = vec![confirmed, open];
        app.current_live_id = Some(2);
        assert!(!history_row_visible(&app.rows[1], Some(2)));
        assert!(history_row_visible(&app.rows[0], Some(2)));
        assert!(row_is_unfinished(&app.rows[1]));
        assert!(!row_is_unfinished(&app.rows[0]));
    }
}
