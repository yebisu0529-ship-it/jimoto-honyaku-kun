//! The 画面 live input. One chosen screen area is read again and again, and
//! only text that changed enters live translation, as captions under a scope
//! of its own. Captured images stay in memory and are never written to disk.
use super::*;

/// Prefix of the caption scope used by screen text. A colon never appears in
/// a YouTube video id, so the two caption sources cannot be confused.
pub(super) const SCREEN_SOURCE: &str = "screen:";

pub(super) struct ScreenFeed {
    screen_id: u32,
    area: (u32, u32, u32, u32),
    pub(super) epoch: u64,
    started: Instant,
    next_at: Instant,
    busy: bool,
    fingerprint: Option<Vec<u8>>,
    last_text: Option<String>,
}

pub(super) enum ScreenFrame {
    Unchanged,
    Text { text: String, fingerprint: Vec<u8> },
}

pub(super) enum ScreenError {
    /// The area can no longer be captured, for example its display is gone.
    Capture(String),
    Ocr(String),
}

/// A small grey thumbnail of the area. Video noise and a blinking cursor stay
/// under the tolerance in `same_screen`, so a still area is not read again.
fn fingerprint(image: &image::RgbaImage) -> Vec<u8> {
    image::imageops::thumbnail(image, 32, 32)
        .pixels()
        .map(|p| {
            ((u32::from(p[0]) * 299 + u32::from(p[1]) * 587 + u32::from(p[2]) * 114) / 1000) as u8
        })
        .collect()
}

fn same_screen(old: &[u8], new: &[u8]) -> bool {
    old.len() == new.len()
        && old
            .iter()
            .zip(new)
            .map(|(a, b)| u64::from(a.abs_diff(*b)))
            .sum::<u64>()
            <= 2 * old.len() as u64
}

