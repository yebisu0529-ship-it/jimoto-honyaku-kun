//! Continuous English ASR. Drafts stay transient; only completed segments enter translation.
use super::*;
use plamo_translate::nemotron::{NemotronEngine, NemotronEvent};

/// What the line beside 「聞く」 and the 音声認識 card say while Nemotron
/// cannot hear the language to translate from.
pub(super) const ENGLISH_ONLY: &str = "Nemotronは英語だけを聞き取ります。中国語などほかの言語は、設定の「音声認識」でQwen3-ASR（GPU・数GB）かWhisper（CPU・約4GB）を選んでください。";

impl App {
    /// Nemotron is the recognizer, and the language to translate from is
    /// neither 英語 nor 自動判定.
    pub(super) fn nemotron_cannot_hear_the_language(&self) -> bool {
        self.settings.asr_backend == "nemotron"
            && !matches!(
                self.settings.translation.source_lang.as_str(),
                "auto" | "en"
            )
    }

    pub(super) fn clear_streaming_audio(&mut self) {
        if let Some(engine) = &self.nemotron {
            self.nemotron_epoch = engine.clear();
        }
        if self.audio_enabled {
            self.document_source_changed(plamo_translate::live_text::SourceKind::Audio);
        }
        self.nemotron_origin_ms = self.audio_clock_ms();
        self.nemotron_needs_origin = true;
        self.asr_partial.clear();
        self.streaming_transcripts.clear();
    }

    pub(super) fn stop_nemotron(&mut self) {
        self.clear_streaming_audio();
        if let Some(mut engine) = self.nemotron.take() {
            engine.stop();
        }
    }

    pub(super) fn start_nemotron(&mut self) -> Result<()> {
        if self.nemotron.is_some() {
            return Ok(());
        }
        let installation = plamo_translate::nemotron_setup::load(&data_root().join("nemotron"))?
            .ok_or_else(|| anyhow::anyhow!("Nemotronの追加データを先に準備してください。"))?;
        let engine = NemotronEngine::start(installation, self.settings.nemotron_context)?;
        self.nemotron_epoch = engine.generation();
        self.nemotron = Some(engine);
        self.asr_started_at = Some(Instant::now());
        Ok(())
    }

    pub(super) fn start_streaming_audio(&mut self) {
        // The language changed while Nemotron was getting ready. The line
        // beside the 聞く button says why nothing starts.
        if self.nemotron_cannot_hear_the_language() {
            self.audio_start_requested = false;
            return;
        }
        self.audio_offset_ms = self
            .audio_offset_ms
            .max(max_audio_end(self.rows.iter().map(|r| &r.record)));
        self.clear_streaming_audio();
        self.nemotron_suspended = false;
        let (tx, rx) = mpsc::sync_channel(4);
        let stop = Arc::new(AtomicBool::new(false));
        let capture_stop = stop.clone();
        let events = self.tx.clone();
        let suppress = self.readout.suppress.clone();
        thread::spawn(move || {
            if let Err(error) =
                platform::capture_loopback_streaming(capture_stop.clone(), tx, 160, suppress)
            {
                let _ = events.send(Event::Info(format!("音声の取得を停止しました: {error:#}")));
            }
            capture_stop.store(true, Ordering::Release);
        });
        self.audio = Some(AudioSession {
            stop,
            rx: AudioReceiver::Streaming(rx),
            started: Instant::now(),
        });
        self.audio_enabled = true;
        self.audio_start_requested = false;
        self.audio_generation += 1;
        self.pending_audio = None;
        self.pending_transcript = None;
        self.tab = Tab::Live;
        self.status = "英語音声を連続して認識し、原文を文章につないで翻訳・保存しています。".into();
    }

    pub(super) fn process_streaming_audio(&mut self) {
        if self.settings.asr_backend != "nemotron" || !self.audio_enabled {
            return;
        }
        if self.nemotron_cannot_hear_the_language() {
            self.stop_audio();
            return;
        }
        let suspended = self.captions_fresh() || self.readout.suppress.load(Ordering::Acquire);
        let resuming = self.nemotron_suspended && !suspended;
        if suspended != self.nemotron_suspended {
            self.clear_streaming_audio();
            self.nemotron_suspended = suspended;
        }
        let frames = match self.audio.as_ref().map(|a| &a.rx) {
            Some(AudioReceiver::Streaming(rx)) => rx.try_iter().collect::<Vec<_>>(),
            _ => return,
        };
        // Audio already queued before the resume edge still belongs to the
        // suppressed interval. Start the new epoch with a freshly captured frame.
        if suspended || resuming {
            return;
        }
        for frame in frames {
            if frame.discontinuity {
                self.clear_streaming_audio();
            }
            if self.nemotron_needs_origin {
                self.nemotron_origin_ms = self.audio_offset_ms.saturating_add(frame.start_ms);
                self.nemotron_needs_origin = false;
            }
            if let Some(engine) = &self.nemotron
                && let Err(error) = engine.push_pcm16(&frame.samples)
            {
                self.clear_streaming_audio();
                self.nemotron_origin_ms = self.audio_offset_ms.saturating_add(frame.end_ms);
                self.status = format!("音声の遅れを解消するため認識を再開します: {error}");
            }
        }
    }

    pub(super) fn poll_nemotron(&mut self) {
        let events = self
            .nemotron
            .as_ref()
            .map(NemotronEngine::poll)
            .unwrap_or_default();
        for event in events {
            self.accept_nemotron_event(event);
        }
    }

