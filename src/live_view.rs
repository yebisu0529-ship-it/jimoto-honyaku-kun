//! View-only live timeline bookkeeping. Does not alter caption identity or rows.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum LatestMode {
    Draft,
    Current,
    Empty,
}

pub(super) fn latest_mode(has_draft: bool, has_current: bool) -> LatestMode {
    if has_draft {
        LatestMode::Draft
    } else if has_current {
        LatestMode::Current
    } else {
        LatestMode::Empty
    }
}
pub(super) fn history_visible(id: u64, pending: bool, current: Option<u64>) -> bool {
    !pending && Some(id) != current
}

#[derive(Clone, Copy, Debug)]
pub(super) struct RowGeometry {
    pub id: u64,
    pub top: f32,
    pub bottom: f32,
}
#[derive(Clone, Copy, Debug)]
struct Anchor {
    id: u64,
    screen_top: f32,
    content_top: f32,
}

#[derive(Default)]
pub(super) struct LiveViewState {
    #[cfg(test)]
    pub(super) history_viewport_height: f32,
    #[cfg(test)]
    pub(super) history_order: Vec<u64>,
    #[cfg(test)]
    pub(super) history_geometry: Vec<RowGeometry>,
    #[cfg(test)]
    pub(super) history_offset: f32,
    #[cfg(test)]
    pub(super) history_rect: Option<eframe::egui::Rect>,
    anchors: Vec<Anchor>,
    auto_scroll: bool,
    force_tail: bool,
    last_offset: Option<f32>,
}
impl LiveViewState {
    /// Only an explicit enable (or first display) forces the tail. New rows
    /// use ScrollArea's sticky tail and never interrupt a user reading above it.
    pub(super) fn prepare_history(&mut self, auto_scroll: bool) -> bool {
        let follow = auto_scroll && !self.auto_scroll;
        self.force_tail = follow;
        self.auto_scroll = auto_scroll;
        if follow {
            self.anchors.clear();
        }
        follow
    }

