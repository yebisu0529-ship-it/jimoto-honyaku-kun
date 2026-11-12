//! A messenger-style transcript of what was said. The live page and the
//! separate subtitle window both draw through this one component.

use std::{
    borrow::Cow,
    collections::{HashMap, HashSet},
    sync::Arc,
    time::Duration,
};

use chrono::{Datelike, NaiveDate, NaiveDateTime, TimeDelta};
use eframe::egui::{
    self, Color32, CornerRadius, FontId, Galley, PointerButton, Pos2, Rect, Sense, Vec2,
    text::CCursor,
    text_selection::{CCursorRange, visuals::paint_text_selection},
};
use plamo_translate::credit;

use super::theme;

/// Stands for speech still being recognised: a bubble of dots without text.
/// It is not a message, so it never counts as a new one.
pub const TYPING: u64 = u64::MAX;

const AVATAR: f32 = 36.0;
/// Between the avatar and the bubble. The tail of the first bubble lives here.
const AVATAR_GAP: f32 = 12.0;
const SIDE: f32 = 8.0;
const END: f32 = 6.0;
const EDGE: f32 = 12.0;
const BUBBLE_X: f32 = SIDE + AVATAR + AVATAR_GAP;
const BUBBLE_RADIUS: u8 = 16;
const TAIL_RADIUS: u8 = 3;
const TAIL: f32 = 8.0;
const MARGIN_X: f32 = 12.0;
const MARGIN_Y: f32 = 9.0;
const STACK_GAP: f32 = 4.0;
const GROUP_GAP: f32 = 12.0;
const DAY_GAP: f32 = 8.0;
const NAME_H: f32 = 18.0;
const TIME_GAP: f32 = 4.0;
const WIDTH_SHARE: f32 = 0.78;
const MIN_WIDTH: f32 = 240.0;
const MIN_ROOM: f32 = 120.0;
const ORIGINAL_GAP: f32 = 8.0;
const DOT_RADIUS: f32 = 3.0;
const DOT_GAP: f32 = 5.0;
const DOTS_WIDTH: f32 = 6.0 * DOT_RADIUS + 2.0 * DOT_GAP;
const DOTS_HEIGHT: f32 = 2.0 * DOT_RADIUS;
const PANEL_INSET: f32 = 6.0;
const PILL_X: f32 = 12.0;
const PILL_Y: f32 = 4.0;
const NEW_PILL_X: f32 = 16.0;
const NEW_PILL_Y: f32 = 7.0;
const NEW_PILL_LIFT: f32 = 14.0;
/// The most characters a bubble takes from a finished paragraph. A bubble holds
/// whole sentences, so one longer sentence is a bubble of its own.
const BUBBLE_CHARS: usize = 60;
/// A drag selecting text scrolls the chat once the pointer is this close to
/// its top or bottom edge, or past it.
const SCROLL_ZONE: f32 = 24.0;
/// Points a second the chat scrolls for each point the pointer is into that zone.
const SCROLL_RATE: f32 = 12.0;
const SCROLL_MAX: f32 = 1600.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    YouTube,
    PcAudio,
    Screen,
}

impl Source {
    /// The kind a saved record carries. Other kinds are not part of the chat.
    pub fn from_kind(kind: &str) -> Option<Self> {
        match kind {
            "YouTube" => Some(Self::YouTube),
            "音声" => Some(Self::PcAudio),
            "画面" => Some(Self::Screen),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::YouTube => "YouTube",
            Self::PcAudio => "PC音声",
            Self::Screen => "画面",
        }
    }

    fn glyph(self) -> &'static str {
        match self {
            Self::YouTube => "YT",
            Self::PcAudio => "音",
            Self::Screen => "画",
        }
    }

    fn color(self) -> Color32 {
        match self {
            Self::YouTube => theme::CHAT_AVATAR_YOUTUBE,
            Self::PcAudio => theme::CHAT_AVATAR_AUDIO,
            Self::Screen => theme::CHAT_AVATAR_SCREEN,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Fixed,
    /// Its translation is still being revised.
    Forming,
    Interrupted,
    Failed,
}

/// When a message was said: the clock time it arrived and, for YouTube, the
/// position in the video.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Moment {
    pub sent: Option<NaiveDateTime>,
    pub video_ms: Option<u64>,
}

