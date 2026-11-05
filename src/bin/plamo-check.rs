//! Command-line diagnostics for the same production capture/translation paths.
use anyhow::{Context, Result};
use plamo_translate::{
    engine::{self, TranslationOptions, Translator},
    platform,
};
use serde_json::json;
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};

struct OwnedCapture {
    stop: Arc<AtomicBool>,
    worker: Option<std::thread::JoinHandle<Result<()>>>,
}
impl OwnedCapture {
    fn finish(&mut self) -> Result<()> {
        self.stop.store(true, Ordering::Release);
        self.worker
            .take()
            .context("Capture worker was already joined")?
            .join()
            .map_err(|_| anyhow::anyhow!("Capture worker panicked"))?
    }
}
impl Drop for OwnedCapture {
    fn drop(&mut self) {
        // Every exit, including a later diagnostic error, stops and joins only
        // the capture worker this invocation owns. No playback is started.
        self.stop.store(true, Ordering::Release);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

fn loopback_asr(translator: &Translator, language: &str) -> (serde_json::Value, bool) {
    const RATE: u32 = 16_000;
    const SECONDS: usize = 8;
    let mut report = json!({"success":false,"stopped":false,"capture_worker_joined":false,
        "source":"default playback device / WASAPI loopback","requested_audio_ms":8000,
        "asr_endpoint":"http://127.0.0.1:17861","translation_endpoint":"http://127.0.0.1:17860",
        "stage":"capture"});
    let outcome = (|| -> Result<()> {
        let stop = Arc::new(AtomicBool::new(false));
        let capture_stop = stop.clone();
        let (tx, rx) = mpsc::sync_channel(8);
        let started = Instant::now();
        let mut capture = OwnedCapture {
            stop,
            worker: Some(std::thread::spawn(move || {
                // Production capture bounds a chunk to four seconds. Collect
                // those chunks into one eight-second WAV, retaining silent gaps.
                platform::capture_loopback(capture_stop, tx, 4.0)
            })),
        };
        let mut chunks = Vec::new();
        // A small allowance covers device initialization and polling latency.
        let deadline = Duration::from_millis(8_500);
        while started.elapsed() < deadline {
            match rx.recv_timeout(
                deadline
                    .saturating_sub(started.elapsed())
                    .min(Duration::from_millis(100)),
            ) {
                Ok(chunk) => chunks.push(chunk),
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }
        }
        report["capture_elapsed_ms"] = json!(started.elapsed().as_millis());
        let stopping = Instant::now();
        let stopped = capture.finish();
        report["stop_elapsed_ms"] = json!(stopping.elapsed().as_millis());
        report["capture_worker_joined"] = json!(capture.worker.is_none());
        report["stopped"] = json!(stopped.is_ok());
        stopped.context("Loopback capture did not finish cleanly")?;
        chunks.extend(rx.try_iter()); // Include the producer's final partial window.
        report["captured_chunks"] = json!(chunks.len());
        if chunks.is_empty() {
            report["failure_kind"] = json!("silence_or_capture_timeout");
            anyhow::bail!(
                "No non-silent loopback audio arrived before the 8.5-second deadline (silence or capture timeout)"
            );
        }
        let mut samples = vec![0.0_f32; RATE as usize * SECONDS];
        let mut received_frames = 0;
        for chunk in chunks {
            anyhow::ensure!(chunk.sample_rate == RATE, "Unexpected loopback sample rate");
            let start = (chunk.start_ms.saturating_mul(RATE as u64) / 1000) as usize;
            if start >= samples.len() {
                continue;
            }
            let count = chunk.samples.len().min(samples.len() - start);
            samples[start..start + count].copy_from_slice(&chunk.samples[..count]);
            received_frames += count;
        }
        let rms = (samples
            .iter()
            .map(|sample| f64::from(*sample).powi(2))
            .sum::<f64>()
            / samples.len() as f64)
            .sqrt();
        report["sample_rate"] = json!(RATE);
        report["audio_ms"] = json!(8000);
        report["received_pcm_ms"] = json!(received_frames * 1000 / RATE as usize);
        report["rms"] = json!(rms);
        if rms < 0.0007 {
            report["failure_kind"] = json!("silence");
            anyhow::bail!(
                "Captured eight-second loopback audio is silent or below the production audio threshold"
            );
        }
        let wav = platform::pcm_to_wav(&samples, RATE)?;
        report["wav_bytes"] = json!(wav.len());
        report["stage"] = json!("asr");
        let started = Instant::now();
        let transcript = engine::transcribe("http://127.0.0.1:17861", wav, language)?;
        report["asr"] = json!({"text":transcript,"elapsed_ms":started.elapsed().as_millis()});
        report["stage"] = json!("translation");
        let started = Instant::now();
        let translated = translator.translate(&transcript, |_| {})?;
        report["translation"] =
            json!({"text":translated,"elapsed_ms":started.elapsed().as_millis()});
        report["stage"] = json!("complete");
        report["success"] = json!(true);
        Ok(())
    })();
    if let Err(error) = outcome {
        report["error"] = json!(format!("{error:#}"));
        (report, false)
    } else {
        (report, true)
    }
}

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let option = |name: &str| {
        args.iter()
            .position(|s| s == name)
            .and_then(|i| args.get(i + 1))
            .cloned()
    };
    let mut result = json!({"screens":platform::screens()?.iter().map(|s|json!({"id":s.id,"name":s.name,"width":s.width,"height":s.height,"x":s.x,"y":s.y,"scale":s.scale})).collect::<Vec<_>>()});
    let translator = Translator::new(TranslationOptions::default())?;
    let mut failed = false;
    if args.iter().any(|s| s == "--health") {
        result["translation"] = json!(translator.health()?);
    }
    if let Some(path) = option("--ocr") {
        let image = image::open(&path).context("Read OCR sample")?.to_rgba8();
        let start = Instant::now();
        let text = platform::ocr(&image, &option("--language").unwrap_or("en".into()))?;
        result["ocr"] = json!({"text":text,"elapsed_ms":start.elapsed().as_millis()});
        let start = Instant::now();
        let translated = translator.translate(&text, |_| {})?;
        result["ocr_translation"] =
            json!({"text":translated,"elapsed_ms":start.elapsed().as_millis()});
    }
    if let Some(text) = option("--text") {
        let start = Instant::now();
        let text = translator.translate(&text, |_| {})?;
        result["translation"] = json!({"text":text,"elapsed_ms":start.elapsed().as_millis()});
    }
    if let Some(path) = option("--asr") {
        let start = Instant::now();
        let text = engine::transcribe(
            "http://127.0.0.1:17861",
            std::fs::read(path)?,
            &option("--language").unwrap_or("en".into()),
        )?;
        result["asr"] = json!({"text":text,"elapsed_ms":start.elapsed().as_millis()});
        let start = Instant::now();
        let translated = translator.translate(&text, |_| {})?;
        result["speech_translation"] =
            json!({"text":translated,"elapsed_ms":start.elapsed().as_millis()});
    }
    if let Some(path) = option("--capture") {
        let screen = platform::screens()?
            .into_iter()
            .next()
            .context("No screen")?;
        platform::capture_screen(screen.id)?.save(&path)?;
        result["capture"] = json!({"saved":path});
    }
    if args.iter().any(|s| s == "--loopback") {
        let stop = Arc::new(AtomicBool::new(false));
        let capture_stop = stop.clone();
        let (tx, rx) = mpsc::sync_channel(1);
        let capture = std::thread::spawn(move || platform::capture_loopback(capture_stop, tx, 2.0));
        let mut chunks = 0;
        let start = Instant::now();
        while start.elapsed() < Duration::from_secs(5) {
            if rx.recv_timeout(Duration::from_millis(200)).is_ok() {
                chunks += 1;
            }
        }
        stop.store(true, Ordering::Relaxed);
        capture
            .join()
            .map_err(|_| anyhow::anyhow!("Capture panicked"))??;
        result["loopback"] = json!({"captured_chunks":chunks,"stopped":true});
    }
    if args.iter().any(|s| s == "--loopback-asr") {
        let (report, success) =
            loopback_asr(&translator, &option("--language").unwrap_or("en".into()));
        result["loopback_asr"] = report;
        failed = !success;
    }
    println!("{}", serde_json::to_string_pretty(&result)?);
    anyhow::ensure!(!failed, "Loopback ASR diagnostic failed; see JSON output");
    Ok(())
}
