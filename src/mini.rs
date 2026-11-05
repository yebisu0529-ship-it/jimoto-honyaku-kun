//! Mini root-window presentation. Source lifecycle stays in the existing App methods.
use super::theme::{self, ACCENT, ERROR, TEXT_MUTED};
use super::*;
use plamo_translate::credit;
use plamo_translate::journal::ParagraphState;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Sources {
    caption: bool,
    audio: bool,
}

struct CaptionMemo {
    text: String,
    video_id: String,
    epoch: u64,
}
impl Sources {
    fn any(self) -> bool {
        self.caption || self.audio
    }
}

#[derive(Default)]
pub(super) struct MiniState {
    manual: bool,
    show_original: bool,
    paused: Option<Sources>,
    caption_memo: Option<CaptionMemo>,
    observed: Sources,
    manual_live_baseline: Option<u64>,
    manual_failure: bool,
    #[cfg(test)]
    body_rect: Option<egui::Rect>,
    #[cfg(test)]
    toolbar_rect: Option<egui::Rect>,
    #[cfg(test)]
    status_rect: Option<egui::Rect>,
    #[cfg(test)]
    scroll_state: Option<(egui::Id, f32)>,
    #[cfg(test)]
    now_rect: Option<egui::Rect>,
    #[cfg(test)]
    now_scroll: Option<(egui::Id, f32)>,
}
impl MiniState {
    pub(super) fn show_manual(&mut self) {
        self.manual_live_baseline.get_or_insert(0);
        self.manual = true;
    }
    pub(super) fn activate_manual(&mut self, rows: &[Row]) {
        self.show_manual();
        self.manual_live_baseline = Some(
            rows.iter()
                .filter(|row| {
                    row.is_confirmed() && LiveKind::from_record(&row.record.kind).is_some()
                })
                .map(|row| row.record.id)
                .max()
                .unwrap_or(0),
        );
        self.clear_manual_failure();
    }
    pub(super) fn manual_failed(&mut self) {
        self.manual_failure = true;
    }
    pub(super) fn clear_manual_failure(&mut self) {
        self.manual_failure = false;
    }
    fn new_live_count(&self, rows: &[Row]) -> usize {
        rows.iter()
            .filter(|row| {
                row.is_confirmed()
                    && LiveKind::from_record(&row.record.kind).is_some()
                    && row.record.id > self.manual_live_baseline.unwrap_or(u64::MAX)
            })
            .count()
    }
    pub(super) fn show_live(&mut self) {
        self.manual = false;
        self.manual_live_baseline = None;
    }
    pub(super) fn is_manual(&self) -> bool {
        self.manual
    }
    /// Call in both normal and mini modes. An independently enabled input
    /// invalidates the old all-input pause/resume snapshot.
    pub(super) fn observe_sources(&mut self, caption: bool, audio: bool) {
        let now = Sources { caption, audio };
        if (!self.observed.caption && caption) || (!self.observed.audio && audio) {
            self.paused = None;
            self.caption_memo = None;
        }
        self.observed = now;
    }
    fn pause(&mut self, sources: Sources) {
        self.paused = sources.any().then_some(sources);
        self.caption_memo = None;
        self.observed = Sources::default();
    }
    fn resume(&mut self) -> Option<Sources> {
        self.paused.take()
    }
}

/// Keep source and target from the same record. A new ASR draft never hides
/// the last completed translation or gets paired with its unrelated source.
fn live_result(rows: &[Row], current: Option<u64>) -> Option<&Row> {
    let active = current.and_then(|id| rows.iter().find(|r| r.record.id == id));
    active
        .filter(|r| LiveKind::from_record(&r.record.kind).is_some() && r.is_confirmed())
        .or_else(|| {
            rows.iter().rev().find(|r| {
                r.is_confirmed()
                    && LiveKind::from_record(&r.record.kind).is_some()
                    && active.is_none_or(|active| {
                        r.record.kind == active.record.kind
                            && r.record.video_id == active.record.video_id
                    })
            })
        })
}

fn manual_request<'a>(rows: &'a [Row], busy: Option<u64>, output: &str) -> Option<&'a Row> {
    busy.and_then(|id| {
        rows.iter()
            .find(|row| row.record.id == id && LiveKind::from_record(&row.record.kind).is_none())
    })
    .or_else(|| {
        rows.iter().rev().find(|row| {
            LiveKind::from_record(&row.record.kind).is_none() && row.record.translation == output
        })
    })
}

fn manual_result<'a>(rows: &'a [Row], busy: Option<u64>, output: &str) -> Option<&'a Row> {
    manual_request(rows, busy, output)
        .filter(|row| row.is_confirmed())
        .or_else(|| {
            rows.iter()
                .rev()
                .find(|row| LiveKind::from_record(&row.record.kind).is_none() && row.is_confirmed())
        })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MiniError {
    TranslationStopped,
    ServiceFailed,
    TranslationFailed,
    SaveFailed,
    ReadoutFailed,
    ManualFailed,
}
impl MiniError {
    fn label(self) -> &'static str {
        match self {
            Self::TranslationStopped => "翻訳停止",
            Self::ServiceFailed => "サービス停止",
            Self::TranslationFailed => "翻訳失敗",
            Self::SaveFailed => "保存失敗",
            Self::ReadoutFailed => "読み上げ失敗",
            Self::ManualFailed => "手動翻訳失敗",
        }
    }
    fn detail(self) -> &'static str {
        match self {
            Self::TranslationStopped => "翻訳モデルが利用できません。新しい訳は届いていません",
            Self::ServiceFailed => "通常表示で入力・翻訳の状態を確認してください",
            Self::TranslationFailed => "翻訳に失敗しました。最後の確定訳を表示しています",
            Self::SaveFailed => "訳文を保存できていません。翻訳機能は利用できます",
            Self::ReadoutFailed => "読み上げに失敗しました。翻訳機能は利用できます",
            Self::ManualFailed => "手動翻訳に失敗しました。表示中の確定訳は変更していません",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MiniStatus {
    Error(MiniError),
    Paused,
    Preparing,
    Translating,
    Receiving,
    NoInput,
}
impl MiniStatus {
    fn select(
        error: Option<MiniError>,
        paused: bool,
        preparing: bool,
        translating: bool,
        receiving: bool,
    ) -> Self {
        if let Some(error) = error {
            Self::Error(error)
        } else if paused {
            Self::Paused
        } else if preparing {
            Self::Preparing
        } else if translating {
            Self::Translating
        } else if receiving {
            Self::Receiving
        } else {
            Self::NoInput
        }
    }
    fn label(self) -> &'static str {
        match self {
            Self::Error(error) => error.label(),
            Self::Paused => "一時停止中",
            Self::Preparing => "準備中",
            Self::Translating => "翻訳中",
            Self::Receiving => "入力中",
            Self::NoInput => "入力なし",
        }
    }
}