impl Moment {
    /// `created_at` is written as local time, so its clock reads as the
    /// reader's own clock did when the message came in.
    pub fn from_record(created_at: &str, start_ms: u64, source: Source) -> Self {
        Self {
            sent: chrono::DateTime::parse_from_rfc3339(created_at)
                .ok()
                .map(|time| time.naive_local()),
            video_ms: (source == Source::YouTube).then_some(start_ms),
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Message<'a> {
    /// Rises with every new message. It identifies the message across frames.
    pub id: u64,
    pub source: Source,
    pub target: &'a str,
    pub original: &'a str,
    pub moment: Moment,
    pub status: Status,
    /// Why a failed message failed. Shown when the note under it is hovered.
    pub failure: Option<&'a str>,
}

impl<'a> Message<'a> {
    pub fn typing(source: Source, original: &'a str) -> Self {
        Message {
            id: TYPING,
            source,
            target: "",
            original,
            moment: Moment::default(),
            status: Status::Forming,
            failure: None,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Options {
    pub text_size: f32,
    pub show_originals: bool,
    /// Stay with the newest message while the reader is at the bottom.
    pub follow: bool,
    /// Some(speaking) while readout is in use. None keeps its commands out.
    pub readout: Option<bool>,
    /// What the chat says while it has no message: what it is for, or what
    /// to do next.
    pub empty: &'static str,
    /// A bubble that starts above the top while the chat follows, older or
    /// newest, is drawn from its first whole line while a line of its text is
    /// whole in view, and keeps its room empty otherwise. In a small window the
    /// room an older bubble kept was most of the chat, and an empty strip of
    /// bubble reads as an empty message, dots alone as speech still being
    /// recognised. Without it, only the newest message is drawn there.
    pub cut_from_a_whole_line: bool,
}

pub enum Action {
    Read(String),
    StopReadout,
}

/// Which of a speaker's texts a place is in. A translation is read before its
/// original.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Field {
    Translation,
    Original,
}

/// A place between two characters of what the chat shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Spot {
    /// The message and the speaker's part of it.
    part: (u64, usize),
    field: Field,
    /// Characters before the place in the field's text.
    char: usize,
}

/// A place in reading order: where its part comes in the chat, then the field
/// and the character.
type Rank = (usize, Field, usize);

/// Text the reader has dragged over. It stays while its messages are in the
/// chat, scrolled out of sight or not.
#[derive(Clone, Copy, Debug)]
struct Selection {
    /// Where the drag began.
    anchor: Spot,
    /// Where the drag has got to.
    focus: Spot,
    /// The button is still held.
    dragging: bool,
}

impl Selection {
    /// Its two ends in reading order, or None when a message it touches is
    /// no longer in the chat.
    fn ends(&self, order: &HashMap<(u64, usize), usize>) -> Option<(Rank, Rank)> {
        let rank = |spot: Spot| order.get(&spot.part).map(|at| (*at, spot.field, spot.char));
        let (a, b) = (rank(self.anchor)?, rank(self.focus)?);
        Some(if a <= b { (a, b) } else { (b, a) })
    }
}

/// A text drawn this frame that a drag can select.
#[derive(Clone, Debug)]
struct Hit {
    /// The message, the speaker's part and the bubble of the part.
    bubble: (u64, usize, usize),
    galley: Arc<Galley>,
    /// Where the galley was drawn.
    at: Pos2,
    field: Field,
    /// Characters of the field's text before the galley's first.
    start: usize,
}

/// Where a bubble's part comes in the chat, and the selection to show.
#[derive(Clone, Copy)]
struct Place {
    at: usize,
    ends: Option<(Rank, Rank)>,
}

/// What the reader has done with the transcript, kept between frames.
#[derive(Clone, Debug)]
pub struct State {
    /// Bubbles whose original is shown the other way round from `all_originals`.
    flipped: HashSet<(u64, usize)>,
    all_originals: bool,
    /// The follow option of the previous frame, to see it being turned on.
    follow_was_on: bool,
    /// The reader was at the bottom on the previous frame.
    at_end: bool,
    /// The newest message the reader has been shown.
    seen: u64,
    content_height: f32,
    start_at_top: bool,
    jump_to_end: bool,
    selection: Option<Selection>,
    /// The selectable texts drawn this frame, where a drag finds its place.
    hits: Vec<Hit>,
    /// The menu asked for the selection to be copied.
    copy_selection: bool,
    /// A menu was open when the chat was last drawn, so an Esc now is the
    /// menu's. egui closes a menu on Esc but leaves the key to the rest of the
    /// frame, and a menu drawn before the chat has closed by the time the chat
    /// reads the key.
    menu_was_open: bool,
    #[cfg(test)]
    panel: Option<Rect>,
    #[cfg(test)]
    body: Option<Rect>,
    #[cfg(test)]
    offset: f32,
    /// What the reader copied. Tests read it instead of the system clipboard.
    #[cfg(test)]
    copied: Option<String>,
}

impl Default for State {
    fn default() -> Self {
        Self {
            flipped: HashSet::new(),
            all_originals: false,
            follow_was_on: true,
            at_end: true,
            seen: 0,
            content_height: 0.0,
            start_at_top: false,
            jump_to_end: false,
            selection: None,
            hits: Vec::new(),
            copy_selection: false,
            menu_was_open: false,
            #[cfg(test)]
            panel: None,
            #[cfg(test)]
            body: None,
            #[cfg(test)]
            offset: 0.0,
            #[cfg(test)]
            copied: None,
        }
    }
}

impl State {
    /// A reader at the start of the transcript who has been shown messages up
    /// to `seen` only.
    pub fn reading_from_start(&mut self, seen: u64) {
        self.at_end = false;
        self.seen = seen;
        self.start_at_top = true;
    }

    /// Show, or hide again, the original of one bubble of a message.
    pub fn toggle_original(&mut self, id: u64, part: usize) {
        if !self.flipped.remove(&(id, part)) {
            self.flipped.insert((id, part));
        }
    }

    /// Forget everything about the reader, as when a new talk begins.
    pub fn clear(&mut self) {
        *self = Self::default();
    }

    /// Take the reader to the newest message, wherever they had scrolled to.
    pub fn go_to_end(&mut self) {
        self.jump_to_end = true;
        self.at_end = true;
    }

    /// The reader was at the newest message when the chat was last drawn.
    pub fn is_at_end(&self) -> bool {
        self.at_end
    }

    /// Text is selected, so the next Esc is for letting it go.
    pub fn has_selection(&self) -> bool {
        self.selection
            .is_some_and(|selection| selection.anchor != selection.focus)
    }

    /// Copies all the chat shows, from the first message to the last, with
    /// the credit.
    pub fn copy_all(&mut self, messages: &[Message<'_>]) {
        let entries = plan(messages);
        let text = self.text(&entries, None);
        if !text.is_empty() {
            self.copy(&text);
        }
    }

    #[cfg(test)]
    pub fn copied(&self) -> Option<&str> {
        self.copied.as_deref()
    }

    #[cfg(test)]
    pub fn panel(&self) -> Option<Rect> {
        self.panel
    }

    #[cfg(test)]
    pub fn body(&self) -> Option<Rect> {
        self.body
    }

    #[cfg(test)]
    pub fn offset(&self) -> f32 {
        self.offset
    }

    /// Draws into all the room `ui` has left.
    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        id: &str,
        messages: &[Message<'_>],
        options: Options,
    ) -> Option<Action> {
        self.follow_options(options);
        let (panel, _) = ui.allocate_exact_size(ui.available_size(), Sense::hover());
        #[cfg(test)]
        {
            self.panel = Some(panel);
        }
        ui.painter().rect_filled(
            panel,
            CornerRadius::same(theme::RADIUS_CARD),
            theme::CHAT_BACKGROUND,
        );
        let inner = panel.shrink(PANEL_INSET);
        if messages.is_empty() {
            self.at_end = true;
            self.seen = 0;
            self.flipped.clear();
            self.selection = None;
            empty_hint(ui, inner, options.empty);
            return None;
        }
        let mut body = ui.new_child(
            egui::UiBuilder::new()
                .id_salt(id)
                .max_rect(inner)
                .layout(egui::Layout::top_down(egui::Align::Min)),
        );
        body.set_clip_rect(inner.intersect(ui.clip_rect()));
        // The app's scroll bar is dark on dark. Here it sits on a light canvas.
        let visuals = &mut body.style_mut().visuals;
        visuals.extreme_bg_color = theme::CHAT_TRACK;
        visuals.widgets.inactive.bg_fill = theme::CHAT_PILL;
        visuals.widgets.hovered.bg_fill = theme::CHAT_PILL_HOVER;
        visuals.widgets.active.bg_fill = theme::CHAT_META;
        // While text is being selected, the words under the pointer stay put.
        let selecting = self.selection.is_some_and(|selection| selection.dragging);
        let following = options.follow && self.at_end && !selecting;
        let mut area = egui::ScrollArea::vertical()
            .id_salt("scroll")
            .auto_shrink([false, false])
            .animated(false)
            .stick_to_bottom(following);
        if std::mem::take(&mut self.start_at_top) {
            area = area.vertical_scroll_offset(0.0);
        }
        let output = area.show_viewport(&mut body, |ui, viewport| {
            self.draw(ui, viewport, messages, options, following)
        });
        #[cfg(test)]
        {
            self.body = Some(output.inner_rect);
            self.offset = output.state.offset.y;
        }
        self.at_end =
            output.state.offset.y + output.inner_rect.height() + 0.5 >= output.content_size.y;
        // The scroll area moves to the new end after this frame has been laid
        // out, so the next frame is wanted at once.
        if following && (output.content_size.y - self.content_height).abs() > 0.5 {
            ui.ctx().request_repaint();
        }
        self.content_height = output.content_size.y;
        let newest = messages
            .iter()
            .filter(|message| message.id != TYPING)
            .map(|message| message.id)
            .max()
            .unwrap_or(0);
        if self.at_end {
            self.seen = newest;
        }
        let unseen = messages
            .iter()
            .filter(|message| message.id != TYPING && message.id > self.seen)
            .count();
        if unseen > 0 && !self.at_end {
            self.new_messages_pill(ui, id, output.inner_rect, unseen);
        }
        let (copy_keys, escape) = ui.input(|input| {
            (
                input
                    .events
                    .iter()
                    .any(|event| matches!(event, egui::Event::Copy)),
                input.key_pressed(egui::Key::Escape),
            )
        });
        let copy_keys = copy_keys && !ui.ctx().wants_keyboard_input();
        if std::mem::take(&mut self.copy_selection) || copy_keys {
            self.copy_selected(messages);
        }
        if escape && !self.menu_was_open {
            self.selection = None;
        }
        self.menu_was_open = egui::Popup::is_any_open(ui.ctx());
        output.inner
    }

    fn copy_selected(&mut self, messages: &[Message<'_>]) {
        let entries = plan(messages);
        let Some(ends) = self
            .selection
            .and_then(|selection| selection.ends(&part_order(&entries)))
        else {
            return;
        };
        let text = self.text(&entries, Some(ends));
        if !text.is_empty() {
            self.copy(&text);
        }
    }

    /// What the chat shows between two places, or all of it: each speaker's
    /// part in turn, its translation and, when shown, its original. Speech
    /// still being recognised and the chat's own labels are left out.
    fn text(&self, entries: &[Entry<'_>], ends: Option<(Rank, Rank)>) -> String {
        let order = part_order(entries);
        let mut text = String::new();
        let mut last = None;
        for entry in entries {
            let Entry::Bubble(bubble) = entry else {
                continue;
            };
            if bubble.message.id == TYPING || !bubble.last_in_part {
                continue;
            }
            let part = (bubble.message.id, bubble.part);
            let fields = if shows_original_as_body(bubble) {
                [None, Some((Field::Original, bubble.original.as_ref()))]
            } else {
                [
                    (!bubble.whole.is_empty()).then_some((Field::Translation, bubble.whole)),
                    self.expanded(bubble)
                        .then_some((Field::Original, bubble.original.as_ref())),
                ]
            };
            for (field, words) in fields.into_iter().flatten() {
                let length = words.chars().count();
                let (from, to) = match ends {
                    None => (0, length),
                    Some(_) => match covered(ends, order[&part], (field, 0), length) {
                        Some(range) => range,
                        None => continue,
                    },
                };
                let piece = char_slice(words, from, to).trim();
                if piece.is_empty() {
                    continue;
                }
                if let Some(before) = last {
                    text.push_str(if before == part { "\n" } else { "\n\n" });
                }
                text.push_str(piece);
                last = Some(part);
            }
        }
        text
    }

    /// The original of `bubble`'s part is shown.
    fn expanded(&self, bubble: &Bubble<'_>) -> bool {
        self.all_originals != self.flipped.contains(&(bubble.message.id, bubble.part))
            && !bubble.original.is_empty()
    }

    fn follow_options(&mut self, options: Options) {
        if self.all_originals != options.show_originals {
            self.all_originals = options.show_originals;
            self.flipped.clear();
        }
        if options.follow && !self.follow_was_on {
            self.go_to_end();
        }
        self.follow_was_on = options.follow;
    }

    fn new_messages_pill(&mut self, ui: &egui::Ui, id: &str, view: Rect, count: usize) {
        let galley = plain(
            ui,
            &format!("↓ 新しいメッセージ {count}件"),
            theme::SIZE_LABEL,
            theme::CHAT_PILL_TEXT,
        );
        let size = galley.size() + egui::vec2(2.0 * NEW_PILL_X, 2.0 * NEW_PILL_Y);
        let pill = Rect::from_center_size(
            egui::pos2(
                view.center().x,
                view.bottom() - NEW_PILL_LIFT - size.y / 2.0,
            ),
            size,
        );
        let response = ui.interact(pill, ui.id().with((id, "new-messages")), Sense::click());
        let fill = if response.hovered() {
            theme::CHAT_PILL_HOVER
        } else {
            theme::CHAT_PILL
        };
        ui.painter().rect_filled(pill, pill_radius(size), fill);
        ui.painter().galley(
            pill.center() - galley.size() / 2.0,
            galley,
            theme::CHAT_PILL_TEXT,
        );
        focus_ring(ui, &response, pill_radius(size));
        response.widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::Button, true, "新しいメッセージへ移動")
        });
        if response
            .on_hover_cursor(egui::CursorIcon::PointingHand)
            .clicked()
        {
            self.go_to_end();
            ui.ctx().request_repaint();
        }
    }

    fn draw(
        &mut self,
        ui: &mut egui::Ui,
        viewport: Rect,
        messages: &[Message<'_>],
        options: Options,
        following: bool,
    ) -> Option<Action> {
        ui.spacing_mut().item_spacing = Vec2::ZERO;
        let metrics = Metrics::new(ui, options.text_size);
        let visible_top = ui.max_rect().top() + viewport.top();
        // While the reader is at the end, a bubble that starts above the top
        // would show only its lower part. It keeps its space but is not drawn,
        // except the newest message, which is drawn even when it is taller
        // than the window. When the options ask for it, the rule is instead
        // that such a bubble, newest or not, is drawn while a line of its text
        // is whole in view. It is drawn without the avatar, the name and the
        // line that would be cut.
        let whole = |rect: Rect| !following || rect.top() >= visible_top - 0.5;
        // paint_lines leaves out the lines above the painter's clip, which at
        // a fractional scale can be a fraction of a pixel off `visible_top`.
        let clip_top = ui.clip_rect().top();
        let mut action = None;
        let mut after_day = false;
        let entries = plan(messages);
        let order = part_order(&entries);
        let ends = self.selection.and_then(|selection| selection.ends(&order));
        if ends.is_none() {
            self.selection = None;
        }
        self.hits.clear();
        ui.add_space(EDGE);
        for (index, entry) in entries.into_iter().enumerate() {
            match entry {
                Entry::Day(date) => {
                    if index > 0 {
                        ui.add_space(GROUP_GAP);
                    }
                    let galley = plain(
                        ui,
                        &day_text(date),
                        theme::SIZE_CAPTION,
                        theme::CHAT_PILL_TEXT,
                    );
                    let size = galley.size() + egui::vec2(2.0 * PILL_X, 2.0 * PILL_Y);
                    let (_, rect) = ui.allocate_space(egui::vec2(metrics.width, size.y));
                    if ui.is_rect_visible(rect) && whole(rect) {
                        let pill = Rect::from_center_size(rect.center(), size);
                        ui.painter()
                            .rect_filled(pill, pill_radius(size), theme::CHAT_PILL);
                        ui.painter().galley(
                            pill.center() - galley.size() / 2.0,
                            galley,
                            theme::CHAT_PILL_TEXT,
                        );
                    }
                    after_day = true;
                }
                Entry::Bubble(bubble) => {
                    ui.add_space(match (index, bubble.first_in_group, after_day) {
                        (0, _, _) => 0.0,
                        (_, true, true) => DAY_GAP,
                        (_, true, false) => GROUP_GAP,
                        _ => STACK_GAP,
                    });
                    after_day = false;
                    let laid = lay_out(ui, &bubble, self.expanded(&bubble), &metrics);
                    let header = if bubble.first_in_group { NAME_H } else { 0.0 };
                    let (_, rect) = ui.allocate_space(egui::vec2(
                        metrics.width,
                        header + laid.size.y + laid.below,
                    ));
                    // While the chat follows, a bubble cut at the top ends in
                    // view, so its last line is whole when any of its lines is.
                    let line_in_view = laid
                        .last_line(rect.top() + header + MARGIN_Y)
                        .is_some_and(|line| line >= clip_top - 0.5);
                    let shown_cut = if options.cut_from_a_whole_line {
                        line_in_view
                    } else {
                        bubble.newest
                    };
                    if ui.is_rect_visible(rect) && (whole(rect) || shown_cut) {
                        let from_top = whole(rect);
                        let place = Place {
                            at: order[&(bubble.message.id, bubble.part)],
                            ends,
                        };
                        let reply =
                            self.paint_bubble(ui, rect, (&bubble, from_top), &laid, options, place);
                        action = action.or(reply);
                    }
                }
            }
        }
        ui.add_space(EDGE);
        if std::mem::take(&mut self.jump_to_end) {
            ui.scroll_to_cursor(Some(egui::Align::BOTTOM));
        }
        self.drag_selection(ui);
        action
    }

    /// Moves the end of a selection being dragged to the pointer, and scrolls
    /// while the pointer is at the top or bottom of the chat or past it. A
    /// click on the chat around the bubbles lets the selection go.
    fn drag_selection(&mut self, ui: &egui::Ui) {
        let view = ui.clip_rect();
        let (down, clicked, pointer, dt) = ui.input(|input| {
            (
                input.pointer.primary_down(),
                input.pointer.primary_clicked(),
                input.pointer.interact_pos(),
                input.stable_dt.min(0.1),
            )
        });
        if clicked
            && ui
                .ctx()
                .interaction_snapshot(|snapshot| snapshot.clicked.is_none())
            && pointer.is_some_and(|at| {
                view.contains(at) && ui.ctx().layer_id_at(at) == Some(ui.layer_id())
            })
        {
            self.selection = None;
        }
        let Some(selection) = self
            .selection
            .as_mut()
            .filter(|selection| selection.dragging)
        else {
            return;
        };
        if !down {
            selection.dragging = false;
            return;
        }
        let Some(pointer) = pointer else {
            return;
        };
        if let Some(spot) = spot_at(&self.hits, pointer) {
            selection.focus = spot;
        }
        ui.ctx().set_cursor_icon(egui::CursorIcon::Text);
        let into = (view.top() + SCROLL_ZONE - pointer.y).max(0.0)
            - (pointer.y - (view.bottom() - SCROLL_ZONE)).max(0.0);
        if into != 0.0 {
            let speed = (into * SCROLL_RATE).clamp(-SCROLL_MAX, SCROLL_MAX);
            ui.scroll_with_delta_animation(
                egui::vec2(0.0, speed * dt),
                egui::style::ScrollAnimation::none(),
            );
            ui.ctx().request_repaint();
        }
    }

    fn paint_bubble(
        &mut self,
        ui: &egui::Ui,
        item: Rect,
        (bubble, from_top): (&Bubble<'_>, bool),
        laid: &Laid,
        options: Options,
        place: Place,
    ) -> Option<Action> {
        let message = bubble.message;
        let piece = (message.id, bubble.part, bubble.piece);
        let top = item.top() + if bubble.first_in_group { NAME_H } else { 0.0 };
        let rect = Rect::from_min_size(egui::pos2(item.left() + BUBBLE_X, top), laid.size);
        let painter = ui.painter();
        let leads = bubble.first_in_group && from_top;
        if leads {
            let center = egui::pos2(item.left() + SIDE + AVATAR / 2.0, item.top() + AVATAR / 2.0);
            painter.circle_filled(center, AVATAR / 2.0, message.source.color());
            let glyph = plain(
                ui,
                message.source.glyph(),
                theme::SIZE_LABEL,
                theme::CHAT_AVATAR_GLYPH,
            );
            painter.galley(center - glyph.size() / 2.0, glyph, theme::CHAT_AVATAR_GLYPH);
            let name = plain(
                ui,
                message.source.name(),
                theme::SIZE_CAPTION,
                theme::CHAT_META,
            );
            painter.galley(egui::pos2(rect.left(), item.top()), name, theme::CHAT_META);
            painter.add(egui::Shape::convex_polygon(
                vec![
                    egui::pos2(rect.left() + TAIL_RADIUS as f32, rect.top()),
                    egui::pos2(rect.left() - TAIL, rect.top()),
                    egui::pos2(rect.left() + TAIL_RADIUS as f32, rect.top() + TAIL + 2.0),
                ],
                theme::CHAT_BUBBLE,
                egui::Stroke::NONE,
            ));
        }
        let radius = CornerRadius {
            nw: if leads { TAIL_RADIUS } else { BUBBLE_RADIUS },
            ..CornerRadius::same(BUBBLE_RADIUS)
        };
        painter.rect_filled(rect, radius, theme::CHAT_BUBBLE);
        let content = rect.min + egui::vec2(MARGIN_X, MARGIN_Y);
        if let Some(body) = &laid.body {
            let galley = match body_field(bubble) {
                Some((field, start)) => self.selectable(
                    ui,
                    Hit {
                        bubble: piece,
                        galley: body.clone(),
                        at: content,
                        field,
                        start,
                    },
                    place,
                ),
                None => body.clone(),
            };
            paint_lines(painter, from_top, content, galley, theme::CHAT_TEXT);
        }
        if let Some(dots) = laid.dots {
            paint_dots(ui, content + dots.to_vec2());
        }
        if let Some(original) = &laid.original {
            let inner = rect.width() - 2.0 * MARGIN_X;
            painter.rect_filled(
                Rect::from_min_size(
                    content + egui::vec2(0.0, original.rule),
                    egui::vec2(inner, 1.0),
                ),
                0,
                theme::CHAT_RULE,
            );
            let at = content + egui::vec2(0.0, original.text_y);
            let galley = self.selectable(
                ui,
                Hit {
                    bubble: piece,
                    galley: original.text.clone(),
                    at,
                    field: Field::Original,
                    start: 0,
                },
                place,
            );
            paint_lines(painter, from_top, at, galley, theme::CHAT_TEXT_MUTED);
            if let Some((video, y)) = &original.video {
                paint_lines(
                    painter,
                    from_top,
                    content + egui::vec2(0.0, *y),
                    video.clone(),
                    theme::CHAT_TEXT_MUTED,
                );
            }
        }
        if let Some(time) = &laid.time {
            let size = time.galley.size();
            let at = if time.beside {
                egui::pos2(rect.right() + TIME_GAP, rect.bottom() - size.y - 1.0)
            } else {
                egui::pos2(rect.right() - size.x, rect.bottom() + 1.0)
            };
            painter.galley(at, time.galley.clone(), theme::CHAT_META);
        }
        if let Some(note) = &laid.note {
            let at = egui::pos2(rect.left(), item.bottom() - note.size().y - 1.0);
            painter.galley(at, note.clone(), theme::CHAT_META);
            if let Some(failure) = message.failure.filter(|failure| !failure.is_empty()) {
                let area = Rect::from_min_size(at, note.size());
                ui.interact(area, ui.id().with(("failure", message.id)), Sense::hover())
                    .on_hover_text(failure);
            }
        }

        let key = (message.id, bubble.part);
        let expandable = !bubble.original.is_empty() && !shows_original_as_body(bubble);
        // On a touch screen a drag scrolls the chat instead.
        let sense = if ui.input(|input| input.has_touch_screen()) {
            Sense::click()
        } else {
            Sense::click_and_drag()
        };
        let response = ui.interact(rect, ui.id().with(("bubble", key, bubble.piece)), sense);
        focus_ring(ui, &response, radius);
        let label = bubble_display_text(bubble);
        response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, label));
        if response.drag_started_by(PointerButton::Primary)
            && let Some(origin) = ui.input(|input| input.pointer.press_origin())
            && let Some(spot) = spot_at(self.hits.iter().filter(|hit| hit.bubble == piece), origin)
        {
            self.selection = Some(Selection {
                anchor: spot,
                focus: spot,
                dragging: true,
            });
        }
        let selected = place.ends.is_some_and(|(low, high)| low < high);
        let mut action = None;
        response.context_menu(|ui| action = self.menu(ui, bubble, options, selected));
        if expandable && response.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        if response.clicked() {
            self.selection = None;
            if expandable {
                self.toggle_original(key.0, key.1);
            }
            ui.ctx().request_repaint();
        }
        action
    }

    /// Notes a text a drag can select, and returns it with the selection on it.
    fn selectable(&mut self, ui: &egui::Ui, hit: Hit, place: Place) -> Arc<Galley> {
        let mut galley = hit.galley.clone();
        if let Some((from, to)) = covered(
            place.ends,
            place.at,
            (hit.field, hit.start),
            galley.end().index,
        ) {
            let mut visuals = ui.visuals().clone();
            visuals.selection.bg_fill = theme::CHAT_SELECTION;
            visuals.selection.stroke.color = theme::CHAT_TEXT;
            paint_text_selection(
                &mut galley,
                &visuals,
                &CCursorRange::two(CCursor::new(from), CCursor::new(to)),
                None,
            );
        }
        self.hits.push(hit);
        galley
    }

    fn menu(
        &mut self,
        ui: &mut egui::Ui,
        bubble: &Bubble<'_>,
        options: Options,
        selected: bool,
    ) -> Option<Action> {
        let text = bubble.whole;
        let readable = bubble.message.status == Status::Fixed && !text.is_empty();
        let mut action = None;
        if selected
            && ui
                .button("選択範囲をコピー")
                .on_hover_text("ドラッグで選んだ文章を、PLaMoのクレジットを付けてコピーします")
                .clicked()
        {
            self.copy_selection = true;
            ui.close();
        }
        if ui
            .add_enabled(!text.is_empty(), egui::Button::new("コピー"))
            .on_hover_text("PLaMoのクレジットを付けてコピーします")
            .clicked()
        {
            self.copy(text);
            ui.close();
        }
        if let Some(speaking) = options.readout {
            if ui
                .add_enabled(readable, egui::Button::new("読み上げ"))
                .clicked()
            {
                action = Some(Action::Read(text.to_owned()));
                ui.close();
            }
            if speaking && ui.add(egui::Button::new("読み上げを停止")).clicked() {
                action = Some(Action::StopReadout);
                ui.close();
            }
        }
        action
    }

    fn copy(&mut self, text: &str) {
        let text = credit::published(text);
        #[cfg(test)]
        {
            self.copied = Some(text);
        }
        #[cfg(not(test))]
        if let Ok(mut clipboard) = arboard::Clipboard::new() {
            let _ = clipboard.set_text(text);
        }
    }
}

struct Metrics {
    width: f32,
    /// How wide text may run inside a bubble.
    wrap: f32,
    text: f32,
    line: f32,
    time_width: f32,
}

impl Metrics {
    fn new(ui: &egui::Ui, text: f32) -> Self {
        let width = ui.available_width();
        // A line wider than the readable measure is hard to follow, however
        // wide the window is.
        let bubble = (width * WIDTH_SHARE)
            .min(theme::prose_width(text))
            .max(MIN_WIDTH)
            .min(width - BUBBLE_X - END)
            .max(MIN_ROOM);
        Self {
            width,
            wrap: bubble - 2.0 * MARGIN_X,
            text,
            line: (text * 1.5).ceil(),
            time_width: plain(ui, "00:00", theme::SIZE_CAPTION, theme::CHAT_META)
                .size()
                .x,
        }
    }
}

/// Paints a text of a bubble at `at`. A bubble that is not drawn `from_top`
/// starts above the view and is drawn although the chat follows its end: it
/// has a whole line of text in view when the options ask for that, and is the
/// newest message otherwise. A line of it that the top of the view would cut
/// is left out, so the first line in view is a whole one.
fn paint_lines(
    painter: &egui::Painter,
    from_top: bool,
    at: Pos2,
    galley: Arc<Galley>,
    color: Color32,
) {
    if from_top {
        painter.galley(at, galley, color);
        return;
    }
    let view = painter.clip_rect();
    let top = galley
        .rows
        .iter()
        .map(|row| at.y + row.rect().top())
        .find(|top| *top >= view.top() - 0.5)
        .unwrap_or(view.bottom());
    painter
        .with_clip_rect(Rect::from_min_max(egui::pos2(view.left(), top), view.max))
        .galley(at, galley, color);
}

fn plain(ui: &egui::Ui, text: &str, size: f32, color: Color32) -> Arc<Galley> {
    ui.painter()
        .layout_no_wrap(text.to_owned(), FontId::proportional(size), color)
}

fn wrapped(
    ui: &egui::Ui,
    text: &str,
    (size, color): (f32, Color32),
    wrap: f32,
    line: Option<f32>,
) -> Arc<Galley> {
    let mut job =
        egui::text::LayoutJob::simple(text.to_owned(), FontId::proportional(size), color, wrap);
    job.sections[0].format.line_height = line;
    ui.painter().layout_job(job)
}

fn pill_radius(size: Vec2) -> CornerRadius {
    CornerRadius::same((size.y / 2.0).round() as u8)
}

fn focus_ring(ui: &egui::Ui, response: &egui::Response, radius: CornerRadius) {
    if response.has_focus() {
        ui.painter().rect_stroke(
            response.rect.expand(2.0),
            radius,
            egui::Stroke::new(2.0_f32, theme::CHAT_TEXT),
            egui::StrokeKind::Outside,
        );
    }
}

/// What an empty chat is for. A window that knows what to do next says that
/// as well.
pub const EMPTY_HINT: &str = "PCで再生中の英語の音声を、ここにやさしい日本語で表示します。";

fn empty_hint(ui: &egui::Ui, inner: Rect, text: &str) {
    let mut job = egui::text::LayoutJob::simple(
        text.to_owned(),
        FontId::proportional(theme::SIZE_BODY),
        theme::CHAT_META,
        (inner.width() - 48.0).max(MIN_ROOM),
    );
    job.halign = egui::Align::Center;
    let galley = ui.painter().layout_job(job);
    ui.painter().galley(
        egui::pos2(inner.center().x, inner.center().y - galley.size().y / 2.0),
        galley,
        theme::CHAT_META,
    );
}

fn paint_dots(ui: &egui::Ui, origin: Pos2) {
    let time = ui.input(|input| input.time);
    for dot in 0..3 {
        let wave =
            ((time * 1.6 - f64::from(dot) * 0.18) * std::f64::consts::TAU).sin() as f32 * 0.5 + 0.5;
        let center = origin
            + egui::vec2(
                DOT_RADIUS + dot as f32 * (2.0 * DOT_RADIUS + DOT_GAP),
                DOTS_HEIGHT / 2.0 - wave * 3.0,
            );
        ui.painter().circle_filled(
            center,
            DOT_RADIUS,
            theme::CHAT_TEXT_FORMING.gamma_multiply(0.45 + 0.55 * wave),
        );
    }
    ui.ctx().request_repaint_after(Duration::from_millis(33));
}

/// One bubble's content, measured.
struct Laid {
    body: Option<Arc<Galley>>,
    /// Where the dots start, inside the bubble's margins.
    dots: Option<Pos2>,
    original: Option<Original>,
    time: Option<Time>,
    note: Option<Arc<Galley>>,
    /// The bubble itself.
    size: Vec2,
    /// Height taken under the bubble by a time and a note.
    below: f32,
}

struct Original {
    rule: f32,
    text: Arc<Galley>,
    text_y: f32,
    video: Option<(Arc<Galley>, f32)>,
}

struct Time {
    galley: Arc<Galley>,
    /// Beside the bubble's bottom right, or under it when there is no room.
    beside: bool,
}

impl Laid {
    /// Where the last line of text starts when the content starts at `top`:
    /// the video time's, the original's or the body's. Dots are not text. It
    /// is added up as paint_lines adds it up, so both agree on a line at the
    /// edge of the view.
    fn last_line(&self, top: f32) -> Option<f32> {
        let (text, y) = match &self.original {
            Some(Original {
                video: Some((video, y)),
                ..
            }) => (video, *y),
            Some(original) => (&original.text, original.text_y),
            None => (self.body.as_ref()?, 0.0),
        };
        Some(top + y + text.rows.last()?.rect().top())
    }
}

fn shows_original_as_body(bubble: &Bubble<'_>) -> bool {
    bubble.message.status == Status::Forming
        && bubble.text.is_empty()
        && !bubble.original.is_empty()
}

fn bubble_display_text<'a>(bubble: &'a Bubble<'_>) -> &'a str {
    if shows_original_as_body(bubble) {
        bubble.original.as_ref()
    } else if bubble.message.status == Status::Forming {
        bubble.text
    } else if bubble.text.is_empty() {
        "訳文はありません。"
    } else {
        bubble.text
    }
}

/// The field a bubble's body shows and its first character's place there,
/// when the body can be selected: the translation, or the original while the
/// translation is still coming. Speech still being recognised and the note
/// for a missing translation cannot.
fn body_field(bubble: &Bubble<'_>) -> Option<(Field, usize)> {
    if bubble.message.id == TYPING {
        None
    } else if shows_original_as_body(bubble) {
        Some((Field::Original, 0))
    } else if bubble.text.is_empty() {
        None
    } else {
        Some((Field::Translation, bubble.start))
    }
}

/// Where each speaker's part comes in the chat, from the top.
fn part_order(entries: &[Entry<'_>]) -> HashMap<(u64, usize), usize> {
    let mut order = HashMap::new();
    for entry in entries {
        if let Entry::Bubble(bubble) = entry {
            let next = order.len();
            order
                .entry((bubble.message.id, bubble.part))
                .or_insert(next);
        }
    }
    order
}

/// The characters of a text that the selection covers, counted from the
/// text's first. The text is `length` characters of a field from `start`, in
/// the part that comes `at`-th.
fn covered(
    ends: Option<(Rank, Rank)>,
    at: usize,
    (field, start): (Field, usize),
    length: usize,
) -> Option<(usize, usize)> {
    let (low, high) = ends?;
    let from = low.max((at, field, start));
    let to = high.min((at, field, start + length));
    (from < to).then(|| (from.2 - start, to.2 - start))
}

/// The place in the drawn texts nearest the pointer, looking up and down first.
fn spot_at<'a>(hits: impl IntoIterator<Item = &'a Hit>, pointer: Pos2) -> Option<Spot> {
    let away = |hit: &Hit| {
        let rect = hit.galley.rect.translate(hit.at.to_vec2());
        let y = (rect.top() - pointer.y)
            .max(pointer.y - rect.bottom())
            .max(0.0);
        let x = (rect.left() - pointer.x)
            .max(pointer.x - rect.right())
            .max(0.0);
        (y, x)
    };
    let hit = hits.into_iter().min_by(|a, b| {
        let (a, b) = (away(a), away(b));
        a.0.total_cmp(&b.0).then(a.1.total_cmp(&b.1))
    })?;
    let cursor = hit.galley.cursor_from_pos(pointer - hit.at);
    Some(Spot {
        part: (hit.bubble.0, hit.bubble.1),
        field: hit.field,
        char: hit.start + cursor.index,
    })
}

fn char_slice(text: &str, from: usize, to: usize) -> &str {
    let byte = |chars: usize| {
        text.char_indices()
            .nth(chars)
            .map_or(text.len(), |(at, _)| at)
    };
    &text[byte(from)..byte(to)]
}

fn lay_out(ui: &egui::Ui, bubble: &Bubble<'_>, expanded: bool, metrics: &Metrics) -> Laid {
    let message = bubble.message;
    let forming = message.status == Status::Forming;
    let shows_original = shows_original_as_body(bubble);
    let text = bubble_display_text(bubble);
    let size = if shows_original {
        theme::SIZE_LABEL
    } else {
        metrics.text
    };
    let color = if forming {
        theme::CHAT_TEXT_FORMING
    } else if bubble.text.is_empty() {
        theme::CHAT_TEXT_MUTED
    } else {
        theme::CHAT_TEXT
    };
    let line = if shows_original {
        None
    } else {
        Some(metrics.line)
    };
    let body = (!text.is_empty()).then(|| wrapped(ui, text, (size, color), metrics.wrap, line));
    let mut width = body.as_ref().map_or(0.0, |body| body.size().x);
    let mut height = body.as_ref().map_or(0.0, |body| body.size().y);
    if body.as_ref().is_some_and(|body| body.rows.len() > 1) {
        width = metrics.wrap;
    }
    let mut dots = None;
    if forming {
        let at = match body.as_ref().and_then(|body| body.rows.last()) {
            Some(last) if last.rect().right() + DOT_GAP + DOTS_WIDTH <= metrics.wrap => egui::pos2(
                last.rect().right() + DOT_GAP,
                last.pos.y + (last.size.y - DOTS_HEIGHT) / 2.0,
            ),
            _ => {
                let at = egui::pos2(0.0, height + (metrics.line - DOTS_HEIGHT) / 2.0);
                height += metrics.line;
                at
            }
        };
        width = width.max(at.x + DOTS_WIDTH);
        dots = Some(at);
    }
    let original = (expanded && bubble.last_in_part && !shows_original).then(|| {
        let rule = height + ORIGINAL_GAP;
        let text_y = rule + 1.0 + ORIGINAL_GAP;
        let text = wrapped(
            ui,
            &bubble.original,
            (theme::SIZE_LABEL, theme::CHAT_TEXT_MUTED),
            metrics.wrap,
            None,
        );
        width = width.max(text.size().x);
        height = text_y + text.size().y;
        let video = message.moment.video_ms.map(|ms| {
            let galley = plain(
                ui,
                &format!("動画 {}", super::clock(ms)),
                theme::SIZE_CAPTION,
                theme::CHAT_TEXT_MUTED,
            );
            let y = height + STACK_GAP;
            width = width.max(galley.size().x);
            height = y + galley.size().y;
            (galley, y)
        });
        Original {
            rule,
            text,
            text_y,
            video,
        }
    });
    let size = egui::vec2(width + 2.0 * MARGIN_X, height + 2.0 * MARGIN_Y);
    let time = bubble
        .last_in_message
        .then_some(message.moment.sent)
        .flatten()
        .map(|sent| {
            let galley = plain(ui, &clock_text(sent), theme::SIZE_CAPTION, theme::CHAT_META);
            let beside = BUBBLE_X + size.x + TIME_GAP + metrics.time_width <= metrics.width - END;
            Time { galley, beside }
        });
    let note = bubble
        .last_in_message
        .then(|| note_text(message.status, !bubble.text.is_empty()))
        .flatten()
        .map(|note| plain(ui, note, theme::SIZE_CAPTION, theme::CHAT_META));
    let below = time
        .as_ref()
        .filter(|time| !time.beside)
        .map_or(0.0, |time| time.galley.size().y + 2.0)
        + note.as_ref().map_or(0.0, |note| note.size().y + 2.0);
    Laid {
        body,
        dots,
        original,
        time,
        note,
        size,
        below,
    }
}

fn note_text(status: Status, has_text: bool) -> Option<&'static str> {
    match status {
        Status::Fixed | Status::Forming => None,
        Status::Interrupted => Some("中断した文章 · 未確定"),
        Status::Failed if has_text => Some("更新に失敗 · 最後の訳文を表示"),
        Status::Failed => Some("翻訳できなかった文章 · 未確定"),
    }
}

enum Entry<'a> {
    Day(NaiveDate),
    Bubble(Bubble<'a>),
}

struct Bubble<'a> {
    message: Message<'a>,
    /// The speaker's share of the message. Its bubbles share one original.
    part: usize,
    /// Which of the part's bubbles this is.
    piece: usize,
    text: &'a str,
    /// All of the part's translation, which copying and reading take.
    whole: &'a str,
    /// Characters of `whole` before `text`.
    start: usize,
    original: Cow<'a, str>,
    first_in_group: bool,
    last_in_part: bool,
    last_in_message: bool,
    newest: bool,
}

/// Lines the messages up as date pills and bubbles. A message becomes one
/// bubble per speaker, and a finished speaker's words are cut at the ends of
/// sentences into bubbles of their own. Words still forming stay in one bubble,
/// because they change as they grow and cut bubbles would jump.
fn plan<'a>(messages: &[Message<'a>]) -> Vec<Entry<'a>> {
    let mut entries = Vec::new();
    let mut day = None;
    for (index, message) in messages.iter().enumerate() {
        let previous = index.checked_sub(1).map(|before| &messages[before]);
        let date = message.moment.sent.map(|sent| sent.date());
        let new_day = date.filter(|date| Some(*date) != day);
        if let Some(date) = new_day {
            entries.push(Entry::Day(date));
            day = Some(date);
        }
        let mut lead = new_day.is_some() || starts_group(previous, message);
        let mut parts = speaker_parts(message.target);
        if parts.is_empty() {
            parts.push("");
        }
        let originals = speaker_parts(message.original);
        let count = parts.len();
        let fallback_original = if originals.len() == count {
            None
        } else if originals.is_empty() {
            Some(Cow::Borrowed(""))
        } else if originals.len() == 1 {
            Some(Cow::Borrowed(originals[0]))
        } else {
            Some(Cow::Owned(originals.join("\n")))
        };
        for (part, whole) in parts.into_iter().enumerate() {
            let texts = if message.status == Status::Forming {
                vec![whole]
            } else {
                bubble_texts(whole)
            };
            let pieces = texts.len();
            let mut from = 0;
            for (piece, text) in texts.into_iter().enumerate() {
                let last_in_part = piece + 1 == pieces;
                let at = whole[from..]
                    .find(text)
                    .map_or(from, |offset| from + offset);
                from = at + text.len();
                entries.push(Entry::Bubble(Bubble {
                    message: *message,
                    part,
                    piece,
                    text,
                    whole,
                    start: whole[..at].chars().count(),
                    // An original that does not split like its translation is
                    // shown whole, with YouTube speaker markers removed.
                    original: match &fallback_original {
                        Some(fallback) => fallback.clone(),
                        None => Cow::Borrowed(originals[part]),
                    },
                    first_in_group: std::mem::take(&mut lead),
                    last_in_part,
                    last_in_message: last_in_part && part + 1 == count,
                    newest: index + 1 == messages.len(),
                }));
            }
        }
    }
    entries
}

