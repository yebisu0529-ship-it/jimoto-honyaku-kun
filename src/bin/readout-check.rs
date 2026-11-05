//! Explicit diagnostic: play fixed Japanese test sentences, test stop, then replay.
use anyhow::{Context, Result, ensure};
use plamo_translate::{
    speech_setup,
    tts::{TtsEngine, TtsEvent},
};
use serde_json::json;
use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

fn wait(
    engine: &TtsEngine,
    suppress: &AtomicBool,
    trace: &mut Vec<serde_json::Value>,
) -> Result<bool> {
    let started = Instant::now();
    let mut playback_seen = false;
    while started.elapsed() < Duration::from_secs(240) {
        playback_seen |= suppress.load(Ordering::Acquire);
        for event in engine.poll() {
            match event {
                TtsEvent::Status(message) => {
                    trace.push(json!({"status":message,"ms":started.elapsed().as_millis()}))
                }
                TtsEvent::Rate(rate) => trace
                    .push(json!({"event":"rate","speed":rate,"ms":started.elapsed().as_millis()})),
                TtsEvent::Started => {
                    trace.push(json!({"event":"started","ms":started.elapsed().as_millis()}))
                }
                TtsEvent::Finished => {
                    trace.push(json!({"event":"finished","ms":started.elapsed().as_millis()}));
                    return Ok(playback_seen);
                }
                TtsEvent::Error(error) => anyhow::bail!("{error}"),
            }
        }
        thread::sleep(Duration::from_millis(10));
    }
    anyhow::bail!("Readout did not finish within the diagnostic timeout")
}
fn main() -> Result<()> {
    let root = PathBuf::from(
        std::env::args_os()
            .nth(1)
            .context("Usage: readout-check SPEECH_INSTALL_ROOT")?,
    );
    let config = speech_setup::load(&root)?.context("Speech models are not installed")?;
    let suppress = Arc::new(AtomicBool::new(false));
    let engine = TtsEngine::start(config, suppress.clone())?;
    let mut trace = Vec::new();
    engine.speak("こんにちは。地元翻訳くんの読み上げテストです。".into());
    ensure!(
        wait(&engine, &suppress, &mut trace)?,
        "Capture was not suppressed during playback"
    );
    ensure!(
        !suppress.load(Ordering::Acquire),
        "Capture suppression was not released"
    );
    engine.speak("停止ボタンの動作を確認しています。".repeat(40));
    thread::sleep(Duration::from_millis(250));
    let stop_started = Instant::now();
    engine.stop();
    ensure!(!engine.is_busy(), "Stop did not clear the busy state");
    let stop_ms = stop_started.elapsed().as_millis();
    thread::sleep(Duration::from_millis(300));
    engine.poll();
    engine.speak("停止後も読み上げできます。".into());
    ensure!(
        wait(&engine, &suppress, &mut trace)?,
        "Replay did not suppress capture"
    );
    ensure!(
        !suppress.load(Ordering::Acquire),
        "Capture remained suppressed after replay"
    );
    drop(engine);
    println!(
        "{}",
        serde_json::to_string_pretty(
            &json!({"played":true,"stop_and_replay":true,"stop_ms":stop_ms,"capture_suppression_released":true,"trace":trace})
        )?
    );
    Ok(())
}
