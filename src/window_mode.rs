//! Root-window geometry commands only; no native calls or application state.
//!
//! egui-winit 0.33.3 processes commands in sequence: InnerSize/OuterPosition/
//! MinInnerSize multiply egui points by native pixels-per-point * egui zoom.
//! Windows resize requests are asynchronous. Clear fullscreen/maximized before
//! requesting geometry, then restore the original flags last. Pure tests cover
//! command construction; real Windows/DPI/monitor round trips remain unverified.
use eframe::egui::{Pos2, Vec2, ViewportCommand, ViewportInfo, WindowLevel};

pub const NORMAL_SIZE: Vec2 = Vec2::new(1180.0, 800.0);
pub const NORMAL_MIN: Vec2 = Vec2::new(860.0, 620.0);
pub const MINI_SIZE: Vec2 = Vec2::new(520.0, 360.0);
pub const MINI_MIN: Vec2 = Vec2::new(400.0, 260.0);
const MAX_SIZE: f32 = 16_384.0;

#[derive(Clone, Copy, Debug)]
struct Geometry {
    // Native logical pixels (DIP), independent of egui zoom.
    size: Vec2,
    // Physical desktop coordinates, preserving negative monitor positions.
    position: Option<Pos2>,
    maximized: bool,
    fullscreen: bool,
}
impl Default for Geometry {
    fn default() -> Self {
        Self {
            size: NORMAL_SIZE,
            position: None,
            maximized: false,
            fullscreen: false,
        }
    }
}

pub struct MiniWindowState {
    mini: bool,
    normal: Geometry,
    snapshot: Option<Geometry>,
    mini_size: Vec2,
    mini_position: Option<Pos2>,
    always_on_top: bool,
    zoom: f32,
    native_scale: f32,
    awaiting_mini: bool,
    awaiting_normal: bool,
}
impl Default for MiniWindowState {
    fn default() -> Self {
        Self {
            mini: false,
            normal: Geometry::default(),
            snapshot: None,
            mini_size: MINI_SIZE,
            mini_position: None,
            always_on_top: false,
            zoom: 1.0,
            native_scale: 1.0,
            awaiting_mini: false,
            awaiting_normal: false,
        }
    }
}
impl MiniWindowState {
    pub fn is_mini(&self) -> bool {
        self.mini
    }
    pub fn normal_geometry_restored(&self) -> bool {
        !self.mini && !self.awaiting_normal
    }
    pub fn always_on_top(&self) -> bool {
        self.mini && self.always_on_top
    }
    /// Current remembered mini size in egui points, suitable for viewport commands.
    pub fn mini_size(&self) -> Vec2 {
        clamp_size(self.mini_size, MINI_MIN * self.zoom, MINI_SIZE) / self.zoom
    }

    /// Compatibility observer; retains the last known zoom (initially 1).
    pub fn observe(&mut self, info: &ViewportInfo) {
        self.native_scale = scale(info.native_pixels_per_point, self.native_scale);
        if info.minimized == Some(true) {
            return;
        }
        let size = info
            .inner_rect
            .and_then(|r| valid_size(r.size()))
            .map(|s| s * self.zoom);
        let position = info
            .outer_rect
            .and_then(|r| valid_position(r.min))
            .map(|p| {
                Pos2::new(
                    p.x * self.native_scale * self.zoom,
                    p.y * self.native_scale * self.zoom,
                )
            });
        if self.mini {
            // Avoid retaining the old normal rectangle during the first pending
            // native resize. A real mini resize is accepted on later frames.
            if self.awaiting_mini {
                if info.maximized == Some(true) || info.fullscreen == Some(true) {
                    return;
                }
                if let (Some(size), Some(snapshot)) = (size, self.snapshot)
                    && near(size, snapshot.size)
                    && !near(size, self.mini_size)
                {
                    return;
                }
                if size.is_none() {
                    return;
                }
                self.awaiting_mini = false;
            }
            if let Some(size) = size {
                self.mini_size = clamp_size(size, MINI_MIN * self.zoom, MINI_SIZE);
            }
            if position.is_some() {
                self.mini_position = position;
            }
        } else {
            if self.awaiting_normal {
                let flags_restored = info.maximized.unwrap_or(false) == self.normal.maximized
                    && info.fullscreen.unwrap_or(false) == self.normal.fullscreen;
                if !flags_restored {
                    return;
                }
                if !size.is_some_and(|size| near(size, self.normal.size)) {
                    return;
                }
                self.awaiting_normal = false;
            }
            if let Some(size) = size {
                self.normal.size = clamp_size(size, NORMAL_MIN, NORMAL_SIZE);
            }
            if position.is_some() {
                self.normal.position = position;
            }
            self.normal.maximized = info.maximized.unwrap_or(self.normal.maximized);
            self.normal.fullscreen = info.fullscreen.unwrap_or(self.normal.fullscreen);
        }
    }