fn starts_group(previous: Option<&Message<'_>>, message: &Message<'_>) -> bool {
    let Some(previous) = previous else {
        return true;
    };
    if previous.source != message.source {
        return true;
    }
    match (previous.moment.sent, message.moment.sent) {
        (Some(before), Some(now)) => now - before >= TimeDelta::minutes(2),
        _ => false,
    }
}

/// YouTube captions mark a new speaker with ">>". The marker is not shown;
/// each speaker gets a bubble of their own.
fn speaker_parts(text: &str) -> Vec<&str> {
    let is_marker = |c: char| c == '>' || c == '＞';
    let mut parts = Vec::new();
    let mut from = 0;
    let mut chars = text.char_indices().peekable();
    while let Some((at, c)) = chars.next() {
        if !is_marker(c) {
            continue;
        }
        let mut end = at + c.len_utf8();
        let mut count = 1;
        while let Some(&(next, d)) = chars.peek()
            && is_marker(d)
        {
            end = next + d.len_utf8();
            count += 1;
            chars.next();
        }
        if count >= 2 {
            parts.push(&text[from..at]);
            from = end;
        }
    }
    parts.push(&text[from..]);
    parts
        .into_iter()
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .collect()
}

/// The texts of the bubbles one speaker's finished words are shown in. A bubble
/// takes whole sentences while they fit in `BUBBLE_CHARS` characters.
fn bubble_texts(words: &str) -> Vec<&str> {
    let mut bubbles = Vec::new();
    let (mut bubble_start, mut sentence_start, mut chars) = (0, 0, 0);
    for sentence_end in sentence_ends(words) {
        let length = words[sentence_start..sentence_end].chars().count();
        if chars > 0 && chars + length > BUBBLE_CHARS {
            bubbles.push(words[bubble_start..sentence_start].trim());
            bubble_start = sentence_start;
            chars = 0;
        }
        chars += length;
        sentence_start = sentence_end;
    }
    bubbles.push(words[bubble_start..].trim());
    bubbles
}

