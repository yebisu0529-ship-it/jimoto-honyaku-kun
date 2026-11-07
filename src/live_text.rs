//! Pure, bounded paragraph accumulation for independent live sources.
//!
//! Media timestamps describe source audio/video. All scheduling uses the caller's
//! monotonic `now_ms` instead, so seeking cannot change debounce or idle timers.

use anyhow::{Result, ensure};
use std::collections::VecDeque;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SourceKind {
    Audio,
    Caption,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct SourceStamp {
    pub kind: SourceKind,
    pub generation: u64,
    pub video_id: String,
    pub epoch: u64,
}

#[derive(Clone, Debug)]
pub struct Policy {
    pub debounce_ms: u64,
    pub retry_ms: u64,
    pub idle_close_ms: u64,
    pub boundary_grace_ms: u64,
    pub min_boundary_chars: usize,
    pub min_request_chars: usize,
    pub max_paragraph_chars: usize,
    pub max_paragraph_ms: u64,
    pub max_pending_closed: usize,
    pub prior_context_chars: usize,
    pub max_fragment_chars: usize,
    pub audio_dedupe_entries: usize,
    pub short_idle_close_ms: u64,
    pub short_chars: usize,
    pub short_words: usize,
    pub min_boundary_cjk_chars: usize,
    pub max_paragraph_cjk_chars: usize,
    pub first_words: usize,
    pub first_cjk_chars: usize,
    pub interim_interval_ms: u64,
    pub interim_added_words: usize,
    pub interim_added_cjk_chars: usize,
    pub previous_pairs: usize,
    pub retries: u32,
    /// Final safety bound when neither sentence punctuation nor an acoustic
    /// gap provides a useful boundary. The ordinary 900/450 limit is a trigger.
    pub absolute_max_chars: usize,
    pub acoustic_gap_ms: u64,
    /// A pause this long after a finished sentence with enough text to stand
    /// alone is taken as the speaker handing over, so the part closes there.
    /// Zero disables it.
    pub turn_pause_ms: u64,
}

impl Default for Policy {
    fn default() -> Self {
        Self {
            debounce_ms: 1_200,
            retry_ms: 1_500,
            idle_close_ms: 12_000,
            boundary_grace_ms: 0,
            min_boundary_chars: 600,
            min_request_chars: 1,
            max_paragraph_chars: 900,
            max_paragraph_ms: 60_000,
            max_pending_closed: 8,
            prior_context_chars: 1_600,
            max_fragment_chars: 16_000,
            audio_dedupe_entries: 256,
            short_idle_close_ms: 15_000,
            short_chars: 40,
            short_words: 6,
            min_boundary_cjk_chars: 300,
            max_paragraph_cjk_chars: 450,
            first_words: 6,
            first_cjk_chars: 12,
            interim_interval_ms: 2_000,
            interim_added_words: 8,
            interim_added_cjk_chars: 16,
            previous_pairs: 3,
            retries: 1,
            absolute_max_chars: 3_000,
            acoustic_gap_ms: 12_000,
            turn_pause_ms: 1_500,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AudioFragment {
    pub id: u64,
    pub text: String,
    pub start_ms: u64,
    pub end_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParagraphSnapshot {
    pub stamp: SourceStamp,
    pub key: u64,
    pub revision: u64,
    pub text: String,
    pub start_ms: u64,
    pub end_ms: u64,
    pub closed: bool,
    pub finalizable: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Request {
    pub id: u64,
    pub snapshot: ParagraphSnapshot,
    pub previous_source: String,
    pub previous_target: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Translation {
    pub snapshot: ParagraphSnapshot,
    pub target: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Acceptance {
    Rejected,
    Interim {
        snapshot: ParagraphSnapshot,
        target: String,
    },
    Committed {
        snapshot: ParagraphSnapshot,
        target: String,
    },
}

#[derive(Clone, Debug, Default)]
pub struct PushOutcome {
    pub changed: bool,
    /// Newly closed paragraphs, including any subsequently returned in overflow.
    pub closed: Vec<ParagraphSnapshot>,
    /// Queue pressure never silently destroys text: the caller must preserve
    /// these untranslated sources and notify the user before discarding them.
    pub overflow: Vec<ParagraphSnapshot>,
}

impl PushOutcome {
    fn merge(&mut self, other: Self) {
        self.changed |= other.changed;
        self.closed.extend(other.closed);
        self.overflow.extend(other.overflow);
    }
}

#[derive(Clone, Debug)]
struct Paragraph {
    snapshot: ParagraphSnapshot,
    created_at: u64,
    changed_at: u64,
    activity_at: u64,
    dirty_since: u64,
    retry_at: u64,
    sent: Option<(u64, bool)>,
    failures: u32,
    last_sent_at: Option<u64>,
    last_sent_text: String,
    spans: Vec<SourceSpan>,
    lookahead_start_words: Option<usize>,
}

#[derive(Clone, Debug)]
struct SourceSpan {
    from: usize,
    to: usize,
    start_ms: u64,
    end_ms: u64,
    wall_ms: u64,
}

impl SourceSpan {
    fn time_at(&self, byte: usize) -> u64 {
        let length = self.to.saturating_sub(self.from).max(1) as u128;
        let position = byte.saturating_sub(self.from).min(self.to - self.from) as u128;
        self.start_ms.saturating_add(
            ((self.end_ms.saturating_sub(self.start_ms) as u128 * position) / length) as u64,
        )
    }
}

impl Paragraph {
    fn changed(&mut self, now_ms: u64) {
        // Preserve the first unsent change during a continuous input burst.
        if self.sent == Some((self.snapshot.revision, self.snapshot.closed)) {
            self.dirty_since = now_ms;
        }
        self.snapshot.revision = self.snapshot.revision.saturating_add(1);
        self.changed_at = now_ms;
        self.activity_at = self.activity_at.max(now_ms);
        self.failures = 0;
    }
}

#[derive(Clone, Debug)]
struct CaptionWindow {
    text: String,
    /// A suffix of this window is the current paragraph's final contribution.
    location: Option<(u64, usize, usize)>, // paragraph key, byte anchor, window byte offset
    start_ms: u64,
    end_ms: u64,
}

#[derive(Clone, Debug)]
pub struct Accumulator {
    stamp: SourceStamp,
    policy: Policy,
    current: Option<Paragraph>,
    pending: VecDeque<Paragraph>,
    previous: VecDeque<Translation>,
    audio_seen: VecDeque<(u64, u64, u64)>,
    caption: Option<CaptionWindow>,
    inflight: Option<Request>,
    interim: Option<Translation>,
    next_key: u64,
    next_id: u64,
}

fn normalized(mut policy: Policy) -> Policy {
    policy.max_paragraph_chars = policy.max_paragraph_chars.max(1);
    policy.max_pending_closed = policy.max_pending_closed.max(1);
    policy.max_fragment_chars = policy.max_fragment_chars.max(policy.max_paragraph_chars);
    policy.audio_dedupe_entries = policy.audio_dedupe_entries.max(1);
    policy.absolute_max_chars = policy.absolute_max_chars.clamp(1, 3_000);
    policy
}

impl Accumulator {
    pub fn new(stamp: SourceStamp, policy: Policy) -> Self {
        Self {
            stamp,
            policy: normalized(policy),
            current: None,
            pending: VecDeque::new(),
            previous: VecDeque::new(),
            audio_seen: VecDeque::new(),
            caption: None,
            inflight: None,
            interim: None,
            next_key: 1,
            next_id: 1,
        }
    }

    pub fn stamp(&self) -> &SourceStamp {
        &self.stamp
    }

    pub fn policy(&self) -> &Policy {
        &self.policy
    }

    /// Text already gathered keeps its paragraphs; only later boundary
    /// decisions follow the new limits.
    pub fn set_policy(&mut self, policy: Policy) {
        self.policy = normalized(policy);
    }

    /// Reset on seek, source stop, generation or video changes. Identifiers are
    /// deliberately not reused, including when the supplied stamp is unchanged.
    pub fn reset(&mut self, stamp: SourceStamp, _now_ms: u64) {
        self.stamp = stamp;
        self.current = None;
        self.pending.clear();
        self.previous.clear();
        self.audio_seen.clear();
        self.caption = None;
        self.inflight = None;
        self.interim = None;
    }

    pub fn current_snapshot(&self) -> Option<ParagraphSnapshot> {
        self.current.as_ref().map(|p| p.snapshot.clone())
    }

    pub fn pending_snapshots(&self) -> Vec<ParagraphSnapshot> {
        self.pending.iter().map(|p| p.snapshot.clone()).collect()
    }

    pub fn interim(&self) -> Option<&Translation> {
        self.interim.as_ref()
    }

    pub fn is_inflight(&self) -> bool {
        self.inflight.is_some()
    }

    /// Playback/capture progress can keep an unchanged source alive without
    /// dirtying it or requesting another translation. The caller must validate
    /// activity (e.g. advancing, unpaused caption position) before calling this.
    pub fn note_activity(&mut self, now_ms: u64) {
        if let Some(p) = self.current.as_mut() {
            p.activity_at = p.activity_at.max(now_ms);
        }
    }

    pub fn push_audio(&mut self, fragment: AudioFragment, now_ms: u64) -> Result<PushOutcome> {
        ensure!(
            self.stamp.kind == SourceKind::Audio,
            "audio supplied to caption accumulator"
        );
        self.validate_input(&fragment.text, fragment.start_ms, fragment.end_ms)?;
        let identity = (fragment.id, fragment.start_ms, fragment.end_ms);
        if self.audio_seen.contains(&identity) || fragment.text.trim().is_empty() {
            return Ok(PushOutcome::default());
        }
        self.audio_seen.push_back(identity);
        while self.audio_seen.len() > self.policy.audio_dedupe_entries {
            self.audio_seen.pop_front();
        }
        let mut outcome = self.tick(now_ms);
        if self.current.as_ref().is_some_and(|p| {
            let gap = fragment.start_ms.saturating_sub(p.snapshot.end_ms);
            p.snapshot.text.chars().count() >= self.soft_limit(&p.snapshot.text)
                && gap >= self.policy.acoustic_gap_ms
                || self.handed_over(&p.snapshot.text, gap)
        }) {
            outcome.merge(self.close_current());
        }
        outcome.merge(self.append(
            &fragment.text,
            fragment.start_ms,
            fragment.end_ms,
            now_ms,
            false,
        ));
        outcome.merge(self.close_at_boundary(now_ms));
        Ok(outcome)
    }

    pub fn push_caption(
        &mut self,
        text: &str,
        start_ms: u64,
        end_ms: u64,
        now_ms: u64,
    ) -> Result<PushOutcome> {
        ensure!(
            self.stamp.kind == SourceKind::Caption,
            "caption supplied to audio accumulator"
        );
        self.validate_input(text, start_ms, end_ms)?;
        let text = text.trim();
        if text.is_empty() {
            return Ok(PushOutcome::default());
        }
        let old = self.caption.clone();
        if old.as_ref().is_some_and(|w| w.text == text) {
            // Equal text alone is not proof of activity. The caller may have
            // separately validated advancing playback via note_activity.
            return Ok(self.tick(now_ms));
        }
        let mut outcome = self.tick(now_ms);
        let mut delta_offset = 0;
        let mut raw_tail = false;
        let mut location = None;
        if let Some(old) = &old {
            if text.starts_with(&old.text) {
                delta_offset = old.text.len();
                raw_tail = true;
                location = self.valid_location(old.location);
            } else {
                let common = common_prefix_bytes(&old.text, text);
                let same_window = start_ms == old.start_ms && end_ms == old.end_ms;
                let common_suffix = common_suffix_bytes(&old.text, text);
                let correction = same_window
                    || common > 0 && (common + common_suffix) * 2 >= old.text.len().min(text.len());
                if correction {
                    if let Some((key, anchor, offset)) = self.correctable_location(old.location)
                        && text.is_char_boundary(offset)
                    {
                        let replacement = &text[offset..];
                        ensure!(
                            old.text.get(..offset) == text.get(..offset),
                            "caption correction crosses a closed paragraph boundary"
                        );
                        let limit = self.policy.absolute_max_chars;
                        let p = self.find_mut(key).expect("validated location");
                        ensure!(
                            p.snapshot.text[..anchor].chars().count() + replacement.chars().count()
                                <= limit,
                            "caption correction exceeds paragraph bound"
                        );
                        if p.snapshot.text[anchor..] != *replacement {
                            p.changed(now_ms);
                            p.snapshot.text.truncate(anchor);
                            p.snapshot.text.push_str(replacement);
                            p.snapshot.end_ms = p.snapshot.end_ms.max(end_ms);
                            p.spans.retain(|s| s.from < anchor);
                            for span in &mut p.spans {
                                if span.to > anchor {
                                    span.end_ms = span.time_at(anchor);
                                    span.to = anchor;
                                }
                            }
                            p.spans.push(SourceSpan {
                                from: anchor,
                                to: p.snapshot.text.len(),
                                start_ms,
                                end_ms,
                                wall_ms: now_ms,
                            });
                            outcome.changed = true;
                        }
                        self.invalidate_incoherent_interim();
                        self.caption = Some(CaptionWindow {
                            text: text.to_owned(),
                            location: Some((key, anchor, offset)),
                            start_ms,
                            end_ms,
                        });
                        outcome.merge(self.close_at_boundary(now_ms));
                        return Ok(outcome);
                    }
                    // A correction to an already closed window must not be
                    // appended as new speech or mutate committed history.
                    self.caption = Some(CaptionWindow {
                        text: text.to_owned(),
                        location: None,
                        start_ms,
                        end_ms,
                    });
                    return Ok(outcome);
                }
                let overlap = suffix_prefix_overlap(&old.text, text);
                if overlap > 0 {
                    delta_offset = overlap;
                    raw_tail = true;
                    let old_start = old.text.len() - overlap;
                    if let Some((key, anchor, offset)) = self.valid_location(old.location) {
                        location = if old_start >= offset {
                            Some((key, anchor + old_start - offset, 0))
                        } else {
                            Some((key, anchor, offset - old_start))
                        };
                    }
                }
            }
        }
        let before = self
            .current
            .as_ref()
            .map(|p| (p.snapshot.key, p.snapshot.text.len()));
        // YouTube marks a new speaker with ">>". Each marker in the new text
        // closes the part before it, and the marker itself is not translated.
        let mut turn_start = None;
        let mut at = delta_offset;
        for (index, piece) in text[delta_offset..].split(SPEAKER_TURN).enumerate() {
            if index > 0 {
                outcome.merge(self.close_current());
                turn_start = Some(at + piece.len() - piece.trim_start().len());
            }
            let mut words = if index > 0 { piece.trim_start() } else { piece };
            if text[at + piece.len()..].starts_with(SPEAKER_TURN) {
                words = words.trim_end();
            }
            outcome.merge(self.append(words, start_ms, end_ms, now_ms, raw_tail && index == 0));
            at += piece.len() + SPEAKER_TURN.len();
        }
        if let Some(offset) = turn_start {
            location = self.current.as_ref().and_then(|p| {
                let tail = text.get(offset..)?;
                p.snapshot
                    .text
                    .ends_with(tail)
                    .then(|| (p.snapshot.key, p.snapshot.text.len() - tail.len(), offset))
            });
        }
        let after = self
            .current
            .as_ref()
            .map(|p| (p.snapshot.key, p.snapshot.text.len()));
        location = self.valid_location(location).or_else(|| {
            let (key, end) = after?;
            let appended = &text[delta_offset..];
            let p = self.current.as_ref()?;
            if p.snapshot.text.ends_with(appended) {
                Some((key, end - appended.len(), delta_offset))
            } else if let Some((old_key, old_end)) = before
                && old_key == key
            {
                Some((key, old_end, delta_offset))
            } else {
                None
            }
        });
        self.caption = Some(CaptionWindow {
            text: text.to_owned(),
            location,
            start_ms,
            end_ms,
        });
        outcome.merge(self.close_at_boundary(now_ms));
        Ok(outcome)
    }

    fn valid_location(&self, location: Option<(u64, usize, usize)>) -> Option<(u64, usize, usize)> {
        location.filter(|(key, anchor, _)| {
            self.current.as_ref().is_some_and(|p| {
                p.snapshot.key == *key && p.snapshot.text.is_char_boundary(*anchor)
            })
        })
    }

    fn correctable_location(
        &self,
        location: Option<(u64, usize, usize)>,
    ) -> Option<(u64, usize, usize)> {
        location.filter(|(key, anchor, _)| {
            self.find(*key)
                .is_some_and(|p| p.snapshot.text.is_char_boundary(*anchor))
        })
    }

    fn invalidate_incoherent_interim(&mut self) {
        if self.interim.as_ref().is_some_and(|t| {
            self.find(t.snapshot.key)
                .is_none_or(|p| !p.snapshot.text.starts_with(&t.snapshot.text))
        }) {
            self.interim = None;
        }
    }

    fn validate_input(&self, text: &str, start: u64, end: u64) -> Result<()> {
        ensure!(end >= start, "source end precedes start");
        ensure!(
            !text.contains("<|plamo:"),
            "reserved translation token in live source"
        );
        ensure!(
            text.chars().count() <= self.policy.max_fragment_chars,
            "live source fragment exceeds configured bound"
        );
        Ok(())
    }

    fn append(
        &mut self,
        text: &str,
        start: u64,
        end: u64,
        now_ms: u64,
        raw_tail: bool,
    ) -> PushOutcome {
        let mut outcome = PushOutcome::default();
        let mut rest = text;
        let mut raw = raw_tail;
        while !rest.is_empty() {
            if self.current.is_none() {
                rest = rest.trim_start();
                if rest.is_empty() {
                    break;
                }
                let key = self.next_key;
                self.next_key = self.next_key.saturating_add(1);
                self.current = Some(Paragraph {
                    snapshot: ParagraphSnapshot {
                        stamp: self.stamp.clone(),
                        key,
                        revision: 0,
                        text: String::new(),
                        start_ms: start,
                        end_ms: end,
                        closed: false,
                        finalizable: false,
                    },
                    created_at: now_ms,
                    changed_at: now_ms,
                    activity_at: now_ms,
                    dirty_since: now_ms,
                    retry_at: 0,
                    sent: None,
                    failures: 0,
                    last_sent_at: None,
                    last_sent_text: String::new(),
                    spans: Vec::new(),
                    lookahead_start_words: None,
                });
                raw = false;
            }
            let p = self.current.as_mut().expect("current paragraph");
            let separator = !raw && needs_space(&p.snapshot.text, rest);
            if p.lookahead_start_words.is_none()
                && p.snapshot.text.chars().count() + usize::from(separator) + rest.chars().count()
                    > self.policy.absolute_max_chars
                && dangling_english_end(&p.snapshot.text)
            {
                p.lookahead_start_words = Some(word_count(&p.snapshot.text));
            }
            let word_room = p.lookahead_start_words.map(|start| {
                16usize.saturating_sub(word_count(&p.snapshot.text).saturating_sub(start))
            });
            if word_room == Some(0) {
                outcome.merge(self.close_current());
                continue;
            }
            let limit = self.policy.absolute_max_chars + if word_room.is_some() { 128 } else { 0 };
            if !p.snapshot.text.is_empty()
                && rest.chars().count() <= limit
                && p.snapshot.text.chars().count() + usize::from(separator) + rest.chars().count()
                    > limit
            {
                if let Some(cut) = last_sentence_boundary(&p.snapshot.text) {
                    outcome.merge(self.split_current(cut, now_ms));
                } else {
                    outcome.merge(self.close_current());
                }
                continue;
            }
            let available =
                limit.saturating_sub(p.snapshot.text.chars().count() + usize::from(separator));
            if available == 0 {
                outcome.merge(self.close_current());
                continue;
            }
            let cut = split_byte(rest, available)
                .min(word_room.map_or(rest.len(), |count| word_limit_byte(rest, count)));
            let piece = &rest[..cut];
            let p = self.current.as_mut().expect("current paragraph");
            let from = p.snapshot.text.len();
            p.changed(now_ms);
            if separator {
                p.snapshot.text.push(' ');
            }
            p.snapshot.text.push_str(piece);
            p.snapshot.start_ms = p.snapshot.start_ms.min(start);
            p.snapshot.end_ms = p.snapshot.end_ms.max(end);
            p.spans.push(SourceSpan {
                from,
                to: p.snapshot.text.len(),
                start_ms: start,
                end_ms: end,
                wall_ms: now_ms,
            });
            outcome.changed = true;
            rest = &rest[cut..];
            raw = true;
            if !rest.is_empty() {
                outcome.merge(self.close_current());
            }
        }
        outcome
    }

    /// Perform idle/boundary/maximum-duration closure even without new input.
    pub fn tick(&mut self, now_ms: u64) -> PushOutcome {
        let idle_close = self.current.as_ref().is_some_and(|p| {
            let idle = now_ms.saturating_sub(p.activity_at);
            let short = p.snapshot.text.chars().count() < self.policy.short_chars
                || (cjk_count(&p.snapshot.text) == 0
                    && word_count(&p.snapshot.text) < self.policy.short_words);
            idle >= if short {
                self.policy.short_idle_close_ms
            } else {
                self.policy.idle_close_ms
            }
        });
        if idle_close {
            self.close_current()
        } else {
            self.close_at_boundary(now_ms)
        }
    }

    /// A pause mid-sentence is a speaker thinking, so only a pause after a
    /// finished sentence counts as handing over.
    fn handed_over(&self, text: &str, pause_ms: u64) -> bool {
        let text = text.trim_end();
        let enough = if cjk_count(text) > 0 {
            cjk_count(text) >= self.policy.first_cjk_chars.max(1)
        } else {
            word_count(text) >= self.policy.first_words.max(1)
        };
        self.policy.turn_pause_ms > 0
            && pause_ms >= self.policy.turn_pause_ms
            && enough
            && last_sentence_boundary(text) == Some(text.len())
    }

    pub fn close(&mut self, _now_ms: u64) -> PushOutcome {
        self.close_current()
    }

    /// Use word-preserving punctuation from an accumulated, still-active
    /// request. Hints are offsets in the untouched source, never replacement
    /// prose. A tail of at least six words supplies right-hand context and
    /// prevents a tagger's automatic final full stop from closing every chunk.
    pub fn apply_sentence_hints(
        &mut self,
        request: &Request,
        boundaries: &[usize],
        now_ms: u64,
    ) -> PushOutcome {
        if !self.matches_inflight(request) || request.snapshot.closed {
            return PushOutcome::default();
        }
        let Some(p) = self.current.as_ref().filter(|p| {
            p.snapshot.key == request.snapshot.key
                && p.snapshot.text.starts_with(&request.snapshot.text)
        }) else {
            return PushOutcome::default();
        };
        if p.snapshot.text.chars().count() < self.paragraph_limit(&p.snapshot.text)
            && now_ms.saturating_sub(p.created_at) < self.policy.max_paragraph_ms
        {
            return PushOutcome::default();
        }
        let text = &request.snapshot.text;
        let cut = boundaries
            .iter()
            .copied()
            .filter(|&cut| {
                cut > 0
                    && cut < text.len()
                    && text.is_char_boundary(cut)
                    && text[..cut].chars().count() >= self.soft_limit(text)
                    && text[..cut]
                        .chars()
                        .next_back()
                        .is_some_and(|c| !c.is_whitespace())
                    && text[cut..].chars().next().is_some_and(char::is_whitespace)
                    && self.has_right_context(&text[cut..])
            })
            .max();
        let Some(cut) = cut else {
            return PushOutcome::default();
        };
        // The pre-translation job covered the old whole paragraph. Its result
        // must not later replace the shorter closed prefix or its new tail.
        self.inflight = None;
        self.split_current(cut, now_ms)
    }

    fn paragraph_limit(&self, text: &str) -> usize {
        if cjk_count(text) > 0 {
            self.policy.max_paragraph_cjk_chars.max(1)
        } else {
            self.policy.max_paragraph_chars
        }
    }

    fn soft_limit(&self, text: &str) -> usize {
        if cjk_count(text) > 0 {
            self.policy.min_boundary_cjk_chars
        } else {
            self.policy.min_boundary_chars
        }
    }

    fn has_right_context(&self, tail: &str) -> bool {
        if cjk_count(tail) > 0 {
            cjk_count(tail) >= self.policy.first_cjk_chars.max(12)
        } else {
            word_count(tail) >= self.policy.first_words.max(6)
        }
    }

    fn close_at_boundary(&mut self, now_ms: u64) -> PushOutcome {
        let Some(p) = self.current.as_ref() else {
            return PushOutcome::default();
        };
        let chars = p.snapshot.text.chars().count();
        let trigger = (chars >= self.paragraph_limit(&p.snapshot.text)
            || now_ms.saturating_sub(p.created_at) >= self.policy.max_paragraph_ms)
            && now_ms.saturating_sub(p.changed_at) >= self.policy.boundary_grace_ms;
        if trigger
            && let Some(cut) = sentence_boundaries(&p.snapshot.text)
                .into_iter()
                .rev()
                .find(|&cut| {
                    p.snapshot.text[..cut].chars().count() >= self.soft_limit(&p.snapshot.text)
                        && self.has_right_context(&p.snapshot.text[cut..])
                })
        {
            return self.split_current(cut, now_ms);
        }
        if chars >= self.policy.absolute_max_chars {
            let words = word_count(&p.snapshot.text);
            if dangling_english_end(&p.snapshot.text)
                && chars < self.policy.absolute_max_chars + 128
                && p.lookahead_start_words
                    .is_none_or(|start| words.saturating_sub(start) < 16)
            {
                self.current
                    .as_mut()
                    .expect("current paragraph")
                    .lookahead_start_words
                    .get_or_insert(words);
                PushOutcome::default()
            } else {
                self.close_current()
            }
        } else {
            PushOutcome::default()
        }
    }

    fn split_current(&mut self, cut: usize, now_ms: u64) -> PushOutcome {
        let Some(mut prefix) = self.current.take() else {
            return PushOutcome::default();
        };
        if cut >= prefix.snapshot.text.len() {
            self.current = Some(prefix);
            return self.close_current();
        }
        let full = prefix.snapshot.text.clone();
        let tail_start = cut + full[cut..].len() - full[cut..].trim_start().len();
        if tail_start == full.len() {
            self.current = Some(prefix);
            return self.close_current();
        }
        let old_key = prefix.snapshot.key;
        let new_key = self.next_key;
        self.next_key = self.next_key.saturating_add(1);
        let split_time = prefix
            .spans
            .iter()
            .find(|s| s.to >= cut)
            .map_or(prefix.snapshot.end_ms, |s| s.time_at(cut))
            .clamp(prefix.snapshot.start_ms, prefix.snapshot.end_ms);
        let tail_time = prefix
            .spans
            .iter()
            .find(|s| s.to > tail_start)
            .map_or(split_time, |s| s.time_at(tail_start))
            .clamp(split_time, prefix.snapshot.end_ms);
        let mut tail = prefix.clone();
        tail.snapshot.key = new_key;
        tail.snapshot.revision = 1;
        tail.snapshot.text = full[tail_start..].to_owned();
        tail.snapshot.start_ms = tail_time;
        tail.snapshot.closed = false;
        tail.snapshot.finalizable = false;
        tail.spans = prefix
            .spans
            .iter()
            .filter(|s| s.to > tail_start)
            .map(|s| {
                let from = s.from.max(tail_start);
                SourceSpan {
                    from: from - tail_start,
                    to: s.to - tail_start,
                    start_ms: s.time_at(from).max(tail_time),
                    end_ms: s.end_ms.max(tail_time),
                    wall_ms: s.wall_ms,
                }
            })
            .collect();
        tail.created_at = tail.spans.first().map_or(now_ms, |s| s.wall_ms);
        tail.dirty_since = now_ms;
        tail.retry_at = 0;
        tail.sent = None;
        tail.failures = 0;
        tail.last_sent_at = None;
        tail.last_sent_text.clear();
        tail.lookahead_start_words = None;
        prefix.changed(now_ms);
        prefix.snapshot.text = full[..cut].trim_end().to_owned();
        prefix.snapshot.end_ms = split_time;
        prefix.spans.retain(|s| s.from < cut);
        for span in &mut prefix.spans {
            if span.to > cut {
                span.end_ms = span.time_at(cut);
                span.to = cut;
            }
        }
        if let Some(window) = self.caption.as_mut()
            && let Some((key, anchor, offset)) = window.location
            && key == old_key
        {
            window.location = if anchor >= tail_start {
                Some((new_key, anchor - tail_start, offset))
            } else {
                Some((new_key, 0, offset + tail_start - anchor))
            };
        }
        self.current = Some(prefix);
        let outcome = self.close_current();
        self.current = Some(tail);
        self.invalidate_incoherent_interim();
        outcome
    }

    fn close_current(&mut self) -> PushOutcome {
        let mut outcome = PushOutcome::default();
        if let Some(mut p) = self.current.take() {
            p.snapshot.closed = true;
            p.snapshot.finalizable = true;
            outcome.changed = true;
            outcome.closed.push(p.snapshot.clone());
            self.pending.push_back(p);
        }
        while self.pending.len() > self.policy.max_pending_closed {
            let p = self.pending.pop_front().expect("bounded queue");
            if self
                .inflight
                .as_ref()
                .is_some_and(|r| r.snapshot.key == p.snapshot.key)
            {
                self.inflight = None;
            }
            outcome.overflow.push(p.snapshot);
        }
        outcome
    }

    /// At most one request exists per accumulator. The caller may additionally
    /// arbitrate a global slot across audio and captions before calling this.
    pub fn next_request(&mut self, now_ms: u64) -> Option<Request> {
        if self.inflight.is_some() {
            return None;
        }
        // An exhausted paragraph remains inspectable/preservable, but must not
        // block every later paragraph forever after its one automatic retry.
        let p = if let Some(p) = self
            .pending
            .iter_mut()
            .find(|p| p.failures <= self.policy.retries)
        {
            p
        } else {
            self.current.as_mut()?
        };
        let signature = (p.snapshot.revision, p.snapshot.closed);
        if p.sent == Some(signature) || now_ms < p.retry_at || p.failures > self.policy.retries {
            return None;
        }
        if !p.snapshot.closed {
            let words = word_count(&p.snapshot.text);
            let cjk = cjk_count(&p.snapshot.text);
            let enough = if p.failures > 0 {
                true
            } else if let Some(last_at) = p.last_sent_at {
                now_ms.saturating_sub(last_at) >= self.policy.interim_interval_ms
                    && (words.saturating_sub(word_count(&p.last_sent_text)) >= self.policy.interim_added_words
                        || cjk.saturating_sub(cjk_count(&p.last_sent_text)) >= self.policy.interim_added_cjk_chars
                        // Corrections are revisions too: they need not add words.
                        || !p.snapshot.text.starts_with(&p.last_sent_text))
            } else {
                words >= self.policy.first_words
                    || cjk >= self.policy.first_cjk_chars
                    || now_ms.saturating_sub(p.dirty_since) >= self.policy.debounce_ms
            };
            if p.snapshot.text.chars().count() < self.policy.min_request_chars || !enough {
                return None;
            }
        }
        p.sent = Some(signature);
        p.last_sent_at = Some(now_ms);
        p.last_sent_text.clone_from(&p.snapshot.text);
        let snapshot = p.snapshot.clone();
        let (previous_source, previous_target) = self.previous_context();
        let request = Request {
            id: self.next_id,
            snapshot,
            previous_source,
            previous_target,
        };
        self.next_id = self.next_id.saturating_add(1);
        self.inflight = Some(request.clone());
        Some(request)
    }

    pub fn accept(&mut self, request: &Request, target: &str, now_ms: u64) -> Acceptance {
        if !self.matches_inflight(request) {
            return Acceptance::Rejected;
        }
        if target.trim().is_empty() || target.contains("<|plamo:") {
            self.fail(request, now_ms);
            return Acceptance::Rejected;
        }
        self.inflight = None;
        let Some(p) = self.find(request.snapshot.key) else {
            return Acceptance::Rejected;
        };
        let latest = p.snapshot.revision == request.snapshot.revision;
        if !latest && !p.snapshot.text.starts_with(&request.snapshot.text) {
            return Acceptance::Rejected;
        }
        if latest && p.snapshot.closed {
            let snapshot = p.snapshot.clone();
            self.pending.retain(|p| p.snapshot.key != snapshot.key);
            let translation = Translation {
                snapshot: snapshot.clone(),
                target: target.to_owned(),
            };
            if self
                .interim
                .as_ref()
                .is_some_and(|t| t.snapshot.key == snapshot.key)
            {
                self.interim = None;
            }
            self.previous.push_back(translation);
            self.bound_previous();
            Acceptance::Committed {
                snapshot,
                target: target.to_owned(),
            }
        } else {
            let mut snapshot = request.snapshot.clone();
            snapshot.closed = false;
            snapshot.finalizable = false;
            let should_update = self.interim.as_ref().is_none_or(|t| {
                (snapshot.key, snapshot.revision) >= (t.snapshot.key, t.snapshot.revision)
            });
            if should_update {
                self.interim = Some(Translation {
                    snapshot: snapshot.clone(),
                    target: target.to_owned(),
                });
            }
            Acceptance::Interim {
                snapshot,
                target: target.to_owned(),
            }
        }
    }

    /// Release a superseded job without consuming the model-failure retry
    /// budget or discarding its source/context. Only the exact active job may
    /// release the slot, so a late cancellation cannot cancel its replacement.
    pub fn cancel_request(&mut self, request: &Request, now_ms: u64) -> bool {
        if !self.matches_inflight(request) {
            return false;
        }
        self.inflight = None;
        if let Some(p) = self.find_mut(request.snapshot.key) {
            p.sent = None;
            p.retry_at = now_ms;
            // A cancelled first/interim job must remain dispatchable even when
            // the unchanged source has not gained eight more words.
            p.last_sent_at = None;
        }
        true
    }

    pub fn fail(&mut self, request: &Request, now_ms: u64) -> bool {
        if !self.matches_inflight(request) {
            return false;
        }
        self.inflight = None;
        let retry_ms = self.policy.retry_ms;
        if let Some(p) = self.find_mut(request.snapshot.key) {
            p.sent = None;
            p.failures = p.failures.saturating_add(1);
            p.retry_at = now_ms.saturating_add(retry_ms);
        }
        true
    }

    fn matches_inflight(&self, request: &Request) -> bool {
        request.snapshot.stamp == self.stamp && self.inflight.as_ref() == Some(request)
    }

    fn find(&self, key: u64) -> Option<&Paragraph> {
        self.current
            .as_ref()
            .filter(|p| p.snapshot.key == key)
            .or_else(|| self.pending.iter().find(|p| p.snapshot.key == key))
    }

    fn find_mut(&mut self, key: u64) -> Option<&mut Paragraph> {
        self.current
            .as_mut()
            .filter(|p| p.snapshot.key == key)
            .or_else(|| self.pending.iter_mut().find(|p| p.snapshot.key == key))
    }

    fn bound_previous(&mut self) {
        // Keep complete recent pairs when possible, with a strict character
        // budget even for a single unusually long model response.
        let cap = self.policy.prior_context_chars;
        while self.previous.len() > self.policy.previous_pairs {
            self.previous.pop_front();
        }
        while self.previous.len() > 1
            && self
                .previous
                .iter()
                .map(|t| t.snapshot.text.chars().count() + t.target.chars().count())
                .sum::<usize>()
                + self.previous.len().saturating_sub(1) * 2
                > cap
        {
            self.previous.pop_front();
        }
        if let Some(last) = self.previous.front_mut() {
            let source_cap = cap / 2;
            last.snapshot.text = tail_chars(&last.snapshot.text, source_cap);
            last.target = tail_chars(
                &last.target,
                cap.saturating_sub(last.snapshot.text.chars().count()),
            );
        }
        if cap == 0 {
            self.previous.clear();
        }
    }

    pub fn previous_context(&self) -> (String, String) {
        (
            self.previous
                .iter()
                .map(|t| t.snapshot.text.as_str())
                .collect::<Vec<_>>()
                .join(" "),
            self.previous
                .iter()
                .map(|t| t.target.as_str())
                .collect::<Vec<_>>()
                .join(" "),
        )
    }
}

fn last_sentence_boundary(text: &str) -> Option<usize> {
    sentence_boundaries(text).into_iter().last()
}

const SPEAKER_TURN: &str = ">>";

fn sentence_boundaries(text: &str) -> Vec<usize> {
    let mut boundaries = Vec::new();
    for (i, c) in text.char_indices() {
        if !".!?。！？".contains(c) {
            continue;
        }
        let before = &text[..i];
        let after = &text[i + c.len_utf8()..];
        if c == '.' {
            if before
                .chars()
                .next_back()
                .is_some_and(|c| c.is_ascii_digit())
                && after.chars().next().is_some_and(|c| c.is_ascii_digit())
            {
                continue;
            }
            let token = before
                .split_whitespace()
                .next_back()
                .unwrap_or("")
                .trim_start_matches(['"', '\'', '(', '[']);
            let lower = token.to_ascii_lowercase();
            if [
                "mr", "mrs", "ms", "dr", "prof", "sr", "jr", "st", "vs", "etc", "e.g", "i.e",
                "u.s", "u.k", "no", "fig", "inc",
            ]
            .contains(&lower.as_str())
                || token.len() == 1 && token.chars().all(|c| c.is_ascii_uppercase())
                || token.contains('.')
                    && token
                        .split('.')
                        .all(|p| p.len() == 1 && p.chars().all(|c| c.is_ascii_alphabetic()))
            {
                continue;
            }
            if after
                .chars()
                .next()
                .is_some_and(|c| c.is_alphanumeric() && !cjk(c))
            {
                continue;
            }
        }
        let mut end = i + c.len_utf8();
        for next in text[end..].chars() {
            if "!?。！？\"'”’）)]」』".contains(next) {
                end += next.len_utf8();
            } else {
                break;
            }
        }
        if text[end..]
            .chars()
            .next()
            .is_none_or(|c| c.is_whitespace() || cjk(c))
        {
            boundaries.push(end);
        }
    }
    boundaries
}

fn cjk(c: char) -> bool {
    matches!(c as u32, 0x2e80..=0x9fff | 0xac00..=0xd7af | 0xf900..=0xfaff | 0x20000..=0x323af)
}

fn word(c: char) -> bool {
    c.is_alphanumeric() && !cjk(c) || matches!(c, '_' | '\'')
}

fn word_count(text: &str) -> usize {
    text.split_whitespace()
        .filter(|s| s.chars().any(word))
        .count()
}

fn word_limit_byte(text: &str, count: usize) -> usize {
    let mut words = 0;
    let mut inside = false;
    for (i, c) in text.char_indices() {
        if c.is_whitespace() {
            inside = false;
        } else if !inside {
            if words == count {
                return i;
            }
            words += 1;
            inside = true;
        }
    }
    text.len()
}

fn dangling_english_end(text: &str) -> bool {
    // This deliberately does not attempt multilingual grammar inference.
    if text.chars().any(|c| c.is_alphabetic() && !c.is_ascii()) {
        return false;
    }
    let tokens: Vec<_> = text
        .split_whitespace()
        .map(|s| {
            s.trim_matches(|c: char| !c.is_ascii_alphanumeric())
                .to_ascii_lowercase()
        })
        .collect();
    let evidence = [
        "the", "is", "will", "we", "this", "of", "and", "with", "for", "our",
    ];
    if evidence
        .iter()
        .filter(|word| tokens.iter().any(|token| token == **word))
        .count()
        < 3
    {
        return false;
    }
    tokens.last().is_some_and(|last| {
        [
            "of", "to", "for", "with", "and", "or", "the", "a", "an", "about", "at", "from", "by",
            "into", "that", "which", "because", "what",
        ]
        .contains(&last.as_str())
            || !last.is_empty() && last.chars().all(|c| c.is_ascii_digit())
    })
}

fn cjk_count(text: &str) -> usize {
    text.chars().filter(|c| cjk(*c)).count()
}

fn needs_space(left: &str, right: &str) -> bool {
    let (Some(a), Some(b)) = (left.chars().next_back(), right.chars().next()) else {
        return false;
    };
    !a.is_whitespace()
        && !b.is_whitespace()
        && !cjk(a)
        && !cjk(b)
        && !"([{（［｛「『“".contains(a)
        && !",.!?:;%)]}、。！？：；％）］｝」』”".contains(b)
}

fn split_byte(text: &str, max_chars: usize) -> usize {
    let cut = text
        .char_indices()
        .nth(max_chars)
        .map_or(text.len(), |(i, _)| i);
    if cut == text.len() {
        return cut;
    }
    // Prefer an actual word boundary, without losing any source characters.
    text[..cut]
        .char_indices()
        .rev()
        .find(|(_, c)| c.is_whitespace())
        .map_or(cut, |(i, c)| {
            let boundary = i + c.len_utf8();
            if boundary >= cut / 2 { boundary } else { cut }
        })
}

fn common_prefix_bytes(a: &str, b: &str) -> usize {
    a.char_indices()
        .zip(b.chars())
        .take_while(|((_, ac), bc)| ac == bc)
        .map(|((i, c), _)| i + c.len_utf8())
        .last()
        .unwrap_or(0)
}

fn common_suffix_bytes(a: &str, b: &str) -> usize {
    a.chars()
        .rev()
        .zip(b.chars().rev())
        .take_while(|(a, b)| a == b)
        .map(|(a, _)| a.len_utf8())
        .sum()
}

fn suffix_prefix_overlap(old: &str, new: &str) -> usize {
    for (start, _) in old.char_indices() {
        let suffix = &old[start..];
        if !new.starts_with(suffix) {
            continue;
        }
        let before = old[..start].chars().next_back();
        let first = suffix.chars().next();
        let last = suffix.chars().next_back();
        let after = new[suffix.len()..].chars().next();
        if before.zip(first).is_some_and(|(a, b)| word(a) && word(b))
            || last.zip(after).is_some_and(|(a, b)| word(a) && word(b))
        {
            continue;
        }
        return suffix.len();
    }
    0
}

fn tail_chars(text: &str, count: usize) -> String {
    let total = text.chars().count();
    text.chars().skip(total.saturating_sub(count)).collect()
}

/// User context is retained byte-for-byte, never truncated to make room for
/// history. History is a separate context field, never appended to source text.
pub fn compose_context(
    user: &str,
    previous_source: &str,
    previous_target: &str,
    max_chars: usize,
) -> Result<String> {
    ensure!(
        user.chars().count() <= 4_000,
        "user context exceeds 4000 characters"
    );
    ensure!(
        !user.contains("<|plamo:"),
        "reserved translation token in user context"
    );
    ensure!(
        user.chars().count() <= max_chars,
        "context capacity cannot preserve user context"
    );
    let previous_source = previous_source.replace("<|plamo:", "＜|plamo:");
    let previous_target = previous_target.replace("<|plamo:", "＜|plamo:");
    let remaining = max_chars
        .min(4_000)
        .saturating_sub(user.chars().count())
        .min(1_600);
    let separator = if user.is_empty() { "" } else { "\n\n" };
    let reference_label = "Reference only: keep terminology and register consistent. Translate the current input only; do not repeat this reference or add facts.\n";
    let source_label = "Previous source:\n";
    let target_label = "\nPrevious translation:\n";
    let overhead = separator.chars().count()
        + reference_label.chars().count()
        + source_label.chars().count()
        + target_label.chars().count();
    if previous_source.is_empty() && previous_target.is_empty() || remaining <= overhead {
        return Ok(user.to_owned());
    }
    let available = remaining - overhead;
    let source = tail_chars(&previous_source, available / 2);
    let target = tail_chars(&previous_target, available - source.chars().count());
    Ok(format!(
        "{user}{separator}{reference_label}{source_label}{source}{target_label}{target}"
    ))
}

const FILLERS: [&str; 6] = ["um", "uh", "er", "erm", "hmm", "mm"];

/// Leaves standalone English hesitation sounds out of the text sent to the
/// translator, which would render them as 「えー」 or 「えーと」.
pub fn without_fillers(text: &str) -> String {
    let mut total = 0;
    let mut previous = "";
    let mut kept = Vec::new();
    for token in text.split_whitespace() {
        total += 1;
        if !is_filler(token, previous) {
            kept.push(token);
        }
        previous = token;
    }
    // A request made only of fillers still needs text to translate.
    if kept.is_empty() || kept.len() == total {
        return text.to_owned();
    }
    kept.join(" ")
}

fn is_filler(token: &str, previous: &str) -> bool {
    let word = token.strip_suffix(',').unwrap_or(token);
    // "5 mm" is a unit.
    if word == "mm" && previous.ends_with(|c: char| c.is_ascii_digit()) {
        return false;
    }
    // Only the first letter may be uppercase; "ER" and "UM" are abbreviations.
    FILLERS
        .iter()
        .any(|filler| word.eq_ignore_ascii_case(filler) && word[1..] == filler[1..])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stamp(kind: SourceKind) -> SourceStamp {
        SourceStamp {
            kind,
            generation: 1,
            video_id: String::new(),
            epoch: 0,
        }
    }

    fn audio(a: &mut Accumulator, id: u64, text: &str, now: u64) -> PushOutcome {
        a.push_audio(
            AudioFragment {
                id,
                text: text.to_owned(),
                start_ms: id * 160,
                end_ms: id * 160 + 160,
            },
            now,
        )
        .unwrap()
    }

    #[test]
    fn a_shorter_policy_splits_the_text_that_arrives_after_it() {
        let mut a = Accumulator::new(stamp(SourceKind::Audio), roomy());
        let sentence = "This sentence is long enough to count as one part of the talk. ";
        audio(&mut a, 1, &sentence.repeat(4), 0);
        assert!(
            a.pending_snapshots().is_empty(),
            "the roomy policy keeps one open paragraph"
        );
        a.set_policy(Policy {
            min_boundary_chars: 60,
            max_paragraph_chars: 120,
            ..Policy::default()
        });
        audio(&mut a, 2, &sentence.repeat(2), 100);
        let closed = a.pending_snapshots();
        assert!(!closed.is_empty(), "the shorter policy closes a part");
        assert!(closed.iter().all(|p| p.text.ends_with('.')), "{closed:?}");
    }

    fn roomy() -> Policy {
        Policy {
            max_paragraph_chars: 2_000,
            max_paragraph_cjk_chars: 2_000,
            min_boundary_chars: 2_000,
            min_boundary_cjk_chars: 2_000,
            max_paragraph_ms: 60_000,
            ..Policy::default()
        }
    }

    #[test]
    fn punctuation_hints_split_a_guarded_raw_prefix_with_lookahead_and_preserve_clocks() {
        let mut a = Accumulator::new(
            stamp(SourceKind::Audio),
            Policy {
                min_boundary_chars: 50,
                max_paragraph_chars: 80,
                ..Policy::default()
            },
        );
        let prefix =
            "The public exhibition contains 150 words and describes the shared learning space";
        let tail = "visitors can then compare this account with the original source material";
        audio(&mut a, 1, prefix, 0);
        audio(&mut a, 2, tail, 200);
        let request = a.next_request(250).unwrap();
        // A final full stop, word-internal offset, and a hint without enough
        // right-hand context must not create a separate translation fragment.
        let end = request.snapshot.text.len();
        assert!(
            !a.apply_sentence_hints(&request, &[end, 3, end - 8], 300)
                .changed
        );
        let outcome = a.apply_sentence_hints(&request, &[prefix.len()], 300);
        assert_eq!(outcome.closed.len(), 1);
        assert_eq!(outcome.closed[0].text, prefix);
        let current = a.current_snapshot().unwrap();
        assert_eq!(current.text, tail);
        assert!(outcome.closed[0].end_ms <= current.start_ms);
        assert!(matches!(
            a.accept(&request, "old whole paragraph", 301),
            Acceptance::Rejected
        ));
        let final_request = a.next_request(301).unwrap();
        assert!(final_request.snapshot.closed);
        assert_eq!(final_request.snapshot.text, prefix);
    }

    #[test]
    fn punctuation_hints_reject_corrected_caption_and_superseded_source() {
        let mut a = Accumulator::new(
            stamp(SourceKind::Caption),
            Policy {
                min_boundary_chars: 20,
                max_paragraph_chars: 40,
                ..Policy::default()
            },
        );
        let old = "The proposal contains 150 words and explains the public exhibition";
        let rest = "visitors should compare every detail with the original source material";
        a.push_caption(&format!("{old} {rest}"), 0, 1_000, 0)
            .unwrap();
        let request = a.next_request(100).unwrap();
        a.push_caption(
            &format!("{} {rest}", old.replace("150", "250")),
            0,
            1_000,
            200,
        )
        .unwrap();
        assert!(!a.apply_sentence_hints(&request, &[old.len()], 201).changed);
        assert!(a.pending_snapshots().is_empty());
        a.reset(
            SourceStamp {
                generation: 2,
                ..stamp(SourceKind::Caption)
            },
            300,
        );
        a.push_caption(&format!("{old} {rest}"), 0, 1_000, 301)
            .unwrap();
        assert!(!a.apply_sentence_hints(&request, &[old.len()], 302).changed);
        assert!(a.pending_snapshots().is_empty());
    }

    #[test]
    fn ten_minutes_of_rolling_caption_windows_keep_every_word_and_ignore_heartbeats() {
        let words: Vec<String> = (0..200).flat_map(|index| {
            format!("Section{index} describes how the library team checks translations carefully before public display.")
                .split_whitespace().map(str::to_owned).collect::<Vec<_>>()
        }).collect();
        assert_eq!(words.len(), 2_400);
        let mut a = Accumulator::new(stamp(SourceKind::Caption), Policy::default());
        let mut final_sources = Vec::new();
        let mut heartbeat_count = 0;
        for end in 1..=words.len() {
            let start = end.saturating_sub(12);
            let text = words[start..end].join(" ");
            let now = end as u64 * 250;
            let outcome = a.push_caption(&text, start as u64 * 250, now, now).unwrap();
            assert!(outcome.overflow.is_empty());
            while let Some(request) = a.next_request(now) {
                if request.snapshot.closed {
                    final_sources.push(request.snapshot.text.clone());
                }
                assert!(!matches!(
                    a.accept(&request, "展示の説明を確認します。", now),
                    Acceptance::Rejected
                ));
            }
            if end % 7 == 0 {
                heartbeat_count += 1;
                assert!(
                    !a.push_caption(&text, start as u64 * 250, now + 100, now + 100)
                        .unwrap()
                        .changed
                );
                assert!(a.next_request(now + 100).is_none());
            }
        }
        a.close(600_000);
        while let Some(request) = a.next_request(600_000) {
            final_sources.push(request.snapshot.text.clone());
            assert!(matches!(
                a.accept(&request, "最後の文章です。", 600_000),
                Acceptance::Committed { .. }
            ));
        }
        assert!(heartbeat_count > 300);
        assert_eq!(
            final_sources
                .join(" ")
                .split_whitespace()
                .map(str::to_owned)
                .collect::<Vec<_>>(),
            words
        );
        assert!(final_sources.len() < 100);
        assert!(final_sources.iter().all(|text| word_count(text) >= 6));
    }

    #[test]
    fn english_fragments_form_one_complete_paragraph() {
        let mut a = Accumulator::new(stamp(SourceKind::Audio), roomy());
        let fragments = [
            "The neighborhood library is preparing a digital exhibition",
            "that connects translated letters with photographs of familiar streets.",
            "Visitors can compare the original text with a translation",
            "and explain which details need a clearer description.",
            "The team will review those comments before the exhibition opens.",
        ];
        for (i, text) in fragments.iter().enumerate() {
            audio(&mut a, i as u64, text, i as u64 * 800);
        }
        let current = a.current_snapshot().unwrap();
        assert_eq!(current.text, fragments.join(" "));
        assert!(!current.closed);
        a.close(4_000);
        let request = a.next_request(4_000).unwrap();
        assert!(matches!(
            a.accept(&request, "図書館の展示についての訳文。", 4_001),
            Acceptance::Committed { .. }
        ));
        assert!(a.pending_snapshots().is_empty());
    }

    #[test]
    fn inner_sentence_boundary_carries_number_unit_and_learning_phrase() {
        let policy = Policy {
            min_boundary_chars: 30,
            max_paragraph_chars: 50,
            absolute_max_chars: 200,
            ..Policy::default()
        };
        let mut a = Accumulator::new(stamp(SourceKind::Audio), policy);
        let first = "The first sentence is complete. Each description has about 150";
        let outcome = audio(&mut a, 1, first, 0);
        assert!(
            outcome.closed.is_empty(),
            "five-word continuation is not enough right context"
        );
        audio(&mut a, 2, "words in English.", 100);
        let all = a.pending_snapshots();
        assert_eq!(all[0].text, "The first sentence is complete.");
        assert_eq!(
            a.current_snapshot().unwrap().text,
            "Each description has about 150 words in English."
        );
        audio(
            &mut a,
            3,
            "The exhibition welcomes everyone. It is a starting point for shared",
            200,
        );
        assert_eq!(
            a.current_snapshot().unwrap().text,
            "It is a starting point for shared"
        );
        audio(&mut a, 4, "learning.", 300);
        a.close(400);
        assert_eq!(
            a.pending_snapshots().last().unwrap().text,
            "It is a starting point for shared learning."
        );
        let reconstructed = a
            .pending_snapshots()
            .iter()
            .map(|p| p.text.as_str())
            .collect::<Vec<_>>()
            .join(" ");
        assert_eq!(
            reconstructed,
            format!(
                "{first} words in English. The exhibition welcomes everyone. It is a starting point for shared learning."
            )
        );
    }

    #[test]
    fn sentence_scanner_ignores_abbreviations_decimals_and_domains() {
        for text in [
            "Dr. Smith has 3.14 litres",
            "The U.S. office",
            "Use e.g. a map",
            "Visit example.com now",
            "A. Smith arrived",
        ] {
            assert_eq!(last_sentence_boundary(text), None, "{text}");
        }
        assert_eq!(
            last_sentence_boundary("Dr. Smith has 3.14 litres. Next"),
            Some("Dr. Smith has 3.14 litres.".len())
        );
        assert_eq!(
            last_sentence_boundary("図書館です。次の展示"),
            Some("図書館です。".len())
        );
    }

    #[test]
    fn carried_span_clock_is_monotonic_and_crossing_request_is_rejected() {
        let policy = Policy {
            min_boundary_chars: 20,
            max_paragraph_chars: 1_000,
            max_paragraph_ms: 1_000,
            absolute_max_chars: 1_200,
            ..Policy::default()
        };
        let mut a = Accumulator::new(stamp(SourceKind::Audio), policy);
        a.push_audio(
            AudioFragment {
                id: 1,
                text: "The first sentence ends here. The next sentence is growing steadily".into(),
                start_ms: 10_000,
                end_ms: 12_000,
            },
            0,
        )
        .unwrap();
        let stale = a.next_request(0).unwrap();
        let outcome = a.tick(1_000);
        let prefix = &outcome.closed[0];
        let tail = a.current_snapshot().unwrap();
        assert!(prefix.start_ms <= prefix.end_ms);
        assert!(prefix.end_ms <= tail.start_ms && tail.start_ms <= tail.end_ms);
        assert_eq!(prefix.start_ms, 10_000);
        assert_eq!(tail.end_ms, 12_000);
        assert_eq!(
            a.accept(&stale, "境界を越えた古い訳", 1_001),
            Acceptance::Rejected
        );
        assert!(a.next_request(1_001).unwrap().snapshot.closed);
    }

    #[test]
    fn caption_location_is_rebased_to_carried_tail_for_correction_and_overlap() {
        let policy = Policy {
            min_boundary_chars: 30,
            max_paragraph_chars: 50,
            ..Policy::default()
        };
        let mut a = Accumulator::new(stamp(SourceKind::Caption), policy);
        a.push_caption(
            "The first sentence is complete. The next exhibit is blue today",
            0,
            1_000,
            0,
        )
        .unwrap();
        a.push_caption(
            "The first sentence is complete. The next exhibit is green today",
            0,
            1_000,
            100,
        )
        .unwrap();
        assert_eq!(
            a.pending_snapshots()[0].text,
            "The first sentence is complete."
        );
        assert_eq!(
            a.current_snapshot().unwrap().text,
            "The next exhibit is green today"
        );
        a.push_caption("exhibit is green today and open", 100, 1_100, 200)
            .unwrap();
        assert_eq!(
            a.current_snapshot().unwrap().text,
            "The next exhibit is green today and open"
        );
    }

    #[test]
    fn punctuation_free_speech_extends_until_acoustic_gap_or_absolute_bound() {
        let policy = Policy {
            min_boundary_chars: 20,
            max_paragraph_chars: 30,
            max_paragraph_ms: 100,
            absolute_max_chars: 120,
            acoustic_gap_ms: 600,
            ..Policy::default()
        };
        let mut a = Accumulator::new(stamp(SourceKind::Audio), policy.clone());
        a.push_audio(
            AudioFragment {
                id: 1,
                text: "one two three four five six seven eight nine ten".into(),
                start_ms: 0,
                end_ms: 1_000,
            },
            0,
        )
        .unwrap();
        assert!(a.tick(100).closed.is_empty());
        let outcome = a
            .push_audio(
                AudioFragment {
                    id: 2,
                    text: "a new section after a real pause".into(),
                    start_ms: 1_700,
                    end_ms: 2_000,
                },
                200,
            )
            .unwrap();
        assert_eq!(outcome.closed.len(), 1);
        assert_eq!(outcome.closed[0].end_ms, 1_000);
        assert_eq!(a.current_snapshot().unwrap().start_ms, 1_700);
        let mut b = Accumulator::new(stamp(SourceKind::Audio), policy);
        let source = "one two three four five six seven eight nine ten ".repeat(8);
        let mut pieces = b
            .push_audio(
                AudioFragment {
                    id: 1,
                    text: source.clone(),
                    start_ms: 0,
                    end_ms: 8_000,
                },
                0,
            )
            .unwrap()
            .closed;
        pieces.extend(b.close(1).closed);
        assert!(pieces.iter().all(|p| p.text.chars().count() <= 120));
        assert!(
            pieces
                .iter()
                .flat_map(|p| p.text.split_whitespace())
                .eq(source.split_whitespace())
        );
    }

    #[test]
    fn english_dangling_cap_gets_bounded_lookahead_without_losing_words() {
        let policy = Policy {
            min_boundary_chars: 30,
            max_paragraph_chars: 50,
            absolute_max_chars: 80,
            ..Policy::default()
        };
        let mut a = Accumulator::new(stamp(SourceKind::Audio), policy.clone());
        let first = "The library will share the material with readers as a short account of";
        audio(&mut a, 1, first, 0);
        let outcome = audio(&mut a, 2, "what changed and why Suggestions that", 1);
        assert!(outcome.closed.is_empty());
        let outcome = audio(&mut a, 3, "still need discussion will remain separate", 2);
        assert!(
            outcome.closed[0]
                .text
                .contains("account of what changed and why")
        );
        assert!(outcome.closed[0].text.chars().count() <= 80 + 128);
        let mut b = Accumulator::new(stamp(SourceKind::Audio), policy);
        audio(&mut b, 1, first, 0);
        let tail = "the ".repeat(40);
        let mut closed = audio(&mut b, 2, &tail, 1).closed;
        closed.extend(b.close(2).closed);
        assert!(closed[0].text.split_whitespace().count() <= first.split_whitespace().count() + 16);
        assert!(closed.iter().all(|p| p.text.chars().count() <= 80 + 128));
        assert!(
            closed
                .iter()
                .flat_map(|p| p.text.split_whitespace())
                .eq(format!("{first} {tail}").split_whitespace())
        );
        assert!(!dangling_english_end("図書館の展示について"));
        assert!(!dangling_english_end(
            "la biblioteca presenta una exposición a"
        ));
    }

    #[test]
    fn audio_dedup_is_identity_only_and_preserves_real_repetition() {
        let mut a = Accumulator::new(stamp(SourceKind::Audio), roomy());
        audio(&mut a, 1, "very", 0);
        assert!(!audio(&mut a, 1, "very", 1).changed);
        audio(&mut a, 2, "very", 2);
        audio(&mut a, 3, "important", 3);
        assert_eq!(a.current_snapshot().unwrap().text, "very very important");
        assert_eq!(a.current_snapshot().unwrap().revision, 3);
    }

    #[test]
    fn caption_heartbeat_extension_rolling_and_correction_do_not_repeat_tail() {
        let mut a = Accumulator::new(stamp(SourceKind::Caption), roomy());
        a.push_caption("We need clean energy", 0, 100, 0).unwrap();
        assert!(
            !a.push_caption("We need clean energy", 0, 200, 200)
                .unwrap()
                .changed
        );
        a.push_caption("We need clean energy for everyone", 0, 300, 300)
            .unwrap();
        a.push_caption("energy for everyone in this town", 100, 400, 400)
            .unwrap();
        a.push_caption("energy for everyone in this city", 100, 400, 500)
            .unwrap();
        assert_eq!(
            a.current_snapshot().unwrap().text,
            "We need clean energy for everyone in this city"
        );
    }

    #[test]
    fn corrections_can_change_word_length_without_appending() {
        let mut a = Accumulator::new(stamp(SourceKind::Caption), roomy());
        a.push_caption("clean energy now", 0, 100, 0).unwrap();
        a.push_caption("cleaner energy now", 10, 120, 100).unwrap();
        assert_eq!(a.current_snapshot().unwrap().text, "cleaner energy now");
        a.push_caption("cleaner energy", 10, 120, 200).unwrap();
        assert_eq!(a.current_snapshot().unwrap().text, "cleaner energy");
    }

    #[test]
    fn closed_uncommitted_caption_correction_invalidates_old_translation() {
        let mut a = Accumulator::new(stamp(SourceKind::Caption), roomy());
        a.push_caption("The library opens on Monday", 0, 100, 0)
            .unwrap();
        a.close(1);
        let stale = a.next_request(1).unwrap();
        a.push_caption("Our library opens on Tuesday", 0, 100, 2)
            .unwrap();
        assert_eq!(
            a.pending_snapshots()[0].text,
            "Our library opens on Tuesday"
        );
        assert_eq!(a.accept(&stale, "月曜に開く", 3), Acceptance::Rejected);
        let corrected = a.next_request(3).unwrap();
        assert!(matches!(
            a.accept(&corrected, "火曜に開く", 4),
            Acceptance::Committed { .. }
        ));
        a.push_caption("Our library opens on Wednesday", 0, 100, 5)
            .unwrap();
        assert!(a.current_snapshot().is_none());
        assert!(a.pending_snapshots().is_empty());
    }

    #[test]
    fn caption_prefix_continues_after_closure_without_repeating_prior_window() {
        let mut a = Accumulator::new(stamp(SourceKind::Caption), roomy());
        a.push_caption("The library has books", 0, 100, 0).unwrap();
        a.close(1);
        a.push_caption("The library has books and maps", 0, 200, 2)
            .unwrap();
        assert_eq!(a.current_snapshot().unwrap().text, "and maps");
        a.push_caption("books and maps for local readers", 100, 300, 3)
            .unwrap();
        assert_eq!(
            a.current_snapshot().unwrap().text,
            "and maps for local readers"
        );
    }

    #[test]
    fn overlap_respects_english_words_and_utf8_cjk_boundaries() {
        assert_eq!(suffix_prefix_overlap("translate", "late at night"), 0);
        assert_eq!(suffix_prefix_overlap("a plan", "planet"), 0);
        let mut a = Accumulator::new(stamp(SourceKind::Caption), roomy());
        a.push_caption("東京の図書館", 0, 100, 0).unwrap();
        a.push_caption("図書館では展示を行います。", 10, 200, 100)
            .unwrap();
        assert_eq!(
            a.current_snapshot().unwrap().text,
            "東京の図書館では展示を行います。"
        );
        let mut b = Accumulator::new(stamp(SourceKind::Audio), roomy());
        audio(&mut b, 1, "これは", 0);
        audio(&mut b, 2, "これは", 100);
        assert_eq!(b.current_snapshot().unwrap().text, "これはこれは");
    }

    #[test]
    fn append_only_old_revision_is_interim_but_corrected_revision_is_rejected() {
        let mut a = Accumulator::new(stamp(SourceKind::Audio), roomy());
        audio(&mut a, 1, "There are enough words for the first request", 0);
        let first = a.next_request(0).unwrap();
        audio(&mut a, 2, "and the source keeps growing", 100);
        a.close(200);
        assert!(
            matches!(a.accept(&first, "古い範囲に対応する訳文", 300), Acceptance::Interim { snapshot, .. } if !snapshot.finalizable && !snapshot.closed)
        );
        let final_request = a.next_request(300).unwrap();
        assert!(matches!(
            a.accept(&final_request, "全体に対応する訳文", 400),
            Acceptance::Committed { .. }
        ));

        let mut b = Accumulator::new(stamp(SourceKind::Caption), roomy());
        b.push_caption("The exhibition opens on Monday next week", 0, 10, 0)
            .unwrap();
        let wrong = b.next_request(0).unwrap();
        b.push_caption("The exhibition opens on Tuesday next week", 0, 10, 100)
            .unwrap();
        assert_eq!(b.accept(&wrong, "月曜に開く", 200), Acceptance::Rejected);
        assert!(b.interim().is_none());
    }

    #[test]
    fn stop_finalizes_latest_revision_and_reset_rejects_old_jobs() {
        let mut a = Accumulator::new(stamp(SourceKind::Audio), roomy());
        audio(&mut a, 1, "There are enough words for the first request", 0);
        let request = a.next_request(0).unwrap();
        assert!(matches!(
            a.accept(&request, "途中の訳文", 10),
            Acceptance::Interim { .. }
        ));
        a.close(20);
        let closed = a.next_request(20).unwrap();
        assert!(closed.snapshot.closed);
        let same_stamp = a.stamp().clone();
        a.reset(same_stamp, 30);
        audio(
            &mut a,
            2,
            "There are enough words for a different request",
            40,
        );
        let fresh = a.next_request(40).unwrap();
        assert_ne!(fresh.id, closed.id);
        assert_ne!(fresh.snapshot.key, closed.snapshot.key);
        assert_eq!(a.accept(&closed, "旧世代の訳文", 50), Acceptance::Rejected);
        assert!(a.is_inflight());
        assert!(!a.fail(&closed, 50));
        assert!(a.fail(&fresh, 50));
    }

    #[test]
    fn busy_input_coalesces_and_never_starves_continuous_burst() {
        let mut a = Accumulator::new(stamp(SourceKind::Audio), roomy());
        audio(&mut a, 0, "one two three four five six", 0);
        let first = a.next_request(0).unwrap();
        for i in 1..20 {
            audio(&mut a, i, "another meaningful section continues", i * 100);
            assert!(a.next_request(i * 100).is_none());
        }
        assert!(matches!(
            a.accept(&first, "途中の訳", 1_900),
            Acceptance::Interim { .. }
        ));
        assert!(a.next_request(1_999).is_none());
        let latest = a.next_request(2_000).unwrap();
        assert_eq!(latest.snapshot.revision, 20);
        assert!(
            latest
                .snapshot
                .text
                .ends_with("another meaningful section continues")
        );
    }

    #[test]
    fn policy_closes_sixty_second_forty_fragment_speech_into_few_paragraphs() {
        let mut a = Accumulator::new(stamp(SourceKind::Audio), Policy::default());
        let mut closed = Vec::new();
        for i in 0..40 {
            let outcome = audio(&mut a, i, "The library welcomes useful ideas.", i * 1_500);
            assert!(outcome.overflow.is_empty());
            closed.extend(outcome.closed);
        }
        closed.extend(a.close(60_000).closed);
        assert!(
            (1..=2).contains(&closed.len()),
            "{} paragraphs",
            closed.len()
        );
        assert_eq!(
            closed
                .iter()
                .map(|p| p.text.split_whitespace().count())
                .sum::<usize>(),
            40 * 5
        );
        assert!(closed.iter().all(|p| p.text.chars().count() <= 900));
    }

    #[test]
    fn advancing_activity_extends_idle_without_changing_source_or_translation_work() {
        let mut a = Accumulator::new(stamp(SourceKind::Caption), Policy::default());
        a.push_caption(
            "The library has enough words for an initial translation",
            1_000,
            4_000,
            0,
        )
        .unwrap();
        let request = a.next_request(0).unwrap();
        assert!(matches!(
            a.accept(&request, "coherent interim", 1),
            Acceptance::Interim { .. }
        ));
        let before = a.current_snapshot().unwrap();
        for now in [3_000, 7_000, 12_000, 18_000, 25_000, 30_000] {
            a.note_activity(now);
            assert!(a.tick(now).closed.is_empty());
            assert_eq!(a.current_snapshot().unwrap(), before);
            assert!(a.next_request(now).is_none());
        }
        assert!(a.tick(41_999).closed.is_empty());
        assert_eq!(a.tick(42_000).closed.len(), 1);
        assert_eq!(a.interim().unwrap().target, "coherent interim");
    }

    #[test]
    fn elapsed_time_alone_does_not_finalize_a_short_cue_and_audio_gaps_stay_joined() {
        let mut a = Accumulator::new(stamp(SourceKind::Caption), Policy::default());
        a.push_caption("The rate remains around 4.1", 100, 3_100, 0)
            .unwrap();
        a.note_activity(60_000);
        assert!(a.tick(60_000).closed.is_empty());
        assert_eq!(
            a.current_snapshot().unwrap().text,
            "The rate remains around 4.1"
        );
        a.push_caption(
            "percent while other conditions remain stable",
            3_100,
            6_100,
            60_001,
        )
        .unwrap();
        assert!(a.pending_snapshots().is_empty());
        assert!(a.current_snapshot().unwrap().text.contains("4.1 percent"));
        let mut b = Accumulator::new(stamp(SourceKind::Audio), Policy::default());
        let source = "The library reviews the evidence carefully ".repeat(16);
        b.push_audio(
            AudioFragment {
                id: 1,
                text: source.clone(),
                start_ms: 0,
                end_ms: 1_000,
            },
            0,
        )
        .unwrap();
        let outcome = b
            .push_audio(
                AudioFragment {
                    id: 2,
                    text: "and records the complete account".into(),
                    start_ms: 8_000,
                    end_ms: 9_000,
                },
                7_000,
            )
            .unwrap();
        assert!(outcome.closed.is_empty());
        assert!(
            b.current_snapshot()
                .unwrap()
                .text
                .ends_with("and records the complete account")
        );
    }

    #[test]
    #[ignore = "private actual-A text-only replay; requires explicit fixture and report paths"]
    fn actual_a_caption_replay_opt_in() {
        use serde::Deserialize;
        #[derive(Deserialize)]
        struct Cue {
            id: u64,
            start_ms: u64,
            end_ms: u64,
            text: String,
        }
        #[derive(Deserialize)]
        struct Fixture {
            video_id: String,
            cues: Vec<Cue>,
        }
        fn drain(a: &mut Accumulator, now: u64, finals: &mut Vec<ParagraphSnapshot>) {
            while let Some(request) = a.next_request(now) {
                if request.snapshot.closed {
                    finals.push(request.snapshot.clone());
                }
                assert!(!matches!(
                    a.accept(&request, "text-only replay placeholder", now),
                    Acceptance::Rejected
                ));
            }
        }
        fn replay(fixture: &Fixture, policy: Policy, activity: bool) -> Vec<ParagraphSnapshot> {
            let mut a = Accumulator::new(
                SourceStamp {
                    video_id: fixture.video_id.clone(),
                    ..stamp(SourceKind::Caption)
                },
                policy,
            );
            let origin = fixture.cues[0].start_ms;
            let mut now = 0;
            let mut finals = Vec::new();
            for cue in &fixture.cues {
                let arrival = cue.end_ms.saturating_sub(origin).max(now);
                while now < arrival {
                    now = (now + 250).min(arrival);
                    if activity {
                        a.note_activity(now);
                    }
                    assert!(a.tick(now).overflow.is_empty());
                    drain(&mut a, now, &mut finals);
                }
                if activity {
                    a.note_activity(now);
                }
                assert!(
                    a.push_caption(&cue.text, cue.start_ms, cue.end_ms, now)
                        .unwrap()
                        .overflow
                        .is_empty()
                );
                drain(&mut a, now, &mut finals);
            }
            assert!(a.close(now).overflow.is_empty());
            drain(&mut a, now, &mut finals);
            finals
        }
        let fixture_path =
            std::env::var_os("JIMOTO_ACTUAL_A_FIXTURE").expect("JIMOTO_ACTUAL_A_FIXTURE");
        let report = std::env::var_os("JIMOTO_ACTUAL_A_REPORT").expect("JIMOTO_ACTUAL_A_REPORT");
        let raw = std::fs::read_to_string(fixture_path).unwrap();
        let fixture: Fixture = serde_json::from_str(raw.trim_start_matches('\u{feff}')).unwrap();
        let old = Policy {
            idle_close_ms: 2_500,
            short_idle_close_ms: 6_000,
            min_boundary_chars: 220,
            max_paragraph_chars: 360,
            min_boundary_cjk_chars: 110,
            max_paragraph_cjk_chars: 180,
            max_paragraph_ms: 20_000,
            absolute_max_chars: 1_200,
            acoustic_gap_ms: 600,
            ..Policy::default()
        };
        let baseline = replay(&fixture, old, false);
        let revised = replay(&fixture, Policy::default(), true);
        assert!(
            (3..=6).contains(&revised.len()),
            "{} paragraphs",
            revised.len()
        );
        let expected = fixture
            .cues
            .iter()
            .map(|cue| cue.text.as_str())
            .collect::<Vec<_>>()
            .join(" ");
        let actual = revised
            .iter()
            .map(|p| p.text.as_str())
            .collect::<Vec<_>>()
            .join(" ");
        assert!(
            actual.split_whitespace().eq(expected.split_whitespace()),
            "Source words/order changed"
        );
        let protected = [
            "4.1 percent",
            "3.6 percent",
            "3.2 and 2.4 percent",
            "2 percent PCE inflation objective",
        ];
        for phrase in protected {
            assert!(
                revised.iter().any(|p| p.text.contains(phrase)),
                "phrase split: {phrase}"
            );
        }
        assert!(
            revised
                .windows(2)
                .all(|pair| pair[0].end_ms <= pair[1].start_ms)
        );
        let as_json = |snapshots: &[ParagraphSnapshot]| {
            snapshots.iter().map(|p|serde_json::json!({"key":p.key,"revision":p.revision,"source":p.text,"source_words":word_count(&p.text),"source_chars":p.text.chars().count(),"start_ms":p.start_ms,"end_ms":p.end_ms})).collect::<Vec<_>>()
        };
        std::fs::write(report,serde_json::to_vec_pretty(&serde_json::json!({"scope":"Only paragraph-tagged YouTube sources from explicitly specified actual-A journal","mode":"pure text replay; wall arrivals derived from source end times, advancing heartbeat simulated every 250ms; no model/GUI/audio/network","input_rows":fixture.cues.len(),"first_row_id":fixture.cues.first().unwrap().id,"last_row_id":fixture.cues.last().unwrap().id,"original_words":word_count(&expected),"original_chars":expected.chars().count(),"old_idle_policy_replay_paragraphs":baseline.len(),"new_policy_replay_paragraphs":revised.len(),"source_words_and_order_preserved":true,"protected_phrases_same_paragraph":protected,"source_clocks_monotonic":true,"old_policy_sources":as_json(&baseline),"new_policy_sources":as_json(&revised)})).unwrap()).unwrap();
    }

    #[test]
    fn bounds_return_overflow_and_keep_only_recent_context() {
        let policy = Policy {
            max_paragraph_chars: 20,
            max_paragraph_cjk_chars: 20,
            max_pending_closed: 2,
            prior_context_chars: 40,
            previous_pairs: 3,
            ..Policy::default()
        };
        let mut a = Accumulator::new(stamp(SourceKind::Audio), policy);
        let mut overflow = Vec::new();
        for i in 0..8 {
            overflow.extend(audio(&mut a, i, "one two three four", i * 10).overflow);
            overflow.extend(a.close(i * 10).overflow);
        }
        assert_eq!(a.pending_snapshots().len(), 2);
        assert_eq!(overflow.len(), 6);
        for now in [100, 200] {
            let request = a.next_request(now).unwrap();
            assert!(matches!(
                a.accept(&request, "a bounded final translation", now),
                Acceptance::Committed { .. }
            ));
        }
        let (source, target) = a.previous_context();
        assert!(source.chars().count() + target.chars().count() <= 40);
    }

    #[test]
    fn failures_retry_once_without_spin_or_head_of_line_deadlock() {
        let mut a = Accumulator::new(stamp(SourceKind::Audio), roomy());
        audio(&mut a, 1, "The first source is ready to translate", 0);
        a.close(0);
        let request = a.next_request(0).unwrap();
        assert!(a.fail(&request, 100));
        assert!(a.next_request(1_599).is_none());
        let retry = a.next_request(1_600).unwrap();
        assert!(a.fail(&retry, 1_700));
        assert!(a.next_request(100_000).is_none());
        audio(&mut a, 2, "A second source must not wait forever", 100_001);
        a.close(100_002);
        assert_ne!(
            a.next_request(100_003).unwrap().snapshot.key,
            request.snapshot.key
        );
        assert_eq!(a.pending_snapshots().len(), 2);
    }

    #[test]
    fn repeated_cancellation_preserves_source_and_failure_retry_budget() {
        let mut a = Accumulator::new(stamp(SourceKind::Audio), roomy());
        audio(&mut a, 1, "The library is ready for the exhibition", 0);
        a.close(0);
        let original = a.pending_snapshots()[0].clone();
        let mut request = a.next_request(0).unwrap();
        for now in 1..=4 {
            assert!(a.cancel_request(&request, now));
            let replacement = a.next_request(now).unwrap();
            assert!(!a.cancel_request(&request, now));
            assert!(a.is_inflight());
            request = replacement;
            assert_eq!(request.snapshot, original);
        }
        assert!(a.fail(&request, 5));
        let retry = a.next_request(1_505).unwrap();
        assert!(a.cancel_request(&retry, 1_506));
        let replacement = a.next_request(1_506).unwrap();
        assert!(matches!(
            a.accept(&replacement, "展示の準備が整いました。", 1_507),
            Acceptance::Committed { .. }
        ));
        assert!(a.pending_snapshots().is_empty());
        assert!(!a.previous_context().0.is_empty());
    }

    #[test]
    fn source_timestamps_do_not_control_wall_clock_or_short_phrase_idle() {
        let mut a = Accumulator::new(stamp(SourceKind::Audio), Policy::default());
        a.push_audio(
            AudioFragment {
                id: 1,
                text: "Thank you".to_owned(),
                start_ms: 9_000_000,
                end_ms: 9_000_200,
            },
            0,
        )
        .unwrap();
        assert!(a.tick(12_000).closed.is_empty());
        assert_eq!(a.tick(15_000).closed.len(), 1);
        assert_eq!(a.pending_snapshots()[0].start_ms, 9_000_000);
    }

    #[test]
    fn reserved_tokens_bounds_and_context_budget_protect_user_instructions() {
        let mut a = Accumulator::new(stamp(SourceKind::Audio), Policy::default());
        assert!(
            a.push_audio(
                AudioFragment {
                    id: 1,
                    text: "<|plamo:tag|>".to_owned(),
                    start_ms: 0,
                    end_ms: 1
                },
                0
            )
            .is_err()
        );
        assert!(
            a.push_audio(
                AudioFragment {
                    id: 1,
                    text: "x".repeat(16_001),
                    start_ms: 0,
                    end_ms: 1
                },
                0
            )
            .is_err()
        );
        let user = "語".repeat(4_000);
        assert_eq!(
            compose_context(&user, "old source", "old target", 4_000).unwrap(),
            user
        );
        let composed = compose_context(
            "Keep this exact. ",
            "x<|plamo:tag|>y",
            &"訳".repeat(1_000),
            100,
        )
        .unwrap();
        assert!(composed.starts_with("Keep this exact. "));
        assert!(!composed.contains("<|plamo:"));
        assert!(composed.chars().count() <= 100);
        assert!(compose_context("<|plamo:tag|>", "", "", 4_000).is_err());
        assert!(compose_context("retain all", "", "", 3).is_err());
        let capped =
            compose_context("user", &"s".repeat(3_000), &"t".repeat(3_000), 4_000).unwrap();
        assert!(capped.chars().count() <= 1_604);
    }

    fn spoken(a: &mut Accumulator, id: u64, text: &str, start_ms: u64, end_ms: u64) {
        a.push_audio(
            AudioFragment {
                id,
                text: text.to_owned(),
                start_ms,
                end_ms,
            },
            end_ms,
        )
        .unwrap();
    }

    #[test]
    fn a_pause_after_a_finished_sentence_hands_over_to_a_new_part() {
        let finished = "The library will open a new exhibition next week.";
        let mut a = Accumulator::new(stamp(SourceKind::Audio), Policy::default());
        spoken(&mut a, 1, finished, 0, 2_000);
        spoken(&mut a, 2, "Thank you for the introduction.", 3_600, 5_000);
        assert_eq!(a.pending_snapshots()[0].text, finished);
        assert_eq!(
            a.current_snapshot().unwrap().text,
            "Thank you for the introduction."
        );

        for (first, gap, why) in [
            (
                "The library will open a new exhibition and",
                1_600,
                "a pause mid-sentence keeps one part",
            ),
            (finished, 1_000, "a short pause keeps one part"),
            (
                "Thank you.",
                1_600,
                "too little text to stand alone keeps one part",
            ),
        ] {
            let mut a = Accumulator::new(stamp(SourceKind::Audio), Policy::default());
            spoken(&mut a, 1, first, 0, 2_000);
            spoken(&mut a, 2, "visitors are welcome.", 2_000 + gap, 4_000 + gap);
            assert!(a.pending_snapshots().is_empty(), "{why}");
        }
    }

    #[test]
    fn a_speaker_marker_in_captions_starts_a_new_part_without_the_marker() {
        let mut a = Accumulator::new(stamp(SourceKind::Caption), Policy::default());
        let first = "I think the exhibition is ready for visitors";
        a.push_caption(first, 0, 3_000, 0).unwrap();
        let window = format!("{first} >> Thank you, let us open the doors");
        a.push_caption(&window, 0, 3_000, 300).unwrap();
        assert_eq!(a.pending_snapshots()[0].text, first);
        assert_eq!(
            a.current_snapshot().unwrap().text,
            "Thank you, let us open the doors"
        );

        a.push_caption(&format!("{window} now"), 0, 3_000, 600)
            .unwrap();
        assert_eq!(
            a.pending_snapshots().len(),
            1,
            "an old marker is not reused"
        );
        assert_eq!(
            a.current_snapshot().unwrap().text,
            "Thank you, let us open the doors now"
        );
        let corrected = format!("{first} >> Thank you, let us open the door now");
        a.push_caption(&corrected, 0, 3_000, 900).unwrap();
        assert_eq!(
            a.current_snapshot().unwrap().text,
            "Thank you, let us open the door now",
            "a correction after the marker edits the new part"
        );
        let all = [a.pending_snapshots(), vec![a.current_snapshot().unwrap()]].concat();
        assert!(
            all.iter().all(|p| !p.text.contains(SPEAKER_TURN)),
            "{all:?}"
        );
    }

    #[test]
    fn standalone_fillers_are_left_out_and_every_other_word_stays() {
        for (text, expected) in [
            (
                "them. Um now, some of the questions that I asked to kick off the reports, uh they want to answer those and answer a whole range of other questions, which might go to other task forces, but I'm impressed by what they're doing. Um I gave them, like I told you uh a year-end deadline.",
                "them. now, some of the questions that I asked to kick off the reports, they want to answer those and answer a whole range of other questions, which might go to other task forces, but I'm impressed by what they're doing. I gave them, like I told you a year-end deadline.",
            ),
            (
                "On November 16th, uh the productivity and jobs task force will be doing a field hearing um inviting a broad range of people who have strong views.",
                "On November 16th, the productivity and jobs task force will be doing a field hearing inviting a broad range of people who have strong views.",
            ),
            (
                "Um, I think, uh, we should go. Hmm, er, erm, mm, okay.",
                "I think, we should go. okay.",
            ),
            (">> Well, um we've got to", ">> Well, we've got to"),
        ] {
            assert_eq!(without_fillers(text), expected, "{text}");
        }
    }

    #[test]
    fn words_that_only_look_like_fillers_are_kept() {
        for text in [
            "Ah, I see.",
            "Uh-huh, mm-hmm.",
            "Yes, uh-huh, I agree. Mm-hmm, go on.",
            "The ER and UM teams met.",
            "An umbrella in summer for her.",
            "Use a 5 mm screw.",
            "Drill a 5 mm, then a 6 mm hole.",
            ">> So the mandate was given to us by Congress.",
            "So it was, um.",
            "Is it, uh? (Er) no.",
            "Well, um,, okay.",
        ] {
            assert_eq!(without_fillers(text), text);
        }
    }

    #[test]
    fn text_with_no_filler_or_nothing_else_comes_back_byte_for_byte() {
        for text in [
            "Um",
            "uh, um,",
            "Um  uh\nhmm",
            "First line.\nSecond  line.",
            "  padded  ",
        ] {
            assert_eq!(without_fillers(text), text);
        }
    }
}