    /// Call each frame before enter/leave, passing Context::zoom_factor().
    pub fn observe_with_zoom(&mut self, info: &ViewportInfo, zoom: f32) {
        self.zoom = scale(Some(zoom), self.zoom);
        if self.mini {
            self.mini_size = clamp_size(self.mini_size, MINI_MIN * self.zoom, MINI_SIZE);
        }
        self.observe(info);
    }

    /// Enter only by an explicit user action. Repeated calls cannot overwrite
    /// the normal snapshot. Decorations remain native and topmost defaults off.
    pub fn enter(&mut self, info: &ViewportInfo) -> Vec<ViewportCommand> {
        if self.mini {
            return Vec::new();
        }
        self.observe(info);
        let snapshot = self.normal;
        self.snapshot = Some(snapshot);
        self.mini = true;
        self.mini_size = clamp_size(self.mini_size, MINI_MIN * self.zoom, MINI_SIZE);
        self.awaiting_mini = true;
        self.awaiting_normal = false;
        let position = self.mini_position.or(snapshot.position);
        self.geometry_commands(self.mini_size, MINI_MIN, position, self.always_on_top)
    }

    pub fn leave(&mut self, info: &ViewportInfo) -> Vec<ViewportCommand> {
        if !self.mini {
            return Vec::new();
        }
        self.observe(info);
        let snapshot = self.snapshot.take().unwrap_or(self.normal);
        self.normal = snapshot;
        self.mini = false;
        self.awaiting_mini = false;
        self.awaiting_normal = true;
        self.always_on_top = false;
        let mut commands =
            self.geometry_commands(snapshot.size, NORMAL_MIN, snapshot.position, false);
        if snapshot.maximized {
            commands.push(ViewportCommand::Maximized(true));
        }
        if snapshot.fullscreen {
            commands.push(ViewportCommand::Fullscreen(true));
        }
        commands
    }

    /// A normal window ignores this setter, including its remembered preference.
    /// Leaving mini always restores normal level and resets the mini toggle off.
    pub fn set_always_on_top(&mut self, enabled: bool) -> Vec<ViewportCommand> {
        if !self.mini || self.always_on_top == enabled {
            return Vec::new();
        }
        self.always_on_top = enabled;
        vec![ViewportCommand::WindowLevel(if enabled {
            WindowLevel::AlwaysOnTop
        } else {
            WindowLevel::Normal
        })]
    }

    fn geometry_commands(
        &self,
        size: Vec2,
        min: Vec2,
        position: Option<Pos2>,
        top: bool,
    ) -> Vec<ViewportCommand> {
        // Mini controls require at least 400x260 readable egui points even when
        // zoomed. Normal minimum retains the existing native-DIP contract.
        let minimum_ui = if self.mini { MINI_MIN } else { min / self.zoom };
        let mut commands = vec![
            ViewportCommand::Fullscreen(false),
            ViewportCommand::Maximized(false),
            ViewportCommand::MinInnerSize(minimum_ui),
            ViewportCommand::Decorations(true),
            ViewportCommand::WindowLevel(if top {
                WindowLevel::AlwaysOnTop
            } else {
                WindowLevel::Normal
            }),
            ViewportCommand::InnerSize(clamp_size(size, minimum_ui * self.zoom, size) / self.zoom),
        ];
        if let Some(position) = position {
            let ppp = self.native_scale * self.zoom;
            commands.push(ViewportCommand::OuterPosition(Pos2::new(
                position.x / ppp,
                position.y / ppp,
            )));
        }
        commands
    }
}

