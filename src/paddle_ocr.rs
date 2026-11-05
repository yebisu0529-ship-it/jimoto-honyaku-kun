//! Image-only local requests to an owned, authenticated native PaddleOCR-VL server.
use crate::{
    ocr_runtime::{ProcessControl, ensure_port_unused},
    paddle_setup::{self, PaddleInstallation},
};
use anyhow::{Context, Result, ensure};
use image::{ExtendedColorType, ImageEncoder, RgbaImage, codecs::png::PngEncoder};
use reqwest::blocking::Client;
use serde_json::{Value, json};
use std::{
    io::{Read, Write},
    path::PathBuf,
    process::{Command, Stdio},
    sync::Mutex,
    thread,
    time::{Duration, Instant},
};

const PORT: u16 = 17865;
const ENDPOINT: &str = "http://127.0.0.1:17865";
const MODEL: &str = "jimoto-paddle-ocr-vl1.6";
const MAX_PIXELS: u64 = 16_000_000;
const MAX_PNG: usize = 8 * 1024 * 1024;
const MAX_RESPONSE: usize = 2 * 1024 * 1024;

pub struct PaddleEngine {
    control: ProcessControl,
    client: Client,
    key: String,
    request: Mutex<()>,
}
impl PaddleEngine {
    /// Blocking ready check. The caller has already verified the selected
    /// llama.cpp runtime and must invoke this on its background OCR worker.
    pub fn start(
        installation: PaddleInstallation,
        llama_server: PathBuf,
        backend: String,
        control: ProcessControl,
    ) -> Result<Self> {
        let result = (|| {
            ensure!(!control.is_cancelled(), "PaddleOCRの起動を中止しました。");
            ensure!(
                matches!(backend.as_str(), "cpu" | "cuda" | "vulkan"),
                "PaddleOCRの実行方式を確認してください。"
            );
            ensure!(
                llama_server.is_absolute() && llama_server.is_file(),
                "PaddleOCR用の実行環境がありません。翻訳モデルの準備を確認してください。"
            );
            crate::installer::safe_file(&llama_server)?;
            paddle_setup::verify_for_start(&installation)?;
            ensure!(!control.is_cancelled(), "PaddleOCRの起動を中止しました。");
            ensure_port_unused(PORT)?;
            let key = uuid::Uuid::new_v4().to_string();
            let mut command = Command::new(&llama_server);
            command
                .arg("-m")
                .arg(&installation.model)
                .arg("--mmproj")
                .arg(&installation.mmproj)
                .args([
                    "--host",
                    "127.0.0.1",
                    "--port",
                    "17865",
                    "--ctx-size",
                    "8192",
                    "--parallel",
                    "1",
                    "--temp",
                    "0",
                    "--alias",
                    MODEL,
                    "--jinja",
                ])
                .env("LLAMA_API_KEY", &key)
                .env("HF_HUB_OFFLINE", "1")
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null());
            if let Some(parent) = llama_server.parent() {
                command.current_dir(parent);
            }
            if backend == "cpu" {
                command.args(["-ngl", "0", "--no-mmproj-offload"]);
            } else {
                command.args(["-ngl", "99"]);
            }
            control.spawn(&mut command)?;
            let client = Client::builder()
                .no_proxy()
                .redirect(reqwest::redirect::Policy::none())
                .connect_timeout(Duration::from_secs(2))
                .timeout(Duration::from_secs(120))
                .build()?;
            let deadline = Instant::now() + Duration::from_secs(180);
            loop {
                ensure!(!control.is_cancelled(), "PaddleOCRの起動を中止しました。");
                control.owned_pid().context(
                    "PaddleOCRの起動に失敗しました。モデルと実行環境を確認してください。",
                )?;
                if control.ensure_listener(PORT).is_ok()
                    && let Ok(response) = client
                        .get(format!("{ENDPOINT}/health"))
                        .bearer_auth(&key)
                        .timeout(Duration::from_secs(1))
                        .send()
                    && response.status().is_success()
                {
                    let health = read_json(response, 64 * 1024)?;
                    ensure!(
                        health["status"] == "ok",
                        "PaddleOCRの起動状態を確認できませんでした。"
                    );
                    control.ensure_listener(PORT)?;
                    let response = client
                        .get(format!("{ENDPOINT}/v1/models"))
                        .bearer_auth(&key)
                        .timeout(Duration::from_secs(2))
                        .send()?;
                    ensure!(
                        response.status().is_success(),
                        "PaddleOCRのモデル情報を取得できませんでした。"
                    );
                    let models = read_json(response, 128 * 1024)?;
                    ensure!(
                        valid_models(&models),
                        "起動したPaddleOCRモデルが一致しません。"
                    );
                    control.ensure_listener(PORT)?;
                    return Ok(Self {
                        control: control.clone(),
                        client,
                        key,
                        request: Mutex::new(()),
                    });
                }
                ensure!(
                    Instant::now() < deadline,
                    "PaddleOCRの読み込みが時間内に完了しませんでした。"
                );
                thread::sleep(Duration::from_millis(100));
            }
        })();
        if result.is_err() {
            control.stop();
        }
        result
    }
    pub fn recognize(&self, image: &RgbaImage) -> Result<String> {
        let _request = self.request.lock().unwrap_or_else(|e| e.into_inner());
        ensure!(!self.control.is_cancelled(), "PaddleOCRを中止しました。");
        let png = encode_png(image)?;
        self.control.ensure_listener(PORT)?;
        let body = json!({"model":MODEL,"messages":[{"role":"user","content":[
            {"type":"image_url","image_url":{"url":format!("data:image/png;base64,{}",base64(&png))}},
            {"type":"text","text":"OCR:"}]}],"temperature":0,"max_tokens":4096,"stream":false});
        let response = self
            .client
            .post(format!("{ENDPOINT}/v1/chat/completions"))
            .bearer_auth(&self.key)
            .json(&body)
            .send()
            .context("PaddleOCRへのローカル接続が切れました。")?;
        ensure!(
            response.status().is_success(),
            "PaddleOCRで文字を読み取れませんでした（HTTP {}）。",
            response.status().as_u16()
        );
        let value = read_json(response, MAX_RESPONSE)?;
        ensure!(!self.control.is_cancelled(), "PaddleOCRを中止しました。");
        self.control.ensure_listener(PORT)?;
        response_text(&value)
    }
}
impl Drop for PaddleEngine {
    fn drop(&mut self) {
        self.control.stop();
    }
}
fn valid_models(value: &Value) -> bool {
    value["data"]
        .as_array()
        .is_some_and(|data| data.len() == 1 && data[0]["id"] == MODEL)
}
fn response_text(value: &Value) -> Result<String> {
    ensure!(
        value["model"] == MODEL,
        "PaddleOCRの応答モデルが一致しません。"
    );
    let choices = value["choices"]
        .as_array()
        .context("PaddleOCRの応答形式を確認できませんでした。")?;
    ensure!(choices.len() == 1, "PaddleOCRの応答件数が不正です。");
    ensure!(
        choices[0]["finish_reason"] != "length",
        "認識結果が長さの上限に達しました。画像の範囲を小さくしてください。"
    );
    ensure!(
        choices[0]["finish_reason"] == "stop",
        "PaddleOCRの文字認識が完了しませんでした。"
    );
    let text = choices[0]["message"]["content"]
        .as_str()
        .context("PaddleOCRの応答に本文がありません。")?;
    ensure!(
        text.len() <= 512 * 1024,
        "PaddleOCRの認識結果が大きすぎます。"
    );
    Ok(text.trim().to_owned())
}
fn read_json(mut response: reqwest::blocking::Response, limit: usize) -> Result<Value> {
    ensure!(
        response.content_length().is_none_or(|n| n <= limit as u64),
        "PaddleOCRの応答が大きすぎます。"
    );
    let mut bytes = Vec::new();
    response
        .by_ref()
        .take(limit as u64 + 1)
        .read_to_end(&mut bytes)?;
    ensure!(bytes.len() <= limit, "PaddleOCRの応答が大きすぎます。");
    serde_json::from_slice(&bytes).context("PaddleOCRの応答を読み取れませんでした。")
}
struct PngBytes(Vec<u8>);
impl Write for PngBytes {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if self.0.len().saturating_add(bytes.len()) > MAX_PNG {
            return Err(std::io::Error::other("PNG exceeds limit"));
        }
        self.0.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
fn encode_png(image: &RgbaImage) -> Result<Vec<u8>> {
    ensure!(
        image.width() > 0
            && image.height() > 0
            && u64::from(image.width()) * u64::from(image.height()) <= MAX_PIXELS,
        "PaddleOCRには1600万画素以内の画像を指定してください。"
    );
    let mut bytes = PngBytes(Vec::new());
    PngEncoder::new(&mut bytes)
        .write_image(
            image.as_raw(),
            image.width(),
            image.height(),
            ExtendedColorType::Rgba8,
        )
        .context("PaddleOCRにはPNG形式で8MiB以内になる画像を指定してください。")?;
    Ok(bytes.0)
}
fn base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for part in bytes.chunks(3) {
        let a = part[0];
        let b = part.get(1).copied().unwrap_or(0);
        let c = part.get(2).copied().unwrap_or(0);
        out.push(ALPHABET[(a >> 2) as usize] as char);
        out.push(ALPHABET[(((a & 3) << 4) | (b >> 4)) as usize] as char);
        out.push(if part.len() > 1 {
            ALPHABET[(((b & 15) << 2) | (c >> 6)) as usize] as char
        } else {
            '='
        });
        out.push(if part.len() > 2 {
            ALPHABET[(c & 63) as usize] as char
        } else {
            '='
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicBool;
    #[test]
    fn image_encoding_is_bounded_and_base64_preserves_binary_png() {
        for (raw, encoded) in [
            ("", ""),
            ("f", "Zg=="),
            ("fo", "Zm8="),
            ("foo", "Zm9v"),
            ("foob", "Zm9vYg=="),
            ("fooba", "Zm9vYmE="),
            ("foobar", "Zm9vYmFy"),
        ] {
            assert_eq!(base64(raw.as_bytes()), encoded);
        }
        assert_eq!(base64(&[0, 255, 128]), "AP+A");
        assert!(encode_png(&RgbaImage::new(0, 0)).is_err());
        assert!(encode_png(&RgbaImage::new(16_001, 1000)).is_err());
        let image = RgbaImage::from_pixel(2, 2, image::Rgba([20, 40, 60, 255]));
        let png = encode_png(&image).unwrap();
        assert_eq!(image::load_from_memory(&png).unwrap().to_rgba8(), image);
        let mut limited = PngBytes(vec![0; MAX_PNG - 1]);
        assert!(limited.write_all(&[1, 2]).is_err());
        assert_eq!(limited.0.len(), MAX_PNG - 1);
    }
    #[test]
    fn completion_requires_our_single_model_and_rejects_truncation() {
        assert!(valid_models(&json!({"data":[{"id":MODEL}]})));
        assert!(!valid_models(
            &json!({"data":[{"id":MODEL},{"id":"other"}]})
        ));
        assert!(!valid_models(&json!({"data":[{"id":"other"}]})));
        let response = |model: &str, finish: &str| json!({"model":model,"choices":[{"finish_reason":finish,"message":{"content":" 日本語\nEnglish "}}]});
        assert_eq!(
            response_text(&response(MODEL, "stop")).unwrap(),
            "日本語\nEnglish"
        );
        assert!(
            response_text(&response(MODEL, "length"))
                .unwrap_err()
                .to_string()
                .contains("上限")
        );
        assert!(response_text(&response("other", "stop")).is_err());
        assert!(response_text(&response(MODEL, "tool_calls")).is_err());
        assert!(response_text(&json!({"model":MODEL,"choices":[]})).is_err());
    }
    #[test]
    fn cancelled_start_never_reads_models_or_launches_a_server() {
        let control = ProcessControl::new();
        control.stop();
        let installation = PaddleInstallation {
            version: 1,
            model: PathBuf::from("missing-model"),
            mmproj: PathBuf::from("missing-projector"),
            model_revision: "missing".into(),
        };
        let error = PaddleEngine::start(
            installation,
            PathBuf::from("missing-runtime"),
            "cuda".into(),
            control,
        )
        .err()
        .unwrap();
        assert!(error.to_string().contains("中止"));
    }

    /// Explicit safe local fixture test: native inference only, no screen/audio/GUI.
    #[test]
    #[ignore = "requires explicit installed native runtime and safe local fixture paths"]
    fn paddle_native_ocr_opt_in() -> Result<()> {
        ensure!(
            std::env::var("PADDLE_NATIVE_VERIFY").as_deref() == Ok("1"),
            "Set PADDLE_NATIVE_VERIFY=1 explicitly"
        );
        let data = PathBuf::from(
            std::env::var_os("PADDLE_VERIFY_DATA").context("dedicated data root required")?,
        );
        let fixtures = PathBuf::from(
            std::env::var_os("PADDLE_VERIFY_FIXTURES").context("safe fixtures root required")?,
        );
        let root = data.join("ocr/paddle");
        let names = [
            "PaddleOCR-VL-1.6-GGUF.gguf",
            "PaddleOCR-VL-1.6-GGUF-mmproj.gguf",
        ];
        let before: Vec<_> = names
            .iter()
            .map(|name| {
                std::fs::metadata(root.join(name)).map(|m| (m.len(), m.modified().unwrap()))
            })
            .collect::<std::io::Result<_>>()?;
        let start = Instant::now();
        let mut stages = Vec::new();
        let installation =
            paddle_setup::install(&root, &AtomicBool::new(false), |p| stages.push(p.stage))?;
        ensure!(
            paddle_setup::load(&root)?.as_ref() == Some(&installation),
            "Committed setup did not reload"
        );
        let after: Vec<_> = names
            .iter()
            .map(|name| {
                std::fs::metadata(root.join(name)).map(|m| (m.len(), m.modified().unwrap()))
            })
            .collect::<std::io::Result<_>>()?;
        ensure!(before == after, "Existing model files were changed");
        eprintln!(
            "PADDLE_SETUP {}",
            json!({"success":true,"milliseconds":start.elapsed().as_millis(),"existing_models_unchanged":true,"stages":stages})
        );
        let runtime = crate::installer::load_installation(&data.join("engine"))?
            .context("Verified translation runtime missing")?;
        let control = ProcessControl::new();
        let start = Instant::now();
        let engine = PaddleEngine::start(
            installation,
            runtime.llama_server,
            runtime.backend,
            control.clone(),
        )?;
        eprintln!("PADDLE_READY_MS {}", start.elapsed().as_millis());
        for (name, expected) in [
            (
                "ocr-card.png",
                vec![
                    "Local translation demo",
                    "Welcome to the local translation demo.",
                    "The dashboard shows the schedule.",
                    "Pause the video, then play it again.",
                    "Public test text only. No personal information.",
                ],
            ),
            (
                "ocr-ja.png",
                vec![
                    "地域図書館の翻訳テスト",
                    "地域図書館では、六週間のデジタル展示を準備しています。",
                    "受付で二台の端末を貸し出し、英語と日本語の説明を表示します。",
                    "利用時間は午前9時から午後5時までです。",
                    "古い歩道橋、週末の市場、川沿いの庭を紹介します。",
                    "これは個人情報を含まない、公開用のテスト文章です。",
                ],
            ),
        ] {
            let image = image::open(fixtures.join(name))?.to_rgba8();
            let start = Instant::now();
            let text = engine.recognize(&image)?;
            let normalize = |s: &str| s.chars().filter(|c| !c.is_whitespace()).collect::<String>();
            let actual = normalize(&text);
            let exact = expected
                .iter()
                .all(|line| actual.contains(&normalize(line)));
            // Only these explicitly public fixtures are printed by this opt-in test.
            eprintln!(
                "PADDLE_FIXTURE {}",
                json!({"name":name,"milliseconds":start.elapsed().as_millis(),"all_expected_lines":exact,"text":text})
            );
            ensure!(exact, "Safe fixture did not match expected text: {name}");
        }
        ensure!(
            engine.recognize(&RgbaImage::new(0, 0)).is_err(),
            "Empty image was accepted"
        );
        // An unauthenticated browser cannot use the local inference endpoint.
        let unauth = engine.client.get(format!("{ENDPOINT}/v1/models")).send()?;
        ensure!(
            unauth.status() == reqwest::StatusCode::UNAUTHORIZED,
            "Native OCR API lacked authentication"
        );
        let start = Instant::now();
        control.stop();
        ensure!(
            engine
                .recognize(&RgbaImage::from_pixel(1, 1, image::Rgba([255; 4])))
                .is_err(),
            "Stopped engine accepted input"
        );
        drop(engine);
        ensure_port_unused(PORT)?;
        eprintln!(
            "PADDLE_STOP {}",
            json!({"success":true,"milliseconds":start.elapsed().as_millis(),"port_closed":true,"invalid_input_rejected":true,"owned_process_waited":true})
        );
        Ok(())
    }
}
