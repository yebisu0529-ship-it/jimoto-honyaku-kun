//! Pinned, resumable PaddleOCR-VL assets; reuse the app's separately verified llama.cpp.
use crate::{
    hardware,
    installer::{self, Artifact, Progress},
};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::atomic::AtomicBool,
};

const METADATA: &[u8] = include_bytes!("../scripts/paddle-ocr-runtime.json");

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct PaddleInstallation {
    pub version: u32,
    pub model: PathBuf,
    pub mmproj: PathBuf,
    pub model_revision: String,
}
#[derive(Deserialize)]
struct Metadata {
    schema_version: u32,
    repo: String,
    revision: String,
    license: String,
    runtime_release: String,
    embedded_chat_template_sha256: String,
    model: Artifact,
    mmproj: Artifact,
    notices: Vec<Artifact>,
}
#[derive(Serialize, Deserialize)]
struct Manifest {
    metadata_sha256: String,
    installation: PaddleInstallation,
}

fn metadata_hash() -> String {
    format!("{:x}", Sha256::digest(METADATA))
}
fn metadata() -> Result<Metadata> {
    let data: Metadata = serde_json::from_slice(METADATA)?;
    ensure!(
        data.schema_version == 1
            && data.repo == "PaddlePaddle/PaddleOCR-VL-1.6-GGUF"
            && data.revision == "511b09642bb324401f15f97cc23bc67e8f0a291d"
            && data.license == "Apache-2.0"
            && data.runtime_release == "b12119"
            && data.embedded_chat_template_sha256
                == "1369e43952c7a84e834be95732d65111e96cbefe30aa32a4826eca344f5ef939"
            && data.notices.len() == 3,
        "PaddleOCRの固定配布情報が不正です。"
    );
    Ok(data)
}
pub(crate) fn downloads() -> Result<Vec<Artifact>> {
    let data = metadata()?;
    Ok([data.model, data.mmproj]
        .into_iter()
        .chain(data.notices)
        .collect())
}
fn expected(root: &Path, data: &Metadata) -> PaddleInstallation {
    PaddleInstallation {
        version: 1,
        model: root.join(&data.model.name),
        mmproj: root.join(&data.mmproj.name),
        model_revision: data.revision.clone(),
    }
}
fn read_small(path: &Path) -> Result<Vec<u8>> {
    installer::safe_file(path)?;
    ensure!(
        fs::metadata(path)?.len() <= 128 * 1024,
        "PaddleOCRの保存情報が大きすぎます。"
    );
    Ok(fs::read(path)?)
}
fn validate_committed(root: &Path, data: &Metadata, value: &PaddleInstallation) -> Result<()> {
    ensure!(
        *value == expected(root, data),
        "PaddleOCRの保存情報が固定された配置と一致しません。"
    );
    for (path, artifact) in [(&value.model, &data.model), (&value.mmproj, &data.mmproj)] {
        installer::safe_file(path)?;
        ensure!(
            fs::metadata(path)?.len() == artifact.bytes,
            "PaddleOCRモデルのサイズが変わっています。準備を確認してください。"
        );
    }
    ensure!(
        read_small(&root.join("metadata.json"))? == METADATA,
        "PaddleOCRの保存済み配布情報が一致しません。"
    );
    for artifact in &data.notices {
        installer::verify_file(
            &root.join("notices").join(&artifact.name),
            artifact,
            &AtomicBool::new(false),
        )?;
    }
    Ok(())
}

/// Cheap startup inspection: committed metadata, model sizes and notice hashes.
/// Full model SHA verification is performed at install and on the OCR load worker.
pub fn load(root: &Path) -> Result<Option<PaddleInstallation>> {
    if !root.exists() {
        return Ok(None);
    }
    installer::safe_components(root, false)?;
    let root = fs::canonicalize(root)?;
    let path = root.join("installation.json");
    installer::safe_file(&path)?;
    if !path.exists() {
        return Ok(None);
    }
    let manifest: Manifest = serde_json::from_slice(&read_small(&path)?)?;
    ensure!(
        manifest.metadata_sha256 == metadata_hash(),
        "PaddleOCRのインストール記録が固定版と一致しません。"
    );
    validate_committed(&root, &metadata()?, &manifest.installation)?;
    Ok(Some(manifest.installation))
}

pub(crate) fn verify_for_start(value: &PaddleInstallation) -> Result<()> {
    let root = value
        .model
        .parent()
        .context("PaddleOCRモデルの配置が不正です。")?;
    ensure!(
        load(root)?.as_ref() == Some(value),
        "PaddleOCRのインストール記録と選択されたモデルが一致しません。"
    );
    let data = metadata()?;
    let cancel = AtomicBool::new(false);
    installer::verify_file(&value.model, &data.model, &cancel)?;
    installer::verify_file(&value.mmproj, &data.mmproj, &cancel)?;
    Ok(())
}

