//! Distribution helper: capacity planning is local; installation is explicit.
use anyhow::{Context, Result};
use plamo_translate::{
    hardware,
    installer::{self, InstallRequest},
    model_catalog::{self, Policy},
};
use serde_json::json;
use std::{
    path::PathBuf,
    sync::{Arc, atomic::AtomicBool},
};
fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let value = |name: &str| {
        args.iter()
            .position(|a| a == name)
            .and_then(|i| args.get(i + 1))
            .cloned()
    };
    let root = value("--root")
        .map(PathBuf::from)
        .unwrap_or_else(|| plamo_translate::storage::data_root().join("engine"));
    let policy = match value("--policy").as_deref().unwrap_or("realtime") {
        "realtime" => Policy::Realtime,
        "balanced" => Policy::Balanced,
        "quality" => Policy::Quality,
        _ => anyhow::bail!("policyにはrealtime/balanced/qualityのいずれかを指定してください"),
    };
    let hardware = hardware::detect(&root)?;
    let rec = model_catalog::recommend(&hardware, policy);
    if !args.iter().any(|a| a == "--install") {
        println!(
            "{}",
            serde_json::to_string_pretty(
                &json!({"hardware":hardware,"recommendation":rec,"download_started":false})
            )?
        );
        return Ok(());
    }
    let model = if let Some(id) = value("--model") {
        rec.alternatives
            .iter()
            .find(|a| a.model.id == id)
            .filter(|a| a.fits)
            .map(|a| a.model.clone())
            .context("指定モデルが見つからないか、現在の容量条件を満たしません")?
    } else {
        anyhow::ensure!(rec.feasible, "モデルを取得できる容量条件が確認できません");
        rec.selected.clone()
    };
    let result = installer::install(
        InstallRequest {
            repo: model.repo,
            revision: model.revision,
            filename: model.filename,
            bytes: model.bytes,
            sha256: model.sha256,
            backend: rec.backend.as_str().into(),
            install_root: root,
        },
        Arc::new(AtomicBool::new(false)),
        |p| {
            eprintln!(
                "{}",
                json!({"stage":p.stage,"downloaded_bytes":p.downloaded_bytes,"total_bytes":p.total_bytes,"message":p.message})
            )
        },
    )?;
    let state = plamo_translate::storage::data_root();
    std::fs::create_dir_all(&state)?;
    let root = result
        .model_path
        .parent()
        .and_then(|p| p.parent())
        .and_then(|p| p.parent())
        .context("保存先の構造が不正です")?;
    let pointer = state.join("selected-engine.tmp");
    std::fs::write(&pointer, json!({"root":root}).to_string())?;
    std::fs::rename(pointer, state.join("selected-engine.json"))?;
    println!(
        "{}",
        json!({"ready":true,"model_path":result.model_path,"llama_server":result.llama_server,"template_path":result.template_path,"backend":result.backend,"asr_backend":rec.asr_backend,"asr_device":rec.asr_device})
    );
    Ok(())
}