    /// Restore a surviving visible row at the same screen coordinate, using the
    /// actual variable card heights. User scrolling takes precedence. Remember
    /// several rows so removing the first visible current/archived row still
    /// leaves a useful anchor. At the content limits the offset must be clamped.
    pub(super) fn finish(
        &mut self,
        rows: &[RowGeometry],
        offset: f32,
        viewport_height: f32,
        content_height: f32,
        user_scrolling: bool,
    ) -> Option<f32> {
        let max = (content_height - viewport_height).max(0.0);
        let force_tail = std::mem::take(&mut self.force_tail);
        let correction = if force_tail {
            ((offset - max).abs() > 0.5).then_some(max)
        } else if !self.auto_scroll || offset < max - 0.5 {
            self.anchors
                .iter()
                .find_map(|anchor| {
                    rows.iter()
                        .find(|row| row.id == anchor.id)
                        .map(|row| (row, anchor))
                })
                .filter(|(row, anchor)| (row.top - anchor.content_top).abs() > 0.5)
                .map(|(row, anchor)| {
                    let user_delta = if user_scrolling {
                        offset - self.last_offset.unwrap_or(offset)
                    } else {
                        0.0
                    };
                    (row.top - anchor.screen_top + user_delta).clamp(0.0, max)
                })
                .filter(|target| (*target - offset).abs() > 0.5)
        } else {
            None
        }
        .or_else(|| (offset > max + 0.5).then_some(max));
        let effective = correction.unwrap_or(offset);
        self.anchors = rows
            .iter()
            .filter(|row| row.bottom > effective && row.top < effective + viewport_height)
            .take(4)
            .map(|row| Anchor {
                id: row.id,
                screen_top: row.top - effective,
                content_top: row.top,
            })
            .collect();
        self.last_offset = Some(effective);
        correction
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn row(id: u64, top: f32, bottom: f32) -> RowGeometry {
        RowGeometry { id, top, bottom }
    }
    #[test]
    fn draft_and_current_never_duplicate_in_history() {
        assert_eq!(latest_mode(true, true), LatestMode::Draft);
        assert_eq!(latest_mode(false, true), LatestMode::Current);
        assert_eq!(latest_mode(false, false), LatestMode::Empty);
        assert!(!history_visible(4, false, Some(4)));
        assert!(!history_visible(3, true, Some(4)));
        assert!(history_visible(2, false, Some(4)));
    }
    #[test]
    fn only_explicit_enable_forces_tail_and_appends_do_not_interrupt_reading() {
        let mut state = LiveViewState::default();
        assert!(state.prepare_history(true));
        assert_eq!(
            state.finish(&[row(1, 0.0, 900.0)], 0.0, 300.0, 900.0, false),
            Some(600.0)
        );
        assert!(!state.prepare_history(true));
        assert_eq!(
            state.finish(&[row(1, 0.0, 900.0)], 200.0, 300.0, 900.0, true),
            None
        );
        assert!(!state.prepare_history(true));
        assert_eq!(
            state.finish(
                &[row(1, 0.0, 900.0), row(2, 900.0, 1100.0)],
                200.0,
                300.0,
                1100.0,
                false
            ),
            None
        );
        assert!(!state.prepare_history(false));
        assert!(state.prepare_history(true));
        assert_eq!(
            state.finish(
                &[row(1, 0.0, 900.0), row(2, 900.0, 1100.0)],
                200.0,
                300.0,
                1100.0,
                false
            ),
            Some(800.0)
        );
    }
    #[test]
    fn variable_height_prepend_and_removed_first_anchor_keep_reading_position() {
        let mut state = LiveViewState::default();
        state.prepare_history(false);
        assert_eq!(
            state.finish(
                &[
                    row(3, 0.0, 100.0),
                    row(2, 100.0, 260.0),
                    row(1, 260.0, 700.0)
                ],
                140.0,
                300.0,
                700.0,
                false
            ),
            None
        );
        state.prepare_history(false);
        assert_eq!(
            state.finish(
                &[
                    row(4, 0.0, 180.0),
                    row(3, 180.0, 280.0),
                    row(2, 280.0, 440.0),
                    row(1, 440.0, 880.0)
                ],
                140.0,
                300.0,
                880.0,
                false
            ),
            Some(320.0)
        );
        // Row 2 disappears; row 1 is the surviving second anchor.
        state.prepare_history(false);
        assert_eq!(
            state.finish(
                &[
                    row(4, 0.0, 180.0),
                    row(3, 180.0, 280.0),
                    row(1, 280.0, 720.0)
                ],
                320.0,
                300.0,
                720.0,
                false
            ),
            Some(160.0)
        );
    }
    #[test]
    fn user_scroll_and_content_limits_override_anchor() {
        let mut state = LiveViewState::default();
        state.prepare_history(false);
        state.finish(&[row(1, 0.0, 900.0)], 200.0, 300.0, 900.0, false);
        assert_eq!(
            state.finish(&[row(1, 0.0, 900.0)], 260.0, 300.0, 900.0, true),
            None
        );
        assert_eq!(
            state.finish(&[row(1, 0.0, 900.0)], 260.0, 300.0, 900.0, false),
            None
        );
        assert_eq!(
            state.finish(&[row(1, 0.0, 100.0)], 260.0, 300.0, 100.0, false),
            Some(0.0)
        );
    }
    #[test]
    fn reading_anchor_survives_older_paragraph_resize_while_auto_scroll_is_on() {
        let mut state = LiveViewState::default();
        state.prepare_history(true);
        state.finish(
            &[
                row(1, 0.0, 200.0),
                row(2, 200.0, 500.0),
                row(3, 500.0, 900.0),
            ],
            600.0,
            300.0,
            900.0,
            false,
        );
        state.prepare_history(true);
        state.finish(
            &[
                row(1, 0.0, 200.0),
                row(2, 200.0, 500.0),
                row(3, 500.0, 900.0),
            ],
            250.0,
            300.0,
            900.0,
            true,
        );
        state.prepare_history(true);
        assert_eq!(
            state.finish(
                &[
                    row(1, 0.0, 320.0),
                    row(2, 320.0, 620.0),
                    row(3, 620.0, 1020.0),
                    row(4, 1020.0, 1200.0)
                ],
                250.0,
                300.0,
                1200.0,
                false
            ),
            Some(370.0)
        );
    }
}
