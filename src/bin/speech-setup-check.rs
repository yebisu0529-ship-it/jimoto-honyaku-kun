use anyhow::{Context, Result};
use plamo_translate::speech_setup;
use std::{
    path::PathBuf,
    sync::{Arc, atomic::AtomicBool},
};
fn main() -> Result<()> {
    let mut args = std::env::args_os().skip(1);
    let root = PathBuf::from(
        args.next()
            .context("Usage: speech-setup-check ABSOLUTE_ROOT [cpu|vulkan]")?,
    );
    let backend = args
        .next()
        .and_then(|value| value.into_string().ok())
        .unwrap_or_else(|| "vulkan".into());
    let installation = speech_setup::install(
        &root,
        &backend,
        Arc::new(AtomicBool::new(false)),
        |progress| {
            println!(
                "{} {} / {} {}",
                progress.stage,
                progress.downloaded_bytes,
                progress.total_bytes.unwrap_or(0),
                progress.message
            );
        },
    )?;
    println!("{}", serde_json::to_string_pretty(&installation)?);
    anyhow::ensure!(
        speech_setup::load(&root)?.as_ref() == Some(&installation),
        "Committed speech installation could not be loaded"
    );
    Ok(())
}
