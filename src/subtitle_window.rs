//! Independent native subtitle viewport. This module never changes source,
//! translation, speech, root-window or Mini lifecycles.
use std::collections::HashMap;

use eframe::egui::{self, Color32, RichText};
use plamo_translate::live_text::{SourceKind, SourceStamp};

use super::{chat, theme};

const BACKGROUND: Color32 = theme::CANVAS;
const MUTED: Color32 = theme::TEXT_SECONDARY;
const ACCENT: Color32 = theme::ACCENT;
const WARNING: Color32 = theme::WARNING;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Updating,
    Final,
    Interrupted,
    Failed,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Playback {
    #[default]
    Running,
    Paused,
    Stopped,
}

/// A coherent source/target pair for one paragraph revision. The source must
/// describe this target, not a newer unaccepted source draft.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct View {
    pub stamp: SourceStamp,
    pub paragraph_key: u64,
    pub revision: u64,
    pub source: String,
    pub target: String,
    pub phase: Phase,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum UpdateOutcome {
    Accepted,
    ScopeCleared,
    RejectedScope,
    RejectedStale,
    #[default]
    Held,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct WindowResponse {
    /// Only this subtitle viewport was closed; the caller need not stop input,
    /// translation, speech, the root window or Mini.
    pub closed: bool,
}

#[derive(Clone, Debug)]
pub struct State {
    pub open: bool,
    pub always_on_top: bool,
    /// Show the original under every translation, not only the clicked ones.
    pub show_source: bool,
    pub font_size: f32,
    /// Leave screen captures while the 画面 input reads the screen.
    pub content_protected: bool,
    applied_protection: Option<bool>,
    scope: Option<SourceStamp>,
    view: Option<View>,
    /// Parts whose translation is final, oldest first. They are drawn as
    /// separate messages that never change, above the part still forming.
    history: Vec<View>,
    /// When each part was said, by paragraph key. The views carry no times.
    moments: HashMap<u64, chat::Moment>,
    playback: Playback,
    waiting_for_update: bool,
    pub(super) chat: chat::State,
    native_size: egui::Vec2,
    native_position: Option<egui::Pos2>,
}

impl Default for State {
    fn default() -> Self {
        Self {
            open: false,
            always_on_top: true,
            show_source: false,
            font_size: 25.0,
            content_protected: false,
            applied_protection: None,
            scope: None,
            view: None,
            history: Vec::new(),
            moments: HashMap::new(),
            playback: Playback::Running,
            waiting_for_update: false,
            chat: chat::State::default(),
            native_size: egui::vec2(720.0, 360.0),
            native_position: None,
        }
    }
}

impl State {
    pub fn viewport_id() -> egui::ViewportId {
        // Keep the old overlay's native identity, independently of the root Mini.
        egui::ViewportId::from_hash_of("subtitle-overlay")
    }

    #[cfg(test)]
    pub fn view(&self) -> Option<&View> {
        self.view.as_ref()
    }

    pub fn scope(&self) -> Option<&SourceStamp> {
        self.scope.as_ref()
    }

    #[cfg(test)]
    pub fn history(&self) -> &[View] {
        &self.history
    }

    /// The caller passes the fixed parts of the current source only.
    pub fn set_history(&mut self, history: Vec<View>) {
        self.history = history;
    }

    /// The paragraph keys of every part the window shows.
    pub fn paragraph_keys(&self) -> impl Iterator<Item = u64> + '_ {
        self.history
            .iter()
            .chain(&self.view)
            .map(|view| view.paragraph_key)
    }

    pub fn set_moments(&mut self, moments: HashMap<u64, chat::Moment>) {
        self.moments = moments;
    }

    #[cfg(test)]
    pub fn scroll_offset(&self) -> f32 {
        self.chat.offset()
    }

    #[cfg(test)]
    pub fn playback(&self) -> Playback {
        self.playback
    }

    /// `expected_scope` is the caller's active source identity. Changing it
    /// clears the old pair immediately, even before the new translation exists.
    /// None retains the current scope, useful while paused/stopped. Neither a
    /// missing snapshot nor elapsed time expires a valid translation.
    pub fn update(
        &mut self,
        expected_scope: Option<&SourceStamp>,
        view: Option<&View>,
        playback: Playback,
    ) -> UpdateOutcome {
        self.playback = playback;
        let changed_scope = expected_scope.is_some_and(|stamp| self.scope.as_ref() != Some(stamp));
        if changed_scope {
            self.scope = expected_scope.cloned();
            self.view = None;
            self.history.clear();
            self.moments.clear();
            self.waiting_for_update = false;
            self.chat.clear();
        }
        let Some(incoming) = view else {
            return if changed_scope {
                UpdateOutcome::ScopeCleared
            } else {
                UpdateOutcome::Held
            };
        };
        if self
            .scope
            .as_ref()
            .is_some_and(|stamp| stamp != &incoming.stamp)
        {
            return UpdateOutcome::RejectedScope;
        }
        if self.view.as_ref().is_some_and(|current| {
            (incoming.paragraph_key, incoming.revision) < (current.paragraph_key, current.revision)
                || (incoming.paragraph_key, incoming.revision)
                    == (current.paragraph_key, current.revision)
                    && current.phase == Phase::Final
                    && incoming.phase != Phase::Final
        }) {
            return UpdateOutcome::RejectedStale;
        }
        if incoming.target.trim().is_empty() {
            // Never pair a new empty/source-only draft with the old target.
            self.waiting_for_update = true;
            return if changed_scope {
                UpdateOutcome::ScopeCleared
            } else {
                UpdateOutcome::Held
            };
        }
        self.scope.get_or_insert_with(|| incoming.stamp.clone());
        self.waiting_for_update = false;
        self.view = Some(incoming.clone());
        UpdateOutcome::Accepted
    }

    /// Closing is deliberately local presentation state only. Retain the pair,
    /// preferences and reading position so reopening resumes the same view.
    pub fn close(&mut self) -> WindowResponse {
        self.open = false;
        WindowResponse { closed: true }
    }

    pub fn builder(&self) -> egui::ViewportBuilder {
        let mut builder = egui::ViewportBuilder::default()
            .with_title("地元翻訳くん · 字幕ウィンドウ")
            .with_inner_size(self.native_size)
            .with_min_inner_size([360.0, 220.0])
            .with_resizable(true)
            .with_active(false)
            .with_window_level(if self.always_on_top {
                egui::WindowLevel::AlwaysOnTop
            } else {
                egui::WindowLevel::Normal
            });
        if let Some(position) = self.native_position {
            builder = builder.with_position(position);
        }
        builder
    }

    /// Native secondary window. No focus/attention requests are issued.
    /// Selecting/copying text can still focus it through ordinary user input.
    pub fn show(&mut self, ctx: &egui::Context) -> WindowResponse {
        if !self.open {
            self.applied_protection = None;
            return WindowResponse::default();
        }
        let mut response = WindowResponse::default();
        ctx.show_viewport_immediate(Self::viewport_id(), self.builder(), |child, _class| {
            // egui has no builder field for this, and a command sent before the
            // window exists is dropped, so the window applies it itself.
            if self.applied_protection != Some(self.content_protected) {
                child.send_viewport_cmd(egui::ViewportCommand::ContentProtected(
                    self.content_protected,
                ));
                self.applied_protection = Some(self.content_protected);
            }
            Self::panel().show(child, |ui| {
                response = self.paint(ui);
            });
            if child.input(|input| input.viewport().close_requested()) {
                response = self.close();
            }
            if response.closed {
                // Remember the geometry only when closing. egui turns a builder
                // that follows the open window into a resize or move that runs
                // after the next frame has read the window, which swaps it
                // between its last two sizes for as long as frames are drawn.
                child.input(|input| {
                    if let Some(inner) = input.viewport().inner_rect {
                        self.native_size = inner.size().max(egui::vec2(360.0, 220.0));
                    }
                    if let Some(outer) = input.viewport().outer_rect {
                        self.native_position = Some(outer.min);
                    }
                });
                // This command is scoped to the native child context, never root.
                child.send_viewport_cmd(egui::ViewportCommand::Close);
            }
        });
        response
    }

    /// The native window's panel. Previews and tests draw through it too.
    pub fn panel() -> egui::CentralPanel {
        egui::CentralPanel::default().frame(egui::Frame::new().fill(BACKGROUND).inner_margin(12))
    }

    /// Headless-previewable content; emits no native close/focus commands.
    pub fn paint(&mut self, ui: &mut egui::Ui) -> WindowResponse {
        let mut closed = false;
        // The chat is what the window is for, so the rows above it sit close.
        ui.spacing_mut().item_spacing.y = theme::SPACE_1;
        ui.horizontal(|ui| {
            ui.checkbox(&mut self.always_on_top, "最前面");
            ui.menu_button("表示", |ui| {
                ui.label(
                    RichText::new("文字サイズ")
                        .size(theme::SIZE_LABEL)
                        .color(MUTED),
                );
                theme::slider(
                    ui,
                    egui::Slider::new(
                        &mut self.font_size,
                        theme::READING_MIN..=theme::SUBTITLE_READING_MAX,
                    )
                    .integer()
                    .suffix(" pt"),
                );
                ui.checkbox(&mut self.show_source, "原文をすべて表示");
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                closed = ui
                    .add(
                        egui::Button::new(RichText::new("閉じる").size(theme::SIZE_LABEL))
                            .min_size(egui::vec2(72.0, theme::CONTROL_H)),
                    )
                    .clicked();
            });
        });
        let (label, color) = self.status();
        ui.scope(|ui| {
            ui.spacing_mut().interact_size.y = 0.0;
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new(label).size(theme::SIZE_LABEL).color(color));
                if let Some(view) = &self.view {
                    ui.label(
                        RichText::new(match source_of(&view.stamp) {
                            chat::Source::PcAudio => "PC音声",
                            chat::Source::Screen => "画面",
                            chat::Source::YouTube => "YouTube字幕",
                        })
                        .size(theme::SIZE_CAPTION)
                        .color(MUTED),
                    );
                }
            });
        });
        let messages = messages(&self.history, self.view.as_ref(), &self.moments);
        self.chat.show(
            ui,
            "subtitle-window-chat",
            &messages,
            chat::Options {
                text_size: self
                    .font_size
                    .clamp(theme::READING_MIN, theme::SUBTITLE_READING_MAX),
                show_originals: self.show_source,
                follow: true,
                readout: None,
            },
        );
        if closed {
            self.close()
        } else {
            WindowResponse::default()
        }
    }

    fn status(&self) -> (String, Color32) {
        let (phase, color) = match self.view.as_ref().map(|view| view.phase) {
            Some(Phase::Updating) => ("訳文を更新中 · 未確定", WARNING),
            Some(Phase::Final) => ("確定した文章", ACCENT),
            Some(Phase::Interrupted) => ("中断した文章 · 未確定", WARNING),
            Some(Phase::Failed) => ("更新に失敗 · 最後の訳文を表示", WARNING),
            None => ("入力待ち", MUTED),
        };
        let mut label = phase.to_owned();
        match self.playback {
            Playback::Running => {}
            Playback::Paused => label.push_str(" · 一時停止中"),
            Playback::Stopped => label.push_str(" · 入力停止中"),
        }
        if self.waiting_for_update {
            label.push_str(" · 新しい訳文を待っています");
        }
        (label, color)
    }
}