fn normalized(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

impl App {
    /// Asks for the area to read; the feed starts once one is chosen.
    pub(super) fn begin_screen_feed(&mut self, ctx: &egui::Context) {
        if self.capture_busy || self.selection.is_some() {
            return;
        }
        self.screen_area_requested = true;
        self.capture(ctx);
        if !self.capture_busy {
            self.screen_area_requested = false;
        }
    }

    pub(super) fn start_screen_feed(&mut self, screen_id: u32, area: (u32, u32, u32, u32)) {
        if !self.screen_ocr_ready() {
            self.request_screen_ocr_setup();
            return;
        }
        self.set_caption_enabled(false);
        self.audio_start_requested = false;
        if self.audio_enabled {
            self.stop_audio();
        }
        self.set_caption_enabled(true);
        self.screen_epoch = self.screen_epoch.wrapping_add(1);
        let now = Instant::now();
        self.screen_feed = Some(ScreenFeed {
            screen_id,
            area,
            epoch: self.screen_epoch,
            started: now,
            next_at: now,
            busy: false,
            fingerprint: None,
            last_text: None,
        });
        self.status = "選んだ範囲の文字を読み取っています。".into();
    }

    pub(super) fn tick_screen_feed(&mut self, ctx: &egui::Context) {
        let Some(feed) = self.screen_feed.as_ref() else {
            return;
        };
        if feed.busy {
            return;
        }
        let now = Instant::now();
        if now < feed.next_at {
            ctx.request_repaint_after(feed.next_at - now);
            return;
        }
        let (screen_id, area, epoch, previous) = (
            feed.screen_id,
            feed.area,
            feed.epoch,
            feed.fingerprint.clone(),
        );
        let recognizer = self.screen_recognizer();
        if let Some(feed) = self.screen_feed.as_mut() {
            feed.busy = true;
        }
        let tx = self.tx.clone();
        let ctx = ctx.clone();
        thread::spawn(move || {
            let result = platform::capture_area(screen_id, area)
                .map_err(|e| ScreenError::Capture(format!("{e:#}")))
                .and_then(|image| {
                    let print = fingerprint(&image);
                    if previous
                        .as_deref()
                        .is_some_and(|old| same_screen(old, &print))
                    {
                        return Ok(ScreenFrame::Unchanged);
                    }
                    recognizer
                        .recognize(&image)
                        .map(|text| ScreenFrame::Text {
                            text,
                            fingerprint: print,
                        })
                        .map_err(|e| ScreenError::Ocr(format!("{e:#}")))
                });
            let _ = tx.send(Event::ScreenFrame { epoch, result });
            ctx.request_repaint();
        });
    }

    pub(super) fn accept_screen_frame(
        &mut self,
        epoch: u64,
        result: std::result::Result<ScreenFrame, ScreenError>,
    ) {
        let interval = Duration::from_millis(self.settings.screen_interval_ms.clamp(500, 10_000));
        let Some(feed) = self.screen_feed.as_mut().filter(|feed| feed.epoch == epoch) else {
            return;
        };
        feed.busy = false;
        feed.next_at = Instant::now() + interval;
        let (text, print) = match result {
            Ok(ScreenFrame::Unchanged) => return,
            Ok(ScreenFrame::Text { text, fingerprint }) => (normalized(&text), fingerprint),
            Err(ScreenError::Ocr(error)) => {
                self.discard_screen_ocr_runtime();
                self.status = format!("画面の文字を読み取れませんでした: {error}");
                return;
            }
            Err(ScreenError::Capture(error)) => {
                self.set_caption_enabled(false);
                self.status = format!("画面を取得できないため、読み取りを止めました: {error}");
                return;
            }
        };
        feed.fingerprint = Some(print);
        if feed.last_text.as_deref() == Some(text.as_str()) {
            return;
        }
        feed.last_text = Some(text.clone());
        let caption = bridge::Caption {
            text,
            video_id: format!("{SCREEN_SOURCE}{}", feed.epoch),
            position: feed.started.elapsed().as_secs_f64(),
            paused: false,
            epoch: 0,
        };
        self.accept_caption(caption);
        self.document_take_caption();
    }

    /// Our windows stay visible to the user but leave every capture while the
    /// feed runs, so it never reads its own translations.
    pub(super) fn sync_capture_protection(&mut self, ctx: &egui::Context) {
        let protect = self.screen_feed.is_some();
        self.subtitle_window.content_protected = protect;
        if self.capture_protected != protect {
            ctx.send_viewport_cmd(egui::ViewportCommand::ContentProtected(protect));
            self.capture_protected = protect;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(app: &mut App, text: &str) {
        let epoch = app.screen_feed.as_ref().unwrap().epoch;
        app.accept_screen_frame(
            epoch,
            Ok(ScreenFrame::Text {
                text: text.into(),
                fingerprint: vec![0; 4],
            }),
        );
    }

    #[test]
    fn a_still_area_is_not_read_again_but_new_text_is() {
        let still = image::RgbaImage::from_pixel(64, 32, image::Rgba([20, 20, 20, 255]));
        let mut noise = still.clone();
        noise.put_pixel(3, 3, image::Rgba([60, 60, 60, 255]));
        let mut text = still.clone();
        for x in 8..56 {
            for y in 12..20 {
                text.put_pixel(x, y, image::Rgba([240, 240, 240, 255]));
            }
        }
        assert!(
            same_screen(&fingerprint(&still), &fingerprint(&noise)),
            "one changed pixel is noise"
        );
        assert!(
            !same_screen(&fingerprint(&still), &fingerprint(&text)),
            "a new line of text is a change"
        );
        assert_eq!(
            normalized(" The museum\n opens  at nine. "),
            "The museum opens at nine."
        );
    }

    #[test]
    fn changed_screen_text_becomes_live_text_and_repeats_are_dropped() {
        let mut app = App::inert();
        app.start_screen_feed(1, (0, 0, 120, 40));
        assert!(
            app.screen_feed.is_some(),
            "Windows OCR needs no preparation"
        );
        frame(
            &mut app,
            "The museum opens at nine.\nTickets are free today.",
        );
        assert!(app.video_id.starts_with(SCREEN_SOURCE));
        assert_eq!(
            app.last_live_text,
            "The museum opens at nine. Tickets are free today."
        );
        let seen = app.last_caption;
        frame(
            &mut app,
            "The museum  opens at nine. Tickets are free today.",
        );
        assert_eq!(
            app.last_caption, seen,
            "the same words are dropped before they reach the captions"
        );
        frame(&mut app, "Please keep your ticket.");
        assert_eq!(app.last_live_text, "Please keep your ticket.");
        app.set_caption_enabled(false);
        assert!(
            app.screen_feed.is_none(),
            "stopping captions stops the feed"
        );
    }

    #[test]
    fn own_windows_leave_captures_only_while_the_feed_runs() {
        let ctx = egui::Context::default();
        let mut app = App::inert();
        let protection = |app: &mut App| -> Vec<bool> {
            let output = ctx.run(egui::RawInput::default(), |ctx| {
                app.sync_capture_protection(ctx)
            });
            output
                .viewport_output
                .values()
                .flat_map(|viewport| viewport.commands.iter())
                .filter_map(|command| match command {
                    egui::ViewportCommand::ContentProtected(on) => Some(*on),
                    _ => None,
                })
                .collect()
        };
        assert!(
            protection(&mut app).is_empty(),
            "nothing changes while idle"
        );
        app.start_screen_feed(1, (0, 0, 120, 40));
        assert_eq!(protection(&mut app), vec![true]);
        assert!(app.subtitle_window.content_protected);
        assert!(protection(&mut app).is_empty(), "sent once");
        app.set_caption_enabled(false);
        assert_eq!(protection(&mut app), vec![false]);
    }
}