fn fixed_region<R>(
    ui: &mut egui::Ui,
    id: &'static str,
    height: f32,
    draw: impl FnOnce(&mut egui::Ui) -> R,
) -> (egui::Rect, R) {
    let (rect, _) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), height),
        egui::Sense::hover(),
    );
    let mut child = ui.new_child(egui::UiBuilder::new().id_salt(id).max_rect(rect));
    child.set_clip_rect(rect.intersect(ui.clip_rect()));
    (rect, draw(&mut child))
}

impl App {
    fn mini_error(&self, live_failed: bool) -> Option<MiniError> {
        if !self.backend_ready && !self.backend_loading {
            Some(MiniError::TranslationStopped)
        } else if !self.service_error.is_empty() {
            Some(MiniError::ServiceFailed)
        } else if live_failed {
            Some(MiniError::TranslationFailed)
        } else if self.mini.manual_failure {
            Some(MiniError::ManualFailed)
        } else if !self.journal_error.is_empty() {
            Some(MiniError::SaveFailed)
        } else if !self.readout.error.is_empty() {
            Some(MiniError::ReadoutFailed)
        } else {
            None
        }
    }

    fn mini_toggle_live(&mut self, ctx: &egui::Context) {
        let sources = Sources {
            caption: self.caption_enabled,
            audio: self.audio_enabled || self.audio_start_requested,
        };
        if sources.any() {
            self.mini.pause(sources);
            if sources.caption {
                self.mini.caption_memo = Some(CaptionMemo {
                    text: self.last_live_text.clone(),
                    video_id: self.video_id.clone(),
                    epoch: self.caption_epoch,
                });
            }
            self.set_caption_enabled(false);
            self.audio_start_requested = false;
            self.stop_audio();
            self.stop_readout();
            return;
        }
        let Some(sources) = self.mini.resume() else {
            return;
        };
        let caption_memo = self.mini.caption_memo.take();
        self.set_caption_enabled(sources.caption);
        if sources.caption
            && let Some(memo) = caption_memo
            && memo.video_id == self.video_id
            && memo.epoch == self.caption_epoch
        {
            self.last_live_text = memo.text;
            // Dedup only: old-generation rows must never be reused or spoken.
            self.last_live_id = None;
        }
        if sources.audio {
            if self.backend_ready && self.asr_ready && self.audio_environment_ready() {
                self.toggle_audio();
            } else {
                self.restore_normal_view(ctx);
                self.tab = Tab::Live;
                self.audio_start_requested = true;
                self.prepare_audio();
            }
        }
        self.mini.observe_sources(
            self.caption_enabled,
            self.audio_enabled || self.audio_start_requested,
        );
    }

    fn mini_source_menu(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let mut captions = self.caption_enabled;
        if ui.checkbox(&mut captions, "YouTube字幕").changed() {
            self.set_caption_enabled(captions);
        }
        let mut audio = self.audio_enabled || self.audio_start_requested;
        if ui.checkbox(&mut audio, "PC音声").changed() {
            if !audio {
                self.audio_start_requested = false;
                self.stop_audio();
            } else if self.backend_ready && self.asr_ready && self.audio_environment_ready() {
                self.toggle_audio();
            } else {
                self.restore_normal_view(ctx);
                self.tab = Tab::Live;
                self.audio_start_requested = true;
                self.prepare_audio();
                ui.close();
            }
        }
    }

    fn mini_menu(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        if ui.button("字幕ウィンドウ").clicked() {
            self.subtitle_window.open = true;
            ui.close();
        }
        if ui.button("範囲を翻訳").clicked() {
            self.capture(ctx);
            ui.close();
        }
        ui.menu_button("画面選択", |ui| {
            for index in 0..self.screens.len() {
                ui.selectable_value(&mut self.screen_index, index, format!("画面 {}", index + 1));
            }
        });
        ui.separator();
        self.mini_source_menu(ui, ctx);
        let mut top = self.window_mode.always_on_top();
        if ui.checkbox(&mut top, "手前に固定").changed() {
            for command in self.window_mode.set_always_on_top(top) {
                ctx.send_viewport_cmd(command);
            }
        }
        ui.checkbox(&mut self.mini.show_original, "確定訳の原文を併記");
        let text = if self.mini.manual {
            manual_result(&self.rows, self.manual_busy, &self.output)
                .map(|row| row.record.translation.as_str())
                .unwrap_or(&self.output)
        } else {
            live_result(&self.rows, self.current_live_id)
                .map(|r| r.record.translation.as_str())
                .unwrap_or("")
        };
        if ui
            .add_enabled(!text.is_empty(), egui::Button::new("訳文をコピー"))
            .on_hover_text("PLaMoのクレジットを付けてコピーします")
            .clicked()
        {
            ctx.copy_text(credit::published(text));
            ui.close();
        }
        if ui
            .add_enabled(self.readout_busy(), egui::Button::new("読み上げを停止"))
            .clicked()
        {
            self.stop_readout();
            ui.close();
        }
        ui.separator();
        if ui.button("履歴").clicked() {
            self.restore_normal_view(ctx);
            self.tab = Tab::Files;
            ui.close();
        }
        if ui.button("設定").clicked() {
            self.restore_normal_view(ctx);
            self.tab = Tab::Settings;
            ui.close();
        }
    }