/// The fixed parts, then the part still forming unless it is already fixed.
fn messages<'a>(
    history: &'a [View],
    view: Option<&'a View>,
    moments: &HashMap<u64, chat::Moment>,
) -> Vec<chat::Message<'a>> {
    let forming = view.filter(|view| {
        !history
            .iter()
            .any(|done| done.paragraph_key == view.paragraph_key)
    });
    history
        .iter()
        .chain(forming)
        .map(|view| chat::Message {
            id: view.paragraph_key,
            source: source_of(&view.stamp),
            target: &view.target,
            original: &view.source,
            moment: moments
                .get(&view.paragraph_key)
                .copied()
                .unwrap_or_default(),
            status: match view.phase {
                Phase::Updating => chat::Status::Forming,
                Phase::Final => chat::Status::Fixed,
                Phase::Interrupted => chat::Status::Interrupted,
                Phase::Failed => chat::Status::Failed,
            },
            failure: None,
        })
        .collect()
}

fn source_of(stamp: &SourceStamp) -> chat::Source {
    match stamp.kind {
        SourceKind::Audio => chat::Source::PcAudio,
        SourceKind::Caption
            if stamp
                .video_id
                .starts_with(super::screen_feed::SCREEN_SOURCE) =>
        {
            chat::Source::Screen
        }
        SourceKind::Caption => chat::Source::YouTube,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn stamp(video: &str, epoch: u64) -> SourceStamp {
        SourceStamp {
            kind: SourceKind::Caption,
            generation: 1,
            video_id: video.into(),
            epoch,
        }
    }
    fn view(key: u64, revision: u64, phase: Phase) -> View {
        View {
            stamp: stamp("video-a", 0),
            paragraph_key: key,
            revision,
            source: "A coherent English paragraph with 150 words about a public exhibition.".into(),
            target: "図書館の展示について、まとまった文章を日本語で表示します。".into(),
            phase,
        }
    }
    fn paint(state: &mut State, ctx: &egui::Context, size: egui::Vec2) -> egui::FullOutput {
        paint_with(state, ctx, size, vec![])
    }
    fn paint_with(
        state: &mut State,
        ctx: &egui::Context,
        size: egui::Vec2,
        events: Vec<egui::Event>,
    ) -> egui::FullOutput {
        ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
                events,
                ..Default::default()
            },
            |ctx| {
                State::panel().show(ctx, |ui| {
                    state.paint(ui);
                });
            },
        )
    }
    /// A turn of the mouse wheel over the reading area, then frames until the
    /// scrolling has stopped. Positive scrolls up.
    fn turn_wheel(state: &mut State, ctx: &egui::Context, size: egui::Vec2, pixels: f32) {
        let pointer = state.chat.body().expect("drawn").center();
        paint_with(state, ctx, size, vec![egui::Event::PointerMoved(pointer)]);
        paint_with(
            state,
            ctx,
            size,
            vec![egui::Event::MouseWheel {
                unit: egui::MouseWheelUnit::Point,
                delta: egui::vec2(0.0, pixels),
                modifiers: egui::Modifiers::NONE,
            }],
        );
        for _ in 0..30 {
            paint(state, ctx, size);
        }
    }
    fn painted_text(output: &egui::FullOutput) -> String {
        fn add(shape: &egui::Shape, text: &mut String) {
            match shape {
                egui::Shape::Text(t) => text.push_str(t.galley.text()),
                egui::Shape::Vec(shapes) => {
                    for s in shapes {
                        add(s, text)
                    }
                }
                _ => {}
            }
        }
        let mut text = String::new();
        for shape in &output.shapes {
            add(&shape.shape, &mut text);
        }
        text
    }
    fn show_native(
        state: &mut State,
        ctx: &egui::Context,
        outer_min: egui::Pos2,
        inner_size: egui::Vec2,
        events: Vec<egui::ViewportEvent>,
    ) {
        let info = egui::ViewportInfo {
            inner_rect: Some(egui::Rect::from_min_size(
                outer_min + egui::vec2(8.0, 31.0),
                inner_size,
            )),
            outer_rect: Some(egui::Rect::from_min_size(
                outer_min,
                inner_size + egui::vec2(16.0, 39.0),
            )),
            events,
            ..Default::default()
        };
        let _ = ctx.run(
            egui::RawInput {
                viewports: std::iter::once((egui::ViewportId::ROOT, info)).collect(),
                ..Default::default()
            },
            |ctx| {
                state.show(ctx);
            },
        );
    }
    #[test]
    fn builder_is_resizable_non_focusing_independent_native_window() {
        let state = State::default();
        let builder = state.builder();
        assert_eq!(builder.inner_size, Some(egui::vec2(720.0, 360.0)));
        assert_eq!(builder.min_inner_size, Some(egui::vec2(360.0, 220.0)));
        assert_eq!(builder.active, Some(false));
        assert_eq!(builder.resizable, Some(true));
        assert_eq!(builder.window_level, Some(egui::WindowLevel::AlwaysOnTop));
        assert_ne!(State::viewport_id(), egui::ViewportId::ROOT);
        assert!(!state.show_source);
    }
    #[test]
    fn snapshot_is_retained_without_expiration_and_while_paused() {
        let mut state = State::default();
        let original = view(1, 1, Phase::Updating);
        assert_eq!(
            state.update(Some(&original.stamp), Some(&original), Playback::Running),
            UpdateOutcome::Accepted
        );
        for _ in 0..100 {
            state.update(None, None, Playback::Paused);
        }
        assert_eq!(state.view(), Some(&original));
        assert_eq!(state.playback(), Playback::Paused);
        assert!(state.status().0.contains("未確定"));
        assert!(state.status().0.contains("一時停止中"));
    }
    #[test]
    fn scope_changes_clear_pairs_and_reject_old_video_epoch_generation() {
        let mut state = State::default();
        let original = view(1, 1, Phase::Final);
        state.update(Some(&original.stamp), Some(&original), Playback::Running);
        for next in [
            stamp("video-b", 0),
            stamp("video-a", 1),
            SourceStamp {
                generation: 2,
                ..stamp("video-a", 0)
            },
        ] {
            assert_eq!(
                state.update(Some(&next), Some(&original), Playback::Running),
                UpdateOutcome::RejectedScope
            );
            assert!(state.view().is_none());
            assert_eq!(state.scope(), Some(&next));
        }
    }
    #[test]
    fn revisions_and_keys_do_not_regress_or_turn_final_into_provisional() {
        let mut state = State::default();
        let current = view(3, 4, Phase::Final);
        state.update(Some(&current.stamp), Some(&current), Playback::Running);
        for old in [
            view(2, 99, Phase::Final),
            view(3, 3, Phase::Final),
            view(3, 4, Phase::Updating),
        ] {
            assert_eq!(
                state.update(None, Some(&old), Playback::Running),
                UpdateOutcome::RejectedStale
            );
        }
        assert_eq!(state.view(), Some(&current));
    }
    #[test]
    fn empty_new_draft_does_not_pair_new_source_with_old_target() {
        let mut state = State::default();
        let original = view(1, 1, Phase::Final);
        state.update(Some(&original.stamp), Some(&original), Playback::Running);
        let mut draft = view(2, 1, Phase::Updating);
        draft.target.clear();
        draft.source = "Unrelated newly arriving source".into();
        assert_eq!(
            state.update(None, Some(&draft), Playback::Running),
            UpdateOutcome::Held
        );
        assert_eq!(state.view(), Some(&original));
    }
    #[test]
    fn close_only_changes_local_open_and_preserves_reading_state() {
        let mut state = State {
            open: true,
            show_source: true,
            ..State::default()
        };
        let original = view(1, 1, Phase::Final);
        state.update(Some(&original.stamp), Some(&original), Playback::Running);
        state.chat.toggle_original(1, 0);
        state.chat.reading_from_start(0);
        let reading = format!("{:?}", state.chat);
        assert!(state.close().closed);
        assert!(!state.open);
        assert_eq!(state.view(), Some(&original));
        assert!(state.show_source);
        assert_eq!(
            format!("{:?}", state.chat),
            reading,
            "what the reader opened and where the reader stopped are kept"
        );
    }
    #[test]
    fn minimum_headless_window_renders_provisional_and_paused_without_root_close() {
        let ctx = egui::Context::default();
        let mut state = State {
            open: true,
            ..State::default()
        };
        let original = view(1, 1, Phase::Updating);
        state.update(Some(&original.stamp), Some(&original), Playback::Paused);
        let output = paint(&mut state, &ctx, egui::vec2(360.0, 220.0));
        let text = painted_text(&output);
        assert!(text.contains("未確定"));
        assert!(text.contains("一時停止中"));
        assert!(
            state.chat.body().unwrap().height() > 120.0,
            "room for a message of two lines at the default size"
        );
        assert!(
            output
                .viewport_output
                .values()
                .all(|v| !v.commands.contains(&egui::ViewportCommand::Close))
        );
    }
    #[test]
    fn long_text_is_scrollable_and_read_position_survives_revision() {
        let ctx = egui::Context::default();
        let size = egui::vec2(360.0, 220.0);
        let mut state = State::default();
        let mut original = view(1, 1, Phase::Updating);
        original.target =
            "A long readable translated paragraph stays here while additional words arrive. "
                .repeat(80);
        state.update(Some(&original.stamp), Some(&original), Playback::Running);
        for _ in 0..3 {
            paint(&mut state, &ctx, size);
        }
        let end = state.scroll_offset();
        assert!(end > 500.0, "the text is far taller than the window: {end}");
        turn_wheel(&mut state, &ctx, size, 300.0);
        let before = state.scroll_offset();
        assert!(before > 100.0 && before < end - 100.0, "{before} of {end}");
        original.revision = 2;
        original.target.push_str("The next sentence arrives.");
        state.update(None, Some(&original), Playback::Running);
        for _ in 0..3 {
            paint(&mut state, &ctx, size);
        }
        assert!((state.scroll_offset() - before).abs() < 1.0);
    }
    #[test]
    fn a_message_of_two_lines_is_drawn_whole_in_the_minimum_window() {
        let ctx = egui::Context::default();
        let size = egui::vec2(360.0, 220.0);
        let mut state = State::default();
        let forming = View {
            target: "お名前は必要ありません。".into(),
            ..view(1, 1, Phase::Updating)
        };
        state.update(Some(&forming.stamp), Some(&forming), Playback::Running);
        let mut output = paint(&mut state, &ctx, size);
        for _ in 0..3 {
            output = paint(&mut state, &ctx, size);
        }
        assert!(
            !cut_at_the_top(&state, &output),
            "nothing of the message is cut at the top"
        );
        let drawn = painted_text(&output);
        assert!(drawn.contains("お名前は必要ありません。"));
        assert_eq!(
            drawn.matches("YouTube").count(),
            2,
            "the status line and the name above the message: {drawn}"
        );

        // A time under the bubble takes the room of the name, which is left
        // out instead of being drawn half.
        state.set_moments(
            [(
                1,
                chat::Moment::from_record("2026-11-07T21:49:52+09:00", 0, chat::Source::YouTube),
            )]
            .into(),
        );
        for _ in 0..3 {
            output = paint(&mut state, &ctx, size);
        }
        let drawn = painted_text(&output);
        assert!(drawn.contains("お名前は必要ありません。") && drawn.contains("21:49"));
        assert_eq!(
            drawn.matches("YouTube").count(),
            1,
            "only the status line: {drawn}"
        );
        assert!(
            !cut_at_the_top(&state, &output),
            "no avatar is drawn cut at the top"
        );
    }

    #[test]
    fn a_dragged_window_is_never_moved_back_and_reopens_where_it_was_left() {
        let ctx = egui::Context::default();
        let mut state = State {
            open: true,
            ..State::default()
        };
        // eframe patches the window it created with every later builder, and
        // runs the resulting commands after that frame has read the window.
        let mut window = state.builder();
        for (x, width) in [(100.0, 726.0), (100.0, 732.0), (112.0, 732.0)] {
            show_native(
                &mut state,
                &ctx,
                egui::pos2(x, 80.0),
                egui::vec2(width, 360.0),
                vec![],
            );
            let (commands, recreate) = window.patch(state.builder());
            assert!(commands.is_empty() && !recreate, "{commands:?}");
        }
        show_native(
            &mut state,
            &ctx,
            egui::pos2(112.0, 80.0),
            egui::vec2(732.0, 360.0),
            vec![egui::ViewportEvent::Close],
        );
        assert!(!state.open);
        state.open = true;
        let reopened = state.builder();
        assert_eq!(reopened.inner_size, Some(egui::vec2(732.0, 360.0)));
        assert_eq!(reopened.position, Some(egui::pos2(112.0, 80.0)));
    }
    #[test]
    fn fixed_parts_stay_as_separate_messages_while_the_next_part_updates() {
        let ctx = egui::Context::default();
        let mut state = State::default();
        let first = View {
            target: "最初に確定した部分です。".into(),
            ..view(1, 2, Phase::Final)
        };
        let second = View {
            target: "次に確定した部分です。".into(),
            ..view(2, 1, Phase::Final)
        };
        let mut forming = View {
            target: "いま話している途中".into(),
            ..view(3, 1, Phase::Updating)
        };
        state.update(Some(&forming.stamp), Some(&forming), Playback::Running);
        state.set_history(vec![first.clone(), second.clone()]);
        let drawn = painted_text(&paint(&mut state, &ctx, egui::vec2(720.0, 360.0)));
        let at = |text: &str| {
            drawn
                .find(text)
                .unwrap_or_else(|| panic!("{text} is not drawn"))
        };
        assert!(
            at(&first.target) < at(&second.target) && at(&second.target) < at(&forming.target),
            "the oldest part comes first and the part still forming comes last"
        );

        forming.revision = 2;
        forming.target = "いま話している途中の続きです".into();
        state.update(None, Some(&forming), Playback::Running);
        let drawn = painted_text(&paint(&mut state, &ctx, egui::vec2(720.0, 360.0)));
        assert!(
            drawn.contains(&first.target) && drawn.contains(&second.target),
            "fixed parts do not change while the next part updates"
        );
        assert!(
            drawn.contains(&forming.target),
            "the forming part updates in place"
        );

        let done = View {
            phase: Phase::Final,
            ..forming.clone()
        };
        state.update(None, Some(&done), Playback::Running);
        state.set_history(vec![first, second, done.clone()]);
        let drawn = painted_text(&paint(&mut state, &ctx, egui::vec2(720.0, 360.0)));
        assert_eq!(
            drawn.matches(done.target.as_str()).count(),
            1,
            "a part that becomes fixed is drawn once"
        );
    }
    #[test]
    fn a_long_fixed_part_is_cut_into_bubbles_as_in_the_chat_and_a_part_still_forming_is_not() {
        let ctx = egui::Context::default();
        let size = egui::vec2(720.0, 1600.0);
        let bubbles =
            |output: &egui::FullOutput| chat::testing::filled(output, theme::CHAT_BUBBLE).len();
        let forming = View {
            target: ("あ".repeat(39) + "。").repeat(3),
            ..view(1, 1, Phase::Updating)
        };
        let mut state = State::default();
        state.update(Some(&forming.stamp), Some(&forming), Playback::Running);
        let mut output = paint(&mut state, &ctx, size);
        for _ in 0..3 {
            output = paint(&mut state, &ctx, size);
        }
        assert_eq!(bubbles(&output), 1, "a part still forming is one bubble");

        let done = View {
            phase: Phase::Final,
            ..forming
        };
        state.update(None, Some(&done), Playback::Running);
        state.set_history(vec![done]);
        for _ in 0..4 {
            output = paint(&mut state, &ctx, size);
        }
        assert!(
            bubbles(&output) > 1,
            "once fixed, it is cut at the ends of sentences"
        );
    }
    // Shapes inside the reading area are clipped to it; the header is not.
    fn cut_at_the_top(state: &State, output: &egui::FullOutput) -> bool {
        chat::testing::cut_at_the_top(output, state.chat.body().unwrap())
    }
    fn talk(state: &mut State, newest: &str) {
        let forming = View {
            target: newest.into(),
            ..view(9, 1, Phase::Updating)
        };
        state.update(Some(&forming.stamp), Some(&forming), Playback::Running);
        state.set_history(
            (1..=8)
                .map(|key| View {
                    target: format!("{key}番目に確定した部分です。前の話の続きを確認できます。"),
                    ..view(key, 1, Phase::Final)
                })
                .collect(),
        );
    }
    #[test]
    fn at_the_newest_message_no_message_is_drawn_cut_at_the_top() {
        let ctx = egui::Context::default();
        let size = egui::vec2(720.0, 360.0);
        let mut state = State::default();
        talk(&mut state, "いま話している途中の部分です。");
        let mut output = paint(&mut state, &ctx, size);
        for _ in 0..3 {
            output = paint(&mut state, &ctx, size);
        }
        assert!(
            state.scroll_offset() > 0.0,
            "the talk is taller than the window"
        );
        assert!(!cut_at_the_top(&state, &output));
        assert!(painted_text(&output).contains("いま話している途中の部分です。"));

        let long = "訳文が長く続いて、ウィンドウの高さを超えています。".repeat(12);
        talk(&mut state, &long);
        for _ in 0..3 {
            output = paint(&mut state, &ctx, size);
        }
        assert!(
            painted_text(&output).contains(&long),
            "the newest message is drawn even when it is taller than the window"
        );
    }
    #[test]
    fn header_buttons_do_not_overlap_at_the_minimum_size() {
        fn flatten<'a>(shape: &'a egui::Shape, shapes: &mut Vec<&'a egui::Shape>) {
            match shape {
                egui::Shape::Vec(inner) => inner.iter().for_each(|s| flatten(s, shapes)),
                shape => shapes.push(shape),
            }
        }
        let ctx = egui::Context::default();
        crate::ui::configure_theme(&ctx);
        let mut state = State::default();
        let output = paint(&mut state, &ctx, egui::vec2(360.0, 220.0));
        let mut shapes = Vec::new();
        output
            .shapes
            .iter()
            .for_each(|clipped| flatten(&clipped.shape, &mut shapes));
        let button = |label: &str| {
            let text = shapes
                .iter()
                .find_map(|shape| match shape {
                    egui::Shape::Text(text) if text.galley.text() == label => {
                        Some(shape.visual_bounding_rect())
                    }
                    _ => None,
                })
                .unwrap_or_else(|| panic!("{label} is not drawn"));
            shapes
                .iter()
                .filter_map(|shape| match shape {
                    egui::Shape::Rect(frame) if frame.rect.contains_rect(text) => Some(frame.rect),
                    _ => None,
                })
                .min_by(|a, b| a.area().total_cmp(&b.area()))
                .unwrap_or_else(|| panic!("{label} has no button frame"))
        };
        assert!(
            !button("表示").intersects(button("閉じる")),
            "閉じる covers 表示"
        );
        for gone in ["原文", "末尾へ", "末尾に追従する"] {
            assert!(
                !shapes.iter().any(|shape| matches!(
                    shape,
                    egui::Shape::Text(text) if text.galley.text() == gone
                )),
                "{gone} is still in the toolbar"
            );
        }
    }
    #[test]
    fn scrolling_up_by_hand_still_shows_the_message_at_the_top_edge() {
        let ctx = egui::Context::default();
        let size = egui::vec2(720.0, 360.0);
        let mut state = State::default();
        talk(&mut state, "いま話している途中の部分です。");
        for _ in 0..3 {
            paint(&mut state, &ctx, size);
        }
        let end = state.scroll_offset();
        turn_wheel(&mut state, &ctx, size, 30.0);
        assert!(
            (end - state.scroll_offset() - 30.0).abs() < 1.5,
            "the wheel moved the view by 30 points: {end} to {}",
            state.scroll_offset()
        );
        let output = paint(&mut state, &ctx, size);
        assert!(
            cut_at_the_top(&state, &output),
            "a message is drawn while it scrolls past the top"
        );
    }

    #[test]
    fn every_original_is_shown_from_the_view_menu_and_one_click_opens_one_original() {
        let ctx = egui::Context::default();
        let size = egui::vec2(720.0, 460.0);
        let mut state = State::default();
        let first = View {
            source: "First original.".into(),
            ..view(1, 1, Phase::Final)
        };
        let second = View {
            source: "Second original.".into(),
            ..view(2, 1, Phase::Final)
        };
        state.update(Some(&second.stamp), Some(&second), Playback::Running);
        state.set_history(vec![first, second]);
        let drawn = painted_text(&paint(&mut state, &ctx, size));
        assert!(!drawn.contains("First original.") && !drawn.contains("Second original."));
        assert!(
            !drawn.contains("原文"),
            "no checkbox for the originals in the toolbar"
        );
        state.chat.toggle_original(1, 0);
        for _ in 0..2 {
            paint(&mut state, &ctx, size);
        }
        let drawn = painted_text(&paint(&mut state, &ctx, size));
        assert!(drawn.contains("First original.") && !drawn.contains("Second original."));
        state.show_source = true;
        for _ in 0..2 {
            paint(&mut state, &ctx, size);
        }
        let drawn = painted_text(&paint(&mut state, &ctx, size));
        assert!(
            drawn.contains("First original.") && drawn.contains("Second original."),
            "the view menu shows every original"
        );
        state.chat.toggle_original(1, 0);
        for _ in 0..2 {
            paint(&mut state, &ctx, size);
        }
        let drawn = painted_text(&paint(&mut state, &ctx, size));
        assert!(
            drawn.contains("Second original.") && !drawn.contains("First original."),
            "with every original shown, a click closes that one"
        );
    }

    #[test]
    fn the_window_shows_when_each_part_was_said_from_the_rows() {
        let ctx = egui::Context::default();
        let size = egui::vec2(720.0, 460.0);
        let mut state = State::default();
        let first = view(1, 1, Phase::Final);
        let forming = view(2, 1, Phase::Updating);
        state.update(Some(&forming.stamp), Some(&forming), Playback::Running);
        state.set_history(vec![first]);
        assert_eq!(state.paragraph_keys().collect::<Vec<_>>(), [1, 2]);
        let drawn = painted_text(&paint(&mut state, &ctx, size));
        assert!(!drawn.contains("21:") && !drawn.contains("11月"));
        state.set_moments(
            [
                (
                    1,
                    chat::Moment::from_record(
                        "2026-11-07T21:46:10+09:00",
                        0,
                        chat::Source::YouTube,
                    ),
                ),
                (
                    2,
                    chat::Moment::from_record(
                        "2026-11-07T21:49:52+09:00",
                        0,
                        chat::Source::YouTube,
                    ),
                ),
            ]
            .into(),
        );
        let drawn = painted_text(&paint(&mut state, &ctx, size));
        for shown in ["21:46", "21:49", "11月7日（土）"] {
            assert!(drawn.contains(shown), "{shown} is not drawn: {drawn}");
        }
        assert_eq!(
            drawn.matches("YouTube").count(),
            3,
            "two groups (names) and the status label: {drawn}"
        );
    }
}