/// Keeps existing top-level GGUF files in place; the shared downloader checks
/// their SHA and never replaces/moves a mismatching existing committed file.
pub fn install(
    root: &Path,
    cancel: &AtomicBool,
    mut on_progress: impl FnMut(Progress),
) -> Result<PaddleInstallation> {
    installer::cancelled(cancel)?;
    ensure!(
        cfg!(all(windows, target_arch = "x86_64")),
        "PaddleOCRの配布版はWindows x64専用です。"
    );
    let data = metadata()?;
    installer::safe_components(root, true)?;
    let root = fs::canonicalize(root)?;
    let _lock = installer::RootLock::acquire(&root)?;
    load(&root).context("既存のPaddleOCRの保存情報を保持しました。")?;
    let installation = expected(&root, &data);
    let notices = root.join("notices");
    installer::safe_components(&notices, true)?;
    let mut required = 128 * 1024 * 1024;
    for (path, artifact) in [
        (&installation.model, &data.model),
        (&installation.mmproj, &data.mmproj),
    ] {
        required += installer::remaining_bytes(artifact, path)?;
    }
    for artifact in &data.notices {
        required += installer::remaining_bytes(artifact, &notices.join(&artifact.name))?;
    }
    let free = hardware::detect(&root)?
        .disk_free_bytes
        .context("PaddleOCRの保存先の空き容量を確認できませんでした。")?;
    ensure!(
        free >= required,
        "PaddleOCRの準備に必要な空き容量が足りません（必要 {:.1} GB、空き {:.1} GB）。",
        required as f64 / 1e9,
        free as f64 / 1e9
    );
    for (path, artifact, stage) in [
        (&installation.model, &data.model, "paddle_model_download"),
        (
            &installation.mmproj,
            &data.mmproj,
            "paddle_projector_download",
        ),
    ] {
        installer::download_official(artifact, path, cancel, stage, &mut on_progress)?;
    }
    for artifact in &data.notices {
        installer::download_official(
            artifact,
            &notices.join(&artifact.name),
            cancel,
            "paddle_notices",
            &mut on_progress,
        )?;
    }
    installer::cancelled(cancel)?;
    installer::atomic_write(&root.join("metadata.json"), METADATA)?;
    installer::atomic_write(
        &root.join("installation.json"),
        &serde_json::to_vec_pretty(&Manifest {
            metadata_sha256: metadata_hash(),
            installation: installation.clone(),
        })?,
    )?;
    on_progress(Progress {
        stage: "paddle_ready".into(),
        downloaded_bytes: data.model.bytes + data.mmproj.bytes,
        total_bytes: Some(data.model.bytes + data.mmproj.bytes),
        message: "PaddleOCRの準備ができました。".into(),
    });
    Ok(installation)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bundled_assets_are_fixed_official_ggufs_and_apache_notices() {
        let data = metadata().unwrap();
        assert_eq!(data.model.bytes + data.mmproj.bytes, 1_817_539_616);
        for file in [&data.model, &data.mmproj] {
            assert!(file.url.starts_with("https://huggingface.co/PaddlePaddle/"));
            assert!(file.url.contains(&format!("/resolve/{}/", data.revision)));
            assert_eq!(file.sha256.len(), 64);
        }
        assert!(data.notices.iter().any(|a| a.name == "LICENSE-APACHE-2.0"));
        assert!(data.notices.iter().all(|a| !a.url.contains("/main/")));
    }
    #[test]
    fn expected_model_paths_stay_in_the_selected_root() {
        let root = std::env::temp_dir().join("paddle-path-fixture");
        let data = metadata().unwrap();
        let config = expected(&root, &data);
        assert_eq!(config.model.parent(), Some(root.as_path()));
        assert_eq!(config.mmproj.parent(), Some(root.as_path()));
        assert_ne!(config.model, config.mmproj);
    }
    #[test]
    fn uncommitted_files_are_not_loaded_and_cancellation_preserves_them() {
        let root = std::env::temp_dir().join(format!("paddle-setup-test-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&root).unwrap();
        let path = root.join("PaddleOCR-VL-1.6-GGUF.gguf");
        fs::write(&path, b"retained incomplete user file").unwrap();
        assert!(load(&root).unwrap().is_none());
        assert!(
            install(&root, &AtomicBool::new(true), |_| panic!(
                "cancelled install emitted progress"
            ))
            .is_err()
        );
        assert_eq!(fs::read(&path).unwrap(), b"retained incomplete user file");
        assert!(!root.join("installation.json").exists());
        // Only files and directory created by this exact test are removed.
        fs::remove_file(path).unwrap();
        fs::remove_dir(root).unwrap();
    }
}