/// Where the sentences of `text` end, the last at the end of the text. A
/// sentence ends after 。？！ or their half-width forms, together with any
/// further such marks and closing brackets that follow. Nothing ends inside
/// 「」, 『』 or （）, unless the text ends with one still open: a bracket
/// without its partner shelters nothing, so the text is cut as if it had no
/// brackets.
fn sentence_ends(text: &str) -> Vec<usize> {
    let is_stop = |c| matches!(c, '。' | '｡' | '？' | '?' | '！' | '!');
    let is_close = |c| matches!(c, '」' | '』' | '）');
    let cut = |brackets_shelter: bool| {
        let mut ends = Vec::new();
        let mut open = 0usize;
        let mut chars = text.char_indices().peekable();
        while let Some((at, c)) = chars.next() {
            if brackets_shelter && matches!(c, '「' | '『' | '（') {
                open += 1;
            } else if brackets_shelter && is_close(c) {
                open = open.saturating_sub(1);
            } else if is_stop(c) && open == 0 {
                let mut end = at + c.len_utf8();
                while let Some(&(next, mark)) = chars.peek()
                    && (is_stop(mark) || is_close(mark))
                {
                    end = next + mark.len_utf8();
                    chars.next();
                }
                ends.push(end);
            }
        }
        (ends, open > 0)
    };
    let mut ends = match cut(true) {
        (_, true) => cut(false).0,
        (ends, false) => ends,
    };
    if ends.last() != Some(&text.len()) {
        ends.push(text.len());
    }
    ends
}

fn clock_text(sent: NaiveDateTime) -> String {
    sent.format("%H:%M").to_string()
}

fn day_text(date: NaiveDate) -> String {
    let weekday =
        ["月", "火", "水", "木", "金", "土", "日"][date.weekday().num_days_from_monday() as usize];
    format!("{}月{}日（{weekday}）", date.month(), date.day())
}

#[cfg(test)]
pub(super) mod testing {
    use super::*;
    use eframe::egui::{Event, FullOutput, PointerButton, RawInput, Shape};

    pub fn context() -> egui::Context {
        let ctx = egui::Context::default();
        crate::ui::configure_theme(&ctx);
        ctx
    }

