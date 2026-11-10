//! The mini view: the chat in a small window, with the main window's words
//! and its one listen button. A reading or a text translation asked for here
//! puts its result on a card in the chat's place, because the 文章 screen is
//! out of sight.
use super::theme::{self, ACCENT, ERROR, SURFACE, TEXT, TEXT_MUTED};
use super::*;
use plamo_translate::credit;

/// Names the 「⋯」 menu, so that a preview or a test can open it.
pub(super) const MENU: &str = "mini-menu";

/// Around everything in the window.
const MARGIN: f32 = 6.0;
/// The least size of the listen button in the toolbar.
const LISTEN_BUTTON: Vec2 = Vec2::new(88.0, theme::CONTROL_H);
/// Height of the row under the chat: one line of small text.
const STATUS_ROW: f32 = 20.0;

/// What takes the room between the toolbar and the status row.
#[derive(Clone, Copy, Default, PartialEq, Eq)]
enum Showing {
    #[default]
    Chat,
    /// The result of a reading or a text translation.
    Card,
}

/// What the mini view remembers. Everything else it shows is worked out from
/// the app's state on each frame.
#[derive(Default)]
pub(super) struct MiniState {
    showing: Showing,
    /// Why the last reading or text translation failed, in the words the
    /// status row was given.
    failure: Option<String>,
}

impl MiniState {
    /// A text translation, or the choosing of a range to read, begins. The
    /// failure before it is over.
    pub(super) fn forget_failure(&mut self) {
        self.failure = None;
    }

    pub(super) fn shows_card(&self) -> bool {
        self.showing == Showing::Card
    }
}

/// What the card says. It comes from the app's state on each frame, so it
/// cannot show the result of one request as the result of another.
#[derive(PartialEq, Eq)]
enum Card<'a> {
    /// A picture is being read, or waits to be. The result before it is not
    /// shown, because it would look like the new one.
    Reading,
    /// A translation in flight, as far as it has come.
    Translating(&'a str),
    Failed(&'a str),
    Translated(&'a str),
    Empty,
}

/// One row of controls from the right edge, exactly one control tall. With
/// the app's usual padding a button is taller than that.
fn control_row(ui: &mut egui::Ui, id: &'static str, add: impl FnOnce(&mut egui::Ui)) {
    ui::live_region(ui, id, theme::CONTROL_H, |ui| {
        ui.spacing_mut().button_padding.y = theme::SPACE_1;
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), add);
    });
}

impl App {
    /// A picture or a range was given to be read. The failure before it is
    /// over, and the mini view shows the card for what comes of it.
    pub(super) fn reading_given(&mut self) {
        self.mini.forget_failure();
        self.open_card();
    }

    /// A reading or a text translation failed, and `self.status` has just
    /// been told why.
    pub(super) fn result_failed(&mut self) {
        self.mini.failure = Some(self.status.clone());
        self.open_card();
    }

    /// A text translation is done. The mini view shows it, also when the
    /// reader went back to the chat while it was coming: what was asked for
    /// is not left out of sight.
    pub(super) fn result_ready(&mut self) {
        self.open_card();
    }

    /// The card is the mini view's. The main window shows the same result on
    /// its 文章 screen.
    fn open_card(&mut self) {
        if self.window_mode.is_mini() {
            self.mini.showing = Showing::Card;
        }
    }

    /// What the mini view opens on. The 文章 screen brings its result when it
    /// has one to show, and every other screen opens the chat.
    pub(super) fn open_mini(&mut self) {
        let result = self.tab == Tab::Text && self.card() != Card::Empty;
        self.mini.showing = if result { Showing::Card } else { Showing::Chat };
    }

