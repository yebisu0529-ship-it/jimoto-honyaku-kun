//! Raw PNG transport to our own isolated CPU Indra child; no image/result files.
use crate::{
    indra_setup::{self, IndraInstallation},
    ocr_runtime::{ProcessControl, ensure_port_unused},
};
use anyhow::{Context, Result, ensure};
use image::{ImageFormat, RgbaImage};
use reqwest::blocking::Client;
use serde_json::Value;
use std::{
    io::{Cursor, Read},
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

const PORT: u16 = 17866;
const ENDPOINT: &str = "http://127.0.0.1:17866";
const MAX_PNG: usize = 8 * 1024 * 1024;
const MAX_PIXELS: u64 = 16_000_000;
pub struct IndraEngine {
    control: ProcessControl,
    client: Client,
    revision: String,
}
impl IndraEngine {
    /// Blocking load/readiness check; the app invokes this on its OCR worker.
    pub fn start(installation: IndraInstallation, control: ProcessControl) -> Result<Self> {
        let result = (|| {
            ensure!(
                !control.is_cancelled(),
                "Indraの起動はキャンセルされました。"
            );
            let root = indra_setup::installation_root(&installation)?;
            ensure_port_unused(PORT)?;
            let mut command = Command::new(&installation.python_path);
            command
                .args(["-I", "-X", "utf8"])
                .arg(root.join("runtime/indra_server.py"))
                .arg("--root")
                .arg(indra_setup::ordinary_path(&root)?)
                .env("PYTHONUTF8", "1")
                .env("PYTHONNOUSERSITE", "1")
                .env("HF_HUB_OFFLINE", "1")
                .env("HF_HUB_DISABLE_IMPLICIT_TOKEN", "1")
                .env("HF_HUB_DISABLE_TELEMETRY", "1")
                .env("HF_HOME", root.join("huggingface"))
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null());
            control.spawn(&mut command)?;
            let client = Client::builder()
                .no_proxy()
                .redirect(reqwest::redirect::Policy::none())
                .connect_timeout(Duration::from_secs(2))
                .timeout(Duration::from_secs(30))
                .build()?;
            let deadline = Instant::now() + Duration::from_secs(90);
            loop {
                ensure!(
                    !control.is_cancelled(),
                    "Indraの起動はキャンセルされました。"
                );
                control
                    .owned_pid()
                    .context("Indraのプロセスが終了しました。専用環境の準備を確認してください。")?;
                if let Ok(response) = client
                    .get(format!("{ENDPOINT}/health"))
                    .timeout(Duration::from_secs(2))
                    .send()
                    && response.status().is_success()
                {
                    control.ensure_listener(PORT)?;
                    let health: Value = response.json()?;
                    ensure!(
                        valid_health(&health, &installation),
                        "起動したIndraの日英モデルまたはCPU実行環境を確認できませんでした。"
                    );
                    break;
                }
                ensure!(
                    Instant::now() < deadline,
                    "Indraの読み込みが時間内に完了しませんでした。"
                );
                thread::sleep(Duration::from_millis(100));
            }
            Ok(Self {
                control: control.clone(),
                client,
                revision: installation.model_revision,
            })
        })();
        if result.is_err() {
            control.stop();
        }
        result
    }
    pub fn recognize(&self, image: &RgbaImage) -> Result<String> {
        ensure!(
            !self.control.is_cancelled(),
            "IndraのOCRはキャンセルされました。"
        );
        self.control.ensure_listener(PORT)?;
        let bytes = encode_png(image)?;
        let mut response = self
            .client
            .post(format!("{ENDPOINT}/ocr"))
            .header("Content-Type", "image/png")
            .body(bytes)
            .send()
            .context("Indraへのローカル接続に失敗しました。")?;
        ensure!(
            response.status().is_success(),
            "IndraのOCRに失敗しました（HTTP {}）。",
            response.status()
        );
        let mut body = Vec::new();
        response
            .by_ref()
            .take(1024 * 1024 + 1)
            .read_to_end(&mut body)?;
        ensure!(body.len() <= 1024 * 1024, "Indraの応答が大きすぎます。");
        let value: Value = serde_json::from_slice(&body)?;
        ensure!(
            value["model_revision"] == self.revision && value["device"] == "cpu",
            "Indraの応答モデルが一致しません。"
        );
        ensure!(
            !self.control.is_cancelled(),
            "IndraのOCRはキャンセルされました。"
        );
        Ok(value["text"]
            .as_str()
            .context("Indraの応答に認識結果がありません。")?
            .to_owned())
    }
}
impl Drop for IndraEngine {
    fn drop(&mut self) {
        self.control.stop();
    }
}
fn valid_health(value: &Value, installation: &IndraInstallation) -> bool {
    value["ready"] == true
        && value["engine"] == "indra-ocr"
        && value["sdk_version"] == installation.sdk_version
        && value["model_revision"] == installation.model_revision
        && value["device"] == "cpu"
        && value["model_set"] == "ja-en"
        && value["metadata_sha256"] == indra_setup::metadata_hash()
}
fn encode_png(image: &RgbaImage) -> Result<Vec<u8>> {
    ensure!(
        image.width() > 0
            && image.height() > 0
            && u64::from(image.width()) * u64::from(image.height()) <= MAX_PIXELS,
        "Indraには1600万画素以内の画像を指定してください。"
    );
    let mut out = Cursor::new(Vec::new());
    image.write_to(&mut out, ImageFormat::Png)?;
    let bytes = out.into_inner();
    ensure!(
        bytes.len() <= MAX_PNG,
        "Indraには8MiB以内のPNG画像を指定してください。"
    );
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore = "explicit local fixture OCR with opt-in prepared environment"]
    fn indra_native_ocr_opt_in() {
        assert_eq!(std::env::var("INDRA_NATIVE_VERIFY").as_deref(), Ok("1"));
        let root = std::path::PathBuf::from(
            std::env::var_os("INDRA_VERIFY_ROOT").expect("dedicated root required"),
        );
        let installation = indra_setup::load(&root).unwrap().unwrap();
        let control = ProcessControl::new();
        let start = Instant::now();
        let engine = IndraEngine::start(installation, control.clone()).unwrap();
        eprintln!("INDRA_READY_MS {}", start.elapsed().as_millis());
        for key in ["INDRA_VERIFY_EN", "INDRA_VERIFY_JA"] {
            let path = std::env::var_os(key).expect("safe local PNG required");
            let image = image::open(path).unwrap().to_rgba8();
            let start = Instant::now();
            let text = engine.recognize(&image).unwrap();
            assert!(!text.trim().is_empty());
            if key.ends_with("JA") {
                assert!(text.chars().any(|c| ('\u{3040}'..='\u{30ff}').contains(&c)
                    || ('\u{4e00}'..='\u{9fff}').contains(&c)));
            }
            eprintln!(
                "INDRA_FIXTURE {key} {}ms {text:?}",
                start.elapsed().as_millis()
            );
        }
        assert_eq!(
            engine
                .client
                .post(format!("{ENDPOINT}/ocr"))
                .header("Content-Type", "image/png")
                .body(b"invalid PNG".to_vec())
                .send()
                .unwrap()
                .status(),
            reqwest::StatusCode::BAD_REQUEST
        );
        assert_eq!(
            engine
                .client
                .get(format!("{ENDPOINT}/health"))
                .header("Origin", "https://example.invalid")
                .send()
                .unwrap()
                .status(),
            reqwest::StatusCode::FORBIDDEN
        );
        let start = Instant::now();
        drop(engine);
        ensure_port_unused(PORT).unwrap();
        assert!(control.is_cancelled());
        eprintln!(
            "INDRA_STOP_MS {} PORT_CLOSED true",
            start.elapsed().as_millis()
        );
    }
    #[test]
    fn png_transport_preserves_pixels_and_rejects_empty() {
        let image = RgbaImage::from_pixel(3, 2, image::Rgba([20, 30, 40, 255]));
        let bytes = encode_png(&image).unwrap();
        assert_eq!(image::load_from_memory(&bytes).unwrap().to_rgba8(), image);
        assert!(encode_png(&RgbaImage::new(0, 0)).is_err());
    }
    #[test]
    fn health_requires_exact_model_and_device() {
        let installation = IndraInstallation {
            version: 1,
            python_path: "unused".into(),
            model_path: "unused".into(),
            sdk_version: "1.1.2".into(),
            model_revision: "pinned".into(),
        };
        let mut value = serde_json::json!({"ready":true,"engine":"indra-ocr","sdk_version":"1.1.2","model_revision":"pinned","device":"cpu","model_set":"ja-en","metadata_sha256":indra_setup::metadata_hash()});
        assert!(valid_health(&value, &installation));
        value["device"] = "cuda".into();
        assert!(!valid_health(&value, &installation));
    }
}
