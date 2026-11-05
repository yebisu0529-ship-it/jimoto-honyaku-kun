use super::*;
use plamo_translate::journal::ParagraphState;

const BG: Color32 = Color32::from_rgb(20, 24, 31);
const SIDEBAR: Color32 = Color32::from_rgb(16, 20, 25);
const SURFACE: Color32 = Color32::from_rgb(27, 33, 42);
const INPUT: Color32 = Color32::from_rgb(17, 22, 28);
const BORDER: Color32 = Color32::from_rgb(42, 50, 61);
const TEXT: Color32 = Color32::from_rgb(230, 234, 240);
const SECONDARY: Color32 = Color32::from_rgb(163, 173, 186);
const MINT: Color32 = Color32::from_rgb(60, 191, 164);
const ERROR: Color32 = Color32::from_rgb(240, 122, 122);

const LIVE_LATEST_HEIGHT: f32 = 312.0;

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
            latest_height: if height < 727.0 {
                152.0 + (height - 620.0).clamp(0.0, 107.0) * (108.0 / 107.0)
            } else if compact {
                260.0
            } else {
                LIVE_LATEST_HEIGHT
            },
            status_height: if compact { 56.0 } else { 88.0 },
            compact,
        }
    }
    fn column_height(self) -> f32 {
        (self.latest_height - 104.0).max(40.0)
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
    let mut read_requested = None;
    live_region(ui, "live-latest-panel", layout.latest_height, |ui| {
        card()
            .inner_margin(egui::Margin::symmetric(16, 12))
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 4.0;
                ui.set_min_width(ui.available_width());
                ui.set_min_height(layout.latest_height - 24.0);
                ui.horizontal(|ui| {
                    let width = (ui.available_width() - 176.0).max(40.0);
                    ui.add_sized(
                        [width, 32.0],
                        egui::Label::new(
                            RichText::new("進行中の文章／最新の対訳")
                                .strong()
                                .size(15.0),
                        )
                        .truncate(),
                    );
                    if ui
                        .add_enabled(!translation.is_empty(), standard("コピー"))
                        .clicked()
                        && let Ok(mut clipboard) = arboard::Clipboard::new()
                    {
                        let _ = clipboard.set_text(translation.to_owned());
                    }
                    if ui
                        .add_enabled(
                            selected.is_some_and(Row::is_confirmed),
                            standard("読み上げ"),
                        )
                        .clicked()
                    {
                        read_requested = Some(translation.to_owned());
                    }
                });
                let state = match mode {
                    LatestMode::Draft => "認識途中 · 原文をつないで翻訳します".into(),
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
                    LatestMode::Empty => "YouTube字幕またはPC音声の入力を待っています".into(),
                };
                ui.add(
                    egui::Label::new(RichText::new(state).size(12.0).color(SECONDARY)).truncate(),
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
                    LatestMode::Current => "原文は非表示です。".to_owned(),
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
                ui.columns(if show_source { 2 } else { 1 }, |columns| {
                    if show_source {
                        live_text_column(
                            &mut columns[0],
                            ("latest-source", id, mode == LatestMode::Draft),
                            "原文",
                            &source,
                            (16.0, SECONDARY),
                            auto_scroll
                                && (mode == LatestMode::Draft
                                    || selected
                                        .is_some_and(|row| row.pending || row.is_provisional())),
                            layout.column_height(),
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
                        (
                            font_size.clamp(18.0, 24.0),
                            if error.is_some() { ERROR } else { TEXT },
                        ),
                        auto_scroll
                            && selected.is_some_and(|row| row.pending || row.is_provisional()),
                        layout.column_height(),
                    );
                });
            });
    });
    read_requested
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
    ui.label(RichText::new(label).size(12.0).color(MINT));
    // Use the same explicit pitch for wrapping and viewport sizing. A sticky
    // tail then lands on complete rows instead of clipping half the first row.
    let row_height = ui
        .fonts_mut(|fonts| fonts.row_height(&egui::FontId::proportional(size)))
        .ceil();
    let rows = ((height - 4.0) / row_height).floor().max(1.0);
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
                        .size(size)
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
    card()
        .show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            ui.horizontal_wrapped(|ui| {
                ui.label(
                    RichText::new(clock(row.record.start_ms))
                        .size(12.0)
                        .color(SECONDARY),
                );
                let color = match row.record.kind.as_str() {
                    "YouTube" => Color32::from_rgb(217, 144, 106),
                    "音声" => Color32::from_rgb(127, 167, 224),
                    _ => Color32::from_rgb(179, 157, 219),
                };
                egui::Frame::new()
                    .stroke(egui::Stroke::new(1.0_f32, color))
                    .corner_radius(4)
                    .inner_margin(4.0)
                    .show(ui, |ui| {
                        ui.label(RichText::new(&row.record.kind).size(12.0).color(color));
                    });
                if ui
                    .add_enabled(!row.record.translation.is_empty(), standard("コピー"))
                    .clicked()
                    && let Ok(mut clipboard) = arboard::Clipboard::new()
                {
                    let _ = clipboard.set_text(row.record.translation.clone());
                }
                if ui
                    .add_enabled(row.is_confirmed(), standard("読み上げ"))
                    .clicked()
                {
                    *read_requested = Some(row.record.translation.clone());
                }
            });
            let unfinished = row.record.paragraph.as_ref().is_some_and(|p| {
                matches!(
                    p.state,
                    ParagraphState::Interrupted | ParagraphState::Failed
                )
            });
            if row.record.paragraph.is_some() {
                ui.label(
                    RichText::new(paragraph_label(row))
                        .size(12.0)
                        .color(if unfinished { ERROR } else { SECONDARY }),
                );
            }
            if let Some(error) = &row.error {
                ui.colored_label(ERROR, error);
            }
            if !row.record.translation.is_empty() {
                ui.add(
                    egui::Label::new(
                        RichText::new(&row.record.translation)
                            .size(font_size.clamp(18.0, 24.0))
                            .color(TEXT),
                    )
                    .wrap()
                    .selectable(true),
                );
            } else {
                secondary(ui, "訳文はありません。");
            }
            if show_original || unfinished {
                ui.add(
                    egui::Label::new(
                        RichText::new(&row.record.source)
                            .size(15.0)
                            .color(SECONDARY),
                    )
                    .wrap()
                    .selectable(true),
                );
            }
        })
        .response
        .rect
}

