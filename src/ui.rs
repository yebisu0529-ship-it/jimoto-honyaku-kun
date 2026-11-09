use super::theme::{
    self, ACCENT, ACCENT_SOFT, BORDER_STRONG, CANVAS, ERROR, INPUT, ON_ACCENT, SURFACE,
    SURFACE_RAISED, TEXT, TEXT_MUTED, TEXT_SECONDARY, WARNING,
};
use super::*;
use plamo_translate::credit;
use plamo_translate::journal::ParagraphState;

/// Height reserved for the page title and its actions.
const HEADER_HEIGHT: f32 = 56.0;
/// Height of the bar under the chat: the large button and the room around it.
const LISTEN_BAR_HEIGHT: f32 = theme::CONTROL_H_LARGE + 2.0 * theme::SPACE_2;
/// The key that does what the button under the chat does.
const LISTEN_SHORTCUT: &str = "Ctrl + Alt + L";
/// The pages the Ctrl+number shortcuts open. The chat is the home screen; the
/// others are reached from the その他 menu.
const PAGE_KEYS: [(Tab, &str, egui::Key); 4] = [
    (Tab::Text, "文章", egui::Key::Num1),
    (Tab::Live, "チャット", egui::Key::Num2),
    (Tab::Files, "履歴", egui::Key::Num3),
    (Tab::Settings, "設定", egui::Key::Num4),
];

/// The shortcut that opens `tab`, as the menu and the hover texts write it.
fn page_shortcut(tab: Tab) -> String {
    PAGE_KEYS
        .iter()
        .find(|(page, _, _)| *page == tab)
        .map(|(_, _, key)| format!("Ctrl + {}", key.symbol_or_name()))
        .unwrap_or_default()
}

/// Below this content width the source and translation cards stack.
const SIDE_BY_SIDE_MIN_WIDTH: f32 = 640.0;
/// A text area never shrinks below three large controls, however short the window.
const MIN_TEXT_AREA: f32 = theme::CONTROL_H_LARGE * 3.0;

/// A card's text area takes the height left under its header.
fn text_area_height(ui: &egui::Ui) -> f32 {
    ui.available_height().max(MIN_TEXT_AREA)
}

/// Measurements captured while drawing, so tests can assert real geometry
/// instead of re-deriving layout from constants.
#[cfg(test)]
#[derive(Default)]
pub(super) struct UiProbe {
    pub menu_button: Option<egui::Rect>,
    pub back_button: Option<egui::Rect>,
    pub listen_bar: Option<egui::Rect>,
    pub listen_button: Option<egui::Rect>,
    pub reading_font: f32,
    pub status_rect: Option<egui::Rect>,
}

/// How far one local model is from being usable.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Readiness {
    Ready,
    Preparing,
    Missing,
}

impl Readiness {
    /// The word printed beside the model name. A ready model needs none.
    fn word(self) -> Option<&'static str> {
        match self {
            Self::Ready => None,
            Self::Preparing => Some("準備中"),
            Self::Missing => Some("未準備"),
        }
    }

    fn color(self) -> Color32 {
        match self {
            Self::Ready => ACCENT,
            Self::Preparing => WARNING,
            Self::Missing => TEXT_MUTED,
        }
    }

    /// A missing model links to the screen that prepares it.
    fn needs_action(self) -> bool {
        self == Self::Missing
    }

    /// What the chat says about all the models together.
    fn headline(self) -> &'static str {
        match self {
            Self::Ready => "準備完了",
            Self::Preparing => "準備中…",
            Self::Missing => "準備が必要です",
        }
    }

    fn headline_color(self) -> Color32 {
        match self {
            Self::Ready => TEXT_SECONDARY,
            Self::Preparing => WARNING,
            Self::Missing => TEXT,
        }
    }

    /// What the bar under the chat says while the translation model cannot
    /// translate yet.
    fn needed(self) -> Option<&'static str> {
        match self {
            Self::Ready => None,
            Self::Preparing => Some("翻訳モデルを準備しています"),
            Self::Missing => Some("翻訳モデルの準備が必要です"),
        }
    }
}

struct ModelStatus {
    name: &'static str,
    readiness: Readiness,
    detail: String,
    opens: Tab,
    /// The chat cannot work without it. The others are prepared when they are
    /// first used.
    required: bool,
}

/// The one state the chat shows for all the models. A missing model that is
/// required decides it; otherwise any model still being prepared does.
fn overall(statuses: &[ModelStatus]) -> Readiness {
    if statuses
        .iter()
        .any(|status| status.required && status.readiness == Readiness::Missing)
    {
        Readiness::Missing
    } else if statuses
        .iter()
        .any(|status| status.readiness == Readiness::Preparing)
    {
        Readiness::Preparing
    } else {
        Readiness::Ready
    }
}

impl ModelStatus {
    fn hover(&self) -> String {
        let state = match self.readiness {
            Readiness::Ready => "準備完了",
            Readiness::Preparing => "準備中",
            Readiness::Missing => "未準備",
        };
        let action = if self.readiness.needs_action() {
            "\nクリックすると準備する画面を開きます"
        } else {
            ""
        };
        format!("{}: {}\n{state}{action}", self.name, self.detail)
    }
}

const DOT: f32 = 8.0;

/// A filled dot for a model that is ready, a ring for one that is not.
fn paint_dot(painter: &egui::Painter, center: egui::Pos2, readiness: Readiness) {
    match readiness {
        Readiness::Ready => {
            painter.circle_filled(center, DOT / 2.0, readiness.color());
        }
        Readiness::Preparing | Readiness::Missing => {
            painter.circle_stroke(
                center,
                DOT / 2.0 - 0.75,
                egui::Stroke::new(1.5_f32, readiness.color()),
            );
        }
    }
}

/// A button for a row that is exactly one control tall. Its face is shorter
/// than its hit area, so it keeps clear of the row's edges.
fn row_button(ui: &mut egui::Ui, label: &str) -> egui::Response {
    let font = egui::FontId::proportional(theme::SIZE_LABEL);
    let galley = ui.painter().layout_no_wrap(label.to_owned(), font, TEXT);
    let size = Vec2::new(galley.size().x + 2.0 * theme::SPACE_3, theme::CONTROL_H);
    let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click());
    let face = rect.shrink2(Vec2::new(0.0, theme::SPACE_1));
    let fill = if response.hovered() {
        SURFACE_RAISED
    } else {
        INPUT
    };
    ui.painter().rect(
        face,
        theme::RADIUS_CONTROL,
        fill,
        egui::Stroke::new(1.0_f32, BORDER_STRONG),
        egui::StrokeKind::Inside,
    );
    ui.painter()
        .galley(face.center() - galley.size() / 2.0, galley, TEXT);
    if response.has_focus() {
        ui.painter().rect_stroke(
            face,
            theme::RADIUS_CONTROL,
            egui::Stroke::new(2.0_f32, ACCENT),
            egui::StrokeKind::Inside,
        );
    }
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, label));
    response
}

/// The one state the chat shows: a dot and a few words and, while a required
/// model is missing, the button to the screen that prepares it. True when that
/// button is clicked.
fn headline(ui: &mut egui::Ui, readiness: Readiness) -> bool {
    let (dot, _) = ui.allocate_exact_size(Vec2::splat(DOT), egui::Sense::hover());
    paint_dot(ui.painter(), dot.center(), readiness);
    ui.label(
        RichText::new(readiness.headline())
            .size(theme::SIZE_CAPTION)
            .color(readiness.headline_color()),
    );
    readiness.needs_action() && row_button(ui, "モデルの準備").clicked()
}

/// A dot, the model name and, unless ready, its state in words.
fn status_chip(ui: &mut egui::Ui, status: &ModelStatus) -> egui::Response {
    let readiness = status.readiness;
    let font = egui::FontId::proportional(theme::SIZE_CAPTION);
    let mut job = egui::text::LayoutJob::default();
    job.append(
        status.name,
        0.0,
        egui::TextFormat::simple(font.clone(), TEXT_SECONDARY),
    );
    if let Some(word) = readiness.word() {
        job.append(
            word,
            theme::SPACE_1,
            egui::TextFormat::simple(font, readiness.color()),
        );
    }
    let galley = ui.painter().layout_job(job);
    let width = theme::SPACE_1 + DOT + theme::SPACE_1 + galley.size().x + theme::SPACE_1;
    let sense = if readiness.needs_action() {
        egui::Sense::click()
    } else {
        egui::Sense::hover()
    };
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, theme::CONTROL_H), sense);
    let face = rect.shrink2(Vec2::new(0.0, theme::SPACE_1));
    if readiness.needs_action() && response.hovered() {
        ui.painter()
            .rect_filled(face, theme::RADIUS_CONTROL, SURFACE_RAISED);
    }
    let center = egui::pos2(rect.left() + theme::SPACE_1 + DOT / 2.0, rect.center().y);
    paint_dot(ui.painter(), center, readiness);
    let text = egui::pos2(
        center.x + DOT / 2.0 + theme::SPACE_1,
        rect.center().y - galley.size().y / 2.0,
    );
    let label = galley.text().to_owned();
    ui.painter().galley(text, galley, TEXT_SECONDARY);
    // The row is exactly one control tall, so the ring is drawn inside it.
    if response.has_focus() {
        ui.painter().rect_stroke(
            face,
            theme::RADIUS_CONTROL,
            egui::Stroke::new(2.0_f32, ACCENT),
            egui::StrokeKind::Inside,
        );
    }
    let kind = if readiness.needs_action() {
        egui::WidgetType::Button
    } else {
        egui::WidgetType::Label
    };
    response.widget_info(|| egui::WidgetInfo::labeled(kind, true, &label));
    response
}