    /// One frame of `add` in a panel as large as the screen.
    pub fn frame(
        ctx: &egui::Context,
        size: Vec2,
        time: f64,
        events: Vec<Event>,
        mut add: impl FnMut(&mut egui::Ui),
    ) -> FullOutput {
        ctx.run(
            RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, size)),
                time: Some(time),
                events,
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default()
                    .frame(egui::Frame::new().inner_margin(12))
                    .show(ctx, &mut add);
            },
        )
    }

    pub fn shapes(output: &FullOutput) -> Vec<(Rect, &Shape)> {
        fn visit<'a>(clip: Rect, shape: &'a Shape, out: &mut Vec<(Rect, &'a Shape)>) {
            match shape {
                Shape::Vec(inner) => inner.iter().for_each(|shape| visit(clip, shape, out)),
                shape => out.push((clip, shape)),
            }
        }
        let mut out = Vec::new();
        for clipped in &output.shapes {
            visit(clipped.clip_rect, &clipped.shape, &mut out);
        }
        out
    }

    /// Every painted text, the room it takes and its colour.
    pub fn texts(output: &FullOutput) -> Vec<(String, Rect, Color32)> {
        shapes(output)
            .into_iter()
            .filter_map(|(_, shape)| match shape {
                Shape::Text(text) => Some((
                    text.galley.text().to_owned(),
                    Rect::from_min_size(
                        text.pos + text.galley.rect.min.to_vec2(),
                        text.galley.size(),
                    ),
                    text.galley.job.sections[0].format.color,
                )),
                _ => None,
            })
            .collect()
    }

    pub fn count_text(output: &FullOutput, wanted: &str) -> usize {
        texts(output)
            .iter()
            .filter(|(text, _, _)| text == wanted)
            .count()
    }

    pub fn has_text_containing(output: &FullOutput, part: &str) -> bool {
        texts(output).iter().any(|(text, _, _)| text.contains(part))
    }

    pub fn find_text(output: &FullOutput, wanted: &str) -> (Rect, Color32) {
        texts(output)
            .into_iter()
            .find(|(text, _, _)| text == wanted)
            .map(|(_, rect, color)| (rect, color))
            .unwrap_or_else(|| panic!("{wanted} is not drawn"))
    }

    /// The rectangles filled with `color`, such as the bubbles.
    pub fn filled(output: &FullOutput, color: Color32) -> Vec<Rect> {
        shapes(output)
            .into_iter()
            .filter_map(|(_, shape)| match shape {
                Shape::Rect(rect) if rect.fill == color => Some(rect.rect),
                _ => None,
            })
            .collect()
    }

    /// The smallest rectangle filled with `color` that holds the text `wanted`.
    pub fn box_of(output: &FullOutput, wanted: &str, color: Color32) -> Rect {
        let (text, _) = find_text(output, wanted);
        filled(output, color)
            .into_iter()
            .filter(|rect| rect.contains_rect(text))
            .min_by(|a, b| a.area().total_cmp(&b.area()))
            .unwrap_or_else(|| panic!("{wanted} is in no box"))
    }

    pub fn bubble_of(output: &FullOutput, wanted: &str) -> Rect {
        box_of(output, wanted, theme::CHAT_BUBBLE)
    }

    /// The little triangles that join the first bubble of a group to its avatar.
    pub fn tails(output: &FullOutput) -> usize {
        shapes(output)
            .into_iter()
            .filter(
                |(_, shape)| matches!(shape, Shape::Path(path) if path.fill == theme::CHAT_BUBBLE),
            )
            .count()
    }

    /// The painted texts with a selection on them.
    pub fn selected(output: &FullOutput) -> Vec<String> {
        shapes(output)
            .into_iter()
            .filter_map(|(_, shape)| match shape {
                Shape::Text(text)
                    if text.galley.rows.iter().any(|row| {
                        row.row
                            .visuals
                            .mesh
                            .vertices
                            .iter()
                            .any(|vertex| vertex.color == theme::CHAT_SELECTION)
                    }) =>
                {
                    Some(text.galley.text().to_owned())
                }
                _ => None,
            })
            .collect()
    }

    pub fn dots(output: &FullOutput) -> Vec<(Pos2, Color32)> {
        shapes(output)
            .into_iter()
            .filter_map(|(_, shape)| match shape {
                Shape::Circle(circle) if circle.radius == DOT_RADIUS => {
                    Some((circle.center, circle.fill))
                }
                _ => None,
            })
            .collect()
    }

    /// A shape in the reading area that starts above the area's top edge.
    /// Shapes in the reading area are clipped to it; the page around it is not.
    pub fn cut_at_the_top(output: &FullOutput, body: Rect) -> bool {
        shapes(output).into_iter().any(|(clip, shape)| {
            clip.top() > body.top() - 8.0 && shape.visual_bounding_rect().top() < clip.top() - 0.5
        })
    }

    /// A line of text in the reading area that the top edge of what it is
    /// clipped to runs through.
    pub fn line_cut_at_the_top(output: &FullOutput, body: Rect) -> bool {
        shapes(output).into_iter().any(|(clip, shape)| match shape {
            Shape::Text(text) if clip.top() > body.top() - 8.0 => {
                text.galley.rows.iter().any(|row| {
                    let line = row.rect().translate(text.pos.to_vec2());
                    line.top() < clip.top() - 0.5 && line.bottom() > clip.top() + 0.5
                })
            }
            _ => false,
        })
    }

    pub fn press(at: Pos2, button: PointerButton, pressed: bool) -> Event {
        Event::PointerButton {
            pos: at,
            button,
            pressed,
            modifiers: egui::Modifiers::NONE,
        }
    }

    pub fn wheel(delta: f32) -> Event {
        Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            delta: egui::vec2(0.0, delta),
            modifiers: egui::Modifiers::NONE,
        }
    }

    /// A chat in a window, drawn frame by frame the way the app draws it.
    pub struct Chat {
        pub ctx: egui::Context,
        pub state: State,
        pub size: Vec2,
        pub options: Options,
        time: f64,
    }

    impl Chat {
        pub fn new(width: f32, height: f32) -> Self {
            Self {
                ctx: context(),
                state: State::default(),
                size: egui::vec2(width, height),
                options: Options {
                    text_size: theme::READING_DEFAULT,
                    show_originals: false,
                    follow: true,
                    readout: None,
                    empty: EMPTY_HINT,
                    cut_from_a_whole_line: false,
                },
                time: 0.0,
            }
        }

        pub fn frame(
            &mut self,
            messages: &[Message<'_>],
            events: Vec<Event>,
        ) -> (FullOutput, Option<Action>) {
            self.time += 0.05;
            let mut action = None;
            let output = frame(&self.ctx, self.size, self.time, events, |ui| {
                action = self
                    .state
                    .show(ui, "chat", messages, self.options)
                    .or(action.take());
            });
            (output, action)
        }

        /// Frames until layout, scroll bar and scrolling have settled.
        pub fn settle(&mut self, messages: &[Message<'_>]) -> FullOutput {
            let mut output = self.frame(messages, vec![]).0;
            for _ in 0..30 {
                output = self.frame(messages, vec![]).0;
            }
            output
        }

        /// Press and release at `at`, then draw once more.
        pub fn click(
            &mut self,
            messages: &[Message<'_>],
            at: Pos2,
            button: PointerButton,
        ) -> (FullOutput, Option<Action>) {
            self.frame(messages, vec![Event::PointerMoved(at)]);
            self.frame(messages, vec![press(at, button, true)]);
            let (_, released) = self.frame(messages, vec![press(at, button, false)]);
            let (output, later) = self.frame(messages, vec![]);
            (output, released.or(later))
        }

        /// Press at `from`, move to `to` in steps, release there, then draw once more.
        pub fn drag(&mut self, messages: &[Message<'_>], from: Pos2, to: Pos2) -> FullOutput {
            self.frame(messages, vec![Event::PointerMoved(from)]);
            self.frame(messages, vec![press(from, PointerButton::Primary, true)]);
            for step in 1..=4 {
                let at = from + (to - from) * (step as f32 / 4.0);
                self.frame(messages, vec![Event::PointerMoved(at)]);
            }
            self.frame(messages, vec![press(to, PointerButton::Primary, false)]);
            self.frame(messages, vec![]).0
        }

        /// Ctrl+C, as the system passes it on.
        pub fn copy_keys(&mut self, messages: &[Message<'_>]) {
            self.frame(messages, vec![Event::Copy]);
        }

        /// A wheel turn over the chat, then frames until the scrolling stops.
        pub fn scroll_up(&mut self, messages: &[Message<'_>], pixels: f32) -> FullOutput {
            let body = self.state.body().expect("the chat has been drawn");
            self.frame(messages, vec![Event::PointerMoved(body.center())]);
            self.frame(messages, vec![wheel(pixels)]);
            self.settle(messages)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::testing::*;
    use super::*;
    use eframe::egui::{Event, PointerButton};

    fn at(hour: u32, minute: u32, second: u32) -> Moment {
        Moment {
            sent: NaiveDate::from_ymd_opt(2026, 11, 7)
                .unwrap()
                .and_hms_opt(hour, minute, second),
            video_ms: None,
        }
    }

    fn message(id: u64, source: Source, target: &str, moment: Moment) -> Message<'_> {
        Message {
            id,
            source,
            target,
            original: "",
            moment,
            status: Status::Fixed,
            failure: None,
        }
    }

    fn lines(count: usize) -> Vec<String> {
        (1..=count)
            .map(|n| format!("{n}番目に確定した部分です。"))
            .collect()
    }

    fn talk(lines: &[String]) -> Vec<Message<'_>> {
        lines
            .iter()
            .zip(1..)
            .map(|(text, id)| message(id, Source::PcAudio, text, at(21, 48, id as u32)))
            .collect()
    }

    fn announces_new_messages(output: &egui::FullOutput) -> bool {
        has_text_containing(output, "新しいメッセージ")
    }

    /// The bubbles drawn, from the top.
    fn bubbles(output: &egui::FullOutput) -> Vec<Rect> {
        let mut bubbles = filled(output, theme::CHAT_BUBBLE);
        bubbles.sort_by(|a, b| a.top().total_cmp(&b.top()));
        bubbles
    }

    #[test]
    fn a_speaker_marker_splits_a_translation_and_is_never_shown() {
        assert_eq!(
            speaker_parts(">> こんにちは。>> やあ。 >>>ようこそ。"),
            ["こんにちは。", "やあ。", "ようこそ。"]
        );
        assert_eq!(speaker_parts("前置き。＞＞ 返事。"), ["前置き。", "返事。"]);
        assert_eq!(
            speaker_parts("AはBより > 大きい。"),
            ["AはBより > 大きい。"]
        );
        assert_eq!(speaker_parts(">> "), Vec::<&str>::new());
        assert_eq!(speaker_parts(""), Vec::<&str>::new());
    }

    /// Seven sentences, as a real talk once ran them together into one bubble.
    const TALK: &str = "とてもすごいですね。皆さん、こんにちは。今日のセッションへようこそ。今日は、自分のコンピューターでローカル翻訳がどのように動くのかについて話します。仕組みは簡単です。クラウドには何も送られません。すべて自分のコンピューターの中に残ります。";

    /// A sentence of a third of what a bubble takes.
    fn third() -> String {
        "あ".repeat(BUBBLE_CHARS / 3 - 1) + "。"
    }

    #[test]
    fn a_finished_paragraph_is_cut_at_sentence_ends_into_bubbles_of_up_to_the_limit() {
        let third = third();
        assert_eq!(
            bubble_texts(&third.repeat(3)),
            [third.repeat(3)],
            "three sentences fill a bubble"
        );
        assert_eq!(
            bubble_texts(&third.repeat(4)),
            [third.repeat(3), third.clone()],
            "the fourth starts the next bubble"
        );
        let bubbles = bubble_texts(TALK);
        assert!(bubbles.len() > 1);
        assert_eq!(bubbles.concat(), TALK, "nothing is lost or added");
        for bubble in &bubbles {
            assert!(bubble.chars().count() <= BUBBLE_CHARS, "{bubble}");
            assert!(
                bubble.ends_with('。'),
                "a bubble ends with a sentence: {bubble}"
            );
        }
        assert_eq!(bubble_texts("はい。"), ["はい。"]);
        assert_eq!(bubble_texts("まだ文の途中"), ["まだ文の途中"]);
        assert_eq!(bubble_texts(""), [""]);
    }

    #[test]
    fn a_sentence_longer_than_the_limit_stays_whole_in_a_bubble_of_its_own() {
        let long = "い".repeat(2 * BUBBLE_CHARS) + "。";
        assert_eq!(bubble_texts(&long), [long.as_str()]);
        assert_eq!(
            bubble_texts(&format!("はい。{long} {long}はい。")),
            ["はい。", long.as_str(), long.as_str(), "はい。"]
        );
    }

    #[test]
    fn nothing_is_cut_inside_a_quote_a_title_or_an_aside() {
        let half = BUBBLE_CHARS / 2;
        let quoted = format!(
            "彼は「{}。{}。」と言いました。",
            "あ".repeat(half),
            "い".repeat(half)
        );
        assert_eq!(bubble_texts(&quoted), [quoted.as_str()]);

        let title = "映画『君の名は。』を見ました。";
        let lead = "あ".repeat(BUBBLE_CHARS - 11) + "。";
        assert_eq!(
            bubble_texts(&format!("{lead}{title}")),
            [lead.as_str(), title],
            "the stop inside 『』 is not an end, though the bubble has room for it"
        );

        let aside = "例です（これは例です。ほかにもあります。）終わり。";
        let lead = "あ".repeat(BUBBLE_CHARS - 12) + "。";
        assert_eq!(
            bubble_texts(&format!("{lead}{aside}")),
            [lead.as_str(), aside]
        );
    }

    #[test]
    fn marks_after_a_stop_stay_with_their_sentence_and_half_width_stops_end_one() {
        let lead = "あ".repeat(BUBBLE_CHARS - 3) + "。」";
        assert_eq!(
            bubble_texts(&format!("{lead}はい。")),
            [lead.as_str(), "はい。"],
            "a closing bracket is not the start of the next bubble"
        );
        let lead = "あ".repeat(BUBBLE_CHARS - 1) + "？！";
        assert_eq!(
            bubble_texts(&format!("{lead}はい。")),
            [lead.as_str(), "はい。"],
            "a run of marks ends one sentence"
        );
        let half = BUBBLE_CHARS / 2;
        let [a, b, c] = [("あ", "?"), ("い", "!"), ("う", "｡")]
            .map(|(letter, stop)| format!("{}{stop}", letter.repeat(half)));
        assert_eq!(
            bubble_texts(&format!("{a}{b}{c}")),
            [a.as_str(), b.as_str(), c.as_str()]
        );
    }

    #[test]
    fn a_bracket_left_open_does_not_keep_the_sentences_after_it_in_one_bubble() {
        let third = third();
        assert_eq!(
            bubble_texts(&format!("彼は「{}", third.repeat(4))),
            [format!("彼は「{}", third.repeat(2)), third.repeat(2)]
        );
    }

    #[test]
    fn the_clock_and_the_day_read_as_they_were_written_in_created_at() {
        let moment =
            Moment::from_record("2026-11-07T21:48:03.123456+09:00", 754_000, Source::YouTube);
        let sent = moment.sent.expect("a valid time");
        assert_eq!(clock_text(sent), "21:48");
        assert_eq!(day_text(sent.date()), "11月7日（土）");
        assert_eq!(moment.video_ms, Some(754_000));
        assert_eq!(day_text(sent.date().succ_opt().unwrap()), "11月8日（日）");
        assert_eq!(
            Moment::from_record("2026-11-08T00:05:00+00:00", 5, Source::PcAudio),
            Moment {
                sent: NaiveDate::from_ymd_opt(2026, 11, 8)
                    .unwrap()
                    .and_hms_opt(0, 5, 0),
                video_ms: None,
            },
            "the clock is the one written, whatever zone this computer is in"
        );
        for unreadable in ["", "昨日", "2026-11-07"] {
            assert_eq!(
                Moment::from_record(unreadable, 0, Source::Screen),
                Moment::default()
            );
        }
    }

    #[test]
    fn messages_of_one_source_less_than_two_minutes_apart_share_an_avatar_and_a_name() {
        let mut chat = Chat::new(720.0, 460.0);
        let output = chat.settle(&[
            message(1, Source::YouTube, "最初です。", at(21, 48, 0)),
            message(2, Source::YouTube, "二つ目です。", at(21, 49, 59)),
            message(3, Source::PcAudio, "別の入力です。", at(21, 50, 0)),
            message(4, Source::PcAudio, "ちょうど二分後です。", at(21, 52, 0)),
        ]);
        assert_eq!(count_text(&output, "YouTube"), 1, "two messages, one group");
        assert_eq!(count_text(&output, "YT"), 1);
        assert_eq!(
            count_text(&output, "PC音声"),
            2,
            "another source, then two minutes"
        );
        assert_eq!(count_text(&output, "音"), 2);
        assert_eq!(tails(&output), 3, "each group opens with one tail");
        let (first, _) = find_text(&output, "最初です。");
        let (second, _) = find_text(&output, "二つ目です。");
        assert!(
            second.top() - first.top() < 2.0 * 51.0,
            "bubbles of a group stack closely"
        );
    }

    #[test]
    fn messages_without_a_time_group_by_source_alone() {
        let mut chat = Chat::new(720.0, 460.0);
        let none = Moment::default();
        let output = chat.settle(&[
            message(1, Source::YouTube, "最初です。", none),
            message(2, Source::YouTube, "二つ目です。", none),
            message(3, Source::Screen, "画面の文字です。", none),
        ]);
        assert_eq!(count_text(&output, "YouTube"), 1);
        assert_eq!(count_text(&output, "画面"), 1);
        assert_eq!(count_text(&output, "画"), 1);
        assert!(
            !has_text_containing(&output, ":") && !has_text_containing(&output, "月"),
            "no clock and no date pill without a time"
        );
    }

    #[test]
    fn a_speaker_change_starts_a_bubble_in_the_same_group_and_the_marker_stays_hidden() {
        let mut chat = Chat::new(720.0, 460.0);
        let output = chat.settle(&[message(
            1,
            Source::YouTube,
            ">> こんにちは。>> やあ、元気ですか。>> はい。",
            at(21, 48, 10),
        )]);
        let bubbles: Vec<Rect> = ["こんにちは。", "やあ、元気ですか。", "はい。"]
            .iter()
            .map(|text| bubble_of(&output, text))
            .collect();
        assert_eq!(count_text(&output, "YouTube"), 1, "same group");
        assert_eq!(tails(&output), 1);
        assert_eq!(count_text(&output, "21:48"), 1, "one message, one time");
        for pair in bubbles.windows(2) {
            assert_eq!(pair[0].left(), pair[1].left());
            assert!((pair[1].top() - pair[0].bottom() - STACK_GAP).abs() < 0.5);
        }
        assert!(
            !has_text_containing(&output, ">"),
            "the marker is not drawn"
        );
        let (time, _) = find_text(&output, "21:48");
        assert!(
            time.left() > bubbles[2].right() && time.bottom() <= bubbles[2].bottom() + 2.0,
            "the time sits beside the bottom right of the last bubble"
        );
    }

    #[test]
    fn a_date_pill_opens_each_day() {
        let mut chat = Chat::new(720.0, 460.0);
        let next_day = Moment {
            sent: NaiveDate::from_ymd_opt(2026, 11, 8)
                .unwrap()
                .and_hms_opt(0, 0, 30),
            video_ms: None,
        };
        let output = chat.settle(&[
            message(1, Source::YouTube, "夜です。", at(23, 59, 40)),
            message(2, Source::YouTube, "日付が変わりました。", next_day),
        ]);
        assert_eq!(count_text(&output, "11月7日（土）"), 1);
        assert_eq!(count_text(&output, "11月8日（日）"), 1);
        assert_eq!(
            count_text(&output, "YouTube"),
            2,
            "a new day starts a new group even a minute later"
        );
        let first = box_of(&output, "11月7日（土）", theme::CHAT_PILL);
        let second = box_of(&output, "11月8日（日）", theme::CHAT_PILL);
        let body = chat.state.body().unwrap();
        assert!((first.center().x - body.center().x).abs() < 1.0, "centred");
        assert!(second.top() > first.bottom());
    }

    #[test]
    fn a_bubble_hugs_short_text_and_wraps_long_text_at_78_percent_of_the_width() {
        let mut chat = Chat::new(720.0, 460.0);
        let long = "とても長い訳文が続いて、吹き出しの幅の上限で折り返されます。".repeat(4);
        let output = chat.settle(&[
            message(1, Source::PcAudio, "はい。", at(21, 48, 0)),
            message(2, Source::PcAudio, &long, at(21, 48, 30)),
        ]);
        let width = chat.state.body().unwrap().width();
        let short = bubble_of(&output, "はい。");
        assert!(short.width() < 120.0, "a short message hugs its text");
        let wide = filled(&output, theme::CHAT_BUBBLE)
            .into_iter()
            .map(|bubble| bubble.width())
            .fold(0.0, f32::max);
        assert!(wide <= 0.78 * width + 0.5, "{wide} of {width}");
        assert!(
            wide >= 0.6 * width,
            "long text fills the line: {wide} of {width}"
        );
    }

    #[test]
    fn a_narrow_chat_still_gives_a_bubble_about_240_points() {
        let mut chat = Chat::new(360.0, 300.0);
        let output = chat.settle(&[message(
            1,
            Source::PcAudio,
            &"狭い画面でも読める幅を保ちます。".repeat(6),
            at(21, 48, 0),
        )]);
        let wide = filled(&output, theme::CHAT_BUBBLE)
            .into_iter()
            .map(|bubble| bubble.width())
            .fold(0.0, f32::max);
        assert!((238.0..=252.0).contains(&wide), "{wide}");
        let (time, _) = find_text(&output, "21:48");
        assert!(
            time.right() <= chat.state.body().unwrap().right() + 0.5,
            "the time is not pushed out of the chat"
        );
    }

    #[test]
    fn clicking_a_bubble_shows_its_original_and_the_video_time_and_clicking_again_hides_them() {
        let mut chat = Chat::new(720.0, 460.0);
        let moment = Moment {
            video_ms: Some(754_000),
            ..at(21, 48, 0)
        };
        let messages = [Message {
            original: "Hello there, nice to meet you.",
            ..message(1, Source::YouTube, "こんにちは、はじめまして。", moment)
        }];
        let output = chat.settle(&messages);
        assert_eq!(count_text(&output, "Hello there, nice to meet you."), 0);
        let bubble = bubble_of(&output, "こんにちは、はじめまして。");

        let (output, _) = chat.click(&messages, bubble.center(), PointerButton::Primary);
        let original = bubble_of(&output, "Hello there, nice to meet you.");
        let (text, color) = find_text(&output, "Hello there, nice to meet you.");
        assert_eq!(
            bubble_of(&output, "こんにちは、はじめまして。"),
            original,
            "the original is inside the same bubble"
        );
        assert!(text.top() > find_text(&output, "こんにちは、はじめまして。").0.bottom());
        assert_eq!(color, theme::CHAT_TEXT_MUTED);
        assert_eq!(count_text(&output, "動画 00:12:34"), 1);
        assert_eq!(
            filled(&output, theme::CHAT_RULE).len(),
            1,
            "a thin line separates them"
        );

        let (output, _) = chat.click(&messages, original.center(), PointerButton::Primary);
        assert_eq!(count_text(&output, "Hello there, nice to meet you."), 0);
        assert_eq!(count_text(&output, "動画 00:12:34"), 0);
    }

    #[test]
    fn the_pointer_shows_that_a_bubble_with_an_original_can_be_clicked() {
        let mut chat = Chat::new(720.0, 460.0);
        let messages = [
            Message {
                original: "Hello.",
                ..message(1, Source::PcAudio, "こんにちは。", at(21, 48, 0))
            },
            message(2, Source::PcAudio, "原文がありません。", at(21, 48, 30)),
        ];
        let output = chat.settle(&messages);
        let with = bubble_of(&output, "こんにちは。").center();
        let without = bubble_of(&output, "原文がありません。").center();
        let (over, _) = chat.frame(&messages, vec![Event::PointerMoved(with)]);
        assert_eq!(
            over.platform_output.cursor_icon,
            egui::CursorIcon::PointingHand
        );
        let (over, _) = chat.frame(&messages, vec![Event::PointerMoved(without)]);
        assert_eq!(over.platform_output.cursor_icon, egui::CursorIcon::Default);
    }

    #[test]
    fn showing_every_original_can_still_be_undone_one_bubble_at_a_time() {
        let mut chat = Chat::new(720.0, 460.0);
        let messages = [
            Message {
                original: "First original.",
                ..message(1, Source::PcAudio, "一つ目。", at(21, 48, 0))
            },
            Message {
                original: "Second original.",
                ..message(2, Source::PcAudio, "二つ目。", at(21, 48, 30))
            },
        ];
        chat.options.show_originals = true;
        let output = chat.settle(&messages);
        assert_eq!(count_text(&output, "First original."), 1);
        assert_eq!(count_text(&output, "Second original."), 1);
        let (text, _) = find_text(&output, "一つ目。");
        let (output, _) = chat.click(&messages, text.center(), PointerButton::Primary);
        assert_eq!(count_text(&output, "First original."), 0);
        assert_eq!(count_text(&output, "Second original."), 1);
        chat.options.show_originals = false;
        let output = chat.settle(&messages);
        assert_eq!(
            count_text(&output, "First original.") + count_text(&output, "Second original."),
            0,
            "turning the option off closes them all"
        );
    }

    #[test]
    fn a_split_translation_pairs_each_bubble_with_its_own_part_of_the_original() {
        let mut chat = Chat::new(720.0, 460.0);
        let messages = [Message {
            original: ">> Hello. >> Hi there.",
            ..message(
                1,
                Source::YouTube,
                ">> こんにちは。>> やあ。",
                at(21, 48, 0),
            )
        }];
        chat.options.show_originals = true;
        let output = chat.settle(&messages);
        assert_eq!(
            bubble_of(&output, "Hello."),
            bubble_of(&output, "こんにちは。")
        );
        assert_eq!(
            bubble_of(&output, "Hi there."),
            bubble_of(&output, "やあ。")
        );
    }

    #[test]
    fn the_bubbles_of_a_long_paragraph_share_one_avatar_one_name_and_one_time() {
        let mut chat = Chat::new(720.0, 700.0);
        let output = chat.settle(&[message(1, Source::PcAudio, TALK, at(21, 48, 10))]);
        let shown = bubbles(&output);
        assert!(shown.len() > 1, "a long paragraph is several bubbles");
        assert_eq!(count_text(&output, "PC音声"), 1);
        assert_eq!(count_text(&output, "音"), 1);
        assert_eq!(tails(&output), 1, "one tail, on the first bubble");
        for pair in shown.windows(2) {
            assert_eq!(pair[0].left(), pair[1].left());
            assert!(
                (pair[1].top() - pair[0].bottom() - STACK_GAP).abs() < 0.5,
                "bubbles of one paragraph stack closely"
            );
        }
        assert_eq!(count_text(&output, "21:48"), 1, "one paragraph, one time");
        let (time, _) = find_text(&output, "21:48");
        let last = shown.last().unwrap();
        assert!(
            time.left() > last.right() && time.bottom() <= last.bottom() + 2.0,
            "the time sits beside the bottom right of the last bubble"
        );
        let mut drawn: Vec<_> = texts(&output)
            .into_iter()
            .filter(|(text, _, _)| text.contains('。'))
            .collect();
        drawn.sort_by(|a, b| a.1.top().total_cmp(&b.1.top()));
        let words: String = drawn.iter().map(|(text, _, _)| text.as_str()).collect();
        assert_eq!(
            words, TALK,
            "read from the top, the bubbles are the paragraph"
        );
    }

    #[test]
    fn a_change_of_speaker_and_the_cuts_between_sentences_work_together() {
        let mut chat = Chat::new(720.0, 700.0);
        let said = format!(">> {TALK} >> はい。");
        let output = chat.settle(&[message(1, Source::YouTube, &said, at(21, 48, 10))]);
        let shown = bubbles(&output);
        assert!(shown.len() > 2, "the first speaker's words are cut too");
        assert_eq!(bubble_of(&output, "はい。"), *shown.last().unwrap());
        assert_eq!(count_text(&output, "YouTube"), 1, "one group");
        assert_eq!(tails(&output), 1);
        assert!(
            !has_text_containing(&output, ">"),
            "the marker is not drawn"
        );
        assert_eq!(count_text(&output, "21:48"), 1, "one message, one time");
        let (time, _) = find_text(&output, "21:48");
        assert!(
            time.left() > shown.last().unwrap().right(),
            "the time is beside the last bubble of the last speaker"
        );
    }

    #[test]
    fn a_paragraph_still_forming_stays_one_bubble_until_it_is_fixed() {
        let mut chat = Chat::new(720.0, 700.0);
        let forming = Message {
            status: Status::Forming,
            ..message(1, Source::PcAudio, TALK, at(21, 48, 10))
        };
        let output = chat.settle(&[forming]);
        assert_eq!(bubbles(&output).len(), 1);
        assert_eq!(count_text(&output, TALK), 1);
        assert_eq!(dots(&output).len(), 3);

        let output = chat.settle(&[Message {
            status: Status::Fixed,
            ..forming
        }]);
        assert!(bubbles(&output).len() > 1, "fixed, it is cut");
        assert!(dots(&output).is_empty());
    }

    #[test]
    fn an_interrupted_paragraph_is_cut_and_its_note_is_under_the_last_bubble() {
        let mut chat = Chat::new(720.0, 700.0);
        let output = chat.settle(&[Message {
            status: Status::Interrupted,
            ..message(1, Source::PcAudio, TALK, at(21, 48, 10))
        }]);
        let shown = bubbles(&output);
        assert!(shown.len() > 1);
        assert_eq!(count_text(&output, "中断した文章 · 未確定"), 1);
        let (note, _) = find_text(&output, "中断した文章 · 未確定");
        assert!(note.top() >= shown.last().unwrap().bottom());
    }

    #[test]
    fn clicking_any_bubble_of_a_paragraph_opens_its_original_under_the_last_one() {
        const ORIGINAL: &str = "That is very impressive. Hello, everyone.";
        let mut chat = Chat::new(720.0, 800.0);
        let messages = [Message {
            original: ORIGINAL,
            ..message(
                1,
                Source::YouTube,
                TALK,
                Moment {
                    video_ms: Some(754_000),
                    ..at(21, 48, 10)
                },
            )
        }];
        let output = chat.settle(&messages);
        let shown = bubbles(&output);
        assert!(shown.len() > 1);
        assert_eq!(count_text(&output, ORIGINAL), 0);

        let (output, _) = chat.click(&messages, shown[0].center(), PointerButton::Primary);
        let last = *bubbles(&output).last().unwrap();
        assert_eq!(count_text(&output, ORIGINAL), 1, "shown once");
        assert_eq!(bubble_of(&output, ORIGINAL), last, "inside the last bubble");
        assert_eq!(count_text(&output, "動画 00:12:34"), 1);
        assert_eq!(filled(&output, theme::CHAT_RULE).len(), 1);

        let middle = bubbles(&output)[1];
        let (output, _) = chat.click(&messages, middle.center(), PointerButton::Primary);
        assert_eq!(
            count_text(&output, ORIGINAL),
            0,
            "a click on another bubble of the paragraph closes it"
        );
        assert_eq!(count_text(&output, "動画 00:12:34"), 0);
    }

    #[test]
    fn right_click_on_any_bubble_of_a_paragraph_copies_and_reads_the_whole_paragraph() {
        let mut chat = Chat::new(720.0, 800.0);
        chat.options.readout = Some(false);
        let messages = [message(1, Source::PcAudio, TALK, at(21, 48, 10))];
        let output = chat.settle(&messages);
        let shown = bubbles(&output);
        assert!(shown.len() > 1);

        let (menu, _) = chat.click(&messages, shown[0].center(), PointerButton::Secondary);
        let (copy, _) = find_text(&menu, "コピー");
        chat.click(&messages, copy.center(), PointerButton::Primary);
        let copied = chat.state.copied.as_deref().expect("コピー was clicked");
        assert!(copied.starts_with(TALK) && copied.contains(credit::CREDIT));

        let last = *shown.last().unwrap();
        let (menu, _) = chat.click(&messages, last.center(), PointerButton::Secondary);
        let (read, _) = find_text(&menu, "読み上げ");
        let (_, action) = chat.click(&messages, read.center(), PointerButton::Primary);
        assert!(matches!(action, Some(Action::Read(text)) if text == TALK));
    }

    #[test]
    fn new_messages_are_counted_by_paragraph_not_by_bubble() {
        let mut chat = Chat::new(720.0, 300.0);
        let spoken = lines(6);
        let mut messages = talk(&spoken);
        chat.settle(&messages);
        chat.scroll_up(&messages, 150.0);
        messages.push(message(7, Source::PcAudio, TALK, at(21, 49, 7)));
        messages.push(message(8, Source::PcAudio, TALK, at(21, 49, 8)));
        let arrived = plan(&messages)
            .into_iter()
            .filter(|entry| matches!(entry, Entry::Bubble(bubble) if bubble.message.id > 6))
            .count();
        assert!(arrived > 2, "two paragraphs came in as {arrived} bubbles");
        let output = chat.settle(&messages);
        assert_eq!(count_text(&output, "↓ 新しいメッセージ 2件"), 1);
    }

    #[test]
    fn a_message_still_forming_is_lighter_and_ends_in_three_moving_dots() {
        let mut chat = Chat::new(720.0, 460.0);
        let forming = Message {
            status: Status::Forming,
            ..message(2, Source::PcAudio, "いま話している途中", at(21, 48, 30))
        };
        let messages = [
            message(1, Source::PcAudio, "確定した部分。", at(21, 48, 0)),
            forming,
        ];
        let output = chat.settle(&messages);
        assert_eq!(find_text(&output, "確定した部分。").1, theme::CHAT_TEXT);
        assert_eq!(
            find_text(&output, "いま話している途中").1,
            theme::CHAT_TEXT_FORMING
        );
        let first = dots(&output);
        assert_eq!(first.len(), 3);
        let text = find_text(&output, "いま話している途中").0;
        assert!(
            first.iter().all(|(center, _)| center.x > text.right()),
            "after the text"
        );
        let (later, _) = chat.frame(&messages, vec![]);
        assert_ne!(dots(&later), first, "the dots move");

        let output = chat.settle(&messages[..1]);
        assert!(dots(&output).is_empty(), "a fixed message has no dots");
    }

    #[test]
    fn a_message_without_text_yet_is_a_bubble_of_dots_only() {
        let mut chat = Chat::new(720.0, 460.0);
        let output = chat.settle(&[Message {
            status: Status::Forming,
            ..message(1, Source::YouTube, "", at(21, 48, 0))
        }]);
        assert_eq!(dots(&output).len(), 3);
        let bubbles = filled(&output, theme::CHAT_BUBBLE);
        assert_eq!(bubbles.len(), 1);
        assert!(bubbles[0].width() < 80.0, "{:?}", bubbles[0]);
        assert!(
            dots(&output)
                .iter()
                .all(|(center, _)| bubbles[0].contains(*center))
        );
    }

    #[test]
    fn typing_is_a_bubble_of_dots_that_is_never_counted_as_a_new_message() {
        let mut chat = Chat::new(720.0, 300.0);
        let spoken = lines(8);
        let mut messages = talk(&spoken);
        chat.settle(&messages);
        chat.scroll_up(&messages, 300.0);
        messages.push(Message::typing(Source::PcAudio, ""));
        let output = chat.settle(&messages);
        assert!(
            !announces_new_messages(&output),
            "only the typing bubble arrived"
        );
    }

    #[test]
    fn a_forming_message_without_a_translation_shows_its_original_small_and_faint_with_dots() {
        let mut chat = Chat::new(720.0, 460.0);
        let finished = message(1, Source::PcAudio, "Hello world.", at(21, 48, 0));
        let forming = Message {
            status: Status::Forming,
            original: "Hello world.",
            ..message(2, Source::PcAudio, "", at(21, 48, 30))
        };
        let output = chat.settle(&[finished, forming]);
        let (finished_rect, _) = texts(&output)
            .into_iter()
            .find(|(text, _, color)| text == "Hello world." && *color == theme::CHAT_TEXT)
            .map(|(_, rect, color)| (rect, color))
            .expect("finished message text");
        let (forming_rect, forming_color) = texts(&output)
            .into_iter()
            .find(|(text, _, color)| text == "Hello world." && *color == theme::CHAT_TEXT_FORMING)
            .map(|(_, rect, color)| (rect, color))
            .expect("forming message text");
        assert_eq!(forming_color, theme::CHAT_TEXT_FORMING);
        assert!(
            forming_rect.width() < 0.9 * finished_rect.width(),
            "smaller than the translation text of a finished message: {} vs {}",
            forming_rect.width(),
            finished_rect.width()
        );
        let bubble = filled(&output, theme::CHAT_BUBBLE)
            .into_iter()
            .find(|rect| rect.contains_rect(forming_rect))
            .expect("forming bubble");
        let dots = dots(&output);
        assert_eq!(dots.len(), 3);
        assert!(
            dots.iter().all(|(center, _)| bubble.contains(*center)),
            "the three dots are inside the same bubble"
        );
        assert!(
            dots.iter()
                .all(|(center, _)| center.x > forming_rect.right()),
            "dots sit after the text"
        );
    }

    #[test]
    fn the_speaker_markers_are_not_shown_and_each_speaker_starts_on_a_line_of_their_own() {
        let mut chat = Chat::new(720.0, 460.0);
        let message = Message {
            status: Status::Forming,
            original: ">> First speaker. ＞＞ Second speaker.",
            ..message(1, Source::YouTube, "", at(21, 48, 0))
        };
        let output = chat.settle(&[message]);
        assert!(!has_text_containing(&output, ">"));
        assert!(!has_text_containing(&output, "＞"));
        assert_eq!(
            count_text(&output, "First speaker.\nSecond speaker."),
            1,
            "each speaker's words start on a line of their own"
        );
    }

    #[test]
    fn the_same_message_with_a_translation_shows_the_translation_and_not_the_original() {
        let mut chat = Chat::new(720.0, 460.0);
        let mut message = Message {
            status: Status::Forming,
            original: "Hello world.",
            ..message(1, Source::PcAudio, "", at(21, 48, 0))
        };
        let output = chat.settle(&[message]);
        assert_eq!(count_text(&output, "Hello world."), 1);
        assert_eq!(count_text(&output, "こんにちは世界。"), 0);

        message.target = "こんにちは世界。";
        let output = chat.settle(&[message]);
        assert_eq!(count_text(&output, "こんにちは世界。"), 1);
        assert_eq!(count_text(&output, "Hello world."), 0);
    }

    #[test]
    fn clicking_a_forming_bubble_with_original_does_not_open_it_when_translated() {
        let mut chat = Chat::new(720.0, 460.0);
        let mut message = Message {
            status: Status::Forming,
            original: "Hello world.",
            ..message(1, Source::PcAudio, "", at(21, 48, 0))
        };
        let output = chat.settle(&[message]);
        assert_eq!(count_text(&output, "Hello world."), 1);
        let bubble = bubble_of(&output, "Hello world.");
        let (over, _) = chat.frame(&[message], vec![Event::PointerMoved(bubble.center())]);
        assert_eq!(
            over.platform_output.cursor_icon,
            egui::CursorIcon::Default,
            "the pointer does not turn into a hand"
        );
        let (output, _) = chat.click(&[message], bubble.center(), PointerButton::Primary);
        assert_eq!(count_text(&output, "Hello world."), 1);

        message.target = "こんにちは世界。";
        let output = chat.settle(&[message]);
        assert_eq!(count_text(&output, "こんにちは世界。"), 1);
        assert_eq!(
            count_text(&output, "Hello world."),
            0,
            "the original is not shown under the translation"
        );
    }

    #[test]
    fn with_show_originals_on_a_forming_message_without_a_translation_shows_its_original_once() {
        let mut chat = Chat::new(720.0, 460.0);
        chat.options.show_originals = true;
        let message = Message {
            status: Status::Forming,
            original: "Hello world.",
            ..message(1, Source::PcAudio, "", at(21, 48, 0))
        };
        let output = chat.settle(&[message]);
        assert_eq!(count_text(&output, "Hello world."), 1);
    }

    #[test]
    fn typing_with_recognised_words_shows_them_small_and_faint() {
        let mut chat = Chat::new(720.0, 460.0);
        let words = "recognised words";
        let finished = message(1, Source::PcAudio, words, at(21, 48, 0));
        let typing = Message::typing(Source::PcAudio, words);
        let output = chat.settle(&[finished, typing]);
        let (finished_rect, _) = texts(&output)
            .into_iter()
            .find(|(text, _, color)| text == words && *color == theme::CHAT_TEXT)
            .map(|(_, rect, color)| (rect, color))
            .expect("finished text");
        let (typing_rect, typing_color) = texts(&output)
            .into_iter()
            .find(|(text, _, color)| text == words && *color == theme::CHAT_TEXT_FORMING)
            .map(|(_, rect, color)| (rect, color))
            .expect("typing text");
        assert_eq!(typing_color, theme::CHAT_TEXT_FORMING);
        assert!(
            typing_rect.width() < 0.9 * finished_rect.width(),
            "typing original is narrower than finished translation: {} vs {}",
            typing_rect.width(),
            finished_rect.width()
        );
        let bubble = filled(&output, theme::CHAT_BUBBLE)
            .into_iter()
            .find(|rect| rect.contains_rect(typing_rect))
            .expect("typing bubble");
        let dots = dots(&output);
        assert_eq!(dots.len(), 3);
        assert!(dots.iter().all(|(center, _)| bubble.contains(*center)));
        assert!(
            dots.iter()
                .all(|(center, _)| center.x > typing_rect.right())
        );
    }

    #[test]
    fn a_reader_who_scrolled_up_is_told_how_many_messages_came_in_and_can_jump_back() {
        let mut chat = Chat::new(720.0, 300.0);
        let spoken = lines(10);
        let mut messages = talk(&spoken[..6]);
        let output = chat.settle(&messages);
        let end = chat.state.offset();
        assert!(end > 50.0, "the talk is taller than the window: {end}");
        assert_eq!(count_text(&output, "6番目に確定した部分です。"), 1);
        assert!(
            !announces_new_messages(&output),
            "nothing to announce while the reader is at the bottom"
        );

        let output = chat.scroll_up(&messages, 150.0);
        let reading = chat.state.offset();
        assert!(
            reading < end - 50.0,
            "the wheel left the bottom: {reading} vs {end}"
        );
        assert!(!announces_new_messages(&output));

        messages = talk(&spoken[..8]);
        let output = chat.settle(&messages);
        assert_eq!(count_text(&output, "↓ 新しいメッセージ 2件"), 1);
        assert!(
            (chat.state.offset() - reading).abs() < 1.0,
            "new messages do not move a reader who is looking higher up"
        );

        messages = talk(&spoken[..9]);
        let output = chat.settle(&messages);
        assert_eq!(count_text(&output, "↓ 新しいメッセージ 3件"), 1);

        let pill = box_of(&output, "↓ 新しいメッセージ 3件", theme::CHAT_PILL);
        let body = chat.state.body().unwrap();
        assert!((pill.center().x - body.center().x).abs() < 1.0, "centred");
        assert!(
            pill.bottom() <= body.bottom() && pill.top() > body.center().y,
            "at the bottom"
        );

        chat.click(&messages, pill.center(), PointerButton::Primary);
        let output = chat.settle(&messages);
        assert!(
            !announces_new_messages(&output),
            "the count starts again from the end"
        );
        assert!(
            chat.state.offset() > reading + 50.0,
            "the pill took the reader back to the end"
        );
        assert_eq!(count_text(&output, "9番目に確定した部分です。"), 1);
        let end = chat.state.offset();

        messages = talk(&spoken[..10]);
        let output = chat.settle(&messages);
        assert!(
            chat.state.offset() > end + 20.0,
            "following again after the jump"
        );
        assert_eq!(count_text(&output, "10番目に確定した部分です。"), 1);
        assert!(!announces_new_messages(&output));
    }

    #[test]
    fn going_to_the_end_shows_the_newest_message_wherever_the_reader_was() {
        let mut chat = Chat::new(720.0, 300.0);
        let spoken = lines(8);
        let mut messages = talk(&spoken[..6]);
        chat.settle(&messages);
        chat.scroll_up(&messages, 150.0);
        let reading = chat.state.offset();
        messages = talk(&spoken[..7]);
        let output = chat.settle(&messages);
        assert_eq!(count_text(&output, "↓ 新しいメッセージ 1件"), 1);

        chat.state.go_to_end();
        let output = chat.settle(&messages);
        assert!(
            chat.state.offset() > reading + 50.0,
            "the view moved from {reading} to {}",
            chat.state.offset()
        );
        assert_eq!(count_text(&output, "7番目に確定した部分です。"), 1);
        assert!(!announces_new_messages(&output));

        // A chat that does not follow starts at its first message.
        let mut chat = Chat::new(720.0, 300.0);
        chat.options.follow = false;
        let output = chat.settle(&messages);
        assert_eq!(chat.state.offset(), 0.0);
        assert_eq!(count_text(&output, "7番目に確定した部分です。"), 0);
        chat.state.go_to_end();
        let output = chat.settle(&messages);
        assert_eq!(count_text(&output, "7番目に確定した部分です。"), 1);
        let end = chat.state.offset();
        messages = talk(&spoken[..8]);
        let output = chat.settle(&messages);
        assert_eq!(
            chat.state.offset(),
            end,
            "it went to the end once and does not follow"
        );
        assert_eq!(count_text(&output, "↓ 新しいメッセージ 1件"), 1);
    }

    #[test]
    fn the_newest_message_taller_than_the_chat_is_drawn_from_a_whole_line_while_the_chat_follows() {
        let long = "訳文が長く続いて、ウィンドウの高さを超えています。".repeat(8);
        let messages = [Message {
            status: Status::Forming,
            ..message(1, Source::PcAudio, &long, Moment::default())
        }];
        for (width, height) in [(400.0, 260.0), (520.0, 360.0), (720.0, 300.0)] {
            let mut chat = Chat::new(width, height);
            let output = chat.settle(&messages);
            let body = chat.state.body().unwrap();
            let (words, _) = find_text(&output, &long);
            assert!(
                words.top() < body.top(),
                "the message fits a chat of {width}x{height}, so nothing is tested"
            );
            assert!(
                !line_cut_at_the_top(&output, body),
                "a line is cut at the top of a chat of {width}x{height}"
            );
            let end = dots(&output);
            assert_eq!(end.len(), 3);
            assert!(
                end.iter().all(|(dot, _)| body.contains(*dot)),
                "the end of the message is not in view"
            );

            // A reader who scrolls by hand moves the lines past the edge.
            let output = chat.scroll_up(&messages, 10.0);
            assert!(
                line_cut_at_the_top(&output, body),
                "scrolling by hand jumps from line to line"
            );
        }
    }

    #[test]
    fn a_message_that_makes_the_chat_taller_asks_for_the_next_frame_at_once() {
        let mut chat = Chat::new(720.0, 300.0);
        let spoken = lines(8);
        chat.settle(&talk(&spoken[..6]));
        let (output, _) = chat.frame(&talk(&spoken[..6]), vec![]);
        assert!(
            output
                .viewport_output
                .values()
                .all(|viewport| !viewport.repaint_delay.is_zero()),
            "nothing changed, nothing to redraw"
        );
        let (output, _) = chat.frame(&talk(&spoken[..7]), vec![]);
        assert!(
            output
                .viewport_output
                .values()
                .any(|viewport| viewport.repaint_delay.is_zero()),
            "the view moves to the new end on the next frame"
        );
    }

    #[test]
    fn a_reader_returning_to_the_bottom_by_hand_resets_the_count() {
        let mut chat = Chat::new(720.0, 300.0);
        let spoken = lines(8);
        let mut messages = talk(&spoken[..6]);
        chat.settle(&messages);
        chat.scroll_up(&messages, 150.0);
        messages = talk(&spoken[..7]);
        let output = chat.settle(&messages);
        assert_eq!(count_text(&output, "↓ 新しいメッセージ 1件"), 1);
        let output = chat.scroll_up(&messages, -600.0);
        assert!(!announces_new_messages(&output));
        messages = talk(&spoken[..8]);
        let output = chat.settle(&messages);
        assert!(
            !announces_new_messages(&output),
            "back at the bottom, the chat follows again"
        );
        assert_eq!(count_text(&output, "8番目に確定した部分です。"), 1);
    }

    #[test]
    fn without_following_a_new_message_waits_behind_the_pill_until_following_is_turned_on() {
        let mut chat = Chat::new(720.0, 300.0);
        let spoken = lines(9);
        let mut messages = talk(&spoken[..6]);
        chat.settle(&messages);
        let end = chat.state.offset();
        chat.options.follow = false;
        messages = talk(&spoken[..7]);
        let output = chat.settle(&messages);
        assert_eq!(count_text(&output, "↓ 新しいメッセージ 1件"), 1);
        assert!(
            (chat.state.offset() - end).abs() < 1.0,
            "the view stays where it was"
        );
        chat.options.follow = true;
        let output = chat.settle(&messages);
        assert!(
            chat.state.offset() > end + 20.0,
            "following again jumps to the end"
        );
        assert_eq!(count_text(&output, "7番目に確定した部分です。"), 1);
        assert!(!announces_new_messages(&output));
    }

    #[test]
    fn an_empty_chat_says_what_it_is_for() {
        let mut chat = Chat::new(720.0, 460.0);
        let output = chat.settle(&[]);
        let hint = "PCで再生中の英語の音声を、ここにやさしい日本語で表示します。";
        let (text, color) = find_text(&output, hint);
        let panel = chat.state.panel().unwrap();
        assert!((text.center().x - panel.center().x).abs() < 1.0);
        assert!((text.center().y - panel.center().y).abs() < 6.0);
        assert_eq!(color, theme::CHAT_META);
        assert!(!filled(&output, theme::CHAT_BACKGROUND).is_empty());

        // The window may say what to do next, on a line of its own.
        chat.options.empty = "ここに訳が出ます。\n下のボタンを押すと始まります。";
        let output = chat.settle(&[]);
        assert_eq!(count_text(&output, hint), 0);
        let (text, _) = find_text(&output, chat.options.empty);
        assert!((text.center().x - panel.center().x).abs() < 1.0);
        assert!((text.center().y - panel.center().y).abs() < 6.0);
        assert!(panel.contains_rect(text));
    }

    #[test]
    fn an_unfinished_message_keeps_its_last_text_with_a_small_note_under_the_bubble() {
        let mut chat = Chat::new(720.0, 460.0);
        for (status, note) in [
            (Status::Failed, "更新に失敗 · 最後の訳文を表示"),
            (Status::Interrupted, "中断した文章 · 未確定"),
        ] {
            let output = chat.settle(&[Message {
                status,
                failure: Some("connection lost"),
                ..message(1, Source::YouTube, "途中までの訳文です。", at(21, 48, 0))
            }]);
            let bubble = bubble_of(&output, "途中までの訳文です。");
            let (text, color) = find_text(&output, note);
            assert!(text.top() >= bubble.bottom(), "under the bubble");
            assert_eq!(color, theme::CHAT_META);
            assert!(dots(&output).is_empty());
        }
        let output = chat.settle(&[Message {
            status: Status::Failed,
            ..message(1, Source::YouTube, "", at(21, 48, 0))
        }]);
        assert_eq!(count_text(&output, "訳文はありません。"), 1);
        assert_eq!(count_text(&output, "翻訳できなかった文章 · 未確定"), 1);
    }

    #[test]
    fn right_click_copies_and_reads_aloud_only_when_readout_is_in_use() {
        let mut chat = Chat::new(720.0, 460.0);
        let messages = [message(
            1,
            Source::YouTube,
            ">> こんにちは。>> やあ。",
            at(21, 48, 0),
        )];
        let output = chat.settle(&messages);
        let first = bubble_of(&output, "こんにちは。");

        let (menu, _) = chat.click(&messages, first.center(), PointerButton::Secondary);
        assert_eq!(count_text(&menu, "コピー"), 1);
        assert_eq!(count_text(&menu, "読み上げ"), 0, "readout is not in use");
        chat.click(&messages, egui::pos2(5.0, 5.0), PointerButton::Primary);

        chat.options.readout = Some(true);
        let output = chat.settle(&messages);
        let second = bubble_of(&output, "やあ。");
        let (menu, _) = chat.click(&messages, second.center(), PointerButton::Secondary);
        assert_eq!(count_text(&menu, "コピー"), 1);
        assert_eq!(
            count_text(&menu, "読み上げを停止"),
            1,
            "readout is speaking"
        );
        let (read, _) = find_text(&menu, "読み上げ");
        let (_, action) = chat.click(&messages, read.center(), PointerButton::Primary);
        assert!(
            matches!(action, Some(Action::Read(text)) if text == "やあ。"),
            "the text of the bubble itself is read, without the marker"
        );
    }

    #[test]
    fn copying_a_bubble_takes_its_translation_with_the_credit() {
        let mut chat = Chat::new(720.0, 460.0);
        let messages = [message(1, Source::PcAudio, "はじめまして。", at(21, 48, 0))];
        let output = chat.settle(&messages);
        let bubble = bubble_of(&output, "はじめまして。");
        let (menu, _) = chat.click(&messages, bubble.center(), PointerButton::Secondary);
        let (copy, _) = find_text(&menu, "コピー");
        assert!(
            chat.state.copied.is_none(),
            "nothing is copied by opening the menu"
        );
        chat.click(&messages, copy.center(), PointerButton::Primary);
        let copied = chat.state.copied.as_deref().expect("コピー was clicked");
        assert!(copied.starts_with("はじめまして。") && copied.contains(credit::CREDIT));
    }

    #[test]
    fn a_message_not_yet_fixed_cannot_be_read_aloud() {
        let mut chat = Chat::new(720.0, 460.0);
        chat.options.readout = Some(false);
        let messages = [Message {
            status: Status::Forming,
            ..message(1, Source::PcAudio, "まだ途中です", at(21, 48, 0))
        }];
        let output = chat.settle(&messages);
        let bubble = bubble_of(&output, "まだ途中です");
        let (menu, _) = chat.click(&messages, bubble.center(), PointerButton::Secondary);
        assert_eq!(
            count_text(&menu, "読み上げを停止"),
            0,
            "nothing is being read"
        );
        let (read, _) = find_text(&menu, "読み上げ");
        let (_, action) = chat.click(&messages, read.center(), PointerButton::Primary);
        assert!(action.is_none());
    }

    fn three() -> [Message<'static>; 3] {
        [
            message(1, Source::PcAudio, "はじめまして。", at(21, 48, 0)),
            message(2, Source::PcAudio, "よろしくお願いします。", at(21, 48, 1)),
            message(3, Source::PcAudio, "今日は晴れです。", at(21, 48, 2)),
        ]
    }

    /// Just before the first character of a drawn text, and just after its last.
    fn ends_of(output: &egui::FullOutput, text: &str) -> (Pos2, Pos2) {
        let (rect, _) = find_text(output, text);
        (
            egui::pos2(rect.left() - 4.0, rect.top() + 4.0),
            egui::pos2(rect.right() + 4.0, rect.bottom() - 4.0),
        )
    }

    #[test]
    fn dragging_over_bubbles_selects_their_text_and_ctrl_c_copies_it_with_the_credit() {
        let mut chat = Chat::new(720.0, 460.0);
        let messages = three();
        let output = chat.settle(&messages);
        let (from, _) = ends_of(&output, "はじめまして。");
        let (_, to) = ends_of(&output, "よろしくお願いします。");
        let output = chat.drag(&messages, from, to);
        assert_eq!(
            selected(&output),
            ["はじめまして。", "よろしくお願いします。"]
        );
        assert!(chat.state.copied.is_none(), "selecting copies nothing yet");
        chat.copy_keys(&messages);
        assert_eq!(
            chat.state.copied.as_deref(),
            Some(credit::published("はじめまして。\n\nよろしくお願いします。").as_str())
        );
    }

    #[test]
    fn a_selection_may_begin_and_end_inside_a_bubble() {
        let mut chat = Chat::new(720.0, 460.0);
        let messages = three();
        let output = chat.settle(&messages);
        let (first, _) = find_text(&output, "はじめまして。");
        let (second, _) = find_text(&output, "よろしくお願いします。");
        chat.drag(&messages, first.center(), second.center());
        chat.copy_keys(&messages);
        let copied = chat.state.copied.clone().expect("the selection is copied");
        let words = copied
            .strip_suffix(&format!("\n\n---\n{}", credit::CREDIT))
            .expect("the credit comes last");
        let (head, tail) = words.split_once("\n\n").expect("two messages");
        assert!(
            !head.is_empty() && head != "はじめまして。" && "はじめまして。".ends_with(head),
            "the end of the first message: {head}"
        );
        assert!(
            !tail.is_empty()
                && tail != "よろしくお願いします。"
                && "よろしくお願いします。".starts_with(tail),
            "the start of the second message: {tail}"
        );
    }

    #[test]
    fn the_menu_offers_to_copy_the_selection_only_when_there_is_one() {
        let mut chat = Chat::new(720.0, 460.0);
        let messages = three();
        let output = chat.settle(&messages);
        let second = bubble_of(&output, "よろしくお願いします。");
        let (menu, _) = chat.click(&messages, second.center(), PointerButton::Secondary);
        assert_eq!(
            count_text(&menu, "選択範囲をコピー"),
            0,
            "nothing is selected"
        );
        chat.click(&messages, egui::pos2(5.0, 5.0), PointerButton::Primary);

        let (from, _) = ends_of(&output, "はじめまして。");
        let (_, to) = ends_of(&output, "よろしくお願いします。");
        chat.drag(&messages, from, to);
        let (menu, _) = chat.click(&messages, second.center(), PointerButton::Secondary);
        assert_eq!(
            count_text(&menu, "コピー"),
            1,
            "the bubble's own copy stays"
        );
        let (item, _) = find_text(&menu, "選択範囲をコピー");
        let (output, _) = chat.click(&messages, item.center(), PointerButton::Primary);
        assert_eq!(
            chat.state.copied.as_deref(),
            Some(credit::published("はじめまして。\n\nよろしくお願いします。").as_str())
        );
        assert_eq!(
            selected(&output).len(),
            2,
            "the selection stays after copying"
        );
    }

    #[test]
    fn a_click_or_escape_lets_the_selection_go() {
        let mut chat = Chat::new(720.0, 460.0);
        let messages = three();
        let output = chat.settle(&messages);
        let (from, _) = ends_of(&output, "はじめまして。");
        let (_, to) = ends_of(&output, "よろしくお願いします。");

        chat.drag(&messages, from, to);
        let third = bubble_of(&output, "今日は晴れです。");
        let (output, _) = chat.click(&messages, third.center(), PointerButton::Primary);
        assert!(selected(&output).is_empty(), "a click on a bubble");
        chat.copy_keys(&messages);
        assert!(chat.state.copied.is_none(), "there is nothing to copy");

        chat.drag(&messages, from, to);
        let body = chat.state.body().expect("the chat has been drawn");
        let beside = egui::pos2(body.center().x, body.bottom() - 8.0);
        let (output, _) = chat.click(&messages, beside, PointerButton::Primary);
        assert!(selected(&output).is_empty(), "a click beside the bubbles");

        chat.drag(&messages, from, to);
        chat.frame(
            &messages,
            vec![Event::Key {
                key: egui::Key::Escape,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            }],
        );
        let (output, _) = chat.frame(&messages, vec![]);
        assert!(selected(&output).is_empty(), "Escape");
    }

    #[test]
    fn an_escape_that_closes_the_menu_keeps_the_selection_and_the_next_lets_it_go() {
        let escape = || Event::Key {
            key: egui::Key::Escape,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        };
        let mut chat = Chat::new(720.0, 460.0);
        let messages = three();
        let output = chat.settle(&messages);
        let (from, _) = ends_of(&output, "はじめまして。");
        let (_, to) = ends_of(&output, "よろしくお願いします。");
        chat.drag(&messages, from, to);
        let second = bubble_of(&output, "よろしくお願いします。");
        let (menu, _) = chat.click(&messages, second.center(), PointerButton::Secondary);
        assert_eq!(count_text(&menu, "選択範囲をコピー"), 1, "the menu is open");

        chat.frame(&messages, vec![escape()]);
        let (output, _) = chat.frame(&messages, vec![]);
        assert_eq!(
            count_text(&output, "選択範囲をコピー"),
            0,
            "Esc closes the menu"
        );
        assert_eq!(
            selected(&output),
            ["はじめまして。", "よろしくお願いします。"],
            "the Esc that closed the menu let go of the selection too"
        );

        chat.frame(&messages, vec![escape()]);
        let (output, _) = chat.frame(&messages, vec![]);
        assert!(
            selected(&output).is_empty(),
            "the next Esc lets go of the selection"
        );
    }

    #[test]
    fn a_selection_outlives_scrolling_out_of_sight() {
        let lines = lines(30);
        let messages = talk(&lines);
        let mut chat = Chat::new(600.0, 400.0);
        let output = chat.settle(&messages);
        let (from, _) = ends_of(&output, &lines[27]);
        let (_, to) = ends_of(&output, &lines[29]);
        chat.drag(&messages, from, to);
        let output = chat.scroll_up(&messages, 3000.0);
        assert!(
            !has_text_containing(&output, &lines[27]),
            "the selection is out of sight"
        );
        chat.copy_keys(&messages);
        assert_eq!(
            chat.state.copied.as_deref(),
            Some(credit::published(&lines[27..].join("\n\n")).as_str())
        );
    }

    #[test]
    fn dragging_above_the_chat_scrolls_it_and_selects_what_comes_into_sight() {
        let lines = lines(30);
        let messages = talk(&lines);
        let mut chat = Chat::new(600.0, 400.0);
        let output = chat.settle(&messages);
        let shown: Vec<&String> = lines
            .iter()
            .filter(|line| count_text(&output, line) == 1)
            .collect();
        let body = chat.state.body().expect("the chat has been drawn");
        let offset = chat.state.offset();
        let (_, start) = ends_of(&output, &lines[29]);
        let above = egui::pos2(start.x, body.top() - 30.0);
        chat.frame(&messages, vec![Event::PointerMoved(start)]);
        chat.frame(&messages, vec![press(start, PointerButton::Primary, true)]);
        for _ in 0..30 {
            chat.frame(&messages, vec![Event::PointerMoved(above)]);
        }
        chat.frame(&messages, vec![press(above, PointerButton::Primary, false)]);
        chat.frame(&messages, vec![]);
        assert!(
            chat.state.offset() < offset - 200.0,
            "the chat scrolled up: {offset} -> {}",
            chat.state.offset()
        );
        chat.copy_keys(&messages);
        let copied = chat.state.copied.clone().expect("the selection is copied");
        let first = copied.split("\n\n").next().expect("a first message");
        assert!(
            !shown.iter().any(|line| line.as_str() == first),
            "it begins with a message that was out of sight: {first}"
        );
        assert!(copied.contains(lines[29].as_str()));
    }

    #[test]
    fn a_selection_over_the_bubbles_of_one_paragraph_copies_it_unbroken() {
        let mut chat = Chat::new(720.0, 700.0);
        let messages = [message(1, Source::PcAudio, TALK, at(21, 48, 0))];
        let pieces = bubble_texts(TALK);
        assert!(pieces.len() >= 3, "the paragraph takes several bubbles");
        let output = chat.settle(&messages);
        let (from, _) = ends_of(&output, pieces[0]);
        let (_, to) = ends_of(&output, pieces[1]);
        chat.drag(&messages, from, to);
        chat.copy_keys(&messages);
        assert_eq!(
            chat.state.copied.as_deref(),
            Some(credit::published(&format!("{}{}", pieces[0], pieces[1])).as_str())
        );
    }

    #[test]
    fn copying_everything_takes_each_part_in_turn_and_the_originals_shown() {
        let mut chat = Chat::new(720.0, 460.0);
        let messages = [
            Message {
                original: "Nice to meet you.",
                ..message(1, Source::YouTube, "はじめまして。", at(21, 48, 0))
            },
            Message {
                original: ">> Hello. >> Hi.",
                ..message(
                    2,
                    Source::YouTube,
                    ">> こんにちは。>> やあ。",
                    at(21, 48, 1),
                )
            },
            Message::typing(Source::PcAudio, "and so"),
        ];
        chat.settle(&messages);
        chat.state.copy_all(&messages);
        assert_eq!(
            chat.state.copied.as_deref(),
            Some(credit::published("はじめまして。\n\nこんにちは。\n\nやあ。").as_str()),
            "speech still being recognised is left out"
        );

        chat.options.show_originals = true;
        chat.settle(&messages);
        chat.state.copy_all(&messages);
        assert_eq!(
            chat.state.copied.as_deref(),
            Some(
                credit::published(
                    "はじめまして。\nNice to meet you.\n\nこんにちは。\nHello.\n\nやあ。\nHi."
                )
                .as_str()
            )
        );
    }
}
