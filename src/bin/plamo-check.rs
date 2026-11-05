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

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let option = |name: &str| {
        args.iter()
            .position(|s| s == name)
            .and_then(|i| args.get(i + 1))
            .cloned()
    };
    let mut result = json!({"screens":platform::screens()?.iter().map(|s|json!({"id":s.id,"name":s.name,"width":s.width,"height":s.height})).collect::<Vec<_>>()});
    let translator = Translator::new(TranslationOptions::default())?;
    if args.iter().any(|s| s == "--health") {
        result["translation"] = json!(translator.health()?);
    }
    if let Some(path) = option("--ocr") {
        let image = image::open(&path).context("Read OCR sample")?.to_rgba8();
        let start = Instant::now();
        let text = platform::ocr(&image, &option("--language").unwrap_or("en".into()))?;
        result["ocr"] = json!({"text":text,"elapsed_ms":start.elapsed().as_millis()});
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
    println!("{}", serde_json::to_string_pretty(&result)?);
    Ok(())
}