/// The latest app message on one truncated line; the full text is on hover.
fn status_message(ui: &mut egui::Ui, parts: &[(&str, Color32)]) {
    let font = egui::FontId::proportional(theme::SIZE_CAPTION);
    let mut job = egui::text::LayoutJob::default();
    let mut shown: Vec<&str> = Vec::new();
    for (text, color) in parts.iter().filter(|(text, _)| !text.is_empty()) {
        // Some failures are copied into more than one message field.
        if shown.contains(text) {
            continue;
        }
        shown.push(text);
        if !job.is_empty() {
            job.append(
                "  ·  ",
                0.0,
                egui::TextFormat::simple(font.clone(), TEXT_MUTED),
            );
        }
        job.append(text, 0.0, egui::TextFormat::simple(font.clone(), *color));
    }
    if job.is_empty() {
        return;
    }
    let full = job.text.clone();
    ui.add(egui::Label::new(job).truncate()).on_hover_text(full);
}

/// A saved timestamp as people read it; anything unparsable is shown as stored.
fn history_time(created_at: &str) -> String {
    chrono::DateTime::parse_from_rfc3339(created_at)
        .map(|time| time.format("%Y/%m/%d %H:%M").to_string())
        .unwrap_or_else(|_| created_at.to_owned())
}

/// Height of the one status line between the live toolbar and the panel.
const STATUS_LINE_HEIGHT: f32 = 24.0;

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

pub(super) fn configure_theme(ctx: &egui::Context) {
    install_fonts(ctx);
    theme::apply(ctx);
}

fn card() -> egui::Frame {
    theme::card()
}

/// Every settings group is a full-width card whose controls keep their own
/// width; `ui.columns` would otherwise stretch each button across the column.
fn settings_card(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui)) {
    card().show(ui, |ui| {
        ui.set_min_width(ui.available_width());
        ui.with_layout(egui::Layout::top_down(egui::Align::Min), add);
    });
}
fn secondary(ui: &mut egui::Ui, text: impl Into<String>) {
    ui.label(theme::label(text));
}
pub(super) fn primary(label: &str) -> egui::Button<'_> {
    egui::Button::new(
        RichText::new(label)
            .size(theme::SIZE_LABEL)
            .strong()
            .color(ON_ACCENT),
    )
    .fill(ACCENT)
    .min_size(Vec2::new(140.0, theme::CONTROL_H_LARGE))
}
fn standard(label: &str) -> egui::Button<'_> {
    egui::Button::new(RichText::new(label).size(theme::SIZE_LABEL).color(TEXT))
        .min_size(Vec2::new(0.0, theme::CONTROL_H))
}
/// The one large button under the chat: the accent colour to start listening,
/// and red to stop.
fn listen_button(listening: bool) -> egui::Button<'static> {
    if listening {
        primary("■ 止める").fill(ERROR)
    } else {
        primary("● 聞く")
    }
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
        ui.checkbox(
            value,
            RichText::new(label).size(theme::SIZE_LABEL).color(TEXT),
        )
    })
    .inner
}

fn badge(ui: &mut egui::Ui, text: &str, color: Color32, fill: Color32) {
    egui::Frame::new()
        .fill(fill)
        .stroke(egui::Stroke::new(1.0_f32, color))
        .corner_radius(4)
        .inner_margin(egui::Margin::symmetric(6, 2))
        .show(ui, |ui| {
            ui.label(RichText::new(text).size(theme::SIZE_CAPTION).color(color));
        });
}

/// What 「聞く」 listens to. Captions and PC audio can run together: 自動 is both,
/// and the PC audio is used while no captions arrive.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum LiveSourceMode {
    #[default]
    Auto,
    Captions,
    Audio,
    Screen,
}

impl LiveSourceMode {
    const ALL: [Self; 4] = [Self::Auto, Self::Captions, Self::Audio, Self::Screen];

    fn from_flags(captions: bool, audio: bool) -> Option<Self> {
        match (captions, audio) {
            (true, false) => Some(Self::Captions),
            (false, true) => Some(Self::Audio),
            (true, true) => Some(Self::Auto),
            (false, false) => None,
        }
    }

    /// The captions and the PC audio this input turns on. The screen feed is a
    /// caption source of its own.
    fn flags(self) -> (bool, bool) {
        match self {
            Self::Auto => (true, true),
            Self::Captions | Self::Screen => (true, false),
            Self::Audio => (false, true),
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Auto => "自動（字幕を優先）",
            Self::Captions => "YouTube字幕だけ",
            Self::Audio => "PC音声だけ",
            Self::Screen => "画面の文字",
        }
    }

    fn hint(self) -> &'static str {
        match self {
            Self::Auto => "字幕を優先し、字幕が止まっている間はPC音声を使います",
            Self::Captions => "YouTubeの字幕だけを翻訳します",
            Self::Audio => "PCで再生中の音声だけを翻訳します",
            Self::Screen => "選んだ画面の範囲を読み続け、文字が変わったときに翻訳します",
        }
    }
}

/// What one choice in the その他 menu asks for. The menu is drawn inside the
/// page header, so the choice is carried out once the header is done.
#[derive(Clone, Copy)]
enum MenuChoice {
    SubtitleWindow,
    Mini,
    Text,
    Range,
    ScreenText,
    /// A choice in 入力の切り替え.
    Input(LiveSourceMode),
    File,
    History,
    Models,
    YouTube,
    Settings,
    Details,
    Clear,
    Export,
}

/// Names the その他 menu, so that a preview can open it.
pub(super) const MORE_MENU: &str = "more-menu";

/// One row of a menu, with its shortcut at the right edge. True when clicked.
fn menu_row(ui: &mut egui::Ui, label: &str, shortcut: &str, hover: &str, enabled: bool) -> bool {
    let mut button = standard(label);
    if !shortcut.is_empty() {
        button = button.shortcut_text(theme::caption(shortcut));
    }
    let response = ui.add_enabled(enabled, button);
    if hover.is_empty() {
        response.clicked()
    } else {
        response.on_hover_text(hover).clicked()
    }
}

/// A button that is filled while it is the chosen one, and tells a screen
/// reader so.
fn choice_button(ui: &mut egui::Ui, label: &str, chosen: bool) -> egui::Response {
    let response = ui.add(standard(label).selected(chosen));
    response
        .widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Button, true, chosen, label));
    response
}

/// The 自動読み上げ switch: in the header of the other screens, and in the bar
/// under the chat.
fn auto_read_switch(ui: &mut egui::Ui, on: bool) -> egui::Response {
    choice_button(ui, "自動読み上げ", on).on_hover_text(readout::AUTO_READ_HINT)
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

/// The live rows as chat messages, oldest first. `visible` is the coherent
/// pair of the current paragraph, which stands in for its row while a split
/// leaves the row's own translation out of date. `drafting` is speech still
/// being recognised that no paragraph holds yet.
fn live_messages<'a>(
    rows: &'a [Row],
    visible: Option<&'a Row>,
    drafting: &'a str,
) -> Vec<chat::Message<'a>> {
    let mut messages: Vec<_> = rows
        .iter()
        .map(|row| {
            visible
                .filter(|visible| visible.record.id == row.record.id)
                .unwrap_or(row)
        })
        .filter_map(|row| {
            let record = &row.record;
            let source = chat::Source::from_kind(&record.kind)?;
            let state = record.paragraph.as_ref().map(|paragraph| paragraph.state);
            let status = if row.error.is_some() || state == Some(ParagraphState::Failed) {
                chat::Status::Failed
            } else if row.pending || row.is_provisional() {
                chat::Status::Forming
            } else if state == Some(ParagraphState::Interrupted) {
                chat::Status::Interrupted
            } else {
                chat::Status::Fixed
            };
            Some(chat::Message {
                id: record.id,
                source,
                target: &record.translation,
                original: &record.source,
                moment: chat::Moment::from_record(&record.created_at, record.start_ms, source),
                status,
                failure: row.error.as_deref(),
            })
        })
        .collect();
    let talking = messages.last().is_some_and(|last| {
        last.source == chat::Source::PcAudio && last.status == chat::Status::Forming
    });
    if !drafting.is_empty() && !talking {
        messages.push(chat::Message::typing(chat::Source::PcAudio, drafting));
    }
    messages
}

mod youtube;

