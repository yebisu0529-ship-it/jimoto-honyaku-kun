use anyhow::{Context, Result};
use serde::Deserialize;
use std::{
    io::Read,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc::SyncSender,
    },
    time::{Duration, Instant},
};
use tiny_http::{Header, Method, Request, Response, Server, StatusCode};

#[derive(Clone, Debug, Deserialize)]
pub struct Caption {
    pub text: String,
    pub video_id: String,
    pub position: f64,
    #[serde(default)]
    pub paused: bool,
    #[serde(default)]
    pub epoch: u64,
}

/// What the bridge last heard from a browser extension.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Contact {
    /// A request with this app's key that carried no caption of a playing
    /// video: the extension's connection check, or a cleared or paused caption.
    Checked,
    /// A caption with text from a playing video, sent with this app's key.
    Caption,
    /// A request that carried a key, but not this app's.
    WrongKey,
}

/// What the bridge has heard from a browser extension since the app started.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Heard {
    /// The latest contact and when it came.
    pub last: Option<(Contact, Instant)>,
    /// When the latest caption with text came. The extension clears the
    /// caption between two lines, so the latest contact alone would read
    /// every pause in the talk as the captions stopping.
    pub caption: Option<Instant>,
}

/// What the bridge has heard. The bridge thread writes it and the screens
/// read it.
#[derive(Default)]
pub struct LastContact(Mutex<Heard>);

impl LastContact {
    pub fn note(&self, contact: Contact) {
        let now = Instant::now();
        let mut heard = self.0.lock().unwrap_or_else(|e| e.into_inner());
        heard.last = Some((contact, now));
        if contact == Contact::Caption {
            heard.caption = Some(now);
        }
    }

    pub fn get(&self) -> Heard {
        *self.0.lock().unwrap_or_else(|e| e.into_inner())
    }
}

fn token_matches(provided: &str, expected: &str) -> bool {
    if provided.len() != expected.len() {
        return false;
    }
    provided
        .bytes()
        .zip(expected.bytes())
        .fold(0_u8, |a, (x, y)| a | (x ^ y))
        == 0
}

pub fn run(
    token: String,
    stop: Arc<AtomicBool>,
    tx: SyncSender<Caption>,
    contact: Arc<LastContact>,
) -> Result<()> {
    let server = Server::http("127.0.0.1:17862")
        .map_err(|e| anyhow::anyhow!("字幕接続ポート17862を開けません: {e}"))?;
    while !stop.load(Ordering::Relaxed) {
        let Some(mut request) = server.recv_timeout(Duration::from_millis(200))? else {
            continue;
        };
        let origin = request
            .headers()
            .iter()
            .find(|h| h.field.equiv("Origin"))
            .map(|h| h.value.as_str().to_owned());
        let allowed_origin = origin.as_deref().is_none_or(|o| {
            o.starts_with("chrome-extension://") || o.starts_with("moz-extension://")
        });
        let mut cors = Vec::new();
        if allowed_origin && let Some(ref origin) = origin {
            cors.push(Header::from_bytes("Access-Control-Allow-Origin", origin.as_str()).unwrap());
            cors.push(Header::from_bytes("Vary", "Origin").unwrap());
        }
        if !allowed_origin {
            let _ = request.respond(Response::empty(StatusCode(403)));
            continue;
        }
        if request.method() == &Method::Options {
            let mut response = Response::empty(StatusCode(204));
            for h in cors {
                response.add_header(h);
            }
            response.add_header(
                Header::from_bytes("Access-Control-Allow-Methods", "POST, GET, OPTIONS").unwrap(),
            );
            response.add_header(
                Header::from_bytes(
                    "Access-Control-Allow-Headers",
                    "Content-Type, X-Plamo-Token",
                )
                .unwrap(),
            );
            let _ = request.respond(response);
            continue;
        }
        let (status, body) = answer(&mut request, &token, &tx, &contact);
        let mut response = Response::from_string(body).with_status_code(StatusCode(status));
        response.add_header(
            Header::from_bytes("Content-Type", "application/json; charset=utf-8").unwrap(),
        );
        response.add_header(Header::from_bytes("Cache-Control", "no-store").unwrap());
        for h in cors {
            response.add_header(h);
        }
        let _ = request.respond(response);
    }
    Ok(())
}

