//! Independent native subtitle viewport. This module never changes source,
//! translation, speech, root-window or Mini lifecycles.
use eframe::egui::{self, Color32, RichText};
use plamo_translate::live_text::{SourceKind, SourceStamp};

use super::theme;

const BACKGROUND: Color32 = theme::CANVAS;
const SURFACE: Color32 = theme::SURFACE;
const TEXT: Color32 = theme::TEXT;
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
    pub show_source: bool,
    pub font_size: f32,
    pub follow_latest: bool,
    scope: Option<SourceStamp>,
    view: Option<View>,
    playback: Playback,
    waiting_for_update: bool,
    scroll_offset: f32,
    jump_to_end: bool,
    native_size: egui::Vec2,
    native_position: Option<egui::Pos2>,
    #[cfg(test)]
    body_rect: Option<egui::Rect>,
}

impl Default for State {
    fn default() -> Self {
        Self {
            open: false,
            always_on_top: true,
            show_source: false,
            font_size: 25.0,
            follow_latest: false,
            scope: None,
            view: None,
            playback: Playback::Running,
            waiting_for_update: false,
            scroll_offset: 0.0,
            jump_to_end: false,
            native_size: egui::vec2(720.0, 360.0),
            native_position: None,
            #[cfg(test)]
            body_rect: None,
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
    pub fn scroll_offset(&self) -> f32 {
        self.scroll_offset
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
            self.waiting_for_update = false;
            self.scroll_offset = 0.0;
            self.jump_to_end = false;
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
            return WindowResponse::default();
        }
        let mut response = WindowResponse::default();
        ctx.show_viewport_immediate(Self::viewport_id(), self.builder(), |child, _class| {
            child.input(|input| {
                if let Some(inner) = input.viewport().inner_rect {
                    self.native_size = inner.size().max(egui::vec2(360.0, 220.0));
                }
                if let Some(outer) = input.viewport().outer_rect {
                    self.native_position = Some(outer.min);
                }
            });
            egui::CentralPanel::default()
                .frame(egui::Frame::new().fill(BACKGROUND).inner_margin(20))
                .show(child, |ui| {
                    response = self.paint(ui);
                });
            if child.input(|input| input.viewport().close_requested()) {
                response = self.close();
            }
            if response.closed {
                // This command is scoped to the native child context, never root.
                child.send_viewport_cmd(egui::ViewportCommand::Close);
            }
        });
        response
    }

    /// Headless-previewable content; emits no native close/focus commands.
    pub fn paint(&mut self, ui: &mut egui::Ui) -> WindowResponse {
        let mut closed = false;
        ui.horizontal(|ui| {
            ui.checkbox(&mut self.always_on_top, "最前面");
            ui.checkbox(&mut self.show_source, "原文");
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
                ui.checkbox(&mut self.follow_latest, "末尾に追従する");
            });
            if ui
                .add(egui::Button::new("末尾へ").min_size(egui::vec2(0.0, theme::CONTROL_H)))
                .clicked()
            {
                self.jump_to_end = true;
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                closed = ui
                    .add(
                        egui::Button::new(RichText::new("閉じる").size(theme::SIZE_LABEL))
                            .min_size(egui::vec2(72.0, theme::CONTROL_H)),
                    )
                    .clicked();
            });
        });
        ui.add_space(6.0);
        let (label, color) = self.status();
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new(label).size(theme::SIZE_LABEL).color(color));
            if let Some(view) = &self.view {
                ui.label(
                    RichText::new(match view.stamp.kind {
                        SourceKind::Audio => "PC音声",
                        SourceKind::Caption => "YouTube字幕",
                    })
                    .size(theme::SIZE_CAPTION)
                    .color(MUTED),
                );
            }
        });
        ui.add_space(8.0);
        let mut area = egui::ScrollArea::vertical()
            .id_salt("subtitle-window-reading")
            .auto_shrink([false, false])
            .stick_to_bottom(self.follow_latest);
        if self.jump_to_end {
            area = area.vertical_scroll_offset(f32::MAX);
            self.jump_to_end = false;
        } else if !self.follow_latest {
            area = area.vertical_scroll_offset(self.scroll_offset);
        }
        let output = area.show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            if let Some(view) = &self.view {
                ui.add(
                    egui::Label::new(
                        RichText::new(&view.target)
                            .size(
                                self.font_size
                                    .clamp(theme::READING_MIN, theme::SUBTITLE_READING_MAX),
                            )
                            .color(TEXT)
                            .line_height(Some(
                                (self
                                    .font_size
                                    .clamp(theme::READING_MIN, theme::SUBTITLE_READING_MAX)
                                    * 1.55)
                                    .ceil(),
                            )),
                    )
                    .wrap()
                    .selectable(true),
                );
                if self.show_source && !view.source.is_empty() {
                    ui.add_space(16.0);
                    egui::Frame::new()
                        .fill(SURFACE)
                        .corner_radius(8)
                        .inner_margin(10)
                        .show(ui, |ui| {
                            ui.label(RichText::new("原文").size(theme::SIZE_CAPTION).color(MUTED));
                            ui.add(
                                egui::Label::new(
                                    RichText::new(&view.source)
                                        .size(theme::SIZE_BODY)
                                        .color(MUTED),
                                )
                                .wrap()
                                .selectable(true),
                            );
                        });
                }
            } else {
                ui.add_space(12.0);
                ui.label(
                    RichText::new("まとまった訳文を待っています。")
                        .size(theme::SIZE_TITLE)
                        .color(TEXT),
                );
                ui.add_space(8.0);
                ui.label(
                    RichText::new("YouTube字幕またはPC音声の訳文がここに表示されます。")
                        .size(theme::SIZE_LABEL)
                        .color(MUTED),
                );
            }
        });
        self.scroll_offset = output.state.offset.y;
        #[cfg(test)]
        {
            self.body_rect = Some(output.inner_rect);
        }
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
        ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    state.paint(ui);
                });
            },
        )
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
            scroll_offset: 121.0,
            ..State::default()
        };
        let original = view(1, 1, Phase::Final);
        state.update(Some(&original.stamp), Some(&original), Playback::Running);
        state.scroll_offset = 121.0;
        assert!(state.close().closed);
        assert!(!state.open);
        assert_eq!(state.view(), Some(&original));
        assert!(state.show_source);
        assert_eq!(state.scroll_offset(), 121.0);
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
        assert!(state.body_rect.unwrap().height() > 80.0);
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
        let mut state = State::default();
        let mut original = view(1, 1, Phase::Updating);
        original.target =
            "A long readable translated paragraph stays here while additional words arrive. "
                .repeat(80);
        state.update(Some(&original.stamp), Some(&original), Playback::Running);
        paint(&mut state, &ctx, egui::vec2(360.0, 220.0));
        state.scroll_offset = 120.0;
        paint(&mut state, &ctx, egui::vec2(360.0, 220.0));
        let before = state.scroll_offset();
        assert!(before > 100.0);
        original.revision = 2;
        original.target.push_str("The next sentence arrives.");
        state.update(None, Some(&original), Playback::Running);
        paint(&mut state, &ctx, egui::vec2(360.0, 220.0));
        assert!((state.scroll_offset() - before).abs() < 1.0);
    }
}