    fn card(&self) -> Card<'_> {
        if self.reading_picture() {
            Card::Reading
        } else if self.manual_busy.is_some() {
            Card::Translating(&self.output)
        } else if let Some(why) = &self.mini.failure {
            Card::Failed(why)
        } else if !self.output.is_empty() {
            Card::Translated(&self.output)
        } else {
            Card::Empty
        }
    }

    pub(super) fn mini_ui(&mut self, ctx: &egui::Context) {
        egui::CentralPanel::default()
            .frame(egui::Frame::new().inner_margin(MARGIN).fill(theme::CANVAS))
            .show(ctx, |ui| {
                ui.spacing_mut().item_spacing = egui::vec2(MARGIN, theme::SPACE_1);
                self.toolbar(ui);
                // The status row has its room kept first, so nothing it says
                // can move or shrink what is above it.
                let room = egui::vec2(
                    ui.available_width(),
                    (ui.available_height() - STATUS_ROW - ui.spacing().item_spacing.y).max(0.0),
                );
                ui.allocate_ui(room, |ui| match self.mini.showing {
                    Showing::Chat => self.live_chat_ui(ui),
                    Showing::Card => self.card_ui(ui),
                });
                self.status_row(ui);
            });
    }

    fn toolbar(&mut self, ui: &mut egui::Ui) {
        control_row(ui, "mini-toolbar", |ui| {
            self.menu(ui);
            ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                if ui
                    .add(ui::standard("通常表示"))
                    .on_hover_text("通常表示に戻る（Esc）")
                    .clicked()
                {
                    self.restore_normal_view(ui.ctx());
                }
                if self.listen_button(ui, LISTEN_BUTTON).clicked() {
                    self.toggle_listening(ui.ctx());
                }
                if self.settings.readout_enabled
                    && ui::auto_read_switch(ui, self.settings.auto_read).clicked()
                {
                    self.toggle_auto_read(ui.ctx());
                }
            });
        });
    }

    /// The 「⋯」 menu holds what the chat's own menu and header offer that a
    /// small window over other work has a use for.
    fn menu(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        let id = egui::Id::new(MENU);
        let open = egui::Popup::is_id_open(&ctx, id);
        let button = ui
            .add(ui::standard("⋯").selected(open))
            .on_hover_text("その他");
        button.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, "その他"));
        let (mut input, mut range) = (None, false);
        egui::Popup::menu(&button).id(id).show(|ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            ui.spacing_mut().button_padding.x = theme::SPACE_2;
            input = self.input_menu(ui);
            self.display_menu(ui);
            let mut top = self.window_mode.always_on_top();
            if ui::toggle(ui, &mut top, "最前面").changed() {
                for command in self.window_mode.set_always_on_top(top) {
                    ctx.send_viewport_cmd(command);
                }
            }
            if ui::COPY_ALL.row(ui, self.can_copy_talk()) {
                self.copy_talk();
            }
            ui.separator();
            range = ui::RANGE.row(ui, self.can_read_range());
            self.screen_menu(ui);
            if ui::SUBTITLE_WINDOW.row(ui, true) {
                self.subtitle_window.open = true;
            }
        });
        // As in the main window, an input is switched and a range is asked
        // for once the menu has been drawn.
        if let Some(mode) = input {
            self.choose_input(&ctx, mode);
        }
        if range {
            self.capture(&ctx);
        }
    }

    /// The compact counterpart of the 文章 screen's 訳文 card.
    fn card_ui(&mut self, ui: &mut egui::Ui) {
        let (text, color, done) = match self.card() {
            Card::Reading => (screen_ocr::READING, TEXT_MUTED, false),
            Card::Translating("") => ("翻訳中", TEXT_MUTED, false),
            Card::Translating(so_far) => (so_far, TEXT, false),
            Card::Failed(why) => (why, ERROR, false),
            Card::Translated(text) => (text, TEXT, true),
            Card::Empty => (ui::TRANSLATION_HINT, TEXT_MUTED, false),
        };
        let (rect, _) = ui.allocate_exact_size(ui.available_size(), egui::Sense::hover());
        ui.painter().rect(
            rect,
            theme::RADIUS_CARD,
            SURFACE,
            theme::hairline(),
            egui::StrokeKind::Inside,
        );
        let mut card = ui.new_child(
            egui::UiBuilder::new()
                .id_salt("mini-card")
                .max_rect(rect.shrink(theme::SPACE_2))
                .layout(egui::Layout::top_down(egui::Align::Min)),
        );
        card.set_clip_rect(rect.intersect(ui.clip_rect()));
        let (mut back, mut copy) = (false, false);
        control_row(&mut card, "header", |ui| {
            // Only a result that is on the card can be copied from it.
            copy = ui::COPY.button(ui, done).clicked();
            ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                back = ui.add(ui::standard(ui::BACK_TO_CHAT)).clicked();
                ui.label(
                    RichText::new(ui::TRANSLATION)
                        .size(theme::SIZE_LABEL)
                        .strong()
                        .color(ACCENT),
                );
            });
        });
        let size = self.settings.font_size;
        let line = card
            .fonts_mut(|fonts| {
                fonts.row_height(&egui::FontId::proportional(theme::reading_size(size)))
            })
            .ceil();
        egui::ScrollArea::vertical()
            .id_salt("body")
            .auto_shrink([false, false])
            .show_viewport(&mut card, |ui, viewport| {
                // A line cut in half would hang at the bottom edge of the
                // smallest window, so only whole lines are in view.
                let seen = viewport.translate(ui.max_rect().min.to_vec2());
                let lines = (seen.height() / line).floor().max(1.0);
                ui.set_clip_rect(ui.clip_rect().intersect(egui::Rect::from_min_size(
                    seen.min,
                    egui::vec2(seen.width(), lines * line),
                )));
                ui.add(
                    egui::Label::new(
                        theme::reading(text, size)
                            .color(color)
                            .line_height(Some(line)),
                    )
                    .wrap()
                    .selectable(true),
                );
            });
        if back {
            self.mini.showing = Showing::Chat;
        }
        if copy {
            ui.ctx().copy_text(credit::published(&self.output));
        }
    }

    /// One line. A failure to save comes first, then what the line beside the
    /// 聞く button says in the main window, then a failure to read aloud.
    fn status_row(&self, ui: &mut egui::Ui) {
        ui::live_region(ui, "mini-status", STATUS_ROW, |ui| {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(RichText::new(credit::LABEL).size(9.0).color(TEXT_MUTED));
                ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                    let unsaved = self.unsaved();
                    ui::status_message(
                        ui,
                        &[
                            (unsaved.as_str(), ERROR),
                            self.listen_state(),
                            (self.readout.error.as_str(), ERROR),
                        ],
                    );
                });
            });
        });
    }
}
