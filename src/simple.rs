//! シンプル翻訳: what is put on the left is translated on the right. The
//! screen translates the input by itself once it has rested for a moment, and
//! at once on 翻訳する or Ctrl+Enter. Its translations, and those of a reading,
//! go one at a time through the manual lane of the translation server; the
//! chat has a lane of its own.
use super::*;

/// How long the input rests before the screen translates it by itself.
pub(super) const PAUSE: Duration = Duration::from_secs(1);

/// The input as a translation sees it: its words, its languages and its style.
/// The same input is the same translation.
#[derive(Clone, Default, PartialEq)]
pub(super) struct Input {
    text: String,
    from: String,
    to: String,
    style: String,
}

impl Input {
    pub(super) fn of(text: &str, options: &TranslationOptions) -> Self {
        Self {
            text: text.to_owned(),
            from: options.source_lang.clone(),
            to: options.target_lang.clone(),
            style: options.style.clone(),
        }
    }

    fn is_empty(&self) -> bool {
        self.text.trim().is_empty()
    }
}

/// What the screen remembers of its translations. The one in flight is
/// `App::manual_busy`.
#[derive(Default)]
pub(super) struct SimpleState {
    /// The input as last seen, and the time it last changed, in egui's seconds.
    seen: Input,
    changed: f64,
    /// What the 訳文 card is about: the input of the last translation sent on
    /// the lane, by any route. The card shows that translation from its first
    /// part on, or nothing if it failed, and the screen does not send the
    /// same input again by itself. Emptying the input forgets it.
    card: Option<Input>,
    /// The translation the card showed when the one in flight was sent. It
    /// stays on the card until the first part of the new one arrives, so a
    /// pause in typing does not empty the card.
    previous: String,
    /// The translation that was in flight when the input was emptied. Its
    /// result is not shown.
    dropped: Option<u64>,
    /// 翻訳する was pressed while another input was being translated.
    pressed: bool,
}

impl SimpleState {
    /// A translation of `input` was sent on the lane while the card showed
    /// `shown`.
    pub(super) fn sent(&mut self, input: Input, shown: String) {
        self.card = Some(input);
        self.previous = shown;
    }

    /// A reading took the lane, and its text is the input now. A press that
    /// waited for the lane is not sent for that text: the reading is what the
    /// reader asked for last. The translation before it is of another text,
    /// so the card does not keep it.
    pub(super) fn reading_sent(&mut self) {
        self.pressed = false;
        self.previous.clear();
    }
}

impl App {
    /// The translation with `id` is shown on the 訳文 card as it comes.
    pub(super) fn on_card(&self, id: u64) -> bool {
        self.manual_busy == Some(id) && self.simple.dropped != Some(id)
    }

    /// What the 訳文 card shows: the last translation as far as it has come.
    /// Until the first part of the one in flight arrives, the one before it
    /// stays.
    pub(super) fn card_text(&self) -> &str {
        if self.manual_busy.is_some() && self.output.is_empty() {
            &self.simple.previous
        } else {
            &self.output
        }
    }

    /// 翻訳する and Ctrl+Enter translate the input at once and write the
    /// translation to 履歴. A translation in flight is waited for. When it is
    /// of the same input, it is the one written to 履歴.
    pub(super) fn manual_translate(&mut self) {
        if !self.backend_ready {
            self.status = "翻訳モデルを準備してからお試しください。".into();
            return;
        }
        let newest = Input::of(&self.input, &self.settings.translation);
        if newest.is_empty() {
            return;
        }
        let Some(id) = self.manual_busy else {
            self.translate(newest.text, Some("文章"));
            return;
        };
        if self.simple.card.as_ref() != Some(&newest) {
            self.simple.pressed = true;
        } else if !self.rows.iter().any(|row| row.record.id == id) {
            self.keep_in_history(id, newest.text, "文章");
        }
    }

    /// Once a frame: notices a change of the input, clears the card when the
    /// input is emptied, and sends what is due on the lane.
    pub(super) fn simple_tick(&mut self, ctx: &egui::Context) {
        let now = ctx.input(|input| input.time);
        let newest = Input::of(&self.input, &self.settings.translation);
        if newest != self.simple.seen {
            if newest.is_empty() && !self.simple.seen.is_empty() {
                self.output.clear();
                self.simple.previous.clear();
                self.simple.card = None;
                self.simple.dropped = self.manual_busy;
                self.simple.pressed = false;
                self.mini.input_emptied();
            }
            self.simple.seen = newest.clone();
            self.simple.changed = now;
        }
        if newest.is_empty() || !self.backend_ready || self.manual_busy.is_some() {
            return;
        }
        if std::mem::take(&mut self.simple.pressed) {
            self.translate(newest.text, Some("文章"));
            return;
        }
        // Only the screen in view translates by itself; a change made on
        // another screen waits until the screen is shown again.
        let shown = self.tab == Tab::Text && !self.window_mode.is_mini();
        if !shown || self.simple.card.as_ref() == Some(&newest) {
            return;
        }
        let due = self.simple.changed + PAUSE.as_secs_f64();
        if now >= due {
            self.translate(newest.text, None);
        } else {
            ctx.request_repaint_after(Duration::from_secs_f64(due - now));
        }
    }
}