    fn accept_nemotron_event(&mut self, event: NemotronEvent) {
        match event {
            NemotronEvent::Starting => self.status = "Nemotronを読み込んでいます…".into(),
            NemotronEvent::Ready { .. } => {
                self.asr_ready = true;
                self.asr_started_at = None;
                self.service_error.clear();
                self.status = "Nemotronの準備ができました。".into();
                if self.audio_start_requested && self.backend_ready && !self.audio_enabled {
                    self.start_audio();
                }
            }
            NemotronEvent::Partial {
                generation, text, ..
            } => {
                if self.accept_streaming_result(generation) {
                    if !text.trim().is_empty() {
                        self.document_note_audio_activity();
                    }
                    self.asr_partial = text;
                    self.archive_completed_audio_for_draft();
                }
            }
            NemotronEvent::Final {
                generation,
                text,
                start_ms,
                end_ms,
            } => {
                if !self.accept_streaming_result(generation) {
                    return;
                }
                self.asr_partial.clear();
                if text.trim().is_empty() {
                    return;
                }
                let start_ms = self.nemotron_origin_ms.saturating_add(start_ms);
                let end_ms = self
                    .nemotron_origin_ms
                    .saturating_add(end_ms)
                    .max(start_ms + 1);
                // Bounded, ordered final segments. Draft deltas never become duplicate rows.
                if self.streaming_transcripts.len() >= 16 {
                    self.streaming_transcripts.pop_front();
                }
                self.streaming_transcripts.push_back(PendingTranscript {
                    text,
                    start_ms,
                    end_ms,
                    generation: self.audio_generation,
                });
            }
            NemotronEvent::Cleared { .. } => {}
            NemotronEvent::Error(error) => {
                self.service_error = format!("Nemotron: {error}");
                self.asr_ready = false;
                self.audio_start_requested = false;
                if self.audio_enabled {
                    self.stop_audio();
                }
                self.stop_nemotron();
            }
            NemotronEvent::Stopped => {
                self.asr_ready = false;
                self.audio_start_requested = false;
                if self.audio_enabled {
                    self.stop_audio();
                }
            }
        }
    }

    fn accept_streaming_result(&self, generation: u64) -> bool {
        generation == self.nemotron_epoch
            && self.audio_enabled
            && !self.nemotron_suspended
            && !self.captions_fresh()
            && !self.readout.suppress.load(Ordering::Acquire)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn active() -> App {
        let mut app = App::inert();
        app.audio_enabled = true;
        app.nemotron_epoch = 4;
        app.nemotron_origin_ms = 1000;
        app
    }

    #[test]
    fn streaming_drafts_do_not_enter_history_and_final_segments_keep_order() {
        let mut app = active();
        app.accept_nemotron_event(NemotronEvent::Partial {
            generation: 4,
            text: "The meeting".into(),
            audio_processed_ms: 100,
        });
        assert_eq!(app.asr_partial, "The meeting");
        assert!(app.rows.is_empty());
        assert!(app.streaming_transcripts.is_empty());
        for (text, start_ms, end_ms) in [("The meeting begins.", 0, 400), ("Welcome.", 400, 700)] {
            app.accept_nemotron_event(NemotronEvent::Final {
                generation: 4,
                text: text.into(),
                start_ms,
                end_ms,
            });
        }
        assert!(app.asr_partial.is_empty());
        let first = app.take_ready_transcript().unwrap();
        let second = app.take_ready_transcript().unwrap();
        assert_eq!(
            (first.text.as_str(), first.start_ms, first.end_ms),
            ("The meeting begins.", 1000, 1400)
        );
        assert_eq!(second.text, "Welcome.");
    }

    #[test]
    fn old_streams_stop_and_readout_cannot_insert_late_finals() {
        let mut app = active();
        let final_event = |generation| NemotronEvent::Final {
            generation,
            text: "late".into(),
            start_ms: 0,
            end_ms: 500,
        };
        app.accept_nemotron_event(final_event(3));
        app.readout.suppress.store(true, Ordering::Release);
        app.accept_nemotron_event(final_event(4));
        app.readout.suppress.store(false, Ordering::Release);
        app.stop_audio();
        app.accept_nemotron_event(final_event(4));
        assert!(app.streaming_transcripts.is_empty());
        assert!(app.rows.is_empty());
    }

    #[test]
    fn fresh_youtube_captions_discard_pending_stream_finals() {
        let mut app = active();
        app.accept_nemotron_event(NemotronEvent::Final {
            generation: 4,
            text: "audio".into(),
            start_ms: 0,
            end_ms: 500,
        });
        app.set_caption_enabled(true);
        app.caption_paused = false;
        app.last_caption = Some(Instant::now());
        assert!(app.take_ready_transcript().is_none());
        assert!(app.streaming_transcripts.is_empty());
    }

    #[test]
    fn resume_discards_buffered_audio_before_choosing_a_new_clock_origin() {
        let mut app = active();
        app.nemotron_suspended = true;
        let (tx, rx) = mpsc::sync_channel(4);
        app.audio = Some(AudioSession {
            stop: Arc::new(AtomicBool::new(false)),
            rx: AudioReceiver::Streaming(rx),
            started: Instant::now(),
        });
        let frame = |start_ms| platform::StreamingAudioFrame {
            samples: vec![1; 2560],
            start_ms,
            end_ms: start_ms + 160,
            discontinuity: false,
        };
        tx.send(frame(640)).unwrap();
        app.process_streaming_audio();
        assert!(app.nemotron_needs_origin);
        tx.send(frame(800)).unwrap();
        app.process_streaming_audio();
        assert!(!app.nemotron_needs_origin);
        assert_eq!(app.nemotron_origin_ms, 800);
    }
}