/// The status and body for a request from an allowed origin, noting in
/// `contact` what it says about the extension.
fn answer(
    request: &mut Request,
    token: &str,
    tx: &SyncSender<Caption>,
    contact: &LastContact,
) -> (u16, String) {
    let authorized = match request
        .headers()
        .iter()
        .find(|h| h.field.equiv("X-Plamo-Token"))
    {
        None => false,
        Some(h) if token_matches(h.value.as_str(), token) => true,
        Some(_) => {
            contact.note(Contact::WrongKey);
            false
        }
    };
    if !authorized {
        (
            401,
            "{\"error\":\"アプリの接続キーを拡張機能に設定してください\"}".to_owned(),
        )
    } else if request.method() == &Method::Get && request.url() == "/health" {
        contact.note(Contact::Checked);
        (200, "{\"ok\":true,\"app\":\"plamo-translate\"}".to_owned())
    } else if request.method() == &Method::Post && request.url() == "/caption" {
        if request.body_length().is_some_and(|n| n > 32_768) {
            (413, "{\"error\":\"too_large\"}".to_owned())
        } else {
            let mut body = String::new();
            let parsed = request
                .as_reader()
                .take(32_769)
                .read_to_string(&mut body)
                .context("字幕の読み込み")
                .and_then(|_| {
                    anyhow::ensure!(body.len() <= 32_768, "字幕が長すぎます");
                    let caption: Caption = serde_json::from_str(&body)?;
                    anyhow::ensure!(
                        caption.text.chars().count() <= 4_000
                            && caption.video_id.len() <= 128
                            && caption.position.is_finite()
                            && caption.position >= 0.0,
                        "字幕の形式が不正です"
                    );
                    Ok(caption)
                });
            match parsed {
                Ok(caption) => {
                    contact.note(if caption.paused || caption.text.trim().is_empty() {
                        Contact::Checked
                    } else {
                        Contact::Caption
                    });
                    match tx.try_send(caption) {
                        Ok(()) => (202, "{\"ok\":true}".to_owned()),
                        Err(_) => (429, "{\"error\":\"busy\"}".to_owned()),
                    }
                }
                Err(_) => (400, "{\"error\":\"invalid_caption\"}".to_owned()),
            }
        }
    } else {
        (404, "{\"error\":\"not_found\"}".to_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;
    use tiny_http::TestRequest;

    #[test]
    fn token_must_match_entire_value() {
        assert!(token_matches("abcd1234", "abcd1234"));
        assert!(!token_matches("abcd1235", "abcd1234"));
        assert!(!token_matches("abcd1234x", "abcd1234"));
    }

    const KEY: &str = "5d3c9a8e-1f2b-4c6d-8e7f-0a1b2c3d4e5f";
    const CAPTION: &str = r#"{"text":"Hello there","video_id":"abc","position":12.5}"#;

    /// A request as the extension sends it, with `key` in its header.
    fn request(method: Method, path: &str, key: Option<&str>, body: &'static str) -> Request {
        let mut request = TestRequest::new()
            .with_method(method)
            .with_path(path)
            .with_body(body);
        if let Some(key) = key {
            request = request.with_header(Header::from_bytes("X-Plamo-Token", key).unwrap());
        }
        request.into()
    }

    /// The status the bridge answers, and what it then last heard.
    fn ask(mut request: Request, tx: &SyncSender<Caption>) -> (u16, Option<Contact>) {
        let contact = LastContact::default();
        let (status, _) = answer(&mut request, KEY, tx, &contact);
        (status, contact.get().last.map(|(heard, _)| heard))
    }

    #[test]
    fn a_wrong_key_is_noted_on_any_path_and_still_refused() {
        let (tx, rx) = mpsc::sync_channel(8);
        for (method, path, body) in [
            (Method::Get, "/health", ""),
            (Method::Post, "/caption", CAPTION),
            (Method::Get, "/elsewhere", ""),
        ] {
            assert_eq!(
                ask(request(method, path, Some("not-this-app"), body), &tx),
                (401, Some(Contact::WrongKey)),
                "{path}"
            );
        }
        assert!(rx.try_recv().is_err(), "no caption gets through");
    }

    #[test]
    fn a_request_without_a_key_is_refused_and_not_noted() {
        let (tx, rx) = mpsc::sync_channel(8);
        assert_eq!(
            ask(request(Method::Get, "/health", None, ""), &tx),
            (401, None)
        );
        assert_eq!(
            ask(request(Method::Post, "/caption", None, CAPTION), &tx),
            (401, None)
        );
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn the_connection_check_with_the_right_key_is_answered_and_noted() {
        let (tx, _rx) = mpsc::sync_channel(8);
        assert_eq!(
            ask(request(Method::Get, "/health", Some(KEY), ""), &tx),
            (200, Some(Contact::Checked))
        );
    }

    #[test]
    fn a_caption_with_text_reaches_the_app_and_is_noted() {
        let (tx, rx) = mpsc::sync_channel(8);
        assert_eq!(
            ask(request(Method::Post, "/caption", Some(KEY), CAPTION), &tx),
            (202, Some(Contact::Caption))
        );
        assert_eq!(
            rx.try_recv().expect("the caption arrives").text,
            "Hello there"
        );
    }

    #[test]
    fn a_caption_of_spaces_still_reaches_the_app_but_counts_only_as_a_check() {
        let (tx, rx) = mpsc::sync_channel(8);
        let cleared = r#"{"text":"   ","video_id":"abc","position":3.0,"paused":true}"#;
        assert_eq!(
            ask(request(Method::Post, "/caption", Some(KEY), cleared), &tx),
            (202, Some(Contact::Checked))
        );
        assert!(rx.try_recv().expect("the clearing arrives").paused);
    }

    #[test]
    fn a_caption_of_a_paused_video_still_reaches_the_app_but_counts_only_as_a_check() {
        let (tx, rx) = mpsc::sync_channel(8);
        let paused = r#"{"text":"Hello there","video_id":"abc","position":3.0,"paused":true}"#;
        assert_eq!(
            ask(request(Method::Post, "/caption", Some(KEY), paused), &tx),
            (202, Some(Contact::Checked))
        );
        assert_eq!(
            rx.try_recv().expect("the caption arrives").text,
            "Hello there"
        );
    }

    #[test]
    fn the_time_of_the_last_caption_outlasts_the_checks_after_it() {
        let contact = LastContact::default();
        contact.note(Contact::Caption);
        let caption = contact.get().caption.expect("a caption was heard");
        contact.note(Contact::Checked);
        let heard = contact.get();
        assert_eq!(heard.last.map(|(last, _)| last), Some(Contact::Checked));
        assert_eq!(heard.caption, Some(caption));
        contact.note(Contact::WrongKey);
        assert_eq!(contact.get().caption, Some(caption));
    }

    #[test]
    fn a_caption_the_busy_app_turns_away_still_counts_as_reaching_it() {
        let (tx, _rx) = mpsc::sync_channel(0);
        assert_eq!(
            ask(request(Method::Post, "/caption", Some(KEY), CAPTION), &tx),
            (429, Some(Contact::Caption))
        );
    }

    #[test]
    fn other_requests_with_the_right_key_are_not_noted() {
        let (tx, _rx) = mpsc::sync_channel(8);
        assert_eq!(
            ask(request(Method::Get, "/elsewhere", Some(KEY), ""), &tx),
            (404, None)
        );
        assert_eq!(
            ask(
                request(Method::Post, "/caption", Some(KEY), "not json"),
                &tx
            ),
            (400, None)
        );
    }
}