impl App {
    /// The top row of every page. The chat is the home screen and shows the
    /// app's name; every other page starts with the way back to the chat. The
    /// one menu is at the right of all of them. Actions sit on the same fixed
    /// row so changing content can never shift the reading area.
    fn page_header(
        &mut self,
        ui: &mut egui::Ui,
        title: &str,
        subtitle: &str,
        actions: impl FnOnce(&mut egui::Ui),
    ) {
        live_region(ui, "page-header", HEADER_HEIGHT, |ui| {
            let home = self.tab == Tab::Live;
            let mut back = false;
            let mut switch_readout = false;
            let mut choice = None;
            let auto_read = self.settings.auto_read;
            // The chat has the switch in the bar under it.
            let readout_here = self.settings.readout_enabled && !home;
            ui.horizontal(|ui| {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    choice = self.more_menu(ui);
                    if readout_here {
                        switch_readout = auto_read_switch(ui, auto_read).clicked();
                    }
                    actions(ui);
                    // The title takes whatever the actions leave, so a narrow
                    // window truncates it instead of overlapping the buttons.
                    ui.with_layout(egui::Layout::left_to_right(egui::Align::Min), |ui| {
                        if !home {
                            let button = ui
                                .add(standard("← チャットに戻る"))
                                .on_hover_text(page_shortcut(Tab::Live));
                            #[cfg(test)]
                            {
                                self.ui_probe.back_button = Some(button.rect);
                            }
                            back = button.clicked();
                        }
                        ui.vertical(|ui| {
                            ui.spacing_mut().item_spacing.y = 2.0;
                            ui.add(egui::Label::new(theme::title(title)).truncate());
                            ui.add(
                                egui::Label::new(
                                    RichText::new(subtitle)
                                        .size(theme::SIZE_CAPTION)
                                        .color(TEXT_MUTED),
                                )
                                .truncate(),
                            );
                        });
                    });
                });
            });
            if back {
                self.tab = Tab::Live;
            }
            if switch_readout {
                self.toggle_auto_read(ui.ctx());
            }
            if let Some(choice) = choice {
                self.follow(ui.ctx(), choice);
            }
        });
    }

    /// The one menu behind the chat. It leads to every other screen and way of
    /// translating; on the chat itself it also holds what to do with the talk.
    fn more_menu(&mut self, ui: &mut egui::Ui) -> Option<MenuChoice> {
        let id = egui::Id::new(MORE_MENU);
        let chat = self.tab == Tab::Live;
        let ready = self.backend_ready;
        let can_range = ready && !self.capture_busy;
        let can_screen = can_range && self.screen_feed.is_none();
        let can_file = ready && !self.file_busy;
        let has_rows = !self.rows.is_empty();
        let shown = self.shown_input();
        let open = egui::Popup::is_id_open(ui.ctx(), id);
        let button = ui.add(standard("⋯ その他").selected(open));
        #[cfg(test)]
        {
            self.ui_probe.menu_button = Some(button.rect);
        }
        let mut choice = None;
        let mut input = None;
        egui::Popup::menu(&button).id(id).show(|ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            ui.spacing_mut().button_padding.x = theme::SPACE_2;
            let mut row = |ui: &mut egui::Ui,
                           wanted: MenuChoice,
                           label: &str,
                           shortcut: &str,
                           hover: &str,
                           enabled: bool| {
                if menu_row(ui, label, shortcut, hover, enabled) {
                    choice = Some(wanted);
                }
            };
            row(
                ui,
                MenuChoice::SubtitleWindow,
                "字幕ウィンドウ",
                "",
                "訳文を独立したウィンドウに表示します。ミニ表示と併用できます。",
                true,
            );
            row(
                ui,
                MenuChoice::Mini,
                "ミニ表示",
                "Ctrl + Shift + M",
                "入力欄を畳み、訳文だけを小窓に表示します。",
                ready,
            );
            row(
                ui,
                MenuChoice::Text,
                "文章を翻訳",
                &page_shortcut(Tab::Text),
                "",
                true,
            );
            row(
                ui,
                MenuChoice::Range,
                "範囲翻訳",
                "Ctrl + Alt + T",
                "画面の一部を選んで翻訳します。",
                can_range,
            );
            if self.screens.len() > 1 {
                ui.menu_button(RichText::new("画面選択").size(theme::SIZE_LABEL), |ui| {
                    for (index, screen) in self.screens.iter().enumerate() {
                        ui.selectable_value(
                            &mut self.screen_index,
                            index,
                            format!("{}: {}", index + 1, screen.name),
                        );
                    }
                });
            }
            row(
                ui,
                MenuChoice::ScreenText,
                "画面の文字を翻訳",
                "",
                LiveSourceMode::Screen.hint(),
                can_screen,
            );
            row(
                ui,
                MenuChoice::File,
                "ファイルを翻訳",
                "",
                "テキスト・Markdown・SRT・WebVTTを翻訳します。字幕の時刻は維持されます。",
                can_file,
            );
            row(
                ui,
                MenuChoice::History,
                "履歴",
                &page_shortcut(Tab::Files),
                "",
                true,
            );
            row(
                ui,
                MenuChoice::Models,
                "モデル",
                "",
                "翻訳モデルの準備を開きます",
                true,
            );
            row(
                ui,
                MenuChoice::YouTube,
                "YouTube字幕の接続",
                "",
                "ChromeやEdgeの拡張機能とつなぎ、YouTubeの字幕を受け取ります。",
                true,
            );
            row(
                ui,
                MenuChoice::Settings,
                "設定",
                &page_shortcut(Tab::Settings),
                "",
                true,
            );
            row(
                ui,
                MenuChoice::Details,
                "状態の詳細",
                "",
                "翻訳・文字認識・音声認識の準備の状態を見ます",
                true,
            );
            if chat {
                ui.separator();
                // Its choices open down the menu, so it comes before the rest
                // to keep them clear of the row at the bottom of the window.
                input = ui
                    .menu_button(RichText::new("入力の切り替え").size(theme::SIZE_LABEL), |ui| {
                        let mut picked = None;
                        for mode in LiveSourceMode::ALL {
                            if choice_button(ui, mode.label(), mode == shown)
                                .on_hover_text(mode.hint())
                                .clicked()
                            {
                                picked = Some(mode);
                            }
                        }
                        picked
                    })
                    .inner
                    .flatten();
                row(ui, MenuChoice::Export, "書き出し", "", "", has_rows);
                ui.menu_button(RichText::new("表示").size(theme::SIZE_LABEL), |ui| {
                    toggle(ui, &mut self.settings.live_show_original, "原文をすべて表示");
                    toggle(ui, &mut self.settings.auto_scroll, "末尾へ自動スクロール")
                        .on_hover_text(
                            "上へスクロールしている間は位置を保ちます。OFFからONに戻すと末尾へ移動します。",
                        );
                    ui.add_space(theme::SPACE_1);
                    ui.label(theme::caption("訳文の文字サイズ"));
                    theme::slider(
                        ui,
                        egui::Slider::new(
                            &mut self.settings.font_size,
                            theme::READING_MIN..=theme::READING_MAX,
                        )
                        .integer()
                        .suffix(" pt"),
                    );
                });
            }
        });
        choice.or(input.map(MenuChoice::Input))
    }

    /// Carries out a choice from the その他 menu.
    fn follow(&mut self, ctx: &egui::Context, choice: MenuChoice) {
        self.details_open = false;
        match choice {
            MenuChoice::SubtitleWindow => self.subtitle_window.open = true,
            MenuChoice::Mini => self.enter_mini_view(ctx),
            MenuChoice::Text => self.tab = Tab::Text,
            MenuChoice::Range => {
                // The reading lands on the 文章 screen; the chat does not show it.
                self.tab = Tab::Text;
                self.capture(ctx);
            }
            MenuChoice::ScreenText => {
                self.tab = Tab::Live;
                self.listen_input = LiveSourceMode::Screen;
                self.apply_input(ctx, LiveSourceMode::Screen);
            }
            MenuChoice::Input(mode) => self.choose_input(ctx, mode),
            MenuChoice::File => {
                // The progress and the cancel button are on the 履歴 screen.
                self.tab = Tab::Files;
                self.translate_file();
            }
            MenuChoice::History => self.tab = Tab::Files,
            MenuChoice::Models => self.open_setup(),
            MenuChoice::YouTube => self.tab = Tab::YouTube,
            MenuChoice::Settings => self.tab = Tab::Settings,
            MenuChoice::Details => self.details_open = true,
            MenuChoice::Clear => {
                let dir = self
                    .journal
                    .as_ref()
                    .and_then(|j| j.path.parent())
                    .map(Path::to_path_buf)
                    .unwrap_or_else(|| data_root().join("sessions"));
                match Journal::new(&dir) {
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
            MenuChoice::Export => self.export_history(),
        }
    }

    /// The subtitle window knows its parts by row id. The rows know when each
    /// one was said.
    pub(super) fn sync_subtitle_moments(&mut self) {
        if !self.subtitle_window.open {
            return;
        }
        let moments = self
            .subtitle_window
            .paragraph_keys()
            .filter_map(|key| {
                let record = &self
                    .rows
                    .iter()
                    .rev()
                    .find(|row| row.record.id == key)?
                    .record;
                let source = chat::Source::from_kind(&record.kind)?;
                Some((
                    key,
                    chat::Moment::from_record(&record.created_at, record.start_ms, source),
                ))
            })
            .collect();
        self.subtitle_window.set_moments(moments);
    }

    /// The input that is on, from what the captions, the PC audio and the
    /// screen feed add up to. None while nothing listens.
    fn current_input(&self) -> Option<LiveSourceMode> {
        if self.screen_feed.is_some() {
            Some(LiveSourceMode::Screen)
        } else {
            LiveSourceMode::from_flags(
                self.caption_enabled,
                self.audio_enabled || self.audio_start_requested,
            )
        }
    }

    /// What the choice in 入力の切り替え shows: the input that is on, or else
    /// the one the next 「聞く」 turns on.
    fn shown_input(&self) -> LiveSourceMode {
        self.current_input().unwrap_or(self.listen_input)
    }

    /// 「聞く」: turns on the chosen input.
    fn start_listening(&mut self, ctx: &egui::Context) {
        if !self.backend_ready {
            self.status = "モデルの準備ができると、聞けます。".into();
            return;
        }
        self.apply_input(ctx, self.listen_input);
    }

    /// 「止める」: turns off every live input. The saved translations stay.
    fn stop_listening(&mut self) {
        self.set_caption_enabled(false);
        self.audio_start_requested = false;
        if self.audio_enabled {
            self.stop_audio();
        }
    }

    /// A choice in 入力の切り替え. An input that is on is switched at once;
    /// otherwise the choice waits for the next 「聞く」.
    fn choose_input(&mut self, ctx: &egui::Context, mode: LiveSourceMode) {
        self.listen_input = mode;
        if self.current_input().is_some_and(|on| on != mode) {
            self.apply_input(ctx, mode);
        }
    }

    /// 聞く and 止める on one key.
    pub(super) fn toggle_listening(&mut self, ctx: &egui::Context) {
        if self.current_input().is_some() {
            self.stop_listening();
        } else {
            self.start_listening(ctx);
        }
    }

    /// Turns on `mode` in place of the input that is on now.
    fn apply_input(&mut self, ctx: &egui::Context, mode: LiveSourceMode) {
        if mode == LiveSourceMode::Screen {
            self.begin_screen_feed(ctx);
            return;
        }
        if self.screen_feed.is_some() {
            self.set_caption_enabled(false);
        }
        self.apply_live_sources(mode);
    }

    /// Turns the captions and the PC audio on or off as `mode` asks. Audio
    /// start is deferred through the existing preparation path when the engine
    /// is not ready.
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

    /// Captions are coming in from the browser extension.
    fn captions_arriving(&self) -> bool {
        self.caption_enabled
            && !self.caption_paused
            && self
                .last_caption
                .is_some_and(|time| time.elapsed() < youtube::ARRIVING_WITHIN)
    }

    /// The line beside the 聞く button: what is wrong, or what is going on.
    fn listen_state(&self) -> (&str, Color32) {
        if !self.service_error.is_empty() {
            return (&self.service_error, ERROR);
        }
        let model = self.translation_readiness();
        if let Some(words) = model.needed() {
            return (words, model.headline_color());
        }
        match self.current_input() {
            None => ("止まっています", TEXT_SECONDARY),
            Some(LiveSourceMode::Screen) => ("画面の文字を読み取っています", ACCENT),
            Some(_) if self.captions_arriving() => ("YouTube字幕を受け取っています", ACCENT),
            Some(_) if self.audio_enabled && self.asr_ready => ("PC音声を聞いています", ACCENT),
            Some(_) if self.audio_start_requested => ("音声認識を準備しています", TEXT_SECONDARY),
            Some(_) if self.caption_enabled => ("YouTube字幕を待っています", TEXT_SECONDARY),
            Some(_) => ("入力を待っています", TEXT_SECONDARY),
        }
    }

    /// The bar under the chat, where a messenger puts its message box: one
    /// large button that listens and stops, and one line about what is going
    /// on. The readout switch is here too when readout is used.
    fn listen_bar(&mut self, ui: &mut egui::Ui) {
        let listening = self.current_input().is_some();
        // A model that went away must not keep the button from stopping.
        let enabled = listening || self.backend_ready;
        let hover = if listening {
            "入力を止めます。保存済みの訳文はそのまま残ります".to_owned()
        } else {
            self.listen_input.hint().to_owned()
        };
        let mut press = false;
        let mut switch_readout = false;
        let mut rects = None;
        live_region(ui, "listen-bar", LISTEN_BAR_HEIGHT, |ui| {
            let bar = ui.max_rect();
            ui.painter().rect(
                bar,
                theme::RADIUS_CARD,
                SURFACE,
                theme::hairline(),
                egui::StrokeKind::Inside,
            );
            let inner = bar.shrink2(Vec2::new(theme::SPACE_3, theme::SPACE_2));
            ui.scope_builder(
                egui::UiBuilder::new()
                    .max_rect(inner)
                    .layout(egui::Layout::right_to_left(egui::Align::Center)),
                |ui| {
                    if self.settings.readout_enabled {
                        switch_readout = auto_read_switch(ui, self.settings.auto_read).clicked();
                    }
                    ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                        ui.spacing_mut().item_spacing.x = theme::SPACE_3;
                        let button = ui
                            .add_enabled(enabled, listen_button(listening))
                            .on_hover_text(format!("{hover}\n{LISTEN_SHORTCUT}"))
                            .on_disabled_hover_text("翻訳モデルの準備ができると、聞けます");
                        press = button.clicked();
                        rects = Some((bar, button.rect));
                        let (words, color) = self.listen_state();
                        ui.add(
                            egui::Label::new(
                                RichText::new(words).size(theme::SIZE_LABEL).color(color),
                            )
                            .truncate(),
                        )
                        .on_hover_text(words);
                    });
                },
            );
        });
        #[cfg(test)]
        {
            self.ui_probe.listen_bar = rects.map(|(bar, _)| bar);
            self.ui_probe.listen_button = rects.map(|(_, button)| button);
        }
        #[cfg(not(test))]
        let _ = rects;
        if switch_readout {
            self.toggle_auto_read(ui.ctx());
        }
        if press {
            self.toggle_listening(ui.ctx());
        }
    }

    /// One reserved line: a failure to save first, then what is happening now,
    /// then whether the timeline is being saved. Service errors are beside the
    /// 聞く button.
    fn live_status_line(&mut self, ui: &mut egui::Ui) {
        live_region(ui, "live-status", STATUS_LINE_HEIGHT, |ui| {
            // The journal error is the storage layer's own wording, so the
            // plain statement comes first and the detail follows it.
            let unsaved = if self.journal_error.is_empty() {
                String::new()
            } else {
                format!("保存できていません（{}）", self.journal_error)
            };
            let activity = if self.readout_busy() {
                self.readout.status.as_str()
            } else if self.live_busy.is_some() {
                "翻訳中"
            } else {
                ""
            };
            let saved = if !self.journal_error.is_empty() {
                ("", TEXT_MUTED)
            } else if self.journal.is_some() {
                ("タイムラインへ自動保存", theme::SUCCESS)
            } else {
                ("プレビュー · 保存されません", TEXT_MUTED)
            };
            ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                status_message(ui, &[(unsaved.as_str(), ERROR), (activity, ACCENT), saved]);
            });
        });
    }

    fn languages(&mut self, ui: &mut egui::Ui) {
        // ComboBox places itself at the top of the free space, so a centred row
        // staggers it against the buttons. Top-aligning keeps one baseline.
        ui.horizontal_top(|ui| {
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
    fn input_card(&mut self, ui: &mut egui::Ui) {
        card().show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.label(
                    RichText::new(format!("原文 · {}字", self.input.chars().count()))
                        .size(theme::SIZE_LABEL)
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
            let height = text_area_height(ui);
            egui::ScrollArea::vertical()
                .id_salt("source-scroll")
                .max_height(height)
                .min_scrolled_height(height)
                .show(ui, |ui| {
                    ui.add_sized(
                        [ui.available_width(), height],
                        egui::TextEdit::multiline(&mut self.input)
                            .hint_text(theme::hint("翻訳したい文章を入力してください"))
                            .font(egui::FontId::proportional(theme::SIZE_BODY))
                            .margin(theme::SPACE_3)
                            .desired_width(f32::INFINITY),
                    );
                });
        });
    }
    /// `service_error` also carries speech-recognition failures, so the
    /// translation state follows the translation engine alone.
    fn translation_readiness(&self) -> Readiness {
        if self.backend_ready {
            Readiness::Ready
        } else if self.backend_loading || self.setup.installing() {
            Readiness::Preparing
        } else {
            Readiness::Missing
        }
    }
    /// The local models in use, in the order the details list them.
    fn model_statuses(&self) -> Vec<ModelStatus> {
        let translation = self.translation_readiness();
        let ocr = if self.screen_ocr_ready() {
            Readiness::Ready
        } else if self.screen_ocr_preparing() {
            Readiness::Preparing
        } else {
            Readiness::Missing
        };
        let speech = if self.asr_ready {
            Readiness::Ready
        } else if self.audio_installing
            || self.audio_start_requested
            || self.asr_started_at.is_some()
        {
            Readiness::Preparing
        } else {
            Readiness::Missing
        };
        let translation_detail = format!(
            "{} · {}",
            self.current_model_label(),
            self.connection_label()
        );
        let ocr_detail = match self.settings.ocr_backend.as_str() {
            "indra" => "Indra OCR",
            "paddle" => "PaddleOCR-VL",
            _ => "Windows OCR",
        };
        let speech_detail = match self.settings.asr_backend.as_str() {
            "nemotron" => "Nemotron Speech",
            "qwen" => "Qwen3-ASR",
            _ => "Whisper",
        };
        let mut statuses = vec![
            ModelStatus {
                name: "翻訳",
                readiness: translation,
                detail: translation_detail,
                opens: Tab::Setup,
                required: true,
            },
            ModelStatus {
                name: "文字認識",
                readiness: ocr,
                detail: ocr_detail.into(),
                opens: Tab::Settings,
                required: false,
            },
            ModelStatus {
                name: "音声認識",
                readiness: speech,
                detail: speech_detail.into(),
                opens: Tab::Settings,
                required: false,
            },
        ];
        if self.settings.readout_enabled {
            statuses.push(ModelStatus {
                name: "読み上げ",
                readiness: if self.readout.ready.is_some() {
                    Readiness::Ready
                } else if self.readout.preparing {
                    Readiness::Preparing
                } else {
                    Readiness::Missing
                },
                detail: "Irodori-TTS".into(),
                opens: Tab::Settings,
                required: false,
            });
        }
        statuses
    }
    fn output_card(&mut self, ui: &mut egui::Ui) {
        card().show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.label(
                    RichText::new("訳文")
                        .size(theme::SIZE_LABEL)
                        .strong()
                        .color(ACCENT),
                );
                if ui
                    .add_enabled(self.manual_output_ready(), standard("コピー"))
                    .on_hover_text("PLaMoのクレジットを付けてコピーします")
                    .clicked()
                    && let Ok(mut clipboard) = arboard::Clipboard::new()
                {
                    let _ = clipboard.set_text(credit::published(&self.output));
                }
                if ui
                    .add_enabled(self.manual_output_ready(), standard("保存"))
                    .clicked()
                {
                    self.save_text();
                }
                if self.settings.readout_enabled {
                    if ui
                        .add_enabled(self.manual_output_ready(), standard("読み上げ"))
                        .clicked()
                    {
                        self.read_translation(self.output.clone());
                    }
                    if self.readout_busy() && ui.add(standard("停止")).clicked() {
                        self.stop_readout();
                    }
                }
            });
            let height = text_area_height(ui);
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
                            .hint_text(theme::hint("訳文はここに表示されます"))
                            .font(egui::FontId::proportional(theme::reading_size(
                                self.settings.font_size,
                            )))
                            .margin(theme::SPACE_3)
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
                "貼り付けた文章や画像の文字を翻訳します。",
                |ui| {
                    if ui.add_enabled(ready, standard("画像から翻訳")).clicked() {
                        open_image = true;
                    }
                },
            );
            if open_image {
                self.open_image();
            }
            ui.add_space(theme::SPACE_2);
            // Languages, then the action they configure, on one line.
            live_region(ui, "text-languages", theme::CONTROL_H_LARGE, |ui| {
                ui.horizontal(|ui| {
                    self.languages(ui);
                    ui.add_space(theme::SPACE_4);
                    if ui
                        .add_enabled(translate_ready, primary("翻訳する"))
                        .on_hover_text("Ctrl + Enter")
                        .clicked()
                    {
                        translate = true;
                    }
                });
            });
            if translate {
                self.manual_translate();
            }
            self.backend_notice(ui);
            ui.add_space(theme::SPACE_2);
            let ready =
                self.backend_ready && self.manual_busy.is_none() && !self.input.trim().is_empty();
            let total = ui.available_width();
            if total < SIDE_BY_SIDE_MIN_WIDTH {
                let half = ((ui.available_height() - theme::SPACE_2) / 2.0).max(0.0);
                ui.allocate_ui_with_layout(
                    Vec2::new(total, half),
                    egui::Layout::top_down(egui::Align::Min),
                    |ui| self.input_card(ui),
                );
                ui.add_space(theme::SPACE_2);
                ui.allocate_ui_with_layout(
                    Vec2::new(total, half),
                    egui::Layout::top_down(egui::Align::Min),
                    |ui| self.output_card(ui),
                );
            } else {
                let input_width = total * 0.42;
                let output_width = total - input_width - theme::SPACE_3;
                // Measured outside the row: a horizontal row reports only one
                // control height as available.
                let height = ui.available_height();
                ui.horizontal(|ui| {
                    ui.allocate_ui_with_layout(
                        Vec2::new(input_width, height),
                        egui::Layout::top_down(egui::Align::Min),
                        |ui| {
                            ui.set_width(input_width);
                            self.input_card(ui);
                        },
                    );
                    ui.add_space(theme::SPACE_3);
                    ui.allocate_ui_with_layout(
                        Vec2::new(output_width, height),
                        egui::Layout::top_down(egui::Align::Min),
                        |ui| {
                            ui.set_width(output_width);
                            self.output_card(ui);
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
        page_column(ui, |ui| {
            let can_clear = !self.rows.is_empty() || !self.asr_partial.trim().is_empty();
            let mut clear = false;
            self.page_header(ui, "Talk風Live翻訳", "PC内で翻訳", |ui| {
                if ui
                    .add_enabled(can_clear, standard("クリア"))
                    .on_hover_text(
                        "表示中の会話を消して、新しく始めます。消した会話は「履歴」に残ります。",
                    )
                    .clicked()
                {
                    clear = true;
                }
            });
            if clear {
                self.follow(ui.ctx(), MenuChoice::Clear);
            }
            ui.add_space(theme::SPACE_2);
            self.live_status_line(ui);
            ui.add_space(theme::SPACE_1);
            #[cfg(test)]
            {
                self.ui_probe.reading_font = theme::reading_size(self.settings.font_size);
            }
            let visible = self.document_visible_row().cloned();
            let messages = live_messages(&self.rows, visible.as_ref(), self.asr_partial.trim());
            let options = chat::Options {
                text_size: theme::reading_size(self.settings.font_size),
                show_originals: self.settings.live_show_original,
                follow: self.settings.auto_scroll,
                readout: self.settings.readout_enabled.then(|| self.readout_busy()),
            };
            // The chat takes the room the bar under it leaves.
            let chat = Vec2::new(
                ui.available_width(),
                (ui.available_height() - LISTEN_BAR_HEIGHT - ui.spacing().item_spacing.y).max(0.0),
            );
            let action = ui
                .allocate_ui(chat, |ui| {
                    self.live_chat.show(ui, "live-chat", &messages, options)
                })
                .inner;
            self.listen_bar(ui);
            match action {
                Some(chat::Action::Read(text)) => self.read_translation(text),
                Some(chat::Action::StopReadout) => self.stop_readout(),
                None => {}
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
                "履歴",
                "これまでの訳文を探せます。ファイルも翻訳できます。",
                |ui| {
                    if ui
                        .add_enabled(ready, primary("ファイルを選んで翻訳"))
                        .on_hover_text(
                            "テキスト・Markdown・SRT・WebVTTを翻訳します。字幕の時刻は維持されます。",
                        )
                        .clicked()
                    {
                        translate_file = true;
                    }
                },
            );
            if translate_file {
                self.translate_file();
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
                            .size(theme::SIZE_LABEL)
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
            // Search and export both act on the list below, so they share its row.
            ui.horizontal(|ui| {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui
                        .add_enabled(has_rows, standard("書き出し"))
                        .on_hover_text("履歴をTXT・JSON・SRTに書き出します")
                        .clicked()
                    {
                        export = true;
                    }
                    ui.add_sized(
                        [ui.available_width(), theme::CONTROL_H],
                        egui::TextEdit::singleline(&mut self.search)
                            .hint_text(theme::hint("原文・訳文を検索"))
                            .margin(theme::SPACE_2),
                    );
                });
            });
            if export {
                self.export_history();
            }
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
                                history_time(&row.record.created_at),
                                row.record.kind
                            ))
                            .size(theme::SIZE_CAPTION)
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
                                    .size(theme::SIZE_LABEL)
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
                                .size(theme::SIZE_BODY)
                                .color(TEXT_SECONDARY),
                        );
                    }
                });
        });
    }

    fn settings_translation_card(&mut self, ui: &mut egui::Ui) {
        ui.label(RichText::new("翻訳モデル").strong());
        ui.label(self.setup_summary());
        secondary(ui, self.connection_label());
        if ui.add(standard("モデルを変更")).clicked() {
            self.open_setup();
        }
    }

    fn settings_asr_card(&mut self, ui: &mut egui::Ui) {
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
        ui.horizontal(|ui| {
            ui.label(RichText::new("YouTube字幕の接続").strong());
            self.link_badge(ui);
        });
        secondary(
            ui,
            "拡張機能の入れ方と接続キーは、専用の画面にまとめました。",
        );
        if ui.add(standard("接続の画面を開く")).clicked() {
            self.tab = Tab::YouTube;
        }
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
                ui.label(RichText::new("文脈（翻訳する文章の背景）").size(theme::SIZE_LABEL));
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
                secondary(
                    ui,
                    "範囲翻訳: Ctrl + Alt + T　 聞く・止める: Ctrl + Alt + L",
                );
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
                    ui.label(RichText::new(label).size(theme::SIZE_LABEL));
                    ui.add(egui::TextEdit::singleline(value).desired_width(f32::INFINITY));
                }
                ui.label(
                    RichText::new(format!("保存先: {}", data_root().display()))
                        .size(theme::SIZE_CAPTION)
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
            // The save action stays in view under the scrolling form.
            let footer = theme::SPACE_3 + theme::CONTROL_H_LARGE;
            egui::ScrollArea::vertical()
                .id_salt("settings-scroll")
                .auto_shrink([false, false])
                .max_height((ui.available_height() - footer).max(0.0))
                .show(ui, |ui| {
                    // Two independent columns above 1100 points so a wide
                    // window does not become one very long form.
                    if ui.available_width() >= 1100.0 {
                        ui.columns(2, |columns| {
                            columns[0].spacing_mut().item_spacing.y = theme::SPACE_3;
                            columns[1].spacing_mut().item_spacing.y = theme::SPACE_3;
                            settings_card(&mut columns[0], |ui| self.settings_translation_card(ui));
                            settings_card(&mut columns[0], |ui| self.readout_settings_ui(ui));
                            settings_card(&mut columns[0], |ui| {
                                self.paragraph_tools_settings_ui(ui)
                            });
                            self.settings_glossary_section(&mut columns[0]);
                            settings_card(&mut columns[1], |ui| self.settings_asr_card(ui));
                            settings_card(&mut columns[1], |ui| self.screen_ocr_settings_ui(ui));
                            settings_card(&mut columns[1], |ui| self.settings_youtube_card(ui));
                            self.settings_shortcuts_section(&mut columns[1]);
                            self.settings_diagnostics_section(&mut columns[1]);
                        });
                    } else {
                        ui.spacing_mut().item_spacing.y = theme::SPACE_3;
                        settings_card(ui, |ui| self.settings_translation_card(ui));
                        settings_card(ui, |ui| self.readout_settings_ui(ui));
                        settings_card(ui, |ui| self.settings_asr_card(ui));
                        settings_card(ui, |ui| self.screen_ocr_settings_ui(ui));
                        settings_card(ui, |ui| self.paragraph_tools_settings_ui(ui));
                        settings_card(ui, |ui| self.settings_youtube_card(ui));
                        self.settings_glossary_section(ui);
                        self.settings_shortcuts_section(ui);
                        self.settings_diagnostics_section(ui);
                    }
                    ui.add_space(theme::SPACE_6);
                    if ui.add(standard("アプリを終了")).clicked() {
                        ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                    ui.add_space(theme::SPACE_4);
                });
            // A hairline marks where the form scrolls under the fixed bar.
            let line = ui.cursor().top() + theme::SPACE_1;
            ui.painter()
                .hline(ui.max_rect().x_range(), line, theme::hairline());
            ui.add_space(theme::SPACE_3);
            let glossary = parse_glossary(&self.glossary_input);
            if ui
                .add_enabled(glossary.is_ok(), primary("設定を保存"))
                .clicked()
            {
                match glossary {
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
        for (tab, _, key) in PAGE_KEYS {
            if ctx.input_mut(|input| input.consume_key(egui::Modifiers::CTRL, key)) {
                self.tab = tab;
            }
        }
        // The chat says one thing about the models. Its errors are beside the
        // 聞く button, so the row does not repeat them.
        let readiness = (self.tab == Tab::Live).then(|| overall(&self.model_statuses()));
        let service_error = if self.tab == Tab::Live {
            ""
        } else {
            self.service_error.as_str()
        };
        let mut open_models = false;
        // One fixed row: a long message is truncated instead of growing the
        // panel and pushing the page up.
        egui::TopBottomPanel::bottom("status")
            .exact_height(theme::CONTROL_H)
            .frame(
                egui::Frame::new()
                    .fill(SURFACE)
                    .inner_margin(egui::Margin::symmetric(theme::GUTTER as i8, 0)),
            )
            .show(ctx, |ui| {
                #[cfg(test)]
                {
                    self.ui_probe.status_rect = Some(ui.max_rect());
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(theme::caption(credit::LABEL));
                    ui.add_space(theme::SPACE_3);
                    ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                        ui.spacing_mut().item_spacing.x = theme::SPACE_1;
                        if let Some(readiness) = readiness {
                            open_models = headline(ui, readiness);
                            ui.add_space(theme::SPACE_3);
                        }
                        let readout = if self.readout.error.is_empty() {
                            (self.readout.status.as_str(), TEXT_SECONDARY)
                        } else {
                            (self.readout.error.as_str(), ERROR)
                        };
                        status_message(
                            ui,
                            &[
                                (service_error, ERROR),
                                (self.status.as_str(), TEXT_SECONDARY),
                                readout,
                            ],
                        );
                    });
                });
            });
        if open_models {
            self.open_setup();
        }
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
                Tab::Setup => self.model_ui(ui),
                Tab::YouTube => self.youtube_ui(ui),
            });
        self.status_details_ui(ctx);
    }

    fn model_ui(&mut self, ui: &mut egui::Ui) {
        page_column(ui, |ui| {
            self.page_header(
                ui,
                "このPCに合うモデルを準備",
                "大きいモデルほど自然に訳せますが、GPUのメモリを多く使います。",
                |_| {},
            );
            self.setup_ui(ui);
        });
    }

    /// The state of every model, one per line. A model that is not prepared
    /// leads to the screen that prepares it.
    fn status_details_ui(&mut self, ctx: &egui::Context) {
        if !self.details_open {
            return;
        }
        let statuses = self.model_statuses();
        let width = (ctx.content_rect().width() - 48.0).clamp(260.0, 420.0);
        let mut opens = None;
        let mut close = ctx.input(|input| input.key_pressed(egui::Key::Escape));
        egui::Window::new("状態の詳細")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.set_min_width(width);
                if !self.service_error.is_empty() {
                    ui.add(
                        egui::Label::new(
                            RichText::new(&self.service_error)
                                .size(theme::SIZE_LABEL)
                                .color(ERROR),
                        )
                        .wrap(),
                    );
                }
                for status in &statuses {
                    ui.horizontal(|ui| {
                        if status_chip(ui, status)
                            .on_hover_text(status.hover())
                            .clicked()
                        {
                            opens = Some(status.opens);
                        }
                        ui.add(egui::Label::new(theme::caption(&status.detail)).truncate());
                    });
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
                    close |= ui.add(standard("閉じる")).clicked();
                });
            });
        match opens {
            Some(Tab::Setup) => self.open_setup(),
            Some(tab) => self.tab = tab,
            None => {}
        }
        if close || opens.is_some() {
            self.details_open = false;
        }
    }
}
#[cfg(test)]
mod home_tests;
#[cfg(test)]
mod listen_tests;
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
                standard_translation: None,
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
    use crate::chat::testing::{cut_at_the_top, dots, find_text, has_text_containing};
    use plamo_translate::journal::{ParagraphMeta, ParagraphState};

    fn paragraph(row: &mut Row, state: ParagraphState) {
        row.record.paragraph = Some(ParagraphMeta {
            revision: 1,
            state,
            close_reason: None,
        });
    }

    #[test]
    fn live_rows_become_messages_in_order_with_their_status_and_other_kinds_stay_out() {
        let mut fixed = row(1, false, false);
        paragraph(&mut fixed, ParagraphState::Final);
        let mut open = row(2, false, false);
        paragraph(&mut open, ParagraphState::Open);
        let pending = row(3, true, false);
        let mut interrupted = row(4, false, false);
        paragraph(&mut interrupted, ParagraphState::Interrupted);
        let failed = row(5, false, true);
        let mut manual = row(6, false, false);
        manual.record.kind = "文章".into();
        let mut video = row(7, false, false);
        video.record.kind = "YouTube".into();
        video.record.start_ms = 754_000;
        video.record.created_at = "2026-11-07T21:48:03+09:00".into();
        let rows = [fixed, open, pending, interrupted, failed, manual, video];

        let messages = live_messages(&rows, None, "");
        assert_eq!(
            messages
                .iter()
                .map(|message| message.id)
                .collect::<Vec<_>>(),
            [1, 2, 3, 4, 5, 7],
            "oldest first, and text typed by hand is not part of the chat"
        );
        use chat::Status::{Failed, Fixed, Forming, Interrupted};
        assert_eq!(
            messages
                .iter()
                .map(|message| message.status)
                .collect::<Vec<_>>(),
            [Fixed, Forming, Forming, Interrupted, Failed, Fixed]
        );
        assert!(
            messages[4]
                .failure
                .is_some_and(|reason| reason.contains("Long error"))
        );
        let video = messages[5];
        assert_eq!(video.source, chat::Source::YouTube);
        assert_eq!(video.moment.video_ms, Some(754_000));
        assert_eq!(
            video.moment.sent.map(|sent| sent.to_string()),
            Some("2026-11-07 21:48:03".to_owned()),
            "the clock comes from created_at"
        );
        assert_eq!(
            messages[0].moment.video_ms, None,
            "only YouTube has a video time"
        );
    }

    #[test]
    fn the_coherent_pair_of_the_current_paragraph_stands_in_for_its_row() {
        let mut open = row(2, false, false);
        paragraph(&mut open, ParagraphState::Open);
        let rows = [row(1, false, false), open.clone()];
        open.record.translation = "分割前の訳文".into();
        let messages = live_messages(&rows, Some(&open), "");
        assert_eq!(messages[1].target, "分割前の訳文");
        assert_eq!(messages[0].target, rows[0].record.translation);
    }

    #[test]
    fn speech_being_recognised_is_a_bubble_of_dots_unless_a_message_is_already_forming() {
        let mut fixed = row(1, false, false);
        paragraph(&mut fixed, ParagraphState::Final);
        let mut open = row(2, false, false);
        paragraph(&mut open, ParagraphState::Open);
        let typing = |messages: &[chat::Message<'_>]| {
            messages
                .last()
                .is_some_and(|message| message.id == chat::TYPING)
        };
        let messages = live_messages(&[], None, "recognised");
        assert!(typing(&messages));
        assert_eq!(messages.last().unwrap().original, "recognised");
        assert!(typing(&live_messages(
            std::slice::from_ref(&fixed),
            None,
            "recognised"
        )));
        assert!(!typing(&live_messages(
            std::slice::from_ref(&fixed),
            None,
            ""
        )));
        assert!(
            !typing(&live_messages(&[fixed.clone(), open], None, "recognised")),
            "the forming message already ends in dots"
        );
        let mut video = fixed;
        video.record.kind = "YouTube".into();
        assert!(
            typing(&live_messages(&[video], None, "recognised")),
            "recognised speech is PC audio, whatever came before"
        );
    }

    #[test]
    fn the_live_page_is_a_chat_with_the_oldest_message_at_the_top() {
        let mut app = App::inert();
        app.tab = Tab::Live;
        app.backend_ready = true;
        let mut rows = Vec::new();
        for (id, kind, clock) in [
            (1, "YouTube", "21:46:10"),
            (2, "YouTube", "21:46:40"),
            (3, "音声", "21:48:20"),
        ] {
            let mut message = row(id, false, false);
            message.record.kind = kind.into();
            message.record.created_at = format!("2026-11-07T{clock}+09:00");
            message.record.source = format!("Original {id}.");
            message.record.translation = format!("{id}番目の訳文です。");
            rows.push(message);
        }
        app.rows = rows;
        let drawn = drawn_text(&mut app, 1280.0, 880.0);
        let at = |text: &str| {
            drawn
                .find(text)
                .unwrap_or_else(|| panic!("{text} is not drawn"))
        };
        assert!(at("1番目の訳文です。") < at("2番目の訳文です。"));
        assert!(at("2番目の訳文です。") < at("3番目の訳文です。"));
        for shown in ["11月7日（土）", "YouTube", "PC音声", "21:46", "21:48"] {
            assert!(drawn.contains(shown), "{shown} is not drawn");
        }
        for gone in [
            "これまでの文章",
            "最新の対訳",
            "新しい順に追従",
            "Original 1.",
        ] {
            assert!(!drawn.contains(gone), "{gone} is still drawn");
        }
    }

    #[test]
    fn a_forming_paragraph_ends_in_dots_and_a_new_draft_does_not_add_a_second_bubble() {
        let mut app = App::inert();
        app.tab = Tab::Live;
        app.backend_ready = true;
        let mut open = row(7, false, false);
        open.record.translation = "文章全体をつないだ継続中の日本語訳".into();
        paragraph(&mut open, ParagraphState::Open);
        app.rows = vec![open];
        app.current_live_id = Some(7);
        app.asr_partial = "Additional unconfirmed draft".into();
        let output = render(&mut app, 860.0, 620.0);
        let (_, color) = find_text(&output, "文章全体をつないだ継続中の日本語訳");
        assert_eq!(color, theme::CHAT_TEXT_FORMING);
        assert_eq!(
            dots(&output).len(),
            3,
            "one forming message, one set of dots"
        );
        assert!(!has_text_containing(
            &output,
            "Additional unconfirmed draft"
        ));

        app.rows[0].record.paragraph.as_mut().unwrap().state = ParagraphState::Final;
        app.current_live_id = None;
        let output = render(&mut app, 860.0, 620.0);
        let (_, color) = find_text(&output, "文章全体をつないだ継続中の日本語訳");
        assert_eq!(color, theme::CHAT_TEXT);
        assert_eq!(
            dots(&output).len(),
            3,
            "the new speech is a bubble of dots after the fixed message"
        );
    }

    #[test]
    fn no_message_is_drawn_cut_at_the_top_while_the_live_chat_follows() {
        for height in [620.0, 727.0, 800.0, 880.0] {
            for size in [18.0, 20.0, 24.0] {
                let mut app = App::inert();
                app.backend_ready = true;
                app.settings.font_size = size;
                app.rows = (1..=14)
                    .map(|id| {
                        let mut message = row(id, false, false);
                        message.record.translation = format!(
                            "{id}番目の訳文です。文章全体の文脈を保ちながら訳文を更新します。"
                        );
                        message
                    })
                    .collect();
                let context = egui::Context::default();
                configure_theme(&context);
                let mut output = None;
                for frame in 0..6 {
                    let mut input = egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(if height == 620.0 { 860.0 } else { 1182.0 }, height),
                        )),
                        time: Some(f64::from(frame) / 60.0),
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
                let body = app.live_chat.body().unwrap();
                assert!(
                    app.live_chat.offset() > 0.0,
                    "the talk is taller than the window"
                );
                assert!(
                    !cut_at_the_top(&output, body),
                    "a message is cut at the top at height={height}, font={size}"
                );
                assert!(
                    has_text_containing(&output, "14番目の訳文です。"),
                    "the newest message is in view at height={height}, font={size}"
                );
            }
        }
    }

    #[test]
    fn the_live_chat_keeps_its_height_with_a_long_service_error_at_the_minimum_window() {
        for (width, height) in [(860.0, 620.0), (1280.0, 800.0)] {
            let mut app = App::inert();
            app.backend_ready = true;
            app.asr_ready = true;
            app.audio_enabled = true;
            app.caption_enabled = true;
            app.rows = (1..=3).map(|id| row(id, false, false)).collect();
            app.current_live_id = Some(3);
            // The error stays in the one reserved status line.
            app.service_error = "Long service error. ".repeat(100);
            live_window(&mut app, width, height);
            let body = app.live_chat.body().unwrap();
            assert!(
                body.height() > 300.0,
                "the chat is only {} points tall at {width}x{height}",
                body.height()
            );
        }
    }

    #[test]
    fn the_live_chat_follows_the_newest_message_until_the_reader_scrolls_up() {
        let context = egui::Context::default();
        configure_theme(&context);
        let mut app = App::inert();
        app.backend_ready = true;
        app.settings.auto_scroll = true;
        let paragraph_row = |id: u64| {
            let mut message = row(id, false, false);
            message.record.translation = format!(
                "Paragraph {id}. {}",
                "A complete translated paragraph. ".repeat(16)
            );
            message
        };
        app.rows = (1..=6).map(paragraph_row).collect();
        let mut frame = 0;
        let mut draw = |app: &mut App, events: Vec<egui::Event>| {
            frame += 1;
            context.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1280.0, 880.0),
                    )),
                    time: Some(f64::from(frame) * 0.05),
                    events,
                    ..Default::default()
                },
                |ctx| app.render_ui(ctx),
            )
        };
        for _ in 0..5 {
            draw(&mut app, vec![]);
        }
        let tail = app.live_chat.offset();
        assert!(tail > 100.0, "the talk is taller than the window: {tail}");

        let pointer = app.live_chat.body().unwrap().center();
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
        let reading = app.live_chat.offset();
        assert!(
            reading < tail - 1.0,
            "the wheel did not leave the tail: {reading} vs {tail}"
        );

        app.rows.push(paragraph_row(7));
        let mut output = draw(&mut app, vec![]);
        for _ in 0..3 {
            output = draw(&mut app, vec![]);
        }
        assert!(
            (app.live_chat.offset() - reading).abs() < 1.0,
            "a new message moved the reader's position"
        );
        assert!(has_text_containing(&output, "↓ 新しいメッセージ 1件"));

        app.settings.auto_scroll = false;
        draw(&mut app, vec![]);
        let waiting = app.live_chat.offset();
        app.settings.auto_scroll = true;
        for _ in 0..3 {
            output = draw(&mut app, vec![]);
        }
        assert!(
            app.live_chat.offset() > waiting + 50.0,
            "turning auto-scroll on again did not return to the tail"
        );
        assert!(!has_text_containing(&output, "新しいメッセージ"));
        let tail = app.live_chat.offset();

        app.rows.push(paragraph_row(8));
        for _ in 0..3 {
            draw(&mut app, vec![]);
        }
        assert!(
            app.live_chat.offset() > tail + 50.0,
            "the tail did not follow the next message"
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

    /// The last of three frames of the app in a fresh window.
    fn render(app: &mut App, width: f32, height: f32) -> egui::FullOutput {
        let context = egui::Context::default();
        configure_theme(&context);
        let mut output = None;
        for frame in 0..3 {
            output = Some(context.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(width, height),
                    )),
                    time: Some(f64::from(frame) / 60.0),
                    ..Default::default()
                },
                |context| app.render_ui(context),
            ));
        }
        output.unwrap()
    }

    fn drawn_text(app: &mut App, width: f32, height: f32) -> String {
        drawn_galleys(app, width, height)
            .into_iter()
            .map(|(text, _)| text)
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Every painted text in the last frame, with the number of rows it took.
    fn drawn_galleys(app: &mut App, width: f32, height: f32) -> Vec<(String, usize)> {
        fn visit(shape: &egui::epaint::Shape, out: &mut Vec<(String, usize)>) {
            match shape {
                egui::epaint::Shape::Text(text) => {
                    out.push((text.galley.job.text.clone(), text.galley.rows.len()))
                }
                egui::epaint::Shape::Vec(shapes) => shapes.iter().for_each(|s| visit(s, out)),
                _ => {}
            }
        }
        let context = egui::Context::default();
        configure_theme(&context);
        let mut texts = Vec::new();
        for frame in 0..3 {
            let output = context.run(
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
            texts.clear();
            for shape in &output.shapes {
                visit(&shape.shape, &mut texts);
            }
        }
        texts
    }

    #[test]
    fn the_status_row_stays_one_row_with_the_credit_and_no_jargon() {
        for tab in [Tab::Text, Tab::Live] {
            for (width, message) in [
                (860.0, String::new()),
                (860.0, "とても長い状態の説明が続きます。".repeat(40)),
                (1280.0, "設定を保存しました。".to_owned()),
            ] {
                let mut app = readout_app(tab);
                app.backend_ready = true;
                app.status = message;
                app.readout.error = "読み上げに失敗しました。".repeat(8);
                let galleys = drawn_galleys(&mut app, width, 620.0);
                assert!(
                    galleys.iter().any(|(text, _)| text == credit::LABEL),
                    "credit missing at {width}"
                );
                for (text, _) in &galleys {
                    for jargon in ["OCR", "STT", "TTS"] {
                        assert!(!text.contains(jargon), "{jargon} still shown at {width}");
                    }
                }
                let (_, rows) = galleys
                    .iter()
                    .find(|(text, _)| text.contains("読み上げに失敗しました"))
                    .expect("status message drawn");
                assert_eq!(*rows, 1, "status message wrapped at {width}");
            }
        }
    }

    #[test]
    fn the_live_page_keeps_one_status_line() {
        let mut app = App::inert();
        app.tab = Tab::Live;
        app.backend_ready = true;
        app.rows = vec![
            row(1, false, false),
            row(2, false, false),
            row(3, false, false),
        ];
        app.current_live_id = Some(3);
        let drawn = drawn_text(&mut app, 1280.0, 880.0);
        assert_eq!(
            drawn.matches("プレビュー · 保存されません").count(),
            1,
            "{drawn}"
        );
    }

    #[test]
    fn a_failing_timeline_says_so_in_japanese_before_its_detail() {
        let mut app = App::inert();
        app.tab = Tab::Live;
        app.backend_ready = true;
        app.journal_error = "Could not append translation history".into();
        let drawn = drawn_text(&mut app, 1280.0, 880.0);
        assert!(
            drawn.contains("保存できていません（Could not append translation history）"),
            "{drawn}"
        );
    }

    #[test]
    fn history_shows_readable_times_and_keeps_unknown_values() {
        assert_eq!(
            history_time("2026-11-05T12:00:00+09:00"),
            "2026/11/05 12:00"
        );
        assert_eq!(history_time(""), "");
        assert_eq!(history_time("昨日"), "昨日");
        let mut app = App::inert();
        app.tab = Tab::Files;
        app.rows = vec![row(1, false, false)];
        app.rows[0].record.created_at = "2026-11-05T12:00:00+09:00".into();
        let drawn = drawn_text(&mut app, 860.0, 620.0);
        assert!(drawn.contains("2026/11/05 12:00 · 音声"), "{drawn}");
        assert!(!drawn.contains("T12:00:00"));
    }

    #[test]
    fn ctrl_number_opens_each_page_from_the_keyboard() {
        let context = egui::Context::default();
        configure_theme(&context);
        let mut app = App::inert();
        app.tab = Tab::Text;
        for (expected, key) in [
            (Tab::Live, egui::Key::Num2),
            (Tab::Files, egui::Key::Num3),
            (Tab::Settings, egui::Key::Num4),
            (Tab::Text, egui::Key::Num1),
        ] {
            let _ = context.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(860.0, 620.0),
                    )),
                    events: vec![egui::Event::Key {
                        key,
                        physical_key: None,
                        pressed: true,
                        repeat: false,
                        modifiers: egui::Modifiers::CTRL,
                    }],
                    ..Default::default()
                },
                |ctx| app.render_ui(ctx),
            );
            assert!(
                app.tab == expected,
                "Ctrl+{} did not open its page",
                key.name()
            );
        }
    }

    #[test]
    fn empty_fields_show_placeholders_in_the_muted_colour() {
        fn visit<'a>(shape: &'a egui::epaint::Shape, out: &mut Vec<&'a egui::epaint::TextShape>) {
            match shape {
                egui::epaint::Shape::Text(text) => out.push(text),
                egui::epaint::Shape::Vec(shapes) => shapes.iter().for_each(|s| visit(s, out)),
                _ => {}
            }
        }
        let context = egui::Context::default();
        configure_theme(&context);
        let mut app = App::inert();
        app.tab = Tab::Text;
        app.backend_ready = true;
        let mut output = None;
        for frame in 0..3 {
            output = Some(context.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1280.0, 880.0),
                    )),
                    time: Some(frame as f64 / 60.0),
                    ..Default::default()
                },
                |ctx| app.render_ui(ctx),
            ));
        }
        let output = output.unwrap();
        let mut texts = Vec::new();
        for shape in &output.shapes {
            visit(&shape.shape, &mut texts);
        }
        for placeholder in [
            "翻訳したい文章を入力してください",
            "訳文はここに表示されます",
        ] {
            let shape = texts
                .iter()
                .find(|text| text.galley.job.text == placeholder)
                .unwrap_or_else(|| panic!("{placeholder} not drawn"));
            let color = shape
                .override_text_color
                .unwrap_or(shape.galley.job.sections[0].format.color);
            assert_eq!(
                color,
                theme::TEXT_MUTED,
                "{placeholder} is drawn like entered text"
            );
        }
    }

    #[test]
    fn each_model_follows_its_own_service_and_errors_reach_the_status_message() {
        let mut app = App::inert();
        app.tab = Tab::Text;
        app.backend_ready = true;
        // Speech-recognition failures share `service_error` with translation.
        app.service_error = "Nemotronを起動できません: 実行環境がありません".into();
        let statuses = app.model_statuses();
        assert_eq!(statuses[0].readiness, Readiness::Ready);
        let speech = statuses.iter().find(|s| s.name == "音声認識").unwrap();
        assert_eq!(speech.readiness, Readiness::Missing);
        assert!(speech.opens == Tab::Settings);
        let drawn = drawn_text(&mut app, 1280.0, 880.0);
        assert!(drawn.contains("Nemotronを起動できません"), "{drawn}");
        app.backend_ready = false;
        app.service_error.clear();
        let translation = &app.model_statuses()[0];
        assert_eq!(translation.readiness, Readiness::Missing);
        assert!(translation.readiness.needs_action());
        assert!(translation.opens == Tab::Setup);
        app.backend_loading = true;
        assert_eq!(app.model_statuses()[0].readiness, Readiness::Preparing);
    }

    #[test]
    fn minimum_window_keeps_32_point_targets_and_a_large_reading_font() {
        let mut app = App::inert();
        app.backend_ready = true;
        app.tab = Tab::Live;
        app.rows = vec![row(1, false, false)];
        app.current_live_id = Some(1);
        live_window(&mut app, 860.0, 620.0);
        let menu = app.ui_probe.menu_button.expect("the menu button is drawn");
        assert!(
            menu.height() >= 32.0,
            "the menu button is {} points tall",
            menu.height()
        );
        assert!(
            app.ui_probe.reading_font >= 20.0,
            "reading font {} is below 20 points",
            app.ui_probe.reading_font
        );
        let body = app.live_chat.body().unwrap();
        assert!(
            body.height() > 300.0,
            "the chat is only {} points tall at 860x620",
            body.height()
        );
    }

    #[test]
    fn growing_translation_never_moves_the_header_or_the_chat_area() {
        let mut app = App::inert();
        app.backend_ready = true;
        app.tab = Tab::Live;
        let mut active = row(2, false, false);
        active.record.translation = "短い訳。".into();
        app.rows = vec![row(1, false, false), active];
        app.current_live_id = Some(2);
        live_window(&mut app, 1280.0, 800.0);
        let before = app.live_chat.panel().unwrap();
        app.rows[1].record.translation = "段落全体を更新する長い日本語の訳文です。".repeat(40);
        live_window(&mut app, 1280.0, 800.0);
        let after = app.live_chat.panel().unwrap();
        assert!((before.top() - after.top()).abs() < 0.5);
        assert!((before.height() - after.height()).abs() < 0.5);
    }

    #[test]
    fn an_unconfirmed_paragraph_is_marked_apart_from_a_confirmed_one() {
        let mut app = App::inert();
        app.backend_ready = true;
        app.tab = Tab::Live;
        let mut confirmed = row(1, false, false);
        confirmed.record.translation = "確定した訳文です。".into();
        paragraph(&mut confirmed, ParagraphState::Final);
        let mut interrupted = row(2, false, false);
        interrupted.record.translation = "中断された訳文です。".into();
        paragraph(&mut interrupted, ParagraphState::Interrupted);
        let mut failed = row(3, false, true);
        failed.record.translation = "失敗した訳文です。".into();
        paragraph(&mut failed, ParagraphState::Failed);
        app.rows = vec![confirmed, interrupted, failed];
        let drawn = drawn_text(&mut app, 1280.0, 880.0);
        assert_eq!(drawn.matches("中断した文章 · 未確定").count(), 1, "{drawn}");
        assert_eq!(
            drawn.matches("更新に失敗 · 最後の訳文を表示").count(),
            1,
            "{drawn}"
        );
        for text in [
            "確定した訳文です。",
            "中断された訳文です。",
            "失敗した訳文です。",
        ] {
            assert!(drawn.contains(text), "{text} is not drawn");
        }
    }

    /// Clicks the one control drawn with exactly `label`, the way a pointer
    /// would: hover, press and release over the centre of its text.
    fn click_label(app: &mut App, label: &str, width: f32, height: f32) {
        fn centres(shape: &egui::epaint::Shape, label: &str, out: &mut Vec<egui::Pos2>) {
            match shape {
                egui::epaint::Shape::Text(text) if text.galley.job.text == label => {
                    out.push(text.visual_bounding_rect().center())
                }
                egui::epaint::Shape::Vec(shapes) => {
                    shapes.iter().for_each(|s| centres(s, label, out))
                }
                _ => {}
            }
        }
        let context = egui::Context::default();
        configure_theme(&context);
        let frame = |app: &mut App, number: u32, events: Vec<egui::Event>| {
            context.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(width, height),
                    )),
                    time: Some(f64::from(number) / 60.0),
                    events,
                    ..Default::default()
                },
                |context| app.render_ui(context),
            )
        };
        let mut found = Vec::new();
        for number in 0..3 {
            found.clear();
            for clipped in frame(app, number, vec![]).shapes {
                centres(&clipped.shape, label, &mut found);
            }
        }
        assert_eq!(
            found.len(),
            1,
            "expected one {label} control, found {found:?}"
        );
        let centre = found[0];
        let press = |pressed| egui::Event::PointerButton {
            pos: centre,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        frame(app, 3, vec![egui::Event::PointerMoved(centre)]);
        frame(app, 4, vec![press(true)]);
        frame(app, 5, vec![press(false)]);
    }

    fn ready_voice() -> plamo_translate::speech_setup::SpeechInstallation {
        plamo_translate::speech_setup::SpeechInstallation {
            executable: Default::default(),
            model: Default::default(),
            backend: String::new(),
            voice: String::new(),
        }
    }
    /// An app whose readout was turned on in settings.
    fn readout_app(tab: Tab) -> App {
        let mut app = App::inert();
        app.tab = tab;
        app.settings.readout_enabled = true;
        app
    }

    #[test]
    fn readout_switch_is_on_every_page_and_turning_it_off_stops_the_voice() {
        for (tab, page, _) in PAGE_KEYS {
            let mut app = readout_app(tab);
            app.readout.ready = Some(ready_voice());
            app.settings.auto_read = true;
            click_label(&mut app, "自動読み上げ", 860.0, 620.0);
            assert!(
                !app.settings.auto_read,
                "{page}: one click turns readout off"
            );
            assert_eq!(
                app.readout.status, "読み上げを停止しました。",
                "{page}: turning it off also stops the sentence being read"
            );
            click_label(&mut app, "自動読み上げ", 860.0, 620.0);
            assert!(app.settings.auto_read, "{page}: the next click turns it on");
        }
        let mut app = readout_app(Tab::Text);
        click_label(&mut app, "自動読み上げ", 860.0, 620.0);
        assert!(
            !app.settings.auto_read,
            "a voice that is not ready stays off"
        );
        assert!(
            app.tab == Tab::Settings && app.readout.confirm,
            "and the click opens the voice preparation instead"
        );

        let mut app = readout_app(Tab::Settings);
        app.readout.ready = Some(ready_voice());
        app.settings.auto_read = true;
        click_label(&mut app, "日本語の訳文を自動で読み上げる", 1280.0, 880.0);
        assert!(
            !app.settings.auto_read,
            "the settings checkbox turns it off"
        );
        assert_eq!(
            app.readout.status, "読み上げを停止しました。",
            "and stops the voice like every other switch"
        );
    }

    #[test]
    fn readout_stays_out_of_the_app_until_its_setting_is_turned_on() {
        const CONTROLS: [&str; 5] = [
            "自動読み上げ",
            "読み上げ",
            "読み上げ未準備",
            "読み上げ準備中",
            "読み上げを停止",
        ];
        let shown = |app: &mut App, width, height| {
            drawn_galleys(app, width, height)
                .into_iter()
                .map(|(text, _)| text)
                .filter(|text| CONTROLS.contains(&text.as_str()))
                .collect::<Vec<_>>()
        };
        for (tab, page, _) in PAGE_KEYS {
            let mut app = App::inert();
            app.tab = tab;
            app.readout.ready = Some(ready_voice());
            app.settings.auto_read = true;
            app.rows.push(row(1, false, false));
            assert_eq!(
                shown(&mut app, 1280.0, 880.0),
                Vec::<String>::new(),
                "{page}"
            );
            app.settings.readout_enabled = true;
            assert!(
                !shown(&mut app, 1280.0, 880.0).is_empty(),
                "{page} shows readout once it is on"
            );
        }
        let mut app = App::inert();
        app.readout.ready = Some(ready_voice());
        app.window_mode.enter(&egui::ViewportInfo::default());
        assert_eq!(shown(&mut app, 400.0, 260.0), Vec::<String>::new(), "mini");
        app.settings.readout_enabled = true;
        assert!(shown(&mut app, 400.0, 260.0).contains(&"自動読み上げ".to_owned()));

        let mut app = App::inert();
        app.readout.ready = Some(ready_voice());
        app.settings.auto_read = true;
        // A non-Japanese target turns an attempt to read into an error message
        // instead of a voice, so the attempt itself can be seen.
        app.settings.translation.target_lang = "en".into();
        app.auto_read_translation("A finished translation.".into(), true);
        assert!(
            app.readout.error.is_empty(),
            "nothing is read while it is off"
        );
        app.settings.readout_enabled = true;
        app.auto_read_translation("A finished translation.".into(), true);
        assert!(
            !app.readout.error.is_empty(),
            "the same text is read once it is on"
        );

        let mut app = App::inert();
        app.tab = Tab::Settings;
        click_label(&mut app, "読み上げを使う", 1280.0, 880.0);
        assert!(
            app.settings.readout_enabled,
            "the settings switch turns it on"
        );
        click_label(&mut app, "読み上げを使う", 1280.0, 880.0);
        assert!(!app.settings.readout_enabled);
        assert_eq!(
            app.readout.status, "読み上げを停止しました。",
            "turning it off stops the voice"
        );
    }
}