pub(super) fn configure_theme(ctx: &egui::Context) {
    install_fonts(ctx);
    let mut style = (*ctx.style()).clone();
    style.visuals = egui::Visuals::dark();
    style
        .text_styles
        .insert(egui::TextStyle::Heading, egui::FontId::proportional(20.0));
    style
        .text_styles
        .insert(egui::TextStyle::Body, egui::FontId::proportional(16.0));
    style
        .text_styles
        .insert(egui::TextStyle::Button, egui::FontId::proportional(14.0));
    style
        .text_styles
        .insert(egui::TextStyle::Small, egui::FontId::proportional(12.0));
    style
        .text_styles
        .insert(egui::TextStyle::Monospace, egui::FontId::monospace(14.0));
    style.spacing.item_spacing = Vec2::new(8.0, 8.0);
    style.spacing.button_padding = Vec2::new(12.0, 6.0);
    style.spacing.interact_size.y = 32.0;
    style.visuals.override_text_color = Some(TEXT);
    style.visuals.panel_fill = BG;
    style.visuals.window_fill = SURFACE;
    style.visuals.extreme_bg_color = INPUT;
    style.visuals.faint_bg_color = SURFACE;
    style.visuals.selection.bg_fill = Color32::from_rgb(31, 74, 67);
    style.visuals.selection.stroke = egui::Stroke::new(2.0_f32, MINT);
    for widget in [
        &mut style.visuals.widgets.inactive,
        &mut style.visuals.widgets.hovered,
        &mut style.visuals.widgets.active,
        &mut style.visuals.widgets.open,
    ] {
        widget.bg_fill = SURFACE;
        widget.weak_bg_fill = SURFACE;
        widget.bg_stroke = egui::Stroke::new(1.0_f32, BORDER);
        widget.fg_stroke = egui::Stroke::new(1.0_f32, TEXT);
        widget.corner_radius = egui::CornerRadius::same(6);
    }
    style.visuals.widgets.hovered.bg_stroke =
        egui::Stroke::new(1.0_f32, Color32::from_rgb(58, 68, 82));
    style.visuals.widgets.active.bg_stroke = egui::Stroke::new(2.0_f32, MINT);
    style.visuals.widgets.noninteractive.bg_stroke = egui::Stroke::new(1.0_f32, BORDER);
    style.visuals.widgets.noninteractive.fg_stroke = egui::Stroke::new(1.0_f32, SECONDARY);
    ctx.set_style(style);
}

