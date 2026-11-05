use anyhow::{Context, Result};
use serde::Deserialize;
use std::{
    io::Read,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::SyncSender,
    },
    time::Duration,
};
use tiny_http::{Header, Method, Response, Server, StatusCode};

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

pub fn run(token: String, stop: Arc<AtomicBool>, tx: SyncSender<Caption>) -> Result<()> {
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
        let authorized = request
            .headers()
            .iter()
            .find(|h| h.field.equiv("X-Plamo-Token"))
            .is_some_and(|h| token_matches(h.value.as_str(), &token));
        let (status, body) = if !authorized {
            (
                401,
                "{\"error\":\"アプリの接続キーを拡張機能に設定してください\"}".to_owned(),
            )
        } else if request.method() == &Method::Get && request.url() == "/health" {
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
                    Ok(caption) => match tx.try_send(caption) {
                        Ok(()) => (202, "{\"ok\":true}".to_owned()),
                        Err(_) => (429, "{\"error\":\"busy\"}".to_owned()),
                    },
                    Err(_) => (400, "{\"error\":\"invalid_caption\"}".to_owned()),
                }
            }
        } else {
            (404, "{\"error\":\"not_found\"}".to_owned())
        };
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

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn token_must_match_entire_value() {
        assert!(token_matches("abcd1234", "abcd1234"));
        assert!(!token_matches("abcd1235", "abcd1234"));
        assert!(!token_matches("abcd1234x", "abcd1234"));
    }
}
