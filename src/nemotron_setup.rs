//! Opt-in official NVIDIA NeMo-Speech.cpp, independent of Python ASR and TTS.
use crate::{
    hardware,
    installer::{self, Artifact, Progress, RuntimeFile},
};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    sync::{Arc, atomic::AtomicBool},
};

const METADATA: &[u8] = include_bytes!("../scripts/nemotron-runtime.json");
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct NemotronInstallation {
    pub version: u32,
    pub executable: PathBuf,
    pub model: PathBuf,
    pub backend: String,
    pub runtime_version: String,
    pub model_revision: String,
    pub model_sha256: String,
}
#[derive(Deserialize)]
struct Metadata {
    schema_version: u32,
    runtime_version: String,
    runtimes: BTreeMap<String, Artifact>,
    model: Artifact,
    model_revision: String,
}
#[derive(Serialize, Deserialize)]
struct Manifest {
    metadata_sha256: String,
    installation: NemotronInstallation,
}
#[derive(Serialize, Deserialize)]
struct RuntimeIndex {
    artifact: Artifact,
    files: Vec<RuntimeFile>,
    executable: String,
}
fn metadata() -> Result<Metadata> {
    let data: Metadata = serde_json::from_slice(METADATA)?;
    ensure!(
        data.schema_version == 1,
        "Nemotronの配布情報の形式が不正です。"
    );
    Ok(data)
}
pub(crate) fn downloads() -> Result<Vec<Artifact>> {
    let data = metadata()?;
    Ok(data.runtimes.into_values().chain([data.model]).collect())
}
fn metadata_hash() -> String {
    format!("{:x}", Sha256::digest(METADATA))
}
fn backend_spec<'a>(data: &'a Metadata, backend: &str) -> Result<&'a Artifact> {
    data.runtimes
        .get(backend)
        .context("Nemotronの実行方式はCPUまたはVulkanを選んでください。")
}
fn runtime_directory(data: &Metadata, backend: &str) -> String {
    format!(
        "nemo-speech-{}-windows-x86_64-{backend}",
        data.runtime_version
    )
}
fn expected(root: &Path, backend: &str, data: &Metadata, executable: &str) -> NemotronInstallation {
    NemotronInstallation {
        version: 1,
        executable: root
            .join("runtime")
            .join(runtime_directory(data, backend))
            .join(executable),
        model: root
            .join("models")
            .join(&data.model_revision)
            .join(&data.model.name),
        backend: backend.into(),
        runtime_version: data.runtime_version.clone(),
        model_revision: data.model_revision.clone(),
        model_sha256: data.model.sha256.clone(),
    }
}
fn read_small(path: &Path) -> Result<Vec<u8>> {
    installer::safe_file(path)?;
    ensure!(
        fs::metadata(path)?.len() <= 4 * 1024 * 1024,
        "Nemotronのインストール記録が大きすぎます。"
    );
    Ok(fs::read(path)?)
}
fn verify_runtime(directory: &Path, spec: &Artifact, cancel: &AtomicBool) -> Result<String> {
    installer::safe_components(directory, false)?;
    let index: RuntimeIndex =
        serde_json::from_slice(&read_small(&directory.join("runtime.json"))?)?;
    let executable = installer::archive_path(&index.executable)?;
    ensure!(
        index.artifact == *spec
            && !index.files.is_empty()
            && executable
                .file_name()
                .is_some_and(|n| n == "nemo-speech.exe"),
        "Nemotronの実行環境の記録が一致しません。"
    );
    let mut found = false;
    for file in index.files {
        let path = directory.join(installer::archive_path(&file.path)?);
        found |= file.path == index.executable;
        installer::verify_file(
            &path,
            &Artifact {
                name: file.path,
                url: String::new(),
                bytes: file.bytes,
                sha256: file.sha256,
            },
            cancel,
        )?;
    }
    ensure!(found, "Nemotronの実行ファイルがありません。");
    Ok(index.executable)
}
/// Read only committed, pinned and contained installation; no model load/network.
pub fn load(root: &Path) -> Result<Option<NemotronInstallation>> {
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
    let data = metadata()?;
    let spec = backend_spec(&data, &manifest.installation.backend)?;
    let runtime = root
        .join("runtime")
        .join(runtime_directory(&data, &manifest.installation.backend));
    let exe = verify_runtime(&runtime, spec, &AtomicBool::new(false))?;
    ensure!(
        manifest.metadata_sha256 == metadata_hash()
            && manifest.installation
                == expected(&root, &manifest.installation.backend, &data, &exe),
        "Nemotronのインストール記録が、固定された配布情報と一致しません。"
    );
    installer::safe_file(&manifest.installation.model)?;
    ensure!(
        fs::metadata(&manifest.installation.model)?.len() == data.model.bytes,
        "Nemotronモデルのサイズが変わっています。再度準備してください。"
    );
    ensure!(
        read_small(&root.join("metadata.json"))? == METADATA,
        "Nemotronの保存済み配布情報が一致しません。"
    );
    Ok(Some(manifest.installation))
}
/// What `install` would download for this backend now: the runtime archive
/// unless it is already unpacked, and the model unless the whole file is saved.
pub fn pending_download_bytes(root: &Path, backend: &str) -> u64 {
    let Ok(data) = metadata() else { return 0 };
    let Ok(spec) = backend_spec(&data, backend) else {
        return 0;
    };
    let mut files = vec![(
        &data.model,
        root.join("models")
            .join(&data.model_revision)
            .join(&data.model.name),
    )];
    if !root
        .join("runtime")
        .join(runtime_directory(&data, backend))
        .exists()
    {
        files.push((spec, root.join("downloads").join(&spec.name)));
    }
    installer::remaining_total(&files)
}
/// Resumable SHA-verified downloads; commit only after both model/runtime pass.
pub fn install(
    root: &Path,
    backend: &str,
    cancel: Arc<AtomicBool>,
    mut on_progress: impl FnMut(Progress),
) -> Result<NemotronInstallation> {
    let data = metadata()?;
    let spec = backend_spec(&data, backend)?;
    installer::cancelled(&cancel)?;
    ensure!(
        cfg!(all(windows, target_arch = "x86_64")),
        "Nemotronの配布版はWindows x64専用です。"
    );
    installer::safe_components(root, true)?;
    let root = fs::canonicalize(root)?;
    let _lock = installer::RootLock::acquire(&root)?;
    load(&root).context("既存のNemotronのインストール記録を保持しました。")?;
    let downloads = root.join("downloads");
    let runtime = root.join("runtime").join(runtime_directory(&data, backend));
    let model = root
        .join("models")
        .join(&data.model_revision)
        .join(&data.model.name);
    for directory in [
        &downloads,
        runtime.parent().unwrap(),
        model.parent().unwrap(),
    ] {
        installer::safe_components(directory, true)?;
    }
    let archive = downloads.join(&spec.name);
    let required = installer::remaining_bytes(&data.model, &model)?
        + if runtime.exists() {
            0
        } else {
            installer::remaining_bytes(spec, &archive)? + 256 * 1024 * 1024
        }
        + 128 * 1024 * 1024;
    let free = hardware::detect(&root)?
        .disk_free_bytes
        .context("Nemotronの保存先の空き容量を確認できませんでした。")?;
    ensure!(
        free >= required,
        "Nemotronの準備に必要な空き容量が足りません（必要 {:.1} GB、空き {:.1} GB）。",
        required as f64 / 1e9,
        free as f64 / 1e9
    );
    let executable = if runtime.exists() {
        verify_runtime(&runtime, spec, &cancel)?
    } else {
        installer::download_official(
            spec,
            &archive,
            &cancel,
            "nemotron_runtime_download",
            &mut on_progress,
        )?;
        // Official archives contain a package directory. Preserve all DLLs and licenses.
        let mut zip = zip::ZipArchive::new(fs::File::open(&archive)?)?;
        let mut executables = Vec::new();
        for i in 0..zip.len() {
            let entry = zip.by_index(i)?;
            let relative = installer::archive_path(entry.name())?;
            if relative.file_name().is_some_and(|n| n == "nemo-speech.exe") {
                executables.push(relative.to_string_lossy().replace('\\', "/"));
            }
        }
        ensure!(
            executables.len() == 1,
            "NemotronのZIP内の実行ファイルを一意に特定できません。"
        );
        let executable = executables.remove(0);
        let staging = runtime
            .parent()
            .unwrap()
            .join(format!(".staging-{}", uuid::Uuid::new_v4()));
        installer::safe_components(&staging, true)?;
        let _guard = installer::StagingGuard {
            path: staging.clone(),
            parent: runtime.parent().unwrap().into(),
        };
        let files = installer::unpack_runtime(
            &[archive],
            &staging,
            &cancel,
            &mut on_progress,
            &executable.to_ascii_lowercase(),
        )?;
        installer::atomic_write(
            &staging.join("runtime.json"),
            &serde_json::to_vec_pretty(&RuntimeIndex {
                artifact: spec.clone(),
                files,
                executable: executable.clone(),
            })?,
        )?;
        installer::cancelled(&cancel)?;
        ensure!(
            !runtime.exists(),
            "Nemotronの保存先がほかの処理で作成されました。"
        );
        fs::rename(staging, &runtime)?;
        executable
    };
    installer::download_official(
        &data.model,
        &model,
        &cancel,
        "nemotron_model_download",
        &mut on_progress,
    )?;
    let installation = expected(&root, backend, &data, &executable);
    installer::atomic_write(&root.join("metadata.json"), METADATA)?;
    installer::cancelled(&cancel)?;
    installer::atomic_write(
        &root.join("installation.json"),
        &serde_json::to_vec_pretty(&Manifest {
            metadata_sha256: metadata_hash(),
            installation: installation.clone(),
        })?,
    )?;
    on_progress(Progress {
        stage: "complete".into(),
        downloaded_bytes: data.model.bytes,
        total_bytes: Some(data.model.bytes),
        message: "Nemotronの準備ができました。".into(),
    });
    Ok(installation)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pins_and_backends_are_fixed() {
        let data = metadata().unwrap();
        assert_eq!(data.model.bytes, 699_872_960);
        assert_eq!(
            data.model.sha256,
            "d9a01898d2a611c8764e23a1c2f45e70bbd5a425dc4de93692ac951dd603812d"
        );
        assert!(backend_spec(&data, "cuda").is_err());
        assert_eq!(backend_spec(&data, "vulkan").unwrap().bytes, 22_417_032);
    }
    #[test]
    fn cancelled_install_does_not_touch_manifest() {
        let root = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
        fs::create_dir(&root).unwrap();
        fs::write(root.join("installation.json"), b"existing").unwrap();
        assert!(install(&root, "cpu", Arc::new(AtomicBool::new(true)), |_| {}).is_err());
        assert_eq!(
            fs::read(root.join("installation.json")).unwrap(),
            b"existing"
        );
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn pending_bytes_are_the_runtime_archive_and_the_model_that_are_not_saved() {
        let data = metadata().unwrap();
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        let archive = |backend| backend_spec(&data, backend).unwrap().bytes;
        assert_eq!(
            pending_download_bytes(root, "vulkan"),
            archive("vulkan") + data.model.bytes
        );
        assert_eq!(
            pending_download_bytes(root, "cpu"),
            archive("cpu") + data.model.bytes
        );
        let model = root
            .join("models")
            .join(&data.model_revision)
            .join(&data.model.name);
        fs::create_dir_all(model.parent().unwrap()).unwrap();
        fs::File::create(&model)
            .unwrap()
            .set_len(data.model.bytes)
            .unwrap();
        assert_eq!(pending_download_bytes(root, "vulkan"), archive("vulkan"));
        fs::create_dir_all(
            root.join("runtime")
                .join(runtime_directory(&data, "vulkan")),
        )
        .unwrap();
        assert_eq!(pending_download_bytes(root, "vulkan"), 0);
        assert_eq!(
            pending_download_bytes(root, "cpu"),
            archive("cpu"),
            "the CPU runtime is another folder"
        );
    }
}