fn scale(value: Option<f32>, fallback: f32) -> f32 {
    value
        .filter(|v| v.is_finite() && *v > 0.0)
        .unwrap_or(fallback)
        .clamp(0.25, 8.0)
}
fn valid_size(size: Vec2) -> Option<Vec2> {
    (size.is_finite() && size.x > 0.0 && size.y > 0.0).then_some(size)
}
fn clamp_size(size: Vec2, min: Vec2, fallback: Vec2) -> Vec2 {
    valid_size(size)
        .unwrap_or(fallback)
        .clamp(min, Vec2::splat(MAX_SIZE))
}
fn valid_position(p: Pos2) -> Option<Pos2> {
    (p.is_finite() && p.x.abs() <= 1_000_000.0 && p.y.abs() <= 1_000_000.0).then_some(p)
}
fn near(a: Vec2, b: Vec2) -> bool {
    (a - b).abs().max_elem() < 1.5
}

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::egui::Rect;
    fn info(
        position: Pos2,
        size: Vec2,
        maximized: bool,
        fullscreen: bool,
        ppp: f32,
    ) -> ViewportInfo {
        ViewportInfo {
            inner_rect: Some(Rect::from_min_size(position, size)),
            outer_rect: Some(Rect::from_min_size(position, size + Vec2::new(0.0, 30.0))),
            maximized: Some(maximized),
            fullscreen: Some(fullscreen),
            native_pixels_per_point: Some(ppp),
            ..Default::default()
        }
    }
    #[test]
    fn normal_roundtrip_and_repeated_calls_keep_original_snapshot() {
        let mut state = MiniWindowState::default();
        let normal = info(
            Pos2::new(-100.0, 50.0),
            Vec2::new(1200.0, 820.0),
            false,
            false,
            1.0,
        );
        let enter = state.enter(&normal);
        assert_eq!(enter[0], ViewportCommand::Fullscreen(false));
        assert_eq!(enter[1], ViewportCommand::Maximized(false));
        assert!(enter.contains(&ViewportCommand::InnerSize(MINI_SIZE)));
        assert!(state.enter(&ViewportInfo::default()).is_empty());
        let mini = info(
            Pos2::new(10.0, 20.0),
            Vec2::new(600.0, 400.0),
            false,
            false,
            1.0,
        );
        state.observe(&mini);
        let leave = state.leave(&mini);
        assert!(leave.contains(&ViewportCommand::InnerSize(Vec2::new(1200.0, 820.0))));
        assert!(leave.contains(&ViewportCommand::OuterPosition(Pos2::new(-100.0, 50.0))));
        assert!(state.leave(&ViewportInfo::default()).is_empty());
        assert!(!state.is_mini());
        assert!(
            state
                .enter(&normal)
                .contains(&ViewportCommand::InnerSize(Vec2::new(600.0, 400.0)))
        );
    }
    #[test]
    fn maximized_and_fullscreen_are_cleared_before_geometry_and_restored_last() {
        for (max, full) in [(true, false), (false, true), (true, true)] {
            let mut state = MiniWindowState::default();
            let normal = info(Pos2::ZERO, Vec2::new(1920.0, 1080.0), max, full, 1.0);
            state.enter(&normal);
            let commands = state.leave(&info(Pos2::ZERO, MINI_SIZE, false, false, 1.0));
            assert_eq!(commands[0], ViewportCommand::Fullscreen(false));
            assert_eq!(commands[1], ViewportCommand::Maximized(false));
            let size = commands
                .iter()
                .position(|c| matches!(c, ViewportCommand::InnerSize(_)))
                .unwrap();
            if max {
                assert!(
                    commands
                        .iter()
                        .position(|c| *c == ViewportCommand::Maximized(true))
                        .unwrap()
                        > size
                );
            }
            if full {
                assert_eq!(commands.last(), Some(&ViewportCommand::Fullscreen(true)));
            }
        }
    }
    #[test]
    fn invalid_geometry_is_finite_clamped_and_topmost_is_explicit_mini_only() {
        let mut state = MiniWindowState::default();
        assert!(state.set_always_on_top(true).is_empty());
        let bad = ViewportInfo {
            inner_rect: Some(Rect::from_min_size(
                Pos2::ZERO,
                Vec2::new(f32::NAN, f32::INFINITY),
            )),
            outer_rect: Some(Rect::from_min_size(Pos2::new(f32::NAN, 0.0), NORMAL_SIZE)),
            native_pixels_per_point: Some(f32::NAN),
            ..Default::default()
        };
        let commands = state.enter(&bad);
        assert!(commands.contains(&ViewportCommand::InnerSize(MINI_SIZE)));
        assert!(!state.always_on_top());
        assert_eq!(
            state.set_always_on_top(true),
            vec![ViewportCommand::WindowLevel(WindowLevel::AlwaysOnTop)]
        );
        assert!(state.set_always_on_top(true).is_empty());
        state.observe(&info(Pos2::ZERO, Vec2::new(1.0, 1.0), false, false, 1.0));
        assert_eq!(state.mini_size(), MINI_MIN);
        let leave = state.leave(&bad);
        assert!(leave.contains(&ViewportCommand::WindowLevel(WindowLevel::Normal)));
        assert!(!state.always_on_top());
        assert!(leave.contains(&ViewportCommand::InnerSize(NORMAL_SIZE)));
    }
    #[test]
    fn dpi_and_zoom_restore_physical_position_and_native_logical_size() {
        let mut state = MiniWindowState::default();
        let normal = info(
            Pos2::new(-100.0, 50.0),
            Vec2::new(1180.0, 800.0),
            false,
            false,
            1.5,
        );
        state.observe_with_zoom(&normal, 1.0);
        state.enter(&normal);
        let mini = info(
            Pos2::new(20.0, 30.0),
            Vec2::new(260.0, 180.0),
            false,
            false,
            2.0,
        );
        state.observe_with_zoom(&mini, 2.0);
        let leave = state.leave(&mini);
        assert!(leave.contains(&ViewportCommand::InnerSize(Vec2::new(590.0, 400.0))));
        assert!(leave.contains(&ViewportCommand::MinInnerSize(Vec2::new(430.0, 310.0))));
        assert!(leave.contains(&ViewportCommand::OuterPosition(Pos2::new(-37.5, 18.75))));
    }
    #[test]
    fn stale_transition_observations_do_not_replace_saved_sizes() {
        let mut state = MiniWindowState::default();
        let normal = info(Pos2::ZERO, NORMAL_SIZE, false, false, 1.0);
        state.enter(&normal);
        state.observe(&normal);
        assert_eq!(state.mini_size(), MINI_SIZE);
        let mini = info(Pos2::ZERO, MINI_SIZE, false, false, 1.0);
        state.observe(&mini);
        state.leave(&mini);
        state.observe(&mini);
        let commands = state.enter(&normal);
        assert!(commands.contains(&ViewportCommand::InnerSize(MINI_SIZE)));
    }
    #[test]
    fn restoration_waits_for_final_size_instead_of_accepting_an_intermediate_resize() {
        let mut state = MiniWindowState::default();
        let normal = info(Pos2::ZERO, NORMAL_SIZE, false, false, 1.0);
        let mini = info(Pos2::ZERO, MINI_SIZE, false, false, 1.0);
        state.enter(&normal);
        state.observe(&mini);
        state.leave(&mini);
        assert!(!state.normal_geometry_restored());
        state.observe(&info(
            Pos2::ZERO,
            Vec2::new(900.0, 650.0),
            false,
            false,
            1.0,
        ));
        assert!(!state.normal_geometry_restored());
        state.observe(&normal);
        assert!(state.normal_geometry_restored());
    }
    #[test]
    fn mini_minimum_stays_readable_in_ui_points_at_every_zoom() {
        for zoom in [1.0, 1.25, 2.0] {
            let mut state = MiniWindowState::default();
            let normal = info(Pos2::ZERO, NORMAL_SIZE / zoom, false, false, 1.5);
            state.observe_with_zoom(&normal, zoom);
            let commands = state.enter(&normal);
            let minimum = commands
                .iter()
                .find_map(|command| match command {
                    ViewportCommand::MinInnerSize(size) => Some(*size),
                    _ => None,
                })
                .unwrap();
            let size = commands
                .iter()
                .find_map(|command| match command {
                    ViewportCommand::InnerSize(size) => Some(*size),
                    _ => None,
                })
                .unwrap();
            assert!(minimum.x >= 400.0 && minimum.y >= 260.0);
            assert!(size.x >= minimum.x && size.y >= minimum.y);
            state.observe_with_zoom(
                &info(Pos2::ZERO, Vec2::new(100.0, 80.0), false, false, 1.5),
                zoom,
            );
            let remembered = state.mini_size();
            assert!(remembered.x >= 400.0 && remembered.y >= 260.0);
        }
    }
}
