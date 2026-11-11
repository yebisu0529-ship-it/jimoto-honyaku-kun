//! Continuous live prose; recognition keeps feeding while one paragraph is translated.
use super::*;
use plamo_translate::{
    journal::{ParagraphMeta, ParagraphState},
    live_text::{
        self, Acceptance, Accumulator, AudioFragment, ParagraphSnapshot, Policy, PushOutcome,
        Request, SourceKind, SourceStamp,
    },
};
use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};

type ParagraphKey = (SourceStamp, u64);

/// The name history and badges show. Screen text arrives as captions under
/// a scope of its own.
fn source_name(kind: SourceKind, video_id: &str) -> &'static str {
    match kind {
        SourceKind::Audio => "音声",
        SourceKind::Caption if video_id.starts_with(super::screen_feed::SCREEN_SOURCE) => "画面",
        SourceKind::Caption => "YouTube",
    }
}
/// Fixed parts kept as messages in the subtitle window.
const SUBTITLE_HISTORY: usize = 100;

/// How much live text becomes one translated, then fixed, part.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum ChunkLength {
    Short,
    #[default]
    Standard,
    Long,
}
impl ChunkLength {
    pub(super) const CHOICES: [(Self, &'static str); 3] = [
        (Self::Short, "短め（1〜2文ほど）"),
        (Self::Standard, "標準（2〜4文ほど）"),
        (Self::Long, "長め（段落ごと）"),
    ];
    /// A part closes at a sentence end once it passes the first pair of
    /// lengths, as soon as it reaches the second pair or the time limit.
    fn policy(self) -> Policy {
        let (min, min_cjk, max, max_cjk, max_ms) = match self {
            Self::Short => (120, 60, 240, 120, 20_000),
            Self::Standard => (240, 120, 480, 240, 30_000),
            Self::Long => return Policy::default(),
        };
        Policy {
            min_boundary_chars: min,
            min_boundary_cjk_chars: min_cjk,
            max_paragraph_chars: max,
            max_paragraph_cjk_chars: max_cjk,
            max_paragraph_ms: max_ms,
            ..Policy::default()
        }
    }
}
struct Flight {
    row_id: u64,
    request: Request,
    options: TranslationOptions,
    cancel: Arc<AtomicBool>,
}

/// A translated live paragraph. For a Japanese target `easy` is `standard`
/// rewritten as やさしい日本語, and that is the text the user reads.
pub(super) struct LiveTranslation {
    standard: String,
    easy: Option<String>,
}

struct VisibleInterim {
    snapshot: ParagraphSnapshot,
    row: Row,
}

pub(super) struct DocumentState {
    audio: Accumulator,
    caption: Accumulator,
    flight: Option<Flight>,
    rows: HashMap<ParagraphKey, u64>,
    keys: HashMap<u64, ParagraphKey>,
    first_row: Option<u64>,
    translated_source: HashMap<ParagraphKey, String>,
    saved_at: HashMap<u64, u64>,
    failures: HashMap<ParagraphKey, (u64, usize)>,
    spoken: HashSet<ParagraphKey>,
    displayed: Option<VisibleInterim>,
    caption_position_ms: Option<u64>,
    pub caption_media_paused: bool,
    started: Instant,
    observed_options: Option<TranslationOptions>,
    configure_after: u64,
    applied_chunk: Option<ChunkLength>,
}
impl Default for DocumentState {
    fn default() -> Self {
        let stamp = |kind| SourceStamp {
            kind,
            generation: 0,
            video_id: String::new(),
            epoch: 0,
        };
        Self {
            audio: Accumulator::new(stamp(SourceKind::Audio), Policy::default()),
            caption: Accumulator::new(stamp(SourceKind::Caption), Policy::default()),
            flight: None,
            rows: HashMap::new(),
            keys: HashMap::new(),
            first_row: None,
            translated_source: HashMap::new(),
            saved_at: HashMap::new(),
            failures: HashMap::new(),
            spoken: HashSet::new(),
            displayed: None,
            caption_position_ms: None,
            caption_media_paused: true,
            started: Instant::now(),
            observed_options: None,
            configure_after: 0,
            applied_chunk: None,
        }
    }
}
impl DocumentState {
    fn now(&self) -> u64 {
        self.started.elapsed().as_millis().min(u64::MAX as u128) as u64
    }
    fn source(&self, kind: SourceKind) -> &Accumulator {
        match kind {
            SourceKind::Audio => &self.audio,
            SourceKind::Caption => &self.caption,
        }
    }
    fn source_mut(&mut self, kind: SourceKind) -> &mut Accumulator {
        match kind {
            SourceKind::Audio => &mut self.audio,
            SourceKind::Caption => &mut self.caption,
        }
    }
    fn snapshots(&self, kind: SourceKind) -> Vec<ParagraphSnapshot> {
        let source = self.source(kind);
        let mut snapshots = source.pending_snapshots();
        snapshots.extend(source.current_snapshot());
        snapshots
    }
    fn cached_source_is_current(&self, cached: &VisibleInterim) -> bool {
        if self.source(cached.snapshot.stamp.kind).stamp() != &cached.snapshot.stamp {
            return false;
        }
        let snapshots = self.snapshots(cached.snapshot.stamp.kind);
        let Some(index) = snapshots
            .iter()
            .position(|snapshot| snapshot.key == cached.snapshot.key)
        else {
            return false;
        };
        // A caption correction must hide the old revision even if it belongs
        // to the same key; a split may span a closed prefix and its new tail.
        snapshots[index..]
            .iter()
            .map(|snapshot| snapshot.text.as_str())
            .collect::<Vec<_>>()
            .join(" ")
            .starts_with(&cached.snapshot.text)
    }
}
impl App {
    /// Feed the independent subtitle viewport from accepted paragraph pairs,
    /// never from a newer draft.
    pub(super) fn document_update_subtitle_window(&mut self) {
        use super::subtitle_window::{Phase, Playback, View};

        let active_kind = if self.caption_enabled && (!self.caption_paused || !self.audio_enabled) {
            Some(SourceKind::Caption)
        } else if self.audio_enabled {
            Some(SourceKind::Audio)
        } else {
            None
        };
        let expected = active_kind
            .map(|kind| self.document_stamp(kind))
            .or_else(|| self.subtitle_window.scope().cloned());
        let playback = match active_kind {
            None => Playback::Stopped,
            Some(SourceKind::Caption) if self.document.caption_media_paused => Playback::Paused,
            Some(_) => Playback::Running,
        };
        let to_view = |row: &Row| {
            let key = self.document.keys.get(&row.record.id)?;
            let stamp = &key.0;
            if expected.as_ref().is_some_and(|scope| scope != stamp) {
                return None;
            }
            let cached = self.document.displayed.as_ref().filter(|cached| {
                cached.row.record.id == row.record.id
                    && self.document.cached_source_is_current(cached)
            });
            let source = cached
                .map(|cached| cached.row.record.source.clone())
                .or_else(|| self.document.translated_source.get(key).cloned())
                .unwrap_or_else(|| row.record.source.clone());
            let target = cached
                .map(|cached| cached.row.record.translation.clone())
                .unwrap_or_else(|| row.record.translation.clone());
            let paragraph = row.record.paragraph.as_ref();
            let revision = cached
                .map(|cached| cached.snapshot.revision)
                .or_else(|| paragraph.map(|paragraph| paragraph.revision))
                .unwrap_or(0);
            let phase = match paragraph.map(|paragraph| paragraph.state) {
                Some(ParagraphState::Final) | None if row.is_confirmed() => Phase::Final,
                Some(ParagraphState::Interrupted) => Phase::Interrupted,
                Some(ParagraphState::Failed) => Phase::Failed,
                _ => Phase::Updating,
            };
            Some(View {
                stamp: stamp.clone(),
                // Row IDs remain monotonic across splits and source queues.
                paragraph_key: row.record.id,
                revision,
                source,
                target,
                phase,
            })
        };
        let made_here = |row: &&Row| {
            self.document
                .first_row
                .is_some_and(|first| row.record.id >= first)
        };
        let current = self.document_visible_row().and_then(to_view);
        let view = current.or_else(|| {
            self.rows
                .iter()
                .rev()
                .take_while(made_here)
                .find_map(to_view)
        });
        let mut history: Vec<View> = self
            .rows
            .iter()
            .rev()
            .take_while(made_here)
            .filter_map(to_view)
            .filter(|view| view.phase == Phase::Final)
            .take(SUBTITLE_HISTORY)
            .collect();
        history.reverse();
        self.subtitle_window
            .update(expected.as_ref(), view.as_ref(), playback);
        self.subtitle_window.set_history(history);
    }