fn card() -> egui::Frame {
    egui::Frame::new()
        .fill(SURFACE)
        .stroke(egui::Stroke::new(1.0_f32, BORDER))
        .corner_radius(8)
        .inner_margin(16.0)
}
fn secondary(ui: &mut egui::Ui, text: impl Into<String>) {
    ui.label(RichText::new(text).size(14.0).color(SECONDARY));
}
pub(super) fn primary(label: &str) -> egui::Button<'_> {
    egui::Button::new(RichText::new(label).color(Color32::from_rgb(11, 21, 18)))
        .fill(MINT)
        .min_size(Vec2::new(132.0, 36.0))
}
fn standard(label: &str) -> egui::Button<'_> {
    egui::Button::new(label).min_size(Vec2::new(0.0, 32.0))
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
            widget.corner_radius = egui::CornerRadius::same(3);
            widget.bg_fill = if checked { MINT } else { INPUT };
            widget.weak_bg_fill = widget.bg_fill;
            widget.bg_stroke = egui::Stroke::new(
                1.5_f32,
                if checked {
                    MINT
                } else {
                    Color32::from_rgb(127, 138, 152)
                },
            );
            widget.fg_stroke = egui::Stroke::new(
                2.0_f32,
                if checked {
                    Color32::from_rgb(11, 21, 18)
                } else {
                    TEXT
                },
            );
        }
        visuals.widgets.active.bg_stroke = egui::Stroke::new(2.0_f32, MINT);
        ui.checkbox(value, label)
    })
    .inner
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
    fn workspace_heading(&mut self, ui: &mut egui::Ui, title: &str) {
        ui.horizontal(|ui| {
            ui.heading(title);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .add_enabled(self.backend_ready, standard("ミニ表示"))
                    .on_hover_text("操作欄を畳み、翻訳だけを小窓に表示します。Ctrl + Shift + M")
                    .clicked()
                {
                    self.enter_mini_view(ui.ctx());
                }
                if ui
                    .add(standard("字幕ウィンドウ"))
                    .on_hover_text("翻訳を独立したウィンドウに表示します。ミニ表示と併用できます。")
                    .clicked()
                {
                    self.subtitle_window.open = true;
                }
            });
        });
    }
    pub(super) fn languages(&mut self, ui: &mut egui::Ui) {
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
                            .font(egui::FontId::proportional(16.0))
                            .margin(12.0)
                            .desired_width(f32::INFINITY),
                    );
                });
        });
    }
    fn output_card(&mut self, ui: &mut egui::Ui, height: f32) {
        card().show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new("訳文").size(14.0).strong().color(MINT));
                if ui
                    .add_enabled(!self.output.is_empty(), standard("コピー"))
                    .clicked()
                    && let Ok(mut clipboard) = arboard::Clipboard::new()
                {
                    let _ = clipboard.set_text(self.output.clone());
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
                            .font(egui::FontId::proportional(17.0))
                            .margin(12.0)
                            .desired_width(f32::INFINITY),
                    );
                });
        });
    }
    pub(super) fn text_ui(&mut self, ui: &mut egui::Ui) {
        self.workspace_heading(ui, "文章翻訳");
        secondary(ui, "文章を入力するか、画面や画像から読み取って翻訳します。");
        ui.add_space(8.0);
        self.languages(ui);
        self.backend_notice(ui);
        ui.add_space(4.0);
        let stacked = ui.available_width() < 650.0;
        let available = (ui.available_height() - 56.0).max(220.0);
        let height = if stacked {
            (available / 2.0 - 82.0).max(64.0)
        } else {
            (available - 82.0).max(120.0)
        };
        if stacked {
            self.input_card(ui, height);
            self.output_card(ui, height);
        } else {
            ui.columns(2, |columns| {
                self.input_card(&mut columns[0], height);
                self.output_card(&mut columns[1], height);
            });
        }
        ui.add_space(4.0);
        let ready =
            self.backend_ready && self.manual_busy.is_none() && !self.input.trim().is_empty();
        ui.horizontal_wrapped(|ui| {
            if ui.add_enabled(ready, primary("翻訳する")).clicked() {
                self.manual_translate();
            }
            ui.label(RichText::new("Ctrl + Enter").size(12.0).color(SECONDARY));
            if self.manual_busy.is_some() {
                ui.spinner();
            }
            if ui
                .add_enabled(self.backend_ready, standard("画像から翻訳"))
                .clicked()
            {
                self.open_image();
            }
        });
        if ready && ui.input(|i| i.modifiers.ctrl && i.key_pressed(egui::Key::Enter)) {
            self.manual_translate();
        }
    }
    pub(super) fn live_ui(&mut self, ui: &mut egui::Ui) {
        let layout = LiveLayout::for_height(ui.ctx().input(|input| input.content_rect().height()));
        self.workspace_heading(ui, "ライブ翻訳");
        ui.add_space(if layout.compact { 4.0 } else { 8.0 });
        let caption_receiving = self.caption_enabled
            && !self.caption_paused
            && self
                .last_caption
                .is_some_and(|time| time.elapsed() < Duration::from_secs(8));
        ui.columns(2, |columns| {
            card()
                .inner_margin(if layout.compact { 8.0 } else { 16.0 })
                .show(&mut columns[0], |ui| {
                    if layout.compact {
                        ui.spacing_mut().item_spacing.y = 4.0;
                    }
                    let mut enabled = self.caption_enabled;
                    if toggle(ui, &mut enabled, "YouTube字幕").changed() {
                        self.set_caption_enabled(enabled);
                    }
                    secondary(
                        ui,
                        if caption_receiving {
                            "字幕を受信中"
                        } else if self.caption_enabled {
                            "字幕の受信待ち"
                        } else {
                            "停止中"
                        },
                    );
                });
            card()
                .inner_margin(if layout.compact { 8.0 } else { 16.0 })
                .show(&mut columns[1], |ui| {
                    if layout.compact {
                        ui.spacing_mut().item_spacing.y = 4.0;
                    }
                    let mut enabled = self.audio_enabled || self.audio_start_requested;
                    let can_toggle =
                        self.backend_ready || self.audio_enabled || self.audio_start_requested;
                    if ui
                        .add_enabled_ui(can_toggle, |ui| toggle(ui, &mut enabled, "PC音声"))
                        .inner
                        .changed()
                    {
                        if !enabled && self.audio_start_requested && !self.audio_enabled {
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
                    secondary(
                        ui,
                        if self.audio_enabled && self.asr_ready {
                            if self.asr_busy {
                                "音声を認識中"
                            } else {
                                "聞き取り中"
                            }
                        } else if self.audio_start_requested {
                            "音声認識の準備中"
                        } else {
                            "停止中"
                        },
                    );
                });
        });
        ui.add_space(if layout.compact { 4.0 } else { 8.0 });
        ui.horizontal_wrapped(|ui| {
            let was_enabled = self.settings.auto_read;
            ui.add_enabled_ui(self.readout.ready.is_some(), |ui| {
                toggle(ui, &mut self.settings.auto_read, "訳文を読み上げ");
            });
            if was_enabled && !self.settings.auto_read {
                self.stop_readout();
            }
            ui.menu_button("表示", |ui| {
                toggle(ui, &mut self.settings.live_show_original, "原文を表示");
                toggle(ui, &mut self.settings.auto_scroll, "末尾へ自動スクロール").on_hover_text(
                    "上へスクロールしている間は位置を保ちます。OFFからONに戻すと末尾へ移動します。",
                );
                ui.add(
                    egui::Slider::new(&mut self.settings.font_size, 18.0..=24.0)
                        .integer()
                        .text("訳文の文字サイズ"),
                );
            });
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
            }
            if ui
                .add_enabled(!self.rows.is_empty(), standard("書き出し"))
                .clicked()
            {
                self.export_history();
            }
        });
        // Reserve status, notices and the stop control even while idle.
        live_region(ui, "live-status", layout.status_height, |ui| {
            if layout.compact {
                ui.spacing_mut().item_spacing.y = 4.0;
            }
            let speaking = self.readout_busy();
            ui.horizontal(|ui| {
                let status = if speaking {
                    self.readout.status.as_str()
                } else if self.live_busy.is_some() {
                    "翻訳中"
                } else if !self.asr_partial.is_empty() {
                    "音声を認識中"
                } else {
                    "最新の対訳を表示します"
                };
                let width = (ui.available_width() - 170.0).max(40.0);
                ui.add_sized(
                    [width, 32.0],
                    egui::Label::new(RichText::new(status).color(if speaking {
                        MINT
                    } else {
                        SECONDARY
                    }))
                    .truncate(),
                );
                if ui
                    .add_enabled(speaking, standard("読み上げを停止"))
                    .clicked()
                {
                    self.stop_readout();
                }
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
                            .color(if errors.is_empty() { SECONDARY } else { ERROR }),
                    )
                    .truncate(),
                )
                .on_hover_text(combined);
            } else {
                ui.add(
                    egui::Label::new(RichText::new(notice).size(12.0).color(SECONDARY)).truncate(),
                );
                ui.add(egui::Label::new(RichText::new(&errors).size(12.0).color(ERROR)).truncate())
                    .on_hover_text(errors);
            }
        });
        let current = self
            .document_visible_row()
            .filter(|row| row.is_provisional() || row.is_confirmed());
        let mode = super::live_view::latest_mode(
            current.is_none() && !self.asr_partial.trim().is_empty(),
            current.is_some(),
        );
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
        ui.add_space(4.0);
        ui.label(RichText::new("文章の履歴").size(14.0).color(SECONDARY));
        let history_ids: Vec<u64> = self
            .rows
            .iter()
            .filter(|row| history_row_visible(row, self.current_live_id))
            .map(|row| row.record.id)
            .collect();
        self.live_view.prepare_history(self.settings.auto_scroll);
        let area = egui::ScrollArea::vertical()
            .id_salt("live-confirmed-history")
            .auto_shrink([false, false])
            .stick_to_bottom(self.settings.auto_scroll);
        let output = area.show_viewport(ui, |ui, viewport| {
            let origin = ui.cursor().top();
            let mut geometry = Vec::with_capacity(history_ids.len());
            if history_ids.is_empty() {
                card().show(ui, |ui| {
                    ui.set_min_width(ui.available_width());
                    ui.label("まとまった文章がここに並びます。");
                    secondary(ui, "YouTube字幕またはPC音声をオンにしてください。");
                });
            }
            for row in self
                .rows
                .iter()
                .filter(|row| history_row_visible(row, self.current_live_id))
            {
                let rect = ui
                    .push_id(("history-row", row.record.id), |ui| {
                        live_history_card(
                            ui,
                            row,
                            self.settings.font_size,
                            self.settings.live_show_original,
                            &mut read_requested,
                        )
                    })
                    .inner;
                geometry.push(super::live_view::RowGeometry {
                    id: row.record.id,
                    top: rect.top() - origin,
                    bottom: rect.bottom() - origin,
                });
                ui.add_space(4.0);
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
        let hovering = pointer.is_some_and(|point| output.inner_rect.expand(18.0).contains(point));
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
    }
    pub(super) fn files_ui(&mut self, ui: &mut egui::Ui) {
        self.workspace_heading(ui, "ファイル・履歴");
        secondary(
            ui,
            "テキスト・Markdown・SRT・WebVTTを翻訳します。字幕の時刻は維持されます。",
        );
        ui.add_space(8.0);
        self.languages(ui);
        self.backend_notice(ui);
        ui.horizontal_wrapped(|ui| {
            if ui
                .add_enabled(
                    self.backend_ready && !self.file_busy,
                    primary("ファイルを選んで翻訳"),
                )
                .clicked()
            {
                self.translate_file();
            }
            if self.file_busy {
                ui.spinner();
                if ui.add(standard("中止")).clicked() {
                    self.file_cancel.store(true, Ordering::Relaxed);
                }
            }
            if ui
                .add_enabled(!self.rows.is_empty(), standard("タイムラインを書き出す"))
                .clicked()
            {
                self.export_history();
            }
        });
        ui.add_space(8.0);
        ui.add_sized(
            [ui.available_width(), 36.0],
            egui::TextEdit::singleline(&mut self.search)
                .hint_text("原文・訳文を検索")
                .margin(8.0),
        );
        egui::ScrollArea::vertical().show(ui, |ui| {
            let search = self.search.to_lowercase();
            for row in self.rows.iter().rev() {
                if !search.is_empty()
                    && !row.record.source.to_lowercase().contains(&search)
                    && !row.record.translation.to_lowercase().contains(&search)
                {
                    continue;
                }
                card().show(ui, |ui| {
                    ui.set_min_width(ui.available_width());
                    ui.label(
                        RichText::new(format!("{} · {}", row.record.created_at, row.record.kind))
                            .size(12.0)
                            .color(SECONDARY),
                    );
                    ui.add(
                        egui::Label::new(RichText::new(&row.record.translation).size(17.0))
                            .wrap()
                            .selectable(true),
                    );
                    ui.add(
                        egui::Label::new(
                            RichText::new(&row.record.source)
                                .size(15.0)
                                .color(SECONDARY),
                        )
                        .wrap()
                        .selectable(true),
                    );
                });
            }
            if self.rows.is_empty() {
                secondary(ui, "翻訳履歴はここに表示されます。");
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
        egui::ScrollArea::vertical().show(ui, |ui| {
        self.workspace_heading(ui, "設定");
            secondary(ui, "翻訳と音声認識はこのPC内で処理します。");
            ui.add_space(8.0);
            card().show(ui, |ui| {
                ui.set_min_width(ui.available_width());
                ui.label(RichText::new("翻訳モデル").strong());
                ui.label(self.setup_summary());
                secondary(ui, self.connection_label());
                if ui.add(standard("モデルを変更")).clicked() {
                    self.open_setup();
                }
            });
            card().show(ui, |ui| {
                ui.set_min_width(ui.available_width());
                self.screen_ocr_settings_ui(ui);
            });
            card().show(ui, |ui| {
                ui.set_min_width(ui.available_width());
                ui.label(RichText::new("音声認識").strong());
                egui::ComboBox::from_id_salt("asr-backend")
                    .selected_text(match self.settings.asr_backend.as_str() {
                        "nemotron" => "Nemotron Speech · 英語 / 連続認識",
                        "qwen" => "Qwen3-ASR · 多言語",
                        _ => "Whisper · 多言語 / CPU",
                    })
                    .show_ui(ui, |ui| {
                        if ui.selectable_value(&mut self.settings.asr_backend, "nemotron".into(), "Nemotron Speech · 英語 / 連続認識").changed() {
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
                secondary(
                    ui,
                    if self.asr_ready {
                        "音声認識の準備完了"
                    } else {
                        "初回は必要なモデルを準備します"
                    },
                );
                if ui.add(standard("音声認識を準備")).clicked() {
                    self.prepare_audio();
                }
            });
            card().show(ui, |ui| {
                ui.set_min_width(ui.available_width());
                self.paragraph_tools_settings_ui(ui);
            });
            card().show(ui, |ui| {
                ui.set_min_width(ui.available_width());
                self.readout_settings_ui(ui);
            });
            card().show(ui, |ui| {
                ui.set_min_width(ui.available_width());
                ui.label(RichText::new("YouTube字幕の接続").strong());
                secondary(
                    ui,
                    "ブラウザーの字幕連携に、下の接続キーを設定してください。",
                );
                ui.label(RichText::new("接続キー").size(14.0));
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
                });
                if ui.add(standard("字幕連携のフォルダーを開く")).clicked() {
                    let mut command = Command::new("explorer.exe");
                    command.arg(app_root().join("extension"));
                    command_hidden(&mut command);
                    let _ = command.spawn();
                }
                secondary(ui, "拡張機能の追加はブラウザーで行ってください。");
            });
            card().show(ui, |ui| {
                ui.set_min_width(ui.available_width());
                ui.label(RichText::new("用語集と文脈").strong());
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
            card().show(ui, |ui| {
                ui.set_min_width(ui.available_width());
                ui.label(RichText::new("ショートカットと保存先").strong());
                secondary(ui, "範囲翻訳: Ctrl + Alt + T　 音声翻訳: Ctrl + Alt + L");
                if ui.add(standard("保存フォルダーを開く")).clicked() {
                    self.open_data_folder();
                }
            });
            egui::CollapsingHeader::new("詳細（トラブル時）")
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
                        RichText::new(format!("保存先: {}", data_root().display())).size(12.0),
                    );
                    if ui.add(standard("翻訳エンジンを起動")).clicked() {
                        self.start_translation_server();
                    }
                });
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
            if ui.add(standard("アプリを終了")).clicked() {
                ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
            }
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
                    .fill(SIDEBAR)
                    .inner_margin(egui::Margin::symmetric(12, 6)),
            )
            .show(ctx, |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.label(
                        RichText::new(format!(
                            "{} · {}",
                            self.current_model_label(),
                            self.connection_label()
                        ))
                        .size(12.0)
                        .color(if self.backend_ready {
                            MINT
                        } else {
                            SECONDARY
                        }),
                    );
                    if !self.status.is_empty() {
                        ui.label(RichText::new(&self.status).size(12.0).color(SECONDARY));
                    }
                    if !self.readout.error.is_empty() {
                        ui.label(RichText::new(&self.readout.error).size(12.0).color(ERROR));
                    } else if !self.readout.status.is_empty() {
                        ui.label(
                            RichText::new(&self.readout.status)
                                .size(12.0)
                                .color(SECONDARY),
                        );
                    }
                });
            });
        egui::SidePanel::left("sidebar")
            .exact_width(200.0)
            .resizable(false)
            .frame(egui::Frame::new().fill(SIDEBAR).inner_margin(12.0))
            .show(ctx, |ui| {
                ui.add_space(12.0);
                ui.label(
                    RichText::new("地元翻訳くん")
                        .size(20.0)
                        .strong()
                        .color(TEXT),
                );
                ui.label(
                    RichText::new("translated by PLaMo")
                        .size(12.0)
                        .color(SECONDARY),
                );
                ui.add_space(24.0);
                for (tab, label) in [
                    (Tab::Text, "文章翻訳"),
                    (Tab::Live, "ライブ翻訳"),
                    (Tab::Files, "ファイル・履歴"),
                    (Tab::Settings, "設定"),
                ] {
                    let selected = self.tab == tab;
                    let response = ui.add_sized(
                        [ui.available_width(), 36.0],
                        egui::Button::new(RichText::new(label).size(14.0).color(TEXT))
                            .fill(if selected {
                                Color32::from_rgb(31, 38, 48)
                            } else {
                                SIDEBAR
                            })
                            .stroke(egui::Stroke::NONE),
                    );
                    if selected {
                        ui.painter().rect_filled(
                            egui::Rect::from_min_size(
                                response.rect.min,
                                Vec2::new(3.0, response.rect.height()),
                            ),
                            0.0,
                            MINT,
                        );
                    }
                    if response.clicked() {
                        self.tab = tab;
                    }
                }
                ui.add_space(16.0);
                ui.separator();
                ui.add_space(8.0);
                if ui
                    .add_sized([ui.available_width(), 36.0], standard("モデルの準備"))
                    .clicked()
                {
                    self.open_setup();
                }
                ui.add_space(16.0);
                if ui
                    .add_enabled(
                        self.backend_ready && !self.capture_busy,
                        egui::Button::new("範囲を翻訳")
                            .min_size(Vec2::new(ui.available_width(), 40.0)),
                    )
                    .clicked()
                {
                    self.capture(ctx);
                }
                ui.label(RichText::new("Ctrl + Alt + T").size(12.0).color(SECONDARY));
                if !self.backend_ready {
                    ui.label(
                        RichText::new("モデルの準備が必要です")
                            .size(12.0)
                            .color(SECONDARY),
                    );
                }
                if self.screens.len() > 1 {
                    egui::ComboBox::from_id_salt("screen")
                        .width(ui.available_width())
                        .selected_text(format!("画面 {}", self.screen_index + 1))
                        .show_ui(ui, |ui| {
                            for (index, screen) in self.screens.iter().enumerate() {
                                ui.selectable_value(
                                    &mut self.screen_index,
                                    index,
                                    format!("{}: {}", index + 1, screen.name),
                                );
                            }
                        });
                }
                ui.add_space(8.0);
                let audio_label = if self.audio_enabled {
                    "音声翻訳を停止"
                } else if self.audio_start_requested {
                    "開始を取り消す"
                } else {
                    "音声翻訳を開始"
                };
                let can_toggle =
                    self.backend_ready || self.audio_enabled || self.audio_start_requested;
                if ui
                    .add_enabled_ui(can_toggle, |ui| {
                        ui.add_sized([ui.available_width(), 40.0], standard(audio_label))
                    })
                    .inner
                    .clicked()
                {
                    if self.audio_start_requested && !self.audio_enabled {
                        self.audio_start_requested = false;
                    } else if self.audio_enabled
                        || (self.audio_environment_ready() && self.asr_ready && self.backend_ready)
                    {
                        self.toggle_audio();
                    } else {
                        self.audio_start_requested = true;
                        self.prepare_audio();
                    }
                }
                ui.label(RichText::new("Ctrl + Alt + L").size(12.0).color(SECONDARY));
                ui.add_space(16.0);
                ui.label(
                    RichText::new("翻訳はこのPC内で処理します")
                        .size(12.0)
                        .color(SECONDARY),
                );
            });
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(BG).inner_margin(24.0))
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
        assert_eq!(LiveLayout::for_height(800.0).latest_height, 260.0);
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
                let japanese:String = "文章全体の文脈を保ちながら、続きを受け取り一つの段落として日本語を更新します。".chars().cycle().take(600).collect();
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
}