    pub(super) fn mini_ui(&mut self, ctx: &egui::Context) {
        let live_count = self.mini.new_live_count(&self.rows);
        self.mini.observe_sources(
            self.caption_enabled,
            self.audio_enabled || self.audio_start_requested,
        );
        egui::CentralPanel::default()
            .frame(egui::Frame::new().inner_margin(6.0).fill(theme::CANVAS))
            .show(ctx, |ui| {
                ui.spacing_mut().item_spacing = egui::vec2(6.0, 4.0);
                let toolbar_height = 36.0; // 32-point controls plus 2 points above/below.
                let (toolbar, ()) = fixed_region(ui, "mini-toolbar", toolbar_height, |ui| {
                    ui.spacing_mut().interact_size.y = theme::CONTROL_H;
                    let toolbar_contents =
                        ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                            if ui
                                .add_sized([72.0, theme::CONTROL_H], egui::Button::new("通常表示"))
                                .on_hover_text("通常表示に戻る（Esc）")
                                .clicked()
                            {
                                self.restore_normal_view(ctx);
                            }
                            let active = self.caption_enabled
                                || self.audio_enabled
                                || self.audio_start_requested;
                            if !active && self.mini.paused.is_none() {
                                ui.menu_button("入力を選ぶ", |ui| {
                                    self.mini_source_menu(ui, ctx)
                                });
                            } else if ui
                                .add_sized(
                                    [88.0, theme::CONTROL_H],
                                    egui::Button::new(if active {
                                        "一時停止"
                                    } else {
                                        "再開"
                                    }),
                                )
                                .on_hover_text("字幕・PC音声のライブ入力だけを停止／再開します")
                                .clicked()
                            {
                                self.mini_toggle_live(ctx);
                            }
                            if ui
                                .add_sized(
                                    [86.0, theme::CONTROL_H],
                                    egui::Button::new(if self.settings.auto_read {
                                        "読上 ON"
                                    } else {
                                        "読上 OFF"
                                    })
                                    .selected(self.settings.auto_read),
                                )
                                .on_hover_text("新しい日本語の確定訳を自動で読み上げます")
                                .clicked()
                            {
                                if self.settings.auto_read {
                                    self.settings.auto_read = false;
                                    self.stop_readout();
                                } else if self.readout.ready.is_some() {
                                    self.settings.auto_read = true;
                                } else {
                                    self.restore_normal_view(ctx);
                                    self.tab = Tab::Settings;
                                    self.readout.confirm = true;
                                }
                            }
                            let menu = ui
                                .menu_button("…", |ui| self.mini_menu(ui, ctx))
                                .response
                                .on_hover_text("表示・入力・履歴・設定");
                            menu.widget_info(|| {
                                egui::WidgetInfo::labeled(
                                    egui::WidgetType::Button,
                                    true,
                                    "表示・入力・履歴・設定",
                                )
                            });
                        });
                    debug_assert!(toolbar_contents.response.rect.is_finite());
                    #[cfg(test)]
                    {
                        self.mini.toolbar_rect = Some(toolbar_contents.response.rect);
                    }
                });
                let _ = toolbar;
                let request = if self.mini.is_manual() {
                    manual_request(&self.rows, self.manual_busy, &self.output)
                } else {
                    self.current_live_id
                        .and_then(|id| self.rows.iter().find(|row| row.record.id == id))
                };
                let paragraph = self
                    .document_visible_row()
                    .filter(|row| row.is_provisional())
                    .cloned();
                let paragraph = paragraph.as_ref();
                let selected = if self.mini.is_manual() {
                    manual_result(&self.rows, self.manual_busy, &self.output)
                } else {
                    live_result(&self.rows, self.current_live_id)
                };
                let interrupted = !self.backend_ready || !self.service_error.is_empty();
                let error = self.mini_error(
                    !self.mini.manual
                        && request.is_some_and(|row| {
                            row.error.is_some()
                                || row
                                    .record
                                    .paragraph
                                    .as_ref()
                                    .is_some_and(|p| p.state == ParagraphState::Failed)
                        }),
                );
                let draft_height = 72.0;
                let (now_rect, inspect_error) =
                    fixed_region(ui, "mini-draft", draft_height, |ui| {
                        ui.spacing_mut().interact_size.y = 16.0;
                        let (label, draft) = if interrupted {
                            (
                                "新しい訳は届いていません",
                                if self.backend_loading {
                                    "翻訳モデルを準備しています"
                                } else {
                                    "通常表示で入力・翻訳の状態を確認してください"
                                },
                            )
                        } else if self.capture_busy
                            || self.selection.is_some()
                            || self.ocr.busy.is_some()
                        {
                            (
                                "いま · 範囲を認識中",
                                "処理中です。表示中の確定訳は保持しています",
                            )
                        } else if self.manual_busy.is_some() {
                            (
                                "いま · 手動翻訳中",
                                "処理中です。表示中の確定訳は保持しています",
                            )
                        } else if let Some(row) = paragraph {
                            (
                                if row
                                    .record
                                    .paragraph
                                    .as_ref()
                                    .is_some_and(|p| p.state == ParagraphState::Closing)
                                {
                                    "いま · 文章をまとめています"
                                } else if !self.asr_partial.trim().is_empty() {
                                    "いま · 継続中の文章（追加で認識中の原文あり）"
                                } else {
                                    "いま · 継続中の文章"
                                },
                                if row.record.translation.trim().is_empty() {
                                    "続きの原文を受け取り、文章全体の訳文を更新します"
                                } else {
                                    row.record.translation.as_str()
                                },
                            )
                        } else if self.mini.manual {
                            (
                                "いま · 手動翻訳",
                                selected
                                    .map(|row| row.record.source.as_str())
                                    .unwrap_or(&self.input),
                            )
                        } else if let Some(row) =
                            request.filter(|row| row.pending && !row.record.translation.is_empty())
                        {
                            (
                                if row.record.kind == "YouTube" {
                                    "いま · 字幕を翻訳中"
                                } else {
                                    "いま · 音声の文章を翻訳中"
                                },
                                row.record.translation.as_str(),
                            )
                        } else if !self.asr_partial.is_empty() {
                            (
                                "いま · PC音声 · 認識途中",
                                "原文を認識しています。文章の訳文を更新します",
                            )
                        } else if let Some(row) = request.filter(|row| row.pending) {
                            (
                                "いま · 翻訳中",
                                if row.record.translation.is_empty() {
                                    row.record.source.as_str()
                                } else {
                                    row.record.translation.as_str()
                                },
                            )
                        } else {
                            ("いま · ライブ入力", "字幕・音声の新しい入力を待っています")
                        };
                        let has_error = error.is_some();
                        let inspect = ui
                            .horizontal(|ui| {
                                ui.add_sized(
                                    [
                                        (ui.available_width()
                                            - if has_error { 130.0 } else { 0.0 })
                                        .max(40.0),
                                        16.0,
                                    ],
                                    egui::Label::new(RichText::new(label).size(12.0).color(ACCENT))
                                        .truncate(),
                                );
                                has_error && ui.small_button("通常表示で確認").clicked()
                            })
                            .inner;
                        let now_scroll = egui::ScrollArea::vertical()
                            .id_salt((
                                "mini-paragraph-now",
                                paragraph.map(|row| row.record.id),
                                self.mini.manual,
                            ))
                            .max_height(44.0)
                            .min_scrolled_height(44.0)
                            .auto_shrink([false, false])
                            .stick_to_bottom(self.settings.auto_scroll)
                            .show(ui, |ui| {
                                let mut response = ui.add(
                                    egui::Label::new(RichText::new(draft).size(17.0))
                                        .wrap()
                                        .selectable(true),
                                );
                                if let Some(row) = paragraph {
                                    let hover = if self.asr_partial.trim().is_empty() {
                                        row.record.source.clone()
                                    } else {
                                        format!(
                                            "{}\n\n追加で認識中の原文（未確定）:\n{}",
                                            row.record.source, self.asr_partial
                                        )
                                    };
                                    response = response.on_hover_text(hover);
                                } else if !self.asr_partial.trim().is_empty() {
                                    response = response.on_hover_text(&self.asr_partial);
                                }
                                let _ = response;
                            });
                        #[cfg(test)]
                        {
                            self.mini.now_scroll = Some((now_scroll.id, now_scroll.state.offset.y));
                        }
                        #[cfg(not(test))]
                        let _ = now_scroll;
                        inspect
                    });
                #[cfg(test)]
                {
                    self.mini.now_rect = Some(now_rect);
                }
                #[cfg(not(test))]
                let _ = now_rect;
                let (heading, source, translation) = if self.mini.manual {
                    (
                        if interrupted {
                            "手動 · 停止前の確定訳"
                        } else {
                            "手動 · 最後の確定訳"
                        }
                        .to_owned(),
                        selected
                            .map(|row| row.record.source.as_str())
                            .unwrap_or(&self.input),
                        selected
                            .map(|row| row.record.translation.as_str())
                            .unwrap_or(if request.is_some() || self.manual_busy.is_some() {
                                "確定した訳文がここに表示されます"
                            } else {
                                &self.output
                            }),
                    )
                } else if let Some(row) = selected {
                    (
                        format!(
                            "{} · {} · {}",
                            row.record.kind,
                            clock(row.record.start_ms),
                            if interrupted {
                                "停止前の確定訳"
                            } else {
                                "最後の確定訳"
                            }
                        ),
                        row.record.source.as_str(),
                        row.record.translation.as_str(),
                    )
                } else {
                    (
                        if interrupted {
                            "停止前の確定訳"
                        } else {
                            "最後の確定訳"
                        }
                        .into(),
                        "",
                        "確定した訳文がここに表示されます",
                    )
                };
                let return_live = fixed_region(ui, "mini-result-heading", 22.0, |ui| {
                    ui.spacing_mut().interact_size.y = 22.0;
                    ui.spacing_mut().button_padding.y = 3.0;
                    ui.horizontal(|ui| {
                        let width = (ui.available_width()
                            - if self.mini.manual { 220.0 } else { 0.0 })
                        .max(40.0);
                        ui.add_sized(
                            [width, 22.0],
                            egui::Label::new(RichText::new(heading).size(12.0).color(ACCENT))
                                .truncate(),
                        );
                        if self.mini.manual {
                            let count_label = format!("ライブ新着{live_count}件");
                            ui.add_sized(
                                [90.0, 22.0],
                                egui::Label::new(RichText::new(&count_label).size(11.0)).truncate(),
                            )
                            .on_hover_text(count_label);
                            ui.button("ライブへ戻る").clicked()
                        } else {
                            false
                        }
                    })
                    .inner
                })
                .1;
                // Reserve the 32-point status row plus its gap; the confirmed
                // reading area still keeps 70 points at the 400x260 minimum.
                let body_height = (ui.available_height() - 36.0).max(16.0);
                let reading_size = theme::reading_size(self.settings.font_size);
                let row_height = ui
                    .fonts_mut(|fonts| fonts.row_height(&egui::FontId::proportional(reading_size)))
                    .ceil();
                let (body, scroll) = fixed_region(ui, "mini-body", body_height, |ui| {
                    egui::ScrollArea::vertical()
                        .id_salt((
                            "mini-text",
                            self.mini.manual,
                            selected.map(|row| row.record.id),
                        ))
                        .auto_shrink([false, false])
                        .max_height(body_height)
                        .show_viewport(ui, |ui, viewport| {
                            // Clip to complete rows so a half-height glyph never
                            // hangs at the bottom edge of a minimum-size window.
                            let visible = viewport.translate(ui.max_rect().min.to_vec2());
                            let rows = (visible.height() / row_height).floor().max(1.0);
                            let clipped = egui::Rect::from_min_size(
                                visible.min,
                                egui::vec2(visible.width(), rows * row_height),
                            );
                            ui.set_clip_rect(ui.clip_rect().intersect(clipped));
                            ui.set_max_width(ui.available_width());
                            ui.add(
                                egui::Label::new(
                                    theme::reading(translation, self.settings.font_size)
                                        .line_height(Some(row_height)),
                                )
                                .wrap()
                                .selectable(true),
                            );
                            if self.mini.show_original {
                                ui.separator();
                                ui.add(
                                    egui::Label::new(
                                        RichText::new(source).size(14.0).color(TEXT_MUTED),
                                    )
                                    .wrap()
                                    .selectable(true),
                                );
                            }
                        })
                });
                #[cfg(test)]
                {
                    self.mini.body_rect = Some(body);
                    self.mini.scroll_state = Some((scroll.id, scroll.state.offset.y));
                }
                #[cfg(not(test))]
                let _ = (body, scroll);
                // One prioritized public status. Raw diagnostics stay in normal view.
                let status = MiniStatus::select(
                    error,
                    self.mini.paused.is_some(),
                    self.backend_loading
                        || self.audio_start_requested
                        || self.readout.preparing
                        || self.capture_busy
                        || self.selection.is_some()
                        || self.ocr.busy.is_some(),
                    self.live_busy.is_some() || self.manual_busy.is_some(),
                    self.caption_enabled || self.audio_enabled,
                );
                let status_response = ui.horizontal(|ui| {
                    ui.label(RichText::new(status.label()).size(12.0).color(
                        if matches!(status, MiniStatus::Error(_)) {
                            ERROR
                        } else {
                            ACCENT
                        },
                    ))
                    .on_hover_text(error.map(MiniError::detail).unwrap_or(status.label()));
                    if error.is_none() && self.readout_busy() {
                        ui.label(RichText::new("読み上げ中").size(11.0));
                    }
                    let notice = !self.document.notice.is_empty()
                        && ui
                            .small_button("お知らせ")
                            .on_hover_text(&self.document.notice)
                            .clicked();
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(RichText::new(credit::LABEL).size(9.0).color(TEXT_MUTED));
                    });
                    notice
                });
                #[cfg(test)]
                {
                    self.mini.status_rect = Some(status_response.response.rect);
                }
                #[cfg(not(test))]
                let _ = &status_response;
                if return_live {
                    self.mini.show_live();
                }
                if inspect_error || status_response.inner {
                    self.restore_normal_view(ctx);
                }
            });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn row(id: u64, kind: &str, pending: bool, translation: &str) -> Row {
        Row {
            record: Record {
                id,
                start_ms: id * 1000,
                end_ms: id * 1000 + 500,
                source: format!("source-{id}"),
                translation: translation.into(),
                kind: kind.into(),
                video_id: String::new(),
                created_at: String::new(),
                paragraph: None,
            },
            pending,
            error: None,
            generation: 0,
        }
    }
    fn painted_text(shapes: &[egui::epaint::ClippedShape]) -> String {
        fn visit(shape: &egui::epaint::Shape, text: &mut String) {
            match shape {
                egui::epaint::Shape::Text(shape) => {
                    text.push_str(&shape.galley.job.text);
                    text.push('\n');
                }
                egui::epaint::Shape::Vec(shapes) => {
                    for shape in shapes {
                        visit(shape, text);
                    }
                }
                _ => {}
            }
        }
        let mut text = String::new();
        for shape in shapes {
            visit(&shape.shape, &mut text);
        }
        text
    }
    fn draw_text(app: &mut App, ctx: &egui::Context) -> String {
        let output = ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(400.0, 260.0),
                )),
                ..Default::default()
            },
            |ctx| app.mini_ui(ctx),
        );
        painted_text(&output.shapes)
    }
    #[test]
    fn manual_activation_uses_current_rows_not_an_old_render_baseline() {
        let mut state = MiniState::default();
        let mut rows = vec![
            row(1, "YouTube", false, "old"),
            row(8, "音声", false, "latest"),
            row(9, "音声", true, "partial"),
            row(10, "文章", false, "manual"),
        ];
        state.manual_failed();
        assert!(!state.is_manual());
        state.clear_manual_failure();
        assert!(!state.is_manual());
        state.activate_manual(&rows);
        assert!(state.is_manual());
        assert!(!state.manual_failure);
        assert_eq!(state.new_live_count(&rows), 0);
        rows[2].pending = false;
        assert_eq!(state.new_live_count(&rows), 1);
        state.activate_manual(&rows);
        assert_eq!(state.new_live_count(&rows), 0);
    }
    #[test]
    fn errors_distinguish_unavailable_translation_from_save_readout_and_manual_failure() {
        let ctx = egui::Context::default();
        super::super::ui::configure_theme(&ctx);
        let mut app = App::inert();
        app.rows = vec![row(1, "YouTube", false, "最後の日本語訳")];
        app.current_live_id = Some(1);
        assert_eq!(app.mini_error(false), Some(MiniError::TranslationStopped));
        let text = draw_text(&mut app, &ctx);
        assert!(text.contains("新しい訳は届いていません"));
        assert!(text.contains("停止前の確定訳"));
        assert_eq!(text.matches("通常表示で確認").count(), 1);
        app.backend_ready = true;
        app.caption_enabled = true;
        app.journal_error = "private path, should not paint".into();
        assert_eq!(app.mini_error(false), Some(MiniError::SaveFailed));
        let text = draw_text(&mut app, &ctx);
        assert!(text.contains("保存失敗"));
        assert!(!text.contains("新しい訳は届いていません"));
        assert!(!text.contains("停止前の確定訳"));
        assert!(!text.contains("private path"));
        assert_eq!(text.matches("通常表示で確認").count(), 1);
        app.journal_error.clear();
        app.readout.error = "private diagnostic".into();
        assert_eq!(app.mini_error(false), Some(MiniError::ReadoutFailed));
        app.readout.error.clear();
        app.mini.manual_failed();
        assert_eq!(app.mini_error(false), Some(MiniError::ManualFailed));
        assert!(!app.mini.is_manual());
        app.service_error = "unavailable service".into();
        assert_eq!(app.mini_error(false), Some(MiniError::ServiceFailed));
    }
    #[test]
    fn capture_and_manual_processing_keep_view_body_and_hide_old_source_in_now_strip() {
        let ctx = egui::Context::default();
        super::super::ui::configure_theme(&ctx);
        let mut app = App::inert();
        app.backend_ready = true;
        app.rows = vec![
            row(1, "YouTube", false, "最後のライブ確定訳"),
            row(2, "文章", true, "partial"),
        ];
        app.current_live_id = Some(1);
        app.input = "OLD-INPUT-DO-NOT-PRESENT-AS-OCR".into();
        draw_text(&mut app, &ctx);
        let body_identity = app.mini.scroll_state.unwrap().0;
        app.capture_busy = true;
        let text = draw_text(&mut app, &ctx);
        assert!(text.contains("範囲を認識中"));
        assert!(!text.contains("OLD-INPUT"));
        assert!(text.contains("最後のライブ確定訳"));
        assert_eq!(app.mini.scroll_state.unwrap().0, body_identity);
        assert!(!app.mini.is_manual());
        app.capture_busy = false;
        app.manual_busy = Some(2);
        let text = draw_text(&mut app, &ctx);
        assert!(text.contains("手動翻訳中"));
        assert!(!text.contains("source-2"));
        assert_eq!(app.mini.scroll_state.unwrap().0, body_identity);
        assert!(!app.mini.is_manual());
        app.mini.manual_failed();
        assert!(!app.mini.is_manual());
    }
    #[test]
    fn draft_keeps_completed_translation_and_its_own_source() {
        let rows = vec![
            row(1, "音声", false, "前の確定訳"),
            row(2, "音声", true, ""),
            row(3, "手動", false, "手動訳"),
        ];
        let selected = live_result(&rows, Some(2)).unwrap();
        assert_eq!(selected.record.id, 1);
        assert_eq!(selected.record.source, "source-1");
        let mut rows = rows;
        rows[1].record.translation = "新しい途中訳".into();
        let selected = live_result(&rows, Some(2)).unwrap();
        assert_eq!(selected.record.id, 1);
        assert_eq!(selected.record.source, "source-1");
        rows[1].pending = false;
        assert_eq!(live_result(&rows, Some(2)).unwrap().record.id, 2);
    }
    #[test]
    fn pause_resumes_exact_sources_and_normal_new_on_invalidates_cache() {
        for sources in [
            Sources {
                caption: true,
                audio: false,
            },
            Sources {
                caption: false,
                audio: true,
            },
            Sources {
                caption: true,
                audio: true,
            },
        ] {
            let mut state = MiniState::default();
            state.observe_sources(sources.caption, sources.audio);
            state.pause(sources);
            state.observe_sources(false, false);
            assert_eq!(state.resume(), Some(sources));
            assert_eq!(state.resume(), None);
            state.pause(sources);
            state.observe_sources(true, false);
            assert_eq!(state.resume(), None);
        }
    }
    #[test]
    fn caption_pause_resume_dedups_same_cue_without_reusing_old_row() {
        let ctx = egui::Context::default();
        let mut app = App::inert();
        app.backend_ready = true;
        app.caption_enabled = true;
        app.last_live_text = "same cue".into();
        app.last_live_id = Some(1);
        app.rows.push(row(1, "YouTube", false, "確定訳"));
        let settings = serde_json::to_value(&app.settings).unwrap();
        app.mini_toggle_live(&ctx);
        assert!(!app.caption_enabled);
        assert!(app.last_live_text.is_empty());
        app.mini_toggle_live(&ctx);
        assert!(app.caption_enabled);
        assert_eq!(app.last_live_text, "same cue");
        assert_eq!(app.last_live_id, None);
        app.accept_caption(bridge::Caption {
            text: "same cue".into(),
            video_id: app.video_id.clone(),
            epoch: app.caption_epoch,
            position: 1.0,
            paused: false,
        });
        app.process(&ctx);
        assert_eq!(app.rows.len(), 1);
        assert_eq!(app.live_busy, None);
        assert!(app.pending_caption.is_none());
        assert_eq!(serde_json::to_value(&app.settings).unwrap(), settings);
        // A first cue after a seek/new video still resets the dedup identity.
        for changed_video in [false, true] {
            app.mini_toggle_live(&ctx);
            app.mini_toggle_live(&ctx);
            app.accept_caption(bridge::Caption {
                text: "same cue".into(),
                video_id: if changed_video {
                    "new-video".into()
                } else {
                    app.video_id.clone()
                },
                epoch: app.caption_epoch + 1,
                position: 0.0,
                paused: false,
            });
            assert!(app.last_live_text.is_empty());
            assert_eq!(app.last_live_id, None);
            assert_eq!(app.pending_caption.as_ref().unwrap().text, "same cue");
            // Translation is deliberately not invoked for a new cue in this test.
        }
    }
    #[test]
    fn normal_source_on_discards_caption_pause_memo() {
        let mut app = App::inert();
        let ctx = egui::Context::default();
        app.caption_enabled = true;
        app.last_live_text = "same cue".into();
        app.mini_toggle_live(&ctx);
        assert!(app.mini.caption_memo.is_some());
        app.mini.observe_sources(false, true);
        assert!(app.mini.caption_memo.is_none());
        assert!(app.mini.paused.is_none());
    }
    #[test]
    fn coherent_provisional_paragraph_survives_new_draft_and_never_enters_confirmed_body() {
        use plamo_translate::journal::{ParagraphMeta, ParagraphState};
        let ctx = egui::Context::default();
        super::super::ui::configure_theme(&ctx);
        let mut app = App::inert();
        app.backend_ready = true;
        app.rows = vec![
            row(1, "音声", false, "前の確定文章"),
            row(
                2,
                "音声",
                false,
                &"いま受け取った文章全体の日本語訳。".repeat(30),
            ),
            row(3, "YouTube", false, "別の入力の確定文章"),
        ];
        app.rows[1].record.paragraph = Some(ParagraphMeta {
            revision: 2,
            state: ParagraphState::Open,
            close_reason: None,
        });
        app.current_live_id = Some(2);
        app.asr_partial = "NEW-UNCONFIRMED-DRAFT".into();
        let text = draw_text(&mut app, &ctx);
        assert!(text.contains("いま受け取った文章全体の日本語訳"));
        assert!(text.contains("前の確定文章"));
        assert!(!text.contains("別の入力の確定文章"));
        assert_eq!(live_result(&app.rows, Some(2)).unwrap().record.id, 1);
        assert_eq!(app.mini.now_rect.unwrap().height(), 72.0);
        assert!(app.mini.body_rect.unwrap().height() >= 70.0);
        app.rows[1].record.paragraph.as_mut().unwrap().state = ParagraphState::Closing;
        app.asr_partial = "NEXT-DRAFT".into();
        assert!(draw_text(&mut app, &ctx).contains("いま受け取った文章全体の日本語訳"));
        app.rows[1].record.paragraph.as_mut().unwrap().state = ParagraphState::Interrupted;
        assert_eq!(live_result(&app.rows, Some(2)).unwrap().record.id, 1);
        app.rows[1].record.video_id = "new-video".into();
        assert!(live_result(&app.rows, Some(2)).is_none());
        app.rows[1].record.paragraph.as_mut().unwrap().state = ParagraphState::Final;
        assert_eq!(live_result(&app.rows, Some(2)).unwrap().record.id, 2);
    }
    #[test]
    fn document_notice_is_not_a_service_error_and_does_not_resize_mini_regions() {
        let ctx = egui::Context::default();
        super::super::ui::configure_theme(&ctx);
        let mut app = App::inert();
        app.backend_ready = true;
        app.caption_enabled = true;
        app.rows = vec![row(1, "YouTube", false, "最後の確定訳")];
        app.current_live_id = Some(1);
        draw_text(&mut app, &ctx);
        let body = app.mini.body_rect;
        let now = app.mini.now_rect;
        app.document.notice = "処理待ちの原文を保存しました。履歴から確認できます。".repeat(200);
        let text = draw_text(&mut app, &ctx);
        assert!(text.contains("お知らせ"));
        assert!(!text.contains("サービス停止"));
        assert!(app.mini_error(false).is_none());
        assert_eq!(app.mini.body_rect, body);
        assert_eq!(app.mini.now_rect, now);
        assert!(app.mini.status_rect.unwrap().bottom() <= 260.0);
    }
    #[test]
    fn updated_idle_paragraph_follows_tail_only_when_auto_scroll_is_enabled() {
        use plamo_translate::journal::{ParagraphMeta, ParagraphState};
        for auto_scroll in [false, true] {
            let ctx = egui::Context::default();
            super::super::ui::configure_theme(&ctx);
            let mut app = App::inert();
            app.backend_ready = true;
            app.settings.auto_scroll = auto_scroll;
            app.rows = vec![row(1, "音声", false, &"長い進行中の文章です。".repeat(100))];
            app.rows[0].record.paragraph = Some(ParagraphMeta {
                revision: 1,
                state: ParagraphState::Open,
                close_reason: None,
            });
            app.current_live_id = Some(1);
            for _ in 0..3 {
                draw_text(&mut app, &ctx);
            }
            let (id, before) = app.mini.now_scroll.unwrap();
            app.rows[0]
                .record
                .translation
                .push_str(&"追加された文章の末尾です。".repeat(40));
            draw_text(&mut app, &ctx);
            let (updated_id, after) = app.mini.now_scroll.unwrap();
            assert_eq!(updated_id, id);
            if auto_scroll {
                assert!(before > 0.0 && after > before);
            } else {
                assert_eq!(before, 0.0);
                assert_eq!(after, before);
            }
            assert!(live_result(&app.rows, Some(1)).is_none());
            assert_eq!(app.mini.now_rect.unwrap().height(), 72.0);
            assert!(app.mini.body_rect.unwrap().height() >= 70.0);
        }
    }
    #[test]
    fn status_has_one_explicit_priority_and_no_receiving_when_error() {
        assert_eq!(
            MiniStatus::select(Some(MiniError::TranslationStopped), true, true, true, true),
            MiniStatus::Error(MiniError::TranslationStopped)
        );
        assert_eq!(
            MiniStatus::select(None, true, true, true, true),
            MiniStatus::Paused
        );
        assert_eq!(
            MiniStatus::select(None, false, true, true, true),
            MiniStatus::Preparing
        );
        assert_eq!(
            MiniStatus::select(None, false, false, true, true),
            MiniStatus::Translating
        );
        assert_eq!(
            MiniStatus::select(None, false, false, false, true),
            MiniStatus::Receiving
        );
        assert_eq!(
            MiniStatus::select(None, false, false, false, false),
            MiniStatus::NoInput
        );
    }
    #[test]
    fn edited_manual_input_does_not_relabel_old_output_and_busy_row_wins() {
        let mut app = App::inert();
        app.input = "new input, not translated yet".into();
        app.output = "old result".into();
        app.rows = vec![
            row(1, "文章", false, "old result"),
            row(2, "YouTube", false, "old result"),
            row(3, "画面OCR", true, ""),
        ];
        let result = manual_result(&app.rows, None, &app.output).unwrap();
        assert_eq!(result.record.id, 1);
        assert_eq!(result.record.source, "source-1");
        assert_ne!(result.record.source, app.input);
        let result = manual_request(&app.rows, Some(3), &app.output).unwrap();
        assert_eq!(result.record.id, 3);
        assert!(result.record.translation.is_empty());
        assert_eq!(
            manual_result(&app.rows, Some(3), &app.output)
                .unwrap()
                .record
                .id,
            1
        );
        app.rows[2].error = Some("private diagnostic path".into());
        app.rows[2].pending = false;
        app.output.clear();
        let result = manual_request(&app.rows, None, &app.output).unwrap();
        assert_eq!(result.record.id, 3);
        assert!(result.error.is_some());
        assert_eq!(
            manual_result(&app.rows, None, &app.output)
                .unwrap()
                .record
                .id,
            1
        );
        app.mini.show_manual();
        assert!(app.mini.is_manual());
        app.mini.show_live();
        assert!(!app.mini.is_manual());
    }
    #[test]
    fn new_record_scrolls_to_top_but_partial_of_same_record_keeps_position() {
        for manual in [false, true] {
            let ctx = egui::Context::default();
            super::super::ui::configure_theme(&ctx);
            let mut app = App::inert();
            let kind = if manual { "文章" } else { "音声" };
            app.rows = vec![row(1, kind, false, &"長文の確定結果です。".repeat(400))];
            app.current_live_id = Some(1);
            app.manual_busy = manual.then_some(1);
            app.mini.manual = manual;
            let draw = |app: &mut App| {
                let _ = ctx.run(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(400.0, 260.0),
                        )),
                        ..Default::default()
                    },
                    |ctx| app.mini_ui(ctx),
                );
            };
            draw(&mut app);
            let (first_id, _) = app.mini.scroll_state.unwrap();
            let mut state = egui::scroll_area::State::load(&ctx, first_id).unwrap();
            state.offset.y = 150.0;
            state.store(&ctx, first_id);
            app.rows
                .push(row(2, kind, true, &"次の長文の途中結果です。".repeat(400)));
            app.current_live_id = Some(2);
            app.manual_busy = manual.then_some(2);
            app.rows[1]
                .record
                .translation
                .push_str("追加された途中結果");
            draw(&mut app);
            let (partial_id, offset) = app.mini.scroll_state.unwrap();
            assert_eq!(partial_id, first_id);
            assert!(offset > 100.0, "partial scrolled to {offset}");
            app.rows[1].pending = false;
            app.manual_busy = None;
            app.output = app.rows[1].record.translation.clone();
            draw(&mut app);
            let (next_id, offset) = app.mini.scroll_state.unwrap();
            assert_ne!(next_id, first_id);
            assert_eq!(offset, 0.0);
        }
    }
    #[test]
    fn mini_minimum_and_default_keep_toolbar_and_scrolling_body_inside_window() {
        for size in [egui::vec2(400.0, 260.0), egui::vec2(520.0, 360.0)] {
            for manual in [false, true] {
                let ctx = egui::Context::default();
                super::super::ui::configure_theme(&ctx);
                let mut app = App::inert();
                app.rows = vec![row(1, "YouTube", false, &"長い確定訳です。".repeat(400))];
                app.current_live_id = Some(1);
                app.asr_partial = "A new and unrelated draft. ".repeat(100);
                app.input = "Long manual source. ".repeat(500);
                app.output = "長い手動翻訳です。".repeat(500);
                app.mini.manual = manual;
                app.mini.show_original = true;
                for frame in 0..3 {
                    let _ = ctx.run(
                        egui::RawInput {
                            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
                            time: Some(frame as f64 / 60.0),
                            ..Default::default()
                        },
                        |ctx| app.mini_ui(ctx),
                    );
                }
                let body = app.mini.body_rect.unwrap();
                let toolbar = app.mini.toolbar_rect.unwrap();
                let status = app.mini.status_rect.unwrap();
                assert!(body.height() >= 70.0, "{size:?}: {body:?}");
                assert!(body.bottom() < size.y - 15.0);
                assert!(toolbar.left() >= 0.0 && toolbar.right() <= size.x);
                assert!(toolbar.top() >= 0.0 && toolbar.bottom() <= body.top());
                assert!(body.right() <= size.x);
                assert!(status.bottom() <= size.y && status.right() <= size.x);
                let old_body = body;
                app.asr_partial.clear();
                app.output.clear();
                app.rows.clear();
                let _ = ctx.run(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
                        ..Default::default()
                    },
                    |ctx| app.mini_ui(ctx),
                );
                assert_eq!(
                    app.mini.body_rect,
                    Some(old_body),
                    "new data moved the body"
                );
                app.mini.show_manual();
                assert!(app.mini.manual);
            }
        }
    }
}