    /// Splitting a paragraph can invalidate its old whole-paragraph target.
    /// Keep that coherent revision only as a transient, explicitly provisional
    /// view while the new closed prefix is translated. It never enters history,
    /// export or speech as a finalized record, nor borrows the new tail's source.
    pub(super) fn document_visible_row(&self) -> Option<&Row> {
        let current = self
            .current_live_id
            .and_then(|id| self.rows.iter().find(|row| row.record.id == id))?;
        if current.is_provisional()
            && current.record.translation.trim().is_empty()
            && let Some(cached) = &self.document.displayed
            && cached.row.record.kind == current.record.kind
            && cached.row.generation == current.generation
            && cached.row.record.video_id == current.record.video_id
            && self.document.cached_source_is_current(cached)
        {
            return Some(&cached.row);
        }
        Some(current)
    }
    pub(super) fn document_take_caption(&mut self) {
        if self.caption_enabled
            && !self.caption_paused
            && let Some(caption) = self.pending_caption.take()
        {
            self.document_ingest_caption(caption);
        }
    }
    pub(super) fn document_source_changed(&mut self, kind: SourceKind) {
        self.document_ensure_scope(kind);
    }
    pub(super) fn document_note_audio_activity(&mut self) {
        self.document_ensure_scope(SourceKind::Audio);
        let now = self.document.now();
        self.document.audio.note_activity(now);
    }
    pub(super) fn document_stamp(&self, kind: SourceKind) -> SourceStamp {
        match kind {
            SourceKind::Audio => SourceStamp {
                kind,
                generation: self.audio_generation,
                video_id: String::new(),
                epoch: if self.settings.asr_backend == "nemotron" {
                    self.nemotron_epoch
                } else {
                    0
                },
            },
            SourceKind::Caption => SourceStamp {
                kind,
                generation: self.caption_generation,
                video_id: self.video_id.clone(),
                epoch: self.caption_epoch,
            },
        }
    }
    fn document_cancel_flight(&mut self) {
        if let Some(flight) = self.document.flight.take() {
            flight.cancel.store(true, Ordering::Release);
            let now = self.document.now();
            self.document
                .source_mut(flight.request.snapshot.stamp.kind)
                .cancel_request(&flight.request, now);
            if self.live_busy == Some(flight.row_id) {
                self.live_busy = None;
            }
            if let Some(row) = self.rows.iter_mut().find(|r| r.record.id == flight.row_id) {
                row.pending = false;
            }
        }
    }
    fn document_ensure_scope(&mut self, kind: SourceKind) {
        let stamp = self.document_stamp(kind);
        let current = self.document.source(kind).stamp();
        if current == &stamp {
            return;
        }
        // A restarted recognizer is still the same audio source. What it heard
        // is closed and translated to the end; later speech gets the new stamp.
        if kind == SourceKind::Audio
            && current.generation == stamp.generation
            && current.video_id == stamp.video_id
        {
            self.document_close(SourceKind::Audio);
            self.document.audio.restamp(stamp);
            return;
        }
        if self
            .document
            .flight
            .as_ref()
            .is_some_and(|flight| flight.request.snapshot.stamp.kind == kind)
        {
            self.document_cancel_flight();
        }
        self.document_interrupt_source(kind, "source_changed");
        if kind == SourceKind::Caption {
            self.document.caption_position_ms = None;
        }
        let now = self.document.now();
        self.document.source_mut(kind).reset(stamp, now);
    }
    fn document_row(&mut self, snapshot: &ParagraphSnapshot) -> u64 {
        let key = (snapshot.stamp.clone(), snapshot.key);
        let id = if let Some(id) = self.document.rows.get(&key) {
            *id
        } else {
            let id = self.next_id;
            self.next_id += 1;
            self.document.rows.insert(key.clone(), id);
            self.document.keys.insert(id, key.clone());
            self.document.first_row.get_or_insert(id);
            self.rows.push(Row {
                record: Record {
                    id,
                    start_ms: snapshot.start_ms,
                    end_ms: snapshot.end_ms,
                    source: snapshot.text.clone(),
                    translation: String::new(),
                    standard_translation: None,
                    kind: source_name(snapshot.stamp.kind, &snapshot.stamp.video_id).into(),
                    video_id: snapshot.stamp.video_id.clone(),
                    created_at: chrono::Local::now().to_rfc3339(),
                    paragraph: Some(ParagraphMeta {
                        revision: snapshot.revision,
                        state: ParagraphState::Open,
                        close_reason: None,
                    }),
                },
                pending: false,
                error: None,
                generation: snapshot.stamp.generation,
            });
            id
        };
        if let Some(row) = self.rows.iter_mut().find(|r| r.record.id == id) {
            let immutable = row.record.paragraph.as_ref().is_some_and(|p| {
                matches!(p.state, ParagraphState::Final | ParagraphState::Interrupted)
            });
            if immutable {
                return id;
            }
            let same_failed = row.record.paragraph.as_ref().is_some_and(|p| {
                p.state == ParagraphState::Failed && p.revision == snapshot.revision
            });
            if self
                .document
                .translated_source
                .get(&key)
                .is_some_and(|source| !snapshot.text.starts_with(source))
            {
                row.record.translation.clear();
                row.record.standard_translation = None;
                self.document.translated_source.remove(&key);
            }
            row.record.source.clone_from(&snapshot.text);
            row.record.start_ms = snapshot.start_ms;
            row.record.end_ms = snapshot.end_ms;
            if !same_failed {
                row.error = None;
                row.record.paragraph = Some(ParagraphMeta {
                    revision: snapshot.revision,
                    state: if snapshot.closed {
                        ParagraphState::Closing
                    } else {
                        ParagraphState::Open
                    },
                    close_reason: None,
                });
            }
        }
        id
    }
    fn document_save(&mut self, id: u64, force: bool) {
        let now = self.document.now();
        if !force
            && self
                .document
                .saved_at
                .get(&id)
                .is_some_and(|last| now.saturating_sub(*last) < 10_000)
        {
            return;
        }
        if let Some(record) = self
            .rows
            .iter()
            .find(|row| row.record.id == id)
            .map(|row| row.record.clone())
        {
            if let Some(journal) = &self.journal
                && let Err(error) = journal.append(&record)
            {
                self.journal_error = error.to_string();
            }
            self.document.saved_at.insert(id, now);
        }
    }
    fn document_preserve_rejected(
        &mut self,
        kind: SourceKind,
        text: String,
        start_ms: u64,
        end_ms: u64,
        error: String,
    ) {
        let id = self.next_id;
        self.next_id += 1;
        self.rows.push(Row {
            record: Record {
                id,
                start_ms,
                end_ms: end_ms.max(start_ms),
                source: text,
                translation: String::new(),
                standard_translation: None,
                kind: source_name(kind, &self.video_id).into(),
                video_id: if kind == SourceKind::Caption {
                    self.video_id.clone()
                } else {
                    String::new()
                },
                created_at: chrono::Local::now().to_rfc3339(),
                paragraph: Some(ParagraphMeta {
                    revision: 0,
                    state: ParagraphState::Failed,
                    close_reason: Some("input_rejected".into()),
                }),
            },
            pending: false,
            error: Some(error),
            generation: self.source_generation(source_name(kind, &self.video_id)),
        });
        self.document_save(id, true);
        self.status = "文章の一部を原文のまま保存しました。履歴で確認できます。".into();
    }
    fn document_apply(&mut self, kind: SourceKind, outcome: PushOutcome) {
        let overflow: HashSet<_> = outcome
            .overflow
            .iter()
            .map(|s| (s.stamp.clone(), s.key))
            .collect();
        for snapshot in outcome.closed.iter().chain(outcome.overflow.iter()) {
            let id = self.document_row(snapshot);
            if overflow.contains(&(snapshot.stamp.clone(), snapshot.key)) {
                if let Some(row) = self.rows.iter_mut().find(|r| r.record.id == id) {
                    row.pending = false;
                    row.record.paragraph = Some(ParagraphMeta {
                        revision: snapshot.revision,
                        state: ParagraphState::Failed,
                        close_reason: Some("queue_pressure".into()),
                    });
                    row.error = Some("翻訳の処理待ちが増えたため、原文を保存しました。".into());
                }
                self.status = "処理待ちの原文を保存しました。履歴から確認できます。".into();
            }
            self.document_save(id, true);
        }
        for snapshot in self.document.snapshots(kind) {
            let id = self.document_row(&snapshot);
            self.document_save(id, false);
        }
        let visible = match kind {
            SourceKind::Caption => self.caption_enabled && !self.caption_paused,
            SourceKind::Audio => self.audio_enabled && !self.captions_fresh(),
        };
        if visible
            && let Some(snapshot) = self.document.source(kind).current_snapshot().or_else(|| {
                self.document
                    .source(kind)
                    .pending_snapshots()
                    .last()
                    .cloned()
            })
        {
            let id = self.document_row(&snapshot);
            self.current_live_id = Some(id);
        }
    }
    pub(super) fn document_ingest_caption(&mut self, caption: bridge::Caption) {
        self.document.caption_media_paused = caption.paused;
        self.document_ensure_scope(SourceKind::Caption);
        let now = self.document.now();
        let start = (caption.position * 1000.0).max(0.0) as u64;
        if !caption.paused
            && !caption.text.trim().is_empty()
            && self
                .document
                .caption_position_ms
                .is_none_or(|previous| start > previous)
        {
            // A stable cue can remain visible for several seconds. Advancing
            // playback is activity, not a new source revision or LLM request.
            self.document.caption.note_activity(now);
            self.document.caption_position_ms = Some(start);
        }
        if caption.text == self.last_live_text {
            return;
        }
        // The extension offers the visible caption again every two seconds.
        // One that cannot join the paragraph is preserved once, not each time.
        self.last_live_text.clone_from(&caption.text);
        match self.document.caption.push_caption(
            &caption.text,
            start,
            start.saturating_add(3_000),
            now,
        ) {
            Ok(outcome) => self.document_apply(SourceKind::Caption, outcome),
            Err(error) => self.document_preserve_rejected(
                SourceKind::Caption,
                caption.text,
                start,
                start.saturating_add(3_000),
                format!("文章をつなげられませんでした: {error}"),
            ),
        }
    }
    pub(super) fn document_ingest_audio(&mut self, transcript: PendingTranscript) {
        if transcript.generation != self.audio_generation
            || !self.audio_enabled
            || self.captions_fresh()
        {
            return;
        }
        self.document_ensure_scope(SourceKind::Audio);
        let now = self.document.now();
        let mut identity = std::hash::DefaultHasher::new();
        transcript.start_ms.hash(&mut identity);
        transcript.end_ms.hash(&mut identity);
        transcript.text.hash(&mut identity);
        let id = identity.finish();
        let fragment = AudioFragment {
            id,
            text: transcript.text,
            start_ms: transcript.start_ms,
            end_ms: transcript.end_ms,
        };
        let original = fragment.clone();
        match self.document.audio.push_audio(fragment, now) {
            Ok(outcome) => self.document_apply(SourceKind::Audio, outcome),
            Err(error) => self.document_preserve_rejected(
                SourceKind::Audio,
                original.text,
                original.start_ms,
                original.end_ms,
                format!("文章をつなげられませんでした: {error}"),
            ),
        }
    }
    pub(super) fn document_close(&mut self, kind: SourceKind) {
        let now = self.document.now();
        let outcome = self.document.source_mut(kind).close(now);
        self.document_apply(kind, outcome);
    }
    fn document_interrupt_source(&mut self, kind: SourceKind, reason: &str) {
        if self
            .document
            .displayed
            .as_ref()
            .is_some_and(|cached| cached.snapshot.stamp.kind == kind)
        {
            self.document.displayed = None;
        }
        for snapshot in self.document.snapshots(kind) {
            let id = self.document_row(&snapshot);
            // A paragraph whose translation already covers all of its text has
            // nothing left to finish. The end of a video, a seek, another video
            // or an input switch stops it complete, not interrupted.
            let covered = self
                .document
                .translated_source
                .get(&(snapshot.stamp.clone(), snapshot.key))
                .is_some_and(|source| *source == snapshot.text);
            if let Some(row) = self.rows.iter_mut().find(|r| r.record.id == id) {
                if row.record.paragraph.as_ref().is_some_and(|p| {
                    p.state != ParagraphState::Failed && p.state != ParagraphState::Final
                }) {
                    let complete = covered && !row.record.translation.trim().is_empty();
                    row.record.paragraph = Some(ParagraphMeta {
                        revision: snapshot.revision,
                        state: if complete {
                            ParagraphState::Final
                        } else {
                            ParagraphState::Interrupted
                        },
                        close_reason: Some(reason.into()),
                    });
                }
                row.pending = false;
            }
            self.document_save(id, true);
            if self.current_live_id == Some(id) {
                self.current_live_id = None;
            }
        }
    }
    pub(super) fn document_shutdown(&mut self) {
        self.document_cancel_flight();
        self.document_interrupt_source(SourceKind::Audio, "interrupted");
        self.document_interrupt_source(SourceKind::Caption, "interrupted");
    }
    pub(super) fn document_reset(&mut self) {
        self.document_cancel_flight();
        self.document = DocumentState::default();
    }
    pub(super) fn document_tick(&mut self) {
        let chunk = self.settings.live_chunk;
        if self.document.applied_chunk != Some(chunk) {
            self.document.audio.set_policy(chunk.policy());
            self.document.caption.set_policy(chunk.policy());
            self.document.applied_chunk = Some(chunk);
        }
        let now = self.document.now();
        // Draft activity must not close the preceding piece while an utterance continues.
        if self.asr_partial.is_empty() || !self.audio_enabled {
            let outcome = self.document.audio.tick(now);
            self.document_apply(SourceKind::Audio, outcome);
        }
        if !self.document.caption_media_paused {
            let outcome = self.document.caption.tick(now);
            self.document_apply(SourceKind::Caption, outcome);
        }
        if self.close_after_restore.is_some() {
            return;
        }
        let options = self.settings.translation.clone();
        if self
            .document
            .observed_options
            .as_ref()
            .is_some_and(|old| old != &options)
        {
            self.document_cancel_flight();
            self.document.configure_after = now.saturating_add(500);
        }
        self.document.observed_options = Some(options.clone());
        if !self.backend_ready
            || self.live_busy.is_some()
            || self.document.flight.is_some()
            || now < self.document.configure_after
        {
            return;
        }
        let audio = self.document.audio.next_request(now);
        let caption = self.document.caption.next_request(now);
        let prefer_caption = self.captions_fresh() || !self.audio_enabled;
        let mut candidates = [audio, caption].into_iter().flatten().collect::<Vec<_>>();
        candidates.sort_by_key(|request| {
            let row_id = self
                .document
                .rows
                .get(&(request.snapshot.stamp.clone(), request.snapshot.key))
                .copied()
                .unwrap_or(u64::MAX);
            (
                !request.snapshot.closed,
                if request.snapshot.closed {
                    row_id
                } else {
                    u64::from(
                        (request.snapshot.stamp.kind == SourceKind::Caption) != prefer_caption,
                    )
                },
            )
        });
        let Some(request) = candidates.first().cloned() else {
            return;
        };
        for unused in candidates.into_iter().skip(1) {
            self.document
                .source_mut(unused.snapshot.stamp.kind)
                .cancel_request(&unused, now);
        }
        let row_id = self.document_row(&request.snapshot);
        if let Some(row) = self.rows.iter_mut().find(|r| r.record.id == row_id) {
            row.pending = true;
        }
        let cancel = Arc::new(AtomicBool::new(false));
        self.document.flight = Some(Flight {
            row_id,
            request: request.clone(),
            options: options.clone(),
            cancel: cancel.clone(),
        });
        self.live_busy = Some(row_id);
        let kind = self.document_tools_kind(request.snapshot.stamp.kind);
        if self.settings.english_punctuation
            && paragraph_tools::use_english_support(&options, kind, &request.snapshot.text)
            && self
                .document
                .source(request.snapshot.stamp.kind)
                .wants_sentence_hints(&request, now)
            && let Some(engine) = self.paragraph_tools.engine.clone()
        {
            let tx = self.tx.clone();
            thread::spawn(move || {
                let result = engine
                    .punctuate(&request.snapshot.text, &cancel)
                    .map_err(|error| format!("{error:#}"));
                let _ = tx.send(Event::DocumentPunctuated {
                    row_id,
                    request,
                    options,
                    result,
                });
            });
        } else {
            self.document_translate(row_id, request, options, cancel);
        }
    }
    pub(super) fn document_punctuated(
        &mut self,
        row_id: u64,
        request: Request,
        options: TranslationOptions,
        result: std::result::Result<plamo_translate::punctuation::PunctuationResult, String>,
    ) {
        let Some(flight) = self.document.flight.as_ref().filter(|flight| {
            flight.row_id == row_id && flight.request == request && flight.options == options
        }) else {
            return;
        };
        if options != self.settings.translation || flight.cancel.load(Ordering::Acquire) {
            self.document_cancel_flight();
            return;
        }
        let cancel = flight.cancel.clone();
        // A failed or out-of-date answer is no failure of the helper's
        // preparation: the paragraph is translated without its hints.
        if let Ok(punctuation) = result
            && punctuation.original == request.snapshot.text
        {
            let now = self.document.now();
            let kind = request.snapshot.stamp.kind;
            let outcome = self.document.source_mut(kind).apply_sentence_hints(
                &request,
                &punctuation.boundaries,
                now,
            );
            if outcome.changed {
                self.document_cancel_flight();
                self.document_apply(kind, outcome);
                return;
            }
            // A punctuation tagger can preserve every word yet still place
            // a comma/full stop badly. Use only guarded boundary hints;
            // PLaMo never reads the tagger's text.
        }
        self.document_translate(row_id, request, options, cancel);
    }
    fn document_tools_kind(&self, kind: SourceKind) -> paragraph_tools::SourceKindForTools {
        if kind == SourceKind::Audio && self.settings.asr_backend == "nemotron" {
            paragraph_tools::SourceKindForTools::EnglishAsr
        } else {
            paragraph_tools::SourceKindForTools::Other
        }
    }
    /// The source and previous source PLaMo reads. English loses the
    /// hesitation sounds of speech, which PLaMo would translate as 「えー」,
    /// and gets rates in quarters and halves as decimals, which it would keep
    /// as 「3と3/4」. Fillers go first, so "3 and uh 3/4" becomes "3.75".
    /// Screen text keeps its fillers, as its author wrote them. Other
    /// languages keep every word ("um" is Portuguese for "a"). The previous
    /// source is language evidence too, so a short first interim of a later
    /// part still counts as English.
    fn document_translator_input(
        &self,
        request: &Request,
        options: &TranslationOptions,
    ) -> (String, String) {
        let source = &request.snapshot.text;
        let previous = &request.previous_source;
        let screen = request
            .snapshot
            .stamp
            .video_id
            .starts_with(super::screen_feed::SCREEN_SOURCE);
        let kind = self.document_tools_kind(request.snapshot.stamp.kind);
        let evidence = format!("{previous} {source}");
        if !paragraph_tools::use_english_support(options, kind, &evidence) {
            (source.clone(), previous.clone())
        } else if screen {
            (
                live_text::with_decimal_fractions(source),
                live_text::with_decimal_fractions(previous),
            )
        } else {
            (
                live_text::with_decimal_fractions(&live_text::without_fillers(source)),
                live_text::with_decimal_fractions(&live_text::without_fillers(previous)),
            )
        }
    }
    fn document_translate(
        &self,
        row_id: u64,
        request: Request,
        options: TranslationOptions,
        cancel: Arc<AtomicBool>,
    ) {
        let (source, previous_source) = self.document_translator_input(&request, &options);
        let tx = self.tx.clone();
        thread::spawn(move || {
            let started = Instant::now();
            let result = (|| -> Result<LiveTranslation> {
                let mut live_options = options.clone();
                live_options.context = live_text::compose_context(
                    &options.context,
                    &previous_source,
                    &request.previous_target,
                    4_000,
                )?;
                let standard =
                    Translator::new(live_options)?.translate_live(&source, &cancel, |_| {})?;
                let easy = if options.target_lang == "ja" {
                    rewrite_as_easy_japanese(&options, &standard, &cancel)?
                } else {
                    None
                };
                Ok(LiveTranslation { standard, easy })
            })()
            .map_err(|error| format!("{error:#}"));
            let _ = tx.send(Event::DocumentDone {
                row_id,
                request,
                options,
                result,
                elapsed: started.elapsed().as_secs_f32(),
            });
        });
    }
    pub(super) fn document_done(
        &mut self,
        row_id: u64,
        request: Request,
        options: TranslationOptions,
        result: std::result::Result<LiveTranslation, String>,
        elapsed: f32,
    ) {
        if !self.document.flight.as_ref().is_some_and(|flight| {
            flight.row_id == row_id && flight.request == request && flight.options == options
        }) {
            return;
        }
        self.document.flight = None;
        if self.live_busy == Some(row_id) {
            self.live_busy = None;
        }
        if let Some(row) = self.rows.iter_mut().find(|r| r.record.id == row_id) {
            row.pending = false;
        }
        let kind = request.snapshot.stamp.kind;
        let now = self.document.now();
        if options != self.settings.translation {
            self.document.source_mut(kind).cancel_request(&request, now);
            return;
        }
        let (acceptance, easy) = match result {
            Ok(LiveTranslation { standard, easy }) => (
                self.document
                    .source_mut(kind)
                    .accept(&request, &standard, now),
                easy,
            ),
            Err(error) => {
                self.document.source_mut(kind).fail(&request, now);
                let key = (request.snapshot.stamp.clone(), request.snapshot.key);
                let failures = self
                    .document
                    .failures
                    .entry(key)
                    .or_insert((request.snapshot.revision, 0));
                if failures.0 != request.snapshot.revision {
                    *failures = (request.snapshot.revision, 0);
                }
                failures.1 += 1;
                if failures.1 > 1 {
                    if let Some(row) = self.rows.iter_mut().find(|r| r.record.id == row_id) {
                        row.error = Some(error.clone());
                        row.record.paragraph = Some(ParagraphMeta {
                            revision: request.snapshot.revision,
                            state: ParagraphState::Failed,
                            close_reason: Some("translation_failed".into()),
                        });
                    }
                    self.document_save(row_id, true);
                }
                self.status = error;
                return;
            }
        };
        let (snapshot, standard, final_paragraph) = match acceptance {
            Acceptance::Rejected => return,
            Acceptance::Interim { snapshot, target } => (snapshot, target, false),
            Acceptance::Committed { snapshot, target } => (snapshot, target, true),
        };
        // Context for the next paragraph stays the ordinary translation that
        // `accept` kept. The user reads the rewrite when there is one.
        let (target, standard_translation) = match easy {
            Some(easy) => (easy, Some(standard)),
            None => (standard, None),
        };
        let key = (snapshot.stamp.clone(), snapshot.key);
        self.document
            .translated_source
            .insert(key.clone(), snapshot.text.clone());
        let source_snapshot = self
            .document
            .snapshots(kind)
            .into_iter()
            .find(|s| s.key == snapshot.key)
            .unwrap_or_else(|| snapshot.clone());
        let id = self.document_row(&source_snapshot);
        if let Some(row) = self.rows.iter_mut().find(|r| r.record.id == id) {
            row.record.translation.clone_from(&target);
            row.record.standard_translation = standard_translation;
            row.error = None;
            if final_paragraph {
                row.record.source.clone_from(&snapshot.text);
                row.record.start_ms = snapshot.start_ms;
                row.record.end_ms = snapshot.end_ms;
                row.record.paragraph = Some(ParagraphMeta {
                    revision: snapshot.revision,
                    state: ParagraphState::Final,
                    close_reason: Some("completed".into()),
                });
            } else {
                let mut visible = row.clone();
                visible.record.source.clone_from(&snapshot.text);
                visible.record.start_ms = snapshot.start_ms;
                visible.record.end_ms = snapshot.end_ms;
                visible.record.paragraph = Some(ParagraphMeta {
                    revision: snapshot.revision,
                    state: ParagraphState::Open,
                    close_reason: None,
                });
                self.document.displayed = Some(VisibleInterim {
                    snapshot: snapshot.clone(),
                    row: visible,
                });
            }
        }
        if final_paragraph
            && self.document.displayed.as_ref().is_some_and(|cached| {
                cached.snapshot.stamp == snapshot.stamp && cached.snapshot.key == snapshot.key
            })
        {
            self.document.displayed = None;
        }
        self.document_save(id, true);
        if final_paragraph && self.document.spoken.insert(key) {
            let name = source_name(kind, &snapshot.stamp.video_id);
            if self.may_read_aloud(name, snapshot.stamp.generation) && self.settings.auto_read {
                self.auto_read_translation(target, true);
            }
        }
        self.status = if final_paragraph {
            format!("文章を確定しました · {elapsed:.1}秒")
        } else {
            "文脈をつないで翻訳しています。".into()
        };
    }
}

/// The rewrite refines a translation that already exists, so when it fails the
/// paragraph keeps the ordinary Japanese. Only a cancellation is an error.
fn rewrite_as_easy_japanese(
    options: &TranslationOptions,
    standard: &str,
    cancel: &AtomicBool,
) -> Result<Option<String>> {
    match Translator::new(options.easy_japanese())
        .and_then(|translator| translator.translate_live(standard, cancel, |_| {}))
    {
        Ok(easy) => Ok(Some(easy)),
        Err(error) if cancel.load(Ordering::Acquire) => Err(error),
        Err(_) => Ok(None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    impl From<&str> for LiveTranslation {
        fn from(standard: &str) -> Self {
            Self {
                standard: standard.into(),
                easy: None,
            }
        }
    }

    fn audio_app() -> App {
        let mut app = App::inert();
        app.audio_enabled = true;
        app
    }
    fn feed(app: &mut App, text: &str, start_ms: u64, end_ms: u64) {
        app.document_ingest_audio(PendingTranscript {
            text: text.into(),
            start_ms,
            end_ms,
            generation: app.audio_generation,
        });
    }
    fn take_job(app: &mut App, kind: SourceKind, now: u64) -> (u64, Request, TranslationOptions) {
        let request = app.document.source_mut(kind).next_request(now).unwrap();
        let row_id = app.document_row(&request.snapshot);
        let options = app.settings.translation.clone();
        app.document.flight = Some(Flight {
            row_id,
            request: request.clone(),
            options: options.clone(),
            cancel: Arc::new(AtomicBool::new(false)),
        });
        app.live_busy = Some(row_id);
        app.rows
            .iter_mut()
            .find(|row| row.record.id == row_id)
            .unwrap()
            .pending = true;
        (row_id, request, options)
    }

    #[test]
    fn separate_subtitle_pairs_only_the_accepted_source_and_target_revision() {
        let mut app = audio_app();
        let source = "The library will open a new exhibition for local families next Friday";
        feed(&mut app, source, 0, 1_000);
        let (id, request, options) = take_job(&mut app, SourceKind::Audio, 1_500);
        app.document_done(
            id,
            request,
            options,
            Ok("図書館は来週金曜日に地域の家族向けの新しい展示を始めます。".into()),
            0.1,
        );
        app.document_update_subtitle_window();
        let accepted = app.subtitle_window.view().unwrap().clone();
        assert_eq!(accepted.source, source);
        feed(
            &mut app,
            "Visitors can compare the original documents and translated descriptions at every desk",
            1_000,
            2_000,
        );
        app.document_update_subtitle_window();
        let pending = app.subtitle_window.view().unwrap();
        assert_eq!(pending.source, accepted.source);
        assert_eq!(pending.target, accepted.target);
        assert_eq!(pending.revision, accepted.revision);
        assert!(app.rows[0].record.source.len() > pending.source.len());

        let (id, request, options) = take_job(&mut app, SourceKind::Audio, 5_000);
        let updated_source = request.snapshot.text.clone();
        app.document_done(
            id,
            request,
            options,
            Ok("図書館は新しい展示を始め、来場者は資料の原文と翻訳を各机で比較できます。".into()),
            0.1,
        );
        app.document_update_subtitle_window();
        assert_eq!(app.subtitle_window.view().unwrap().source, updated_source);
        assert!(app.subtitle_window.view().unwrap().revision > accepted.revision);
    }

    #[test]
    fn separate_subtitle_retains_paused_and_stopped_text_but_clears_a_new_video() {
        use super::super::subtitle_window::Playback;
        let mut app = App::inert();
        app.caption_enabled = true;
        let caption = |video: &str, paused| bridge::Caption {
            video_id: video.into(),
            text: "The library will hold a public exhibition about the town's history next Friday"
                .into(),
            position: 1.0,
            paused,
            epoch: 1,
        };
        app.accept_caption(caption("video-a", false));
        app.document_take_caption();
        let (id, request, options) = take_job(&mut app, SourceKind::Caption, 1_500);
        app.document_done(
            id,
            request,
            options,
            Ok("図書館は町の歴史に関する公開展示を来週金曜日に開きます。".into()),
            0.1,
        );
        app.document_update_subtitle_window();
        let accepted = app.subtitle_window.view().unwrap().clone();
        app.accept_caption(caption("video-a", true));
        app.document_update_subtitle_window();
        assert_eq!(app.subtitle_window.view(), Some(&accepted));
        assert_eq!(app.subtitle_window.playback(), Playback::Paused);
        app.set_caption_enabled(false);
        app.document_update_subtitle_window();
        assert_eq!(app.subtitle_window.view().unwrap().target, accepted.target);
        assert_eq!(app.subtitle_window.playback(), Playback::Stopped);
        app.set_caption_enabled(true);
        app.accept_caption(caption("video-b", false));
        app.document_update_subtitle_window();
        assert!(app.subtitle_window.view().is_none());
        assert_eq!(app.subtitle_window.scope().unwrap().video_id, "video-b");
    }

    #[test]
    fn new_timeline_clears_subtitle_content_without_closing_the_window() {
        let mut app = audio_app();
        feed(
            &mut app,
            "The exhibition will open on Friday and visitors can compare both languages",
            0,
            1_000,
        );
        let (id, request, options) = take_job(&mut app, SourceKind::Audio, 1_500);
        app.document_done(
            id,
            request,
            options,
            Ok("展示は金曜日に始まります。".into()),
            0.1,
        );
        app.document_update_subtitle_window();
        app.subtitle_window.open = true;
        app.subtitle_window.show_source = true;
        app.subtitle_window.font_size = 30.0;
        app.audio_enabled = false;
        app.rows.clear();
        app.invalidate_timeline();
        app.document_update_subtitle_window();
        assert!(app.subtitle_window.open);
        assert!(app.subtitle_window.show_source);
        assert_eq!(app.subtitle_window.font_size, 30.0);
        assert!(app.subtitle_window.view().is_none());
    }

    #[test]
    fn chunk_length_setting_reaches_both_live_sources_on_the_next_tick() {
        let saved_before: Settings = serde_json::from_str("{}").unwrap();
        assert_eq!(
            saved_before.live_chunk,
            ChunkLength::Standard,
            "settings saved before the option open as standard"
        );
        let mut app = App::inert();
        app.settings.live_chunk = ChunkLength::Short;
        app.document_tick();
        for source in [&app.document.audio, &app.document.caption] {
            assert_eq!(source.policy().max_paragraph_chars, 240);
            assert_eq!(source.policy().max_paragraph_cjk_chars, 120);
        }
        app.settings.live_chunk = ChunkLength::Long;
        app.document_tick();
        assert_eq!(
            app.document.caption.policy().max_paragraph_chars,
            Policy::default().max_paragraph_chars,
            "long keeps the previous paragraph length"
        );
    }
    #[test]
    fn subtitle_window_keeps_every_fixed_part_as_its_own_message() {
        let mut app = audio_app();
        feed(
            &mut app,
            "The library is preparing a public exhibition for local families.",
            0,
            2_000,
        );
        app.document_close(SourceKind::Audio);
        let (id, request, options) = take_job(&mut app, SourceKind::Audio, 2_000);
        let fixed = "図書館は地域の家族向けの公開展示を準備しています。";
        app.document_done(id, request, options, Ok(fixed.into()), 0.1);
        feed(
            &mut app,
            "Visitors can compare the original documents and translated descriptions",
            3_000,
            5_000,
        );
        let (id, request, options) = take_job(&mut app, SourceKind::Audio, 5_000);
        let forming = "来場者は原本の資料と翻訳された説明を比べられます";
        app.document_done(id, request, options, Ok(forming.into()), 0.1);
        app.document_update_subtitle_window();
        let history = app.subtitle_window.history();
        assert_eq!(history.len(), 1, "{history:?}");
        assert_eq!(history[0].target, fixed);
        assert_eq!(app.subtitle_window.view().unwrap().target, forming);
    }
    #[test]
    fn tagged_split_keeps_raw_text_and_rejects_the_old_whole_paragraph_job() {
        let mut app = audio_app();
        let section = "The exhibition contains about 150 English words and a Japanese translation A shared glossary keeps local place names consistent while staff explain unclear descriptions in ordinary language Visitors can compare the original material with a translation before leaving their comments";
        let prefix = [section; 3].join(" ");
        let tail = "The library team will review every comment on Friday and keep unfinished suggestions separate from completed revisions";
        feed(&mut app, &prefix, 0, 1_000);
        feed(&mut app, tail, 1_000, 2_000);
        let (first_id, first, first_options) = take_job(&mut app, SourceKind::Audio, 3_000);
        let translated_source = first.snapshot.text.clone();
        app.document_done(
            first_id,
            first,
            first_options,
            Ok("全体に対応する古い途中訳".into()),
            0.1,
        );
        let continuation =
            "No personal information is required and the next version will be published later";
        feed(&mut app, continuation, 2_000, 3_000);
        let (id, request, options) = take_job(&mut app, SourceKind::Audio, 5_000);
        let cancelled = app.document.flight.as_ref().unwrap().cancel.clone();
        app.document_punctuated(
            id,
            request.clone(),
            options.clone(),
            Ok(plamo_translate::punctuation::PunctuationResult {
                original: request.snapshot.text.clone(),
                restored: "diagnostic text is never translated or saved".into(),
                boundaries: vec![prefix.len()],
            }),
        );
        assert!(cancelled.load(Ordering::Acquire));
        assert!(app.document.flight.is_none());
        assert_eq!(app.rows[0].record.source, prefix);
        assert!(app.rows[0].record.translation.is_empty());
        assert_eq!(app.rows[1].record.source, format!("{tail} {continuation}"));
        let visible = app.document_visible_row().unwrap();
        assert_eq!(visible.record.source, translated_source);
        assert_eq!(visible.record.translation, "全体に対応する古い途中訳");
        assert!(visible.is_provisional() && !visible.is_confirmed());
        assert_eq!(app.document.audio.pending_snapshots()[0].text, prefix);
        app.document_done(
            id,
            request.clone(),
            options.clone(),
            Ok("late translation of the old full paragraph".into()),
            0.1,
        );
        assert!(app.rows.iter().all(|row| !row.is_confirmed()));
        app.audio_generation += 1;
        app.document_source_changed(SourceKind::Audio);
        app.document_punctuated(id, request, options, Err("late tagger result".into()));
        assert!(app.paragraph_tools.error.is_empty());
        assert!(app.document.audio.pending_snapshots().is_empty());
        assert!(app.document.displayed.is_none());
    }

    #[test]
    fn advancing_caption_heartbeats_and_player_pause_do_not_finalize_fragments() {
        let mut app = App::inert();
        app.caption_enabled = true;
        let caption = |text: &str, position: f64, paused| bridge::Caption {
            text: text.into(),
            video_id: "caption-cadence".into(),
            position,
            paused,
            epoch: 1,
        };
        let first = "The labor market remains strong while inflation";
        app.accept_caption(caption(first, 10.0, false));
        app.document_take_caption();
        let (id, request, options) = take_job(&mut app, SourceKind::Caption, 1_500);
        app.document_done(
            id,
            request,
            options,
            Ok("労働市場は引き続き堅調ですが、インフレは".into()),
            0.1,
        );
        let revision = app.document.caption.current_snapshot().unwrap().revision;
        for second in (2..=30).step_by(2) {
            app.document.started = Instant::now() - Duration::from_secs(second);
            app.accept_caption(caption(first, 10.0 + second as f64, false));
            app.document_take_caption();
            app.document_tick();
            assert!(app.document.caption.pending_snapshots().is_empty());
            assert_eq!(
                app.document.caption.current_snapshot().unwrap().revision,
                revision
            );
            assert!(app.document.caption.next_request(second * 1_000).is_none());
        }
        app.accept_caption(caption("", 40.0, true));
        app.document.started = Instant::now() - Duration::from_secs(120);
        app.document_tick();
        assert!(app.rows[0].is_provisional());
        assert!(app.document.caption.pending_snapshots().is_empty());
        let next = "is still above the target and requires continued attention";
        app.accept_caption(caption(next, 40.1, false));
        app.document_take_caption();
        app.document_tick();
        assert_eq!(
            app.document.caption.current_snapshot().unwrap().text,
            format!("{first} {next}")
        );
        assert!(app.document.caption.pending_snapshots().is_empty());
        assert_eq!(app.rows.len(), 1);
        assert!(app.document.spoken.is_empty());
    }

    #[test]
    fn all_recognized_words_accumulate_while_model_is_busy_and_only_exact_audio_repeats_dedup() {
        let mut app = audio_app();
        app.live_busy = Some(999);
        feed(&mut app, "We will review the exhibition together", 0, 1_000);
        feed(&mut app, "We will review the exhibition together", 0, 1_000);
        feed(
            &mut app,
            "We will review the exhibition together",
            1_000,
            2_000,
        );
        let mut expected =
            "We will review the exhibition together We will review the exhibition together"
                .to_owned();
        for index in 0..40 {
            let fragment = format!("section{index} describes a different public object");
            feed(
                &mut app,
                &fragment,
                2_000 + index * 1_000,
                3_000 + index * 1_000,
            );
            expected.push(' ');
            expected.push_str(&fragment);
        }
        let combined = app
            .rows
            .iter()
            .map(|row| row.record.source.as_str())
            .collect::<Vec<_>>()
            .join(" ");
        assert_eq!(
            combined.split_whitespace().collect::<Vec<_>>(),
            expected.split_whitespace().collect::<Vec<_>>()
        );
        assert_eq!(app.live_busy, Some(999));
        assert!(app.rows.iter().all(|row| !row.is_confirmed()));
        assert!(app.rows.len() < 10);
    }

    #[test]
    fn paragraph_translation_replaces_interim_and_finalizes_once_without_replaying_history() {
        let mut app = audio_app();
        feed(
            &mut app,
            "The library is preparing a public exhibition",
            0,
            2_000,
        );
        let (id, request, options) = take_job(&mut app, SourceKind::Audio, 1_500);
        app.document_done(
            id,
            request,
            options,
            Ok("図書館は公開展示を準備しています。".into()),
            0.1,
        );
        assert!(app.rows[0].is_provisional());
        assert!(!app.rows[0].is_confirmed());
        assert!(app.document.spoken.is_empty());
        feed(
            &mut app,
            "and visitors can compare the original objects with their translations",
            2_000,
            5_000,
        );
        app.document_close(SourceKind::Audio);
        let (id, request, options) = take_job(&mut app, SourceKind::Audio, 5_000);
        let final_text =
            "図書館は公開展示を準備しており、来場者は展示物の原物と翻訳を比較できます。";
        app.document_done(
            id,
            request.clone(),
            options.clone(),
            Ok(final_text.into()),
            0.2,
        );
        assert_eq!(app.rows.len(), 1);
        assert_eq!(app.rows[0].record.translation, final_text);
        assert!(app.rows[0].is_confirmed());
        assert_eq!(app.document.spoken.len(), 1);
        assert!(app.document.displayed.is_none());
        app.document_done(id, request, options, Ok("late duplicate".into()), 0.2);
        assert_eq!(app.rows[0].record.translation, final_text);
        assert_eq!(app.document.spoken.len(), 1);
    }

    #[test]
    fn correction_and_new_video_cannot_publish_an_old_translation_or_context() {
        let mut app = App::inert();
        app.caption_enabled = true;
        app.caption_paused = false;
        app.video_id = "video-a".into();
        app.caption_epoch = 2;
        let caption = |text: &str, position| bridge::Caption {
            text: text.into(),
            video_id: "video-a".into(),
            position,
            epoch: 2,
            paused: false,
        };
        app.document_ingest_caption(caption(
            "The loan period is five days for registered readers",
            1.0,
        ));
        let (id, request, options) = take_job(&mut app, SourceKind::Caption, 1_500);
        app.document_ingest_caption(caption(
            "The loan period is six days for registered readers",
            1.1,
        ));
        app.document_done(id, request, options, Ok("貸出期間は5日です。".into()), 0.1);
        assert!(app.rows[0].record.translation.is_empty());
        let (id, request, options) = take_job(&mut app, SourceKind::Caption, 4_000);
        app.document_done(id, request, options, Ok("貸出期間は6日です。".into()), 0.1);
        assert!(app.document.displayed.is_some());
        app.document_ingest_caption(caption(
            "The loan period is seven days for registered readers",
            1.2,
        ));
        assert!(
            app.document_visible_row()
                .unwrap()
                .record
                .translation
                .is_empty()
        );
        let (id, request, options) = take_job(&mut app, SourceKind::Caption, 6_500);
        let cancelled = app.document.flight.as_ref().unwrap().cancel.clone();
        app.video_id = "video-b".into();
        app.caption_epoch = 3;
        app.caption_generation += 1;
        app.document_source_changed(SourceKind::Caption);
        assert!(cancelled.load(Ordering::Acquire));
        assert_eq!(
            app.rows[0].record.paragraph.as_ref().unwrap().state,
            ParagraphState::Interrupted
        );
        app.document_done(id, request, options, Ok("late old video".into()), 0.1);
        assert!(app.rows[0].record.translation.is_empty());
        app.document_ingest_caption(bridge::Caption {
            text: "The other program begins with a different subject".into(),
            video_id: "video-b".into(),
            position: 0.0,
            epoch: 3,
            paused: false,
        });
        let (_, request, _) = take_job(&mut app, SourceKind::Caption, 5_000);
        assert!(request.previous_source.is_empty() && request.previous_target.is_empty());
    }

    fn tip_caption(text: &str, position: f64, paused: bool, epoch: u64) -> bridge::Caption {
        bridge::Caption {
            text: text.into(),
            video_id: "tip".into(),
            position,
            paused,
            epoch,
        }
    }

    const TIP: &str = "thanks for watching this short tip and see you next time";
    const TIP_TARGET: &str = "この短いコツを見てくれてありがとう。またね。";

    fn translated_tip() -> App {
        let mut app = App::inert();
        app.caption_enabled = true;
        app.accept_caption(tip_caption(TIP, 5.0, false, 1));
        app.document_take_caption();
        let (id, request, options) = take_job(&mut app, SourceKind::Caption, 1_500);
        app.document_done(id, request, options, Ok(TIP_TARGET.into()), 0.1);
        app
    }

    #[test]
    fn a_video_that_ends_keeps_its_translated_last_paragraph_finished() {
        let mut app = translated_tip();
        // The extension reports the end of a video as a new epoch with an empty caption.
        app.accept_caption(tip_caption("", 9.9, true, 2));
        let row = &app.rows[0];
        assert_eq!(
            row.record.paragraph.as_ref().map(|p| p.state),
            Some(ParagraphState::Final),
            "the translation covers the whole paragraph"
        );
        assert!(row.is_confirmed());
        assert_eq!(row.record.source, TIP);
        assert_eq!(row.record.translation, TIP_TARGET);
    }

    #[test]
    fn a_video_that_ends_before_its_last_words_are_translated_stays_interrupted() {
        let mut app = translated_tip();
        app.accept_caption(tip_caption(
            &format!("{TIP} and remember to subscribe"),
            7.0,
            false,
            1,
        ));
        app.document_take_caption();
        app.accept_caption(tip_caption("", 9.9, true, 2));
        let row = &app.rows[0];
        assert_eq!(
            row.record.paragraph.as_ref().map(|p| p.state),
            Some(ParagraphState::Interrupted),
            "the last words were never translated"
        );
        assert!(!row.is_confirmed());
    }

    #[test]
    fn captions_taking_over_keep_the_translated_audio_paragraph_finished() {
        let mut app = audio_app();
        app.caption_enabled = true;
        app.settings.asr_backend = "nemotron".into();
        let target = "字幕について質問したくて、動画を一時停止しました。";
        feed(
            &mut app,
            "I paused the video to ask a question about the captions",
            0,
            3_000,
        );
        let (id, request, options) = take_job(&mut app, SourceKind::Audio, 4_000);
        app.document_done(id, request, options, Ok(target.into()), 0.1);
        // Captions close the audio paragraph and the recognizer restarts in a
        // new epoch. The paragraph keeps its translation and gets its closing
        // pass instead of being cut off.
        app.accept_caption(tip_caption("Thanks for joining us today.", 0.6, false, 1));
        app.nemotron_epoch += 1;
        app.process_streaming_audio();
        let (id, request, options) = take_job(&mut app, SourceKind::Audio, 4_500);
        assert!(request.snapshot.closed);
        app.document_done(id, request, options, Ok(target.into()), 0.1);
        let row = app.rows.iter().find(|row| row.record.id == id).unwrap();
        assert_eq!(row.record.translation, target);
        assert_eq!(
            row.record.paragraph.as_ref().map(|p| p.state),
            Some(ParagraphState::Final),
            "the question was translated in full before the captions came"
        );
        assert!(row.is_confirmed());
    }

    #[test]
    fn captions_coming_and_going_still_translate_the_audio_paragraph_heard_before() {
        let mut app = audio_app();
        app.caption_enabled = true;
        app.settings.asr_backend = "nemotron".into();
        let heard = "In 2028 we expect to hold two strategy meetings and six policy meetings";
        feed(&mut app, heard, 0, 4_000);
        // YouTube comes back into view: its captions close the audio paragraph
        // and suspend the recognizer, which starts a new epoch.
        app.accept_caption(tip_caption("Thanks for joining us today.", 0.6, false, 1));
        app.nemotron_epoch += 1;
        app.process_streaming_audio();
        assert!(app.nemotron_suspended);
        let (id, request, options) = take_job(&mut app, SourceKind::Audio, 4_500);
        assert_eq!(request.snapshot.text, heard);
        // The cue disappears before that closing translation returns, so the
        // recognizer resumes in yet another epoch.
        app.accept_caption(tip_caption("", 2.0, false, 1));
        app.nemotron_epoch += 1;
        app.process_streaming_audio();
        assert!(!app.nemotron_suspended);
        let target = "2028年には、戦略の会合を2回、政策の会合を6回開く予定です。";
        app.document_done(id, request, options, Ok(target.into()), 0.1);
        let row = app.rows.iter().find(|row| row.record.id == id).unwrap();
        assert_eq!(row.record.translation, target);
        assert_eq!(
            row.record.paragraph.as_ref().map(|p| p.state),
            Some(ParagraphState::Final),
            "the paragraph heard before the captions came ends translated"
        );
        assert!(row.is_confirmed());
    }

    #[test]
    fn stop_flushes_last_source_without_reading_it_after_input_stops() {
        let mut app = audio_app();
        app.settings.readout_enabled = true;
        app.settings.auto_read = true;
        app.readout.ready = Some(plamo_translate::speech_setup::SpeechInstallation {
            executable: PathBuf::new(),
            model: PathBuf::new(),
            backend: String::new(),
            voice: String::new(),
        });
        app.settings.tts_voice_profile = "missing".into();
        feed(
            &mut app,
            "Readers can return to this complete explanation later",
            0,
            3_000,
        );
        app.stop_audio();
        assert!(app.document.audio.current_snapshot().is_none());
        let (id, request, options) = take_job(&mut app, SourceKind::Audio, 4_000);
        assert!(request.snapshot.closed);
        app.document_done(
            id,
            request,
            options,
            Ok("読者はあとでこの説明を読み返せます。".into()),
            0.1,
        );
        assert!(app.rows[0].is_confirmed());
        assert!(app.readout.engine.is_none());
        assert_eq!(
            app.readout.error, "",
            "the paragraph is not read after its input stopped"
        );
    }

    #[test]
    fn automatic_live_readout_never_opens_the_voice_preparation() {
        let mut app = audio_app();
        app.settings.readout_enabled = true;
        app.settings.auto_read = true;
        feed(
            &mut app,
            "Readers can return to this complete explanation later",
            0,
            3_000,
        );
        // Quiet closes a short paragraph, the way document_tick does; only a
        // closed paragraph reaches the live view and automatic readout.
        let outcome = app.document.audio.tick(20_000);
        app.document_apply(SourceKind::Audio, outcome);
        let (id, request, options) = take_job(&mut app, SourceKind::Audio, 20_000);
        assert!(request.snapshot.closed, "the quiet closed the paragraph");
        app.document_done(
            id,
            request,
            options,
            Ok("読者はあとでこの説明を読み返せます。".into()),
            0.1,
        );
        assert!(
            app.rows[0].is_confirmed(),
            "the paragraph reached the live view"
        );
        assert!(
            !app.readout.confirm,
            "with no voice, automatic readout stays quiet instead of asking again"
        );
    }

    fn live_request(
        kind: SourceKind,
        video_id: &str,
        text: &str,
        previous_source: &str,
    ) -> Request {
        Request {
            id: 1,
            snapshot: ParagraphSnapshot {
                stamp: SourceStamp {
                    kind,
                    generation: 1,
                    video_id: video_id.into(),
                    epoch: 0,
                },
                key: 0,
                revision: 0,
                text: text.into(),
                start_ms: 0,
                end_ms: 0,
                closed: false,
                finalizable: false,
            },
            previous_source: previous_source.into(),
            previous_target: String::new(),
        }
    }

    /// What the substitute server answers to one completion request.
    enum Completion {
        Text(&'static str),
        Failure,
    }

    /// Answers like the local PLaMo server and hands back every prompt it was
    /// sent, in order. Each completion gets the next scripted answer; a request
    /// beyond the script fails but is still counted.
    fn translation_stub(script: Vec<Completion>) -> (String, mpsc::Receiver<String>) {
        let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
        let endpoint = format!("http://{}", server.server_addr().to_ip().unwrap());
        let (prompts_tx, prompts) = mpsc::channel();
        thread::spawn(move || {
            let mut script = script.into_iter();
            for mut request in server.incoming_requests() {
                let url = request.url().to_owned();
                let response = match url.as_str() {
                    "/health" => tiny_http::Response::from_string("ok"),
                    "/v1/models" => {
                        tiny_http::Response::from_string(r#"{"data":[{"id":"plamo3"}]}"#)
                    }
                    _ => {
                        let mut body = String::new();
                        request.as_reader().read_to_string(&mut body).unwrap();
                        let body: serde_json::Value = serde_json::from_str(&body).unwrap();
                        let _ = prompts_tx.send(body["prompt"].as_str().unwrap().to_owned());
                        match script.next() {
                            Some(Completion::Text(text)) => {
                                let chunk = serde_json::json!({
                                    "choices": [{"text": text, "finish_reason": "stop"}]
                                });
                                tiny_http::Response::from_string(format!(
                                    "data: {chunk}\n\ndata: [DONE]\n\n"
                                ))
                            }
                            Some(Completion::Failure) | None => {
                                tiny_http::Response::from_string("model failed")
                                    .with_status_code(500)
                            }
                        }
                    }
                };
                let _ = request.respond(response);
            }
        });
        (endpoint, prompts)
    }

    fn caption_app(endpoint: String) -> App {
        let mut app = App::inert();
        app.caption_enabled = true;
        app.caption_paused = false;
        app.backend_ready = true;
        app.settings.translation.endpoint = endpoint;
        app
    }

    fn live_caption(app: &mut App, text: &str, position: f64) {
        app.document_ingest_caption(bridge::Caption {
            text: text.into(),
            video_id: "video-a".into(),
            position,
            paused: false,
            epoch: 2,
        });
    }

    /// Waits for the translation thread and applies its result as the event
    /// loop does.
    fn finish_translation(app: &mut App) {
        let Ok(Event::DocumentDone {
            row_id,
            request,
            options,
            result,
            elapsed,
        }) = app.rx.recv_timeout(Duration::from_secs(10))
        else {
            panic!("the translation thread sent no result");
        };
        app.document_done(row_id, request, options, result, elapsed);
    }

    #[test]
    fn spoken_fillers_stay_in_the_source_but_not_in_what_the_translator_reads() {
        let mut app = App::inert();
        app.caption_enabled = true;
        app.caption_paused = false;
        let caption = |text: &str, position| bridge::Caption {
            text: text.into(),
            video_id: "video-a".into(),
            position,
            paused: false,
            epoch: 2,
        };
        let first =
            "Um, the committee will review the report and uh we can share it with the public.";
        let second = "Well, um we should begin the second part of the meeting now.";
        app.document_ingest_caption(caption(first, 1.0));
        app.document_ingest_caption(caption(&format!("{first} >> {second}"), 2.0));
        let (id, request, options) = take_job(&mut app, SourceKind::Caption, 3_000);
        assert_eq!(
            request.snapshot.text, first,
            "the marker closed the first part"
        );
        app.document_done(
            id,
            request,
            options,
            Ok("委員会は報告書を確認し、一般にも共有できます。".into()),
            0.1,
        );

        let (id, request, options) = take_job(&mut app, SourceKind::Caption, 9_000);
        assert_eq!(request.snapshot.text, second);
        assert_eq!(request.previous_source, first);
        assert_eq!(
            app.document_translator_input(&request, &options),
            (
                "Well, we should begin the second part of the meeting now.".to_owned(),
                "the committee will review the report and we can share it with the public."
                    .to_owned()
            )
        );
        app.document_done(
            id,
            request,
            options,
            Ok("では、会議の後半を始めましょう。".into()),
            0.1,
        );
        assert_eq!(app.rows[0].record.source, first);
        assert_eq!(app.rows[1].record.source, second);
        app.document_update_subtitle_window();
        assert_eq!(app.subtitle_window.history()[0].source, first);
        assert_eq!(app.subtitle_window.view().unwrap().source, second);
    }

    #[test]
    fn fillers_stay_in_screen_text_and_in_text_that_may_not_be_english() {
        let mut app = App::inert();
        let auto = TranslationOptions::default();
        let spoken = "Um, the museum will open at nine and uh we can see this";
        let previous = "Uh, we can see the sign.";
        let input = |app: &App, request: Request, options: &TranslationOptions| {
            app.document_translator_input(&request, options)
        };
        let pair = |source: &str, previous: &str| (source.to_owned(), previous.to_owned());

        assert_eq!(
            input(
                &app,
                live_request(SourceKind::Caption, "video-a", spoken, previous),
                &auto
            ),
            pair(
                "the museum will open at nine and we can see this",
                "we can see the sign."
            ),
            "spoken English loses its fillers"
        );
        assert_eq!(
            input(
                &app,
                live_request(SourceKind::Caption, "screen:1", spoken, previous),
                &auto
            ),
            pair(spoken, previous),
            "screen text keeps what its author wrote"
        );

        let german = TranslationOptions {
            source_lang: "de".into(),
            ..auto.clone()
        };
        let er = "Er kommt um 5 Uhr.";
        assert_eq!(
            input(
                &app,
                live_request(SourceKind::Caption, "video-a", er, ""),
                &german
            ),
            pair(er, ""),
            "er and um are German words"
        );
        let portuguese = "Eu tenho um livro e um caderno para a aula de hoje.";
        assert_eq!(
            input(
                &app,
                live_request(SourceKind::Caption, "video-a", portuguese, ""),
                &auto
            ),
            pair(portuguese, ""),
            "um is a Portuguese word"
        );

        let short = "Well, um we should";
        assert_eq!(
            input(
                &app,
                live_request(SourceKind::Caption, "video-a", short, previous),
                &auto
            ),
            pair("Well, we should", "we can see the sign."),
            "the previous part shows the language"
        );
        assert_eq!(
            input(
                &app,
                live_request(SourceKind::Caption, "video-a", short, ""),
                &auto
            ),
            pair(short, ""),
            "a short text alone is not known to be English"
        );

        let speech = live_request(SourceKind::Audio, "", short, "");
        assert_eq!(
            input(&app, speech.clone(), &auto),
            pair("Well, we should", ""),
            "the English recognizer shows the language"
        );
        app.settings.asr_backend = "whisper".into();
        assert_eq!(
            input(&app, speech, &auto),
            pair(short, ""),
            "another recognizer does not"
        );
    }

    /// What PLaMo reads for a caption that follows `previous`, with the source
    /// language set to `source_lang`.
    fn translator_input(
        video_id: &str,
        text: &str,
        previous: &str,
        source_lang: &str,
    ) -> (String, String) {
        let options = TranslationOptions {
            source_lang: source_lang.into(),
            ..Default::default()
        };
        let request = live_request(SourceKind::Caption, video_id, text, previous);
        App::inert().document_translator_input(&request, &options)
    }

    #[test]
    fn a_rate_in_fractions_stays_in_the_source_but_is_a_decimal_in_what_the_translator_reads() {
        let mut app = App::inert();
        app.caption_enabled = true;
        app.caption_paused = false;
        let caption = |text: &str, position| bridge::Caption {
            text: text.into(),
            video_id: "video-a".into(),
            position,
            paused: false,
            epoch: 2,
        };
        let first = "The Committee decided to maintain the target range for the federal funds rate at 3 and 3/4 to 4%.";
        let second = "Then we can lower it by 1/4 percentage point to 3-3/4 to 4 percent.";
        app.document_ingest_caption(caption(first, 1.0));
        app.document_ingest_caption(caption(&format!("{first} >> {second}"), 2.0));
        let (id, request, options) = take_job(&mut app, SourceKind::Caption, 3_000);
        assert_eq!(
            request.snapshot.text, first,
            "the marker closed the first part"
        );
        assert_eq!(
            app.document_translator_input(&request, &options),
            (
                "The Committee decided to maintain the target range for the federal funds rate at 3.75 to 4%."
                    .to_owned(),
                String::new()
            )
        );
        app.document_done(
            id,
            request,
            options,
            Ok("委員会は政策金利を3.75から4%に据え置くことを決めました。".into()),
            0.1,
        );

        let (id, request, options) = take_job(&mut app, SourceKind::Caption, 9_000);
        assert_eq!(request.snapshot.text, second);
        assert_eq!(request.previous_source, first);
        assert_eq!(
            app.document_translator_input(&request, &options),
            (
                "Then we can lower it by 0.25 percentage point to 3.75 to 4 percent.".to_owned(),
                "The Committee decided to maintain the target range for the federal funds rate at 3.75 to 4%."
                    .to_owned()
            )
        );
        app.document_done(
            id,
            request,
            options,
            Ok("そのうえで、0.25ポイント下げて3.75から4%にできます。".into()),
            0.1,
        );
        assert_eq!(app.rows[0].record.source, first);
        assert_eq!(app.rows[1].record.source, second);
        app.document_update_subtitle_window();
        assert_eq!(app.subtitle_window.history()[0].source, first);
        assert_eq!(app.subtitle_window.view().unwrap().source, second);
    }

    #[test]
    fn fillers_are_left_out_before_the_fractions_are_read() {
        assert_eq!(
            translator_input(
                "video-a",
                "the target range at 3 and uh 3/4 to 4%",
                "",
                "en"
            ),
            ("the target range at 3.75 to 4%".to_owned(), String::new())
        );
    }

    #[test]
    fn screen_text_keeps_its_fillers_but_its_fractions_become_decimals() {
        assert_eq!(
            translator_input(
                "screen:1",
                "Um, the Committee lowered the target range to 3-3/4 to 4 percent.",
                "Uh, the range was 4-1/4 to 4-1/2 percent.",
                "en"
            ),
            (
                "Um, the Committee lowered the target range to 3.75 to 4 percent.".to_owned(),
                "Uh, the range was 4.25 to 4.5 percent.".to_owned()
            )
        );
    }

    #[test]
    fn fractions_stay_in_text_that_is_not_english() {
        let source = "Die Spanne liegt bei 3-3/4 bis 4 Prozent.";
        let previous = "Zuvor lag sie bei 4-1/4 bis 4-1/2 Prozent.";
        assert_eq!(
            translator_input("video-a", source, previous, "de"),
            (source.to_owned(), previous.to_owned())
        );
    }

    #[test]
    fn a_live_caption_reaches_the_translation_server_without_its_fillers() {
        let (endpoint, prompts) =
            translation_stub(vec![Completion::Text("訳"), Completion::Text("訳")]);
        let mut app = App::inert();
        app.caption_enabled = true;
        app.caption_paused = false;
        app.backend_ready = true;
        app.settings.translation.endpoint = endpoint;
        let caption =
            "Um, the committee will review the report and uh we can share it with the public.";
        app.document_ingest_caption(bridge::Caption {
            text: caption.into(),
            video_id: "video-a".into(),
            position: 1.0,
            paused: false,
            epoch: 2,
        });
        app.document_tick();
        let prompt = prompts.recv_timeout(Duration::from_secs(10)).unwrap();
        assert!(
            prompt.contains(
                "<|plamo:tag|>input<|plamo:msg|>the committee will review the report and we can share it with the public.<|plamo:tag|>output"
            ),
            "{prompt}"
        );
        assert_eq!(app.rows[0].record.source, caption);
    }

    #[test]
    fn a_live_caption_reaches_the_translation_server_with_its_rate_as_a_decimal() {
        let (endpoint, prompts) =
            translation_stub(vec![Completion::Text("訳"), Completion::Text("訳")]);
        let mut app = caption_app(endpoint);
        let caption = "The Committee decided to maintain the target range for the federal funds rate at 3 and 3/4 to 4%.";
        live_caption(&mut app, caption, 1.0);
        app.document_tick();
        let prompt = prompts.recv_timeout(Duration::from_secs(10)).unwrap();
        assert!(
            prompt.contains(
                "<|plamo:tag|>input<|plamo:msg|>The Committee decided to maintain the target range for the federal funds rate at 3.75 to 4%.<|plamo:tag|>output"
            ),
            "{prompt}"
        );
        assert_eq!(app.rows[0].record.source, caption);
    }

    #[test]
    fn the_translation_server_never_reads_the_tagger_text_or_fillers() {
        let source = "Well, um we should begin";
        let previous =
            "Um, the committee will review the report and uh we can share it with the public.";
        for tagger in [
            Ok(plamo_translate::punctuation::PunctuationResult {
                original: source.into(),
                restored: "Well, Um. We should begin.".into(),
                boundaries: Vec::new(),
            }),
            Err("tagger failed".to_owned()),
        ] {
            let (endpoint, prompts) =
                translation_stub(vec![Completion::Text("訳"), Completion::Text("訳")]);
            let mut app = App::inert();
            app.settings.translation.endpoint = endpoint;
            let options = app.settings.translation.clone();
            let mut request = live_request(SourceKind::Caption, "video-a", source, previous);
            request.previous_target = "委員会は報告書を確認し、一般にも共有できます。".into();
            app.document.flight = Some(Flight {
                row_id: 7,
                request: request.clone(),
                options: options.clone(),
                cancel: Arc::new(AtomicBool::new(false)),
            });
            app.document_punctuated(7, request, options, tagger);
            let prompt = prompts.recv_timeout(Duration::from_secs(10)).unwrap();
            assert!(
                prompt.contains(
                    "<|plamo:tag|>input<|plamo:msg|>Well, we should begin<|plamo:tag|>output"
                ),
                "{prompt}"
            );
            assert!(
                prompt.contains(
                    "the committee will review the report and we can share it with the public."
                ),
                "{prompt}"
            );
            assert!(
                !prompt.contains("Um") && !prompt.contains(" uh ") && !prompt.contains("We should"),
                "{prompt}"
            );
        }
    }

    #[test]
    fn a_paragraph_the_helper_could_not_mark_is_translated_and_the_settings_say_nothing_failed() {
        let source = "The committee will review the report and share it with the public";
        for tagger in [
            Err("Punctuation output changed original words, names or numbers".to_owned()),
            Ok(plamo_translate::punctuation::PunctuationResult {
                original: "an older revision of the paragraph".into(),
                restored: "An older revision of the paragraph.".into(),
                boundaries: Vec::new(),
            }),
        ] {
            let (endpoint, prompts) = translation_stub(vec![Completion::Text("訳")]);
            let mut app = App::inert();
            app.settings.translation.endpoint = endpoint;
            let options = app.settings.translation.clone();
            let request = live_request(SourceKind::Caption, "video-a", source, "");
            app.document.flight = Some(Flight {
                row_id: 7,
                request: request.clone(),
                options: options.clone(),
                cancel: Arc::new(AtomicBool::new(false)),
            });
            app.document_punctuated(7, request, options, tagger);
            let prompt = prompts.recv_timeout(Duration::from_secs(10)).unwrap();
            assert!(prompt.contains(source), "{prompt}");

            let output = crate::chat::testing::frame(
                &crate::chat::testing::context(),
                egui::vec2(860.0, 900.0),
                0.0,
                vec![],
                |ui| app.paragraph_tools_settings_ui(ui),
            );
            assert!(
                !crate::chat::testing::has_text_containing(
                    &output,
                    "区切り補助を準備できませんでした"
                ),
                "one paragraph is shown as a failed preparation"
            );
        }
    }

    #[test]
    fn a_caption_the_paragraph_cannot_take_is_kept_once_while_it_stays_on_screen() {
        let mut app = App::inert();
        app.caption_enabled = true;
        app.caption_paused = false;
        for position in [1.0, 3.0, 5.0] {
            live_caption(&mut app, "Read the <|plamo:tag|> marker aloud", position);
        }
        let failed = app
            .rows
            .iter()
            .filter(|row| {
                row.record
                    .paragraph
                    .as_ref()
                    .is_some_and(|p| p.state == ParagraphState::Failed)
            })
            .count();
        assert_eq!(failed, 1, "the rejected caption is saved once");
    }

    const COMMITTEE: &str =
        "The committee will review the report and we can share it with the public.";
    const ORDINARY: &str = "委員会は報告書を確認し、一般にも共有できます。";
    const EASY: &str = "委員会は報告書を見ます。みんなにも見せることができます。";

    #[test]
    fn a_japanese_paragraph_is_shown_as_easy_japanese_and_history_keeps_the_ordinary_translation() {
        let (endpoint, prompts) =
            translation_stub(vec![Completion::Text(ORDINARY), Completion::Text(EASY)]);
        let history = tempfile::tempdir().unwrap();
        let mut app = caption_app(endpoint);
        app.journal = Some(Journal::new(history.path()).unwrap());
        app.settings.translation.source_lang = "en".into();
        app.settings.translation.context = "町の図書館の会議".into();
        app.settings.translation.glossary = vec![("committee".into(), "委員会".into())];
        live_caption(&mut app, COMMITTEE, 1.0);
        app.document_close(SourceKind::Caption);
        app.document_tick();
        finish_translation(&mut app);

        let first = prompts.try_recv().unwrap();
        assert!(
            first.contains(&format!(
                "<|plamo:msg|>{COMMITTEE}<|plamo:tag|>output<|plamo:key|>lang<|plamo:val|>ja<|plamo:key|>form<|plamo:val|>plain<|plamo:msg|>"
            )),
            "{first}"
        );
        assert!(
            first.contains("町の図書館の会議") && first.contains("<|plamo:index|>committee"),
            "{first}"
        );
        assert_eq!(
            prompts.try_recv().unwrap(),
            format!(
                "<|plamo:tag|>dataset<|plamo:msg|>translation<|plamo:tag|>input<|plamo:key|>lang<|plamo:val|>ja<|plamo:msg|>{ORDINARY}<|plamo:tag|>output<|plamo:key|>lang<|plamo:val|>ja(easy)<|plamo:msg|>"
            )
        );
        assert!(prompts.try_recv().is_err(), "exactly two requests");

        let row = &app.rows[0];
        assert!(row.is_confirmed());
        assert_eq!(row.record.translation, EASY);
        assert_eq!(row.record.standard_translation.as_deref(), Some(ORDINARY));
        app.document_update_subtitle_window();
        assert_eq!(app.subtitle_window.view().unwrap().target, EASY);

        let (_, saved) = Journal::open_latest(history.path()).unwrap();
        assert_eq!(saved.len(), 1);
        assert_eq!(saved[0].translation, EASY);
        assert_eq!(saved[0].standard_translation.as_deref(), Some(ORDINARY));
    }

    #[test]
    fn the_next_paragraph_gets_the_ordinary_translation_as_its_context() {
        let (endpoint, prompts) = translation_stub(vec![
            Completion::Text(ORDINARY),
            Completion::Text(EASY),
            Completion::Text("では、会議の後半を始めましょう。"),
            Completion::Text("では、会議の後ろ半分を始めましょう。"),
        ]);
        let mut app = caption_app(endpoint);
        live_caption(&mut app, COMMITTEE, 1.0);
        app.document_close(SourceKind::Caption);
        app.document_tick();
        finish_translation(&mut app);
        live_caption(
            &mut app,
            "Well, we should begin the second part of the meeting now.",
            2.0,
        );
        app.document_tick();
        finish_translation(&mut app);

        let prompts: Vec<String> = prompts.try_iter().collect();
        assert_eq!(prompts.len(), 4);
        assert!(
            prompts[2].contains(&format!("Previous translation:\n{ORDINARY}")),
            "{}",
            prompts[2]
        );
        assert!(!prompts[2].contains(EASY), "{}", prompts[2]);
    }

    #[test]
    fn a_failed_rewrite_keeps_the_ordinary_translation_and_the_paragraph() {
        for rewrite in [Completion::Failure, Completion::Text("")] {
            let (endpoint, prompts) = translation_stub(vec![Completion::Text(ORDINARY), rewrite]);
            let mut app = caption_app(endpoint);
            live_caption(&mut app, COMMITTEE, 1.0);
            app.document_close(SourceKind::Caption);
            app.document_tick();
            finish_translation(&mut app);

            assert_eq!(prompts.try_iter().count(), 2, "the rewrite was tried");
            let row = &app.rows[0];
            assert!(row.is_confirmed());
            assert_eq!(row.record.translation, ORDINARY);
            assert_eq!(row.record.standard_translation, None);
        }
    }

    #[test]
    fn other_target_languages_are_translated_in_one_request() {
        let (endpoint, prompts) =
            translation_stub(vec![Completion::Text("The committee reviews the report.")]);
        let mut app = caption_app(endpoint);
        app.settings.translation.source_lang = "de".into();
        app.settings.translation.target_lang = "en".into();
        live_caption(
            &mut app,
            "Der Ausschuss prüft den Bericht und macht ihn der Öffentlichkeit zugänglich.",
            1.0,
        );
        app.document_close(SourceKind::Caption);
        app.document_tick();
        finish_translation(&mut app);

        assert_eq!(prompts.try_iter().count(), 1);
        assert_eq!(
            app.rows[0].record.translation,
            "The committee reviews the report."
        );
        assert_eq!(app.rows[0].record.standard_translation, None);
    }

    #[test]
    fn a_cancelled_rewrite_is_an_error_but_a_failed_one_is_not() {
        let (endpoint, prompts) = translation_stub(vec![Completion::Failure]);
        let options = TranslationOptions {
            endpoint,
            ..Default::default()
        };
        assert!(matches!(
            rewrite_as_easy_japanese(&options, ORDINARY, &AtomicBool::new(false)),
            Ok(None)
        ));
        assert_eq!(
            prompts.try_iter().count(),
            1,
            "the failing request was made"
        );
        assert!(rewrite_as_easy_japanese(&options, ORDINARY, &AtomicBool::new(true)).is_err());
        assert_eq!(
            prompts.try_iter().count(),
            0,
            "a cancelled rewrite asks nothing"
        );
    }

    #[test]
    fn a_corrected_source_drops_the_rewrite_with_its_ordinary_translation() {
        let mut app = App::inert();
        app.caption_enabled = true;
        app.caption_paused = false;
        live_caption(
            &mut app,
            "The loan period is five days for registered readers",
            1.0,
        );
        let (id, request, options) = take_job(&mut app, SourceKind::Caption, 1_500);
        let five_days = LiveTranslation {
            standard: "貸出期間は5日です。".into(),
            easy: Some("登録した人は、5日、本を借りられます。".into()),
        };
        app.document_done(id, request, options, Ok(five_days), 0.1);
        assert_eq!(
            app.rows[0].record.standard_translation.as_deref(),
            Some("貸出期間は5日です。")
        );
        app.document_update_subtitle_window();
        assert_eq!(
            app.subtitle_window.view().unwrap().target,
            "登録した人は、5日、本を借りられます。"
        );

        live_caption(
            &mut app,
            "The loan period is six days for registered readers",
            1.1,
        );
        assert!(app.rows[0].record.translation.is_empty());
        assert_eq!(app.rows[0].record.standard_translation, None);
    }
}
