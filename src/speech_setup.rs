//! Optional, pinned speech.cpp / Irodori-TTS installation, isolated from translation.
use crate::{
    hardware,
    installer::{self, Artifact, Progress, RuntimeFile, Saved},
};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::{Arc, atomic::AtomicBool},
};

const METADATA: &[u8] = include_bytes!("../scripts/speech-runtime.json");
const RUNTIME_DIRECTORY: &str = "speech-0.10.1-windows-x64-vulkan";
const RECORD_MISMATCH: &str = "読み上げのインストール記録が選択されたモデルと一致しません。";

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct SpeechInstallation {
    pub executable: PathBuf,
    pub model: PathBuf,
    pub backend: String,
    pub voice: String,
}
impl SpeechInstallation {
    /// This record is `expected`, written while the folder was somewhere else.
    /// Only the path fields are left out of the comparison.
    fn moved_to(&self, root: &Path, expected: &Self) -> bool {
        let pinned = Self {
            executable: expected.executable.clone(),
            model: expected.model.clone(),
            ..self.clone()
        };
        pinned == *expected
            && installer::relocated(root, &expected.executable, &self.executable)
            && installer::relocated(root, &expected.model, &self.model)
    }
}
#[derive(Deserialize)]
struct RuntimeSpec {
    #[serde(flatten)]
    artifact: Artifact,
    executable: String,
}
#[derive(Deserialize)]
struct ModelSpec {
    #[serde(flatten)]
    artifact: Artifact,
    revision: String,
}
#[derive(Deserialize)]
struct Metadata {
    schema_version: u32,
    runtime: RuntimeSpec,
    model: ModelSpec,
    notices: Vec<Artifact>,
}
#[derive(Serialize, Deserialize)]
struct RuntimeIndex {
    artifact: Artifact,
    files: Vec<RuntimeFile>,
}
#[derive(Serialize, Deserialize)]
struct Manifest {
    version: u32,
    metadata_sha256: String,
    installation: SpeechInstallation,
}
fn metadata() -> Result<Metadata> {
    metadata_of(METADATA)
}
fn metadata_of(release: &[u8]) -> Result<Metadata> {
    let metadata: Metadata = serde_json::from_slice(release)?;
    ensure!(
        metadata.schema_version == 1,
        "読み上げモデルの情報形式が不正です。"
    );
    Ok(metadata)
}
pub(crate) fn downloads() -> Result<Vec<Artifact>> {
    let data = metadata()?;
    Ok([data.runtime.artifact, data.model.artifact]
        .into_iter()
        .chain(data.notices)
        .collect())
}
fn metadata_hash(release: &[u8]) -> String {
    format!("{:x}", Sha256::digest(release))
}
/// A record keeps the SHA-256 of the metadata bytes it was installed with.
/// Line endings in those bytes vary between builds without changing any
/// pinned artifact, so the parsed content decides whether it is this model.
fn installed_metadata_matches(release: &[u8], recorded_sha256: &str, installed: &[u8]) -> bool {
    let parse = |bytes: &[u8]| serde_json::from_slice::<serde_json::Value>(bytes).ok();
    format!("{:x}", Sha256::digest(installed)) == recorded_sha256
        && parse(installed).is_some_and(|content| parse(release) == Some(content))
}
fn read_small(path: &Path) -> Result<Vec<u8>> {
    installer::safe_file(path)?;
    ensure!(
        fs::metadata(path)?.len() <= 2 * 1024 * 1024,
        "読み上げのインストール記録が大きすぎます。"
    );
    Ok(fs::read(path)?)
}
fn validate_backend(backend: &str) -> Result<()> {
    ensure!(
        matches!(backend, "cpu" | "vulkan"),
        "読み上げの実行方式はCPUまたはVulkanを選んでください。"
    );
    Ok(())
}
fn check_platform() -> Result<()> {
    ensure!(
        cfg!(all(windows, target_arch = "x86_64")),
        "この読み上げ配布版はWindows x64専用です。"
    );
    #[cfg(windows)]
    {
        let windows = std::env::var_os("SystemRoot")
            .context("Windowsのシステムフォルダーを確認できません。")?;
        ensure!(
            Path::new(&windows).join("System32/vulkan-1.dll").is_file(),
            "読み上げにはVulkanの実行環境が必要です。GPUドライバーを導入・更新してから、もう一度準備してください。CPUで使う場合も必要です。"
        );
    }
    Ok(())
}
fn expected(root: &Path, backend: &str, metadata: &Metadata) -> SpeechInstallation {
    SpeechInstallation {
        executable: root
            .join("runtime")
            .join(RUNTIME_DIRECTORY)
            .join(&metadata.runtime.executable),
        model: root
            .join("models")
            .join(&metadata.model.revision)
            .join(&metadata.model.artifact.name),
        backend: backend.into(),
        voice: "none".into(),
    }
}
fn verify_runtime(root: &Path, spec: &RuntimeSpec, cancel: &AtomicBool) -> Result<()> {
    installer::safe_components(root, false)?;
    let index: RuntimeIndex = serde_json::from_slice(&read_small(&root.join("runtime.json"))?)?;
    ensure!(
        index.artifact == spec.artifact && !index.files.is_empty(),
        "保存済みの読み上げ実行環境の記録が一致しません。"
    );
    let mut executable = false;
    for record in index.files {
        let relative = installer::archive_path(&record.path)?;
        let path = root.join(relative);
        installer::verify_file(
            &path,
            &Artifact {
                name: record.path.clone(),
                url: String::new(),
                bytes: record.bytes,
                sha256: record.sha256,
            },
            cancel,
        )?;
        executable |= record.path == spec.executable;
    }
    ensure!(executable, "読み上げ実行環境にspeech.exeがありません。");
    Ok(())
}
/// Inspect only a committed, contained installation. The 1.92GB model was hashed
/// before commit; startup checks its size and hashes the much smaller runtime.
pub fn load(root: &Path) -> Result<Option<SpeechInstallation>> {
    read_saved(METADATA, root)?.current(RECORD_MISMATCH)
}

fn read_saved(release: &[u8], root: &Path) -> Result<Saved<SpeechInstallation>> {
    if !root.exists() {
        return Ok(Saved::Absent);
    }
    installer::safe_components(root, false)?;
    let root = fs::canonicalize(root)?;
    let path = root.join("installation.json");
    installer::safe_file(&path)?;
    if !path.exists() {
        return Ok(Saved::Absent);
    }
    let manifest: Manifest = serde_json::from_slice(&read_small(&path)?)?;
    let metadata = metadata_of(release)?;
    let saved = manifest.installation;
    validate_backend(&saved.backend)?;
    let installed_metadata = read_small(&root.join("metadata.json"))?;
    let wanted = expected(&root, &saved.backend, &metadata);
    let current = saved == wanted;
    ensure!(
        manifest.version == 1
            && installed_metadata_matches(release, &manifest.metadata_sha256, &installed_metadata)
            && (current || saved.moved_to(&root, &wanted)),
        RECORD_MISMATCH
    );
    installer::safe_file(&wanted.model)?;
    ensure!(
        fs::metadata(&wanted.model)?.len() == metadata.model.artifact.bytes,
        "保存済みの読み上げモデルのサイズが変わっています。再度準備してください。"
    );
    let cancel = AtomicBool::new(false);
    verify_runtime(
        wanted.executable.parent().unwrap(),
        &metadata.runtime,
        &cancel,
    )?;
    for notice in &metadata.notices {
        installer::verify_file(&root.join("notices").join(&notice.name), notice, &cancel)?;
    }
    Ok(if current {
        Saved::Current(wanted)
    } else {
        Saved::Moved
    })
}
/// What `install` would download now: the runtime archive unless it is already
/// unpacked, the model unless the whole file is saved, and the unsaved notices.
pub fn pending_download_bytes(root: &Path) -> u64 {
    let Ok(metadata) = metadata() else { return 0 };
    let installation = expected(root, "vulkan", &metadata);
    let mut files = vec![(&metadata.model.artifact, installation.model)];
    if !installation.executable.parent().unwrap().exists() {
        files.push((
            &metadata.runtime.artifact,
            root.join("downloads").join(&metadata.runtime.artifact.name),
        ));
    }
    files.extend(
        metadata
            .notices
            .iter()
            .map(|notice| (notice, root.join("notices").join(&notice.name))),
    );
    installer::remaining_total(&files)
}
/// Download and verify the optional runtime/model without changing PATH, browser,
/// translation settings, or any existing installation until the final commit.
pub fn install(
    root: &Path,
    backend: &str,
    cancel: Arc<AtomicBool>,
    on_progress: impl FnMut(Progress),
) -> Result<SpeechInstallation> {
    validate_backend(backend)?;
    installer::cancelled(&cancel)?;
    check_platform()?;
    install_release(METADATA, root, backend, cancel, on_progress)
}

/// `install` for the pinned `release`, which tests replace with a small one.
fn install_release(
    release: &[u8],
    root: &Path,
    backend: &str,
    cancel: Arc<AtomicBool>,
    mut on_progress: impl FnMut(Progress),
) -> Result<SpeechInstallation> {
    let metadata = metadata_of(release)?;
    installer::safe_components(root, true)?;
    let root = fs::canonicalize(root)?;
    let _lock = installer::RootLock::acquire(&root)?;
    // An unrelated or damaged pre-existing manifest is never replaced as a side
    // effect of preparing this optional feature. Incomplete downloads have no
    // manifest and continue through the normal resumable path below. The record
    // of a moved folder is neither: it passes, the files are verified below at
    // this folder's paths, and the record is written for them.
    read_saved(release, &root)
        .context("既存の読み上げ設定を保持しました。インストール記録を確認してください。")?;
    let installation = expected(&root, backend, &metadata);
    let runtime = installation.executable.parent().unwrap();
    let downloads = root.join("downloads");
    let notices = root.join("notices");
    for directory in [
        &downloads,
        &notices,
        installation.model.parent().unwrap(),
        runtime.parent().unwrap(),
    ] {
        installer::safe_components(directory, true)?;
    }
    let archive = downloads.join(&metadata.runtime.artifact.name);
    let required = installer::remaining_bytes(&metadata.model.artifact, &installation.model)?
        + if runtime.exists() {
            0
        } else {
            installer::remaining_bytes(&metadata.runtime.artifact, &archive)?
                + 2 * 1024 * 1024 * 1024
        }
        + 256 * 1024 * 1024;
    let free = hardware::detect(&root)?
        .disk_free_bytes
        .context("保存先の空き容量を確認できませんでした。読み上げの準備を開始していません。")?;
    ensure!(
        free >= required,
        "読み上げの準備に必要な空き容量が足りません（必要 {:.1} GB、空き {:.1} GB）。",
        required as f64 / 1e9,
        free as f64 / 1e9
    );
    if runtime.exists() {
        verify_runtime(runtime, &metadata.runtime, &cancel)?;
    } else {
        installer::download_official(
            &metadata.runtime.artifact,
            &archive,
            &cancel,
            "speech_runtime_download",
            &mut on_progress,
        )?;
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
            &metadata.runtime.executable,
        )?;
        installer::atomic_write(
            &staging.join("runtime.json"),
            &serde_json::to_vec_pretty(&RuntimeIndex {
                artifact: metadata.runtime.artifact,
                files,
            })?,
        )?;
        installer::cancelled(&cancel)?;
        ensure!(
            !runtime.exists(),
            "読み上げ実行環境の保存先が他の処理で作成されました。"
        );
        fs::rename(staging, runtime)?;
    }
    installer::download_official(
        &metadata.model.artifact,
        &installation.model,
        &cancel,
        "speech_model_download",
        &mut on_progress,
    )?;
    for notice in metadata.notices {
        installer::download_official(
            &notice,
            &notices.join(&notice.name),
            &cancel,
            "speech_notices",
            &mut on_progress,
        )?;
    }
    installer::atomic_write(&root.join("metadata.json"), release)?;
    installer::cancelled(&cancel)?;
    installer::atomic_write(
        &root.join("installation.json"),
        &serde_json::to_vec_pretty(&Manifest {
            version: 1,
            metadata_sha256: metadata_hash(release),
            installation: installation.clone(),
        })?,
    )?;
    on_progress(Progress {
        stage: "complete".into(),
        downloaded_bytes: metadata.model.artifact.bytes,
        total_bytes: Some(metadata.model.artifact.bytes),
        message: "読み上げの準備ができました。".into(),
    });
    Ok(installation)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::installer::fixtures::{self, Folder};

    const MODEL: &[u8] = b"test speech model";
    const NOTICE: &[u8] = b"test notice";
    const REVISION: &str = "0123456789abcdef";

    fn release() -> Vec<u8> {
        let with = |artifact: Artifact, field: &str, value: &str| {
            let mut spec = serde_json::to_value(artifact).unwrap();
            spec[field] = value.into();
            spec
        };
        serde_json::to_vec_pretty(&serde_json::json!({
            "schema_version": 1,
            "runtime": with(
                fixtures::artifact("speech-test.zip", b"runtime archive"),
                "executable",
                "speech.exe",
            ),
            "model": with(fixtures::artifact("test-voice.gguf", MODEL), "revision", REVISION),
            "notices": [fixtures::artifact("README.md", NOTICE)],
        }))
        .unwrap()
    }
    fn runtime_folder(root: &Path, data: &Metadata) -> PathBuf {
        let wanted = expected(root, "cpu", data);
        wanted.executable.parent().unwrap().to_path_buf()
    }
    fn model_file(root: &Path, data: &Metadata) -> PathBuf {
        expected(root, "cpu", data).model
    }
    /// What a real install leaves under `root`: the downloads unpacked and
    /// saved, then the record.
    fn installed(root: &Path, release: &[u8]) -> SpeechInstallation {
        let data = metadata_of(release).unwrap();
        let runtime = runtime_folder(root, &data);
        let index = RuntimeIndex {
            artifact: data.runtime.artifact.clone(),
            files: vec![
                fixtures::package_file(&runtime, "speech.exe", b"executable"),
                fixtures::package_file(&runtime, "vulkan-shim.dll", b"library"),
            ],
        };
        fixtures::write(
            &runtime.join("runtime.json"),
            &serde_json::to_vec(&index).unwrap(),
        );
        fixtures::write(&model_file(root, &data), MODEL);
        for notice in &data.notices {
            fixtures::write(&root.join("notices").join(&notice.name), NOTICE);
        }
        install_at(release, root).0.unwrap()
    }
    /// The install, and the stages it reported.
    fn install_at(release: &[u8], root: &Path) -> (Result<SpeechInstallation>, Vec<String>) {
        let mut stages = Vec::new();
        let result = install_release(
            release,
            root,
            "cpu",
            Arc::new(AtomicBool::new(false)),
            |progress| stages.push(progress.stage),
        );
        (result, stages)
    }
    fn edit_record(root: &Path, change: impl FnOnce(&mut Manifest)) {
        let path = root.join("installation.json");
        let mut record: Manifest = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        change(&mut record);
        fs::write(path, serde_json::to_vec_pretty(&record).unwrap()).unwrap();
    }
    /// An installation made in one folder and moved to another.
    fn moved(folder: &Folder, release: &[u8]) -> (PathBuf, PathBuf, SpeechInstallation) {
        let before = folder.path.join("before").join("speech");
        let after = folder.path.join("after").join("speech");
        let installation = installed(&before, release);
        fs::create_dir_all(after.parent().unwrap()).unwrap();
        fs::rename(&before, &after).unwrap();
        (before, after, installation)
    }
    /// Moves an installation and lets `spoil` change it at its new place. The
    /// install has to refuse it for `evidence` and change no file.
    #[track_caller]
    fn assert_refused(release: &[u8], evidence: &str, spoil: impl FnOnce(&Path)) {
        let folder = Folder::new();
        let (_, after, _) = moved(&folder, release);
        spoil(&after);
        let files = fixtures::snapshot(&after);
        let error = format!("{:#}", install_at(release, &after).0.unwrap_err());
        assert!(error.contains(evidence), "{error}");
        assert_eq!(
            fixtures::snapshot(&after),
            files,
            "a refused install changes no file"
        );
    }
    #[track_caller]
    fn assert_record_refused(
        release: &[u8],
        evidence: &str,
        change: impl FnOnce(&Path, &mut Manifest),
    ) {
        assert_refused(release, evidence, |root| {
            edit_record(root, |record| change(root, record))
        });
    }

    #[test]
    fn a_moved_installation_is_verified_where_it_now_is_and_its_record_rewritten() {
        let release = release();
        let folder = Folder::new();
        let (before, after, was) = moved(&folder, &release);
        assert!(
            matches!(read_saved(&release, &after), Ok(Saved::Moved)),
            "a reader still refuses the old paths"
        );
        let (result, stages) = install_at(&release, &after);
        let installation = result.unwrap();
        assert!(
            stages.iter().all(|s| s == "verify" || s == "complete"),
            "nothing is downloaded: {stages:?}"
        );
        let here = |path: &Path| after.join(path.strip_prefix(&before).unwrap());
        assert_eq!(
            installation,
            SpeechInstallation {
                executable: here(&was.executable),
                model: here(&was.model),
                ..was.clone()
            }
        );
        let record: Manifest =
            serde_json::from_slice(&fs::read(after.join("installation.json")).unwrap()).unwrap();
        assert_eq!(record.installation, installation);
        assert!(matches!(
            read_saved(&release, &after),
            Ok(Saved::Current(now)) if now == installation
        ));
        assert!(!before.exists(), "the old folder is neither read nor made");
        let files = fixtures::snapshot(&after);
        assert_eq!(install_at(&release, &after).0.unwrap(), installation);
        assert_eq!(
            fixtures::snapshot(&after),
            files,
            "installing again changes nothing"
        );
    }
    #[test]
    fn a_copied_installation_leaves_the_folder_it_was_copied_from_untouched() {
        let release = release();
        let folder = Folder::new();
        let source = folder.path.join("source").join("speech");
        let copy = folder.path.join("copy").join("speech");
        installed(&source, &release);
        fixtures::copy_tree(&source, &copy);
        let files = fixtures::snapshot(&source);
        assert!(matches!(read_saved(&release, &copy), Ok(Saved::Moved)));
        install_at(&release, &copy).0.unwrap();
        assert_eq!(fixtures::snapshot(&source), files);
        assert!(matches!(
            read_saved(&release, &copy),
            Ok(Saved::Current(now)) if now.model.starts_with(&copy)
        ));
    }
    #[test]
    fn a_moved_record_of_another_pinned_release_still_blocks_the_install() {
        let release = release();
        let mismatch = RECORD_MISMATCH;
        assert_record_refused(&release, mismatch, |_, m| {
            m.metadata_sha256 = "0".repeat(64)
        });
        assert_record_refused(&release, mismatch, |_, m| m.version = 2);
        assert_record_refused(&release, mismatch, |_, m| {
            m.installation.voice = "someone".into()
        });
        let other = String::from_utf8(release.clone())
            .unwrap()
            .replace(REVISION, "fedcba9876543210");
        assert_refused(&release, mismatch, |root| {
            fs::write(root.join("metadata.json"), &other).unwrap();
            edit_record(root, |m| {
                m.metadata_sha256 = metadata_hash(other.as_bytes())
            });
        });
    }
    #[test]
    fn a_record_that_names_other_files_than_the_folder_holds_is_not_a_moved_one() {
        let release = release();
        let mismatch = RECORD_MISMATCH;
        assert_record_refused(&release, mismatch, |_, m| {
            m.installation.model.set_file_name("other.gguf")
        });
        assert_record_refused(&release, mismatch, |root, m| {
            m.installation.model = root.join("test-voice.gguf")
        });
        assert_record_refused(&release, mismatch, |_, m| {
            m.installation.executable = PathBuf::from("speech.exe")
        });
    }
    #[test]
    fn a_moved_installation_with_a_damaged_file_is_refused_and_the_old_record_kept() {
        let release = release();
        let data = metadata_of(&release).unwrap();
        assert_refused(
            &release,
            "SHA256が一致しません: vulkan-shim.dll",
            |root| fixtures::corrupt(&runtime_folder(root, &data).join("vulkan-shim.dll")),
        );
        assert_refused(
            &release,
            "SHA256が一致しません: test-voice.gguf",
            |root| fixtures::corrupt(&model_file(root, &data)),
        );
        assert_refused(&release, "サイズ", |root| {
            fs::write(model_file(root, &data), &MODEL[1..]).unwrap()
        });
        assert_refused(&release, "SHA256が一致しません: README.md", |root| {
            fixtures::corrupt(&root.join("notices/README.md"))
        });
    }
    #[test]
    fn pins_are_exact_and_do_not_require_a_reference_voice() {
        let metadata = metadata().unwrap();
        assert_eq!(metadata.model.artifact.bytes, 1_920_321_792);
        assert_eq!(
            metadata.model.artifact.sha256,
            "e237dc3eefe469b8bba87872beacf12fb7fe00dbdf85767edca8ec04ab910d13"
        );
        assert_eq!(metadata.runtime.artifact.bytes, 29_548_632);
        assert_eq!(metadata.runtime.executable, "speech.exe");
        assert_eq!(metadata.notices.len(), 3);
        assert_eq!(
            expected(Path::new("C:/speech"), "cpu", &metadata).voice,
            "none"
        );
    }
    #[test]
    fn cancelled_setup_preserves_the_existing_manifest() {
        let root = std::env::temp_dir().join(format!("speech-test-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&root).unwrap();
        fs::write(root.join("installation.json"), b"previous").unwrap();
        assert!(install(&root, "cpu", Arc::new(AtomicBool::new(true)), |_| {}).is_err());
        assert_eq!(
            fs::read(root.join("installation.json")).unwrap(),
            b"previous"
        );
        // An uncancelled retry also preserves a manifest from another tool;
        // it must fail before any network request or installation replacement.
        assert!(install(&root, "cpu", Arc::new(AtomicBool::new(false)), |_| {}).is_err());
        assert_eq!(
            fs::read(root.join("installation.json")).unwrap(),
            b"previous"
        );
        fs::remove_file(root.join("installation.json")).unwrap();
        fs::remove_dir(root).unwrap();
    }
    #[test]
    fn incomplete_or_escaped_installation_is_never_loaded() {
        let root = std::env::temp_dir().join(format!("speech-test-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&root).unwrap();
        assert!(load(&root).unwrap().is_none());
        let manifest = Manifest {
            version: 1,
            metadata_sha256: metadata_hash(METADATA),
            installation: SpeechInstallation {
                executable: root.join("../speech.exe"),
                model: root.join("../model.gguf"),
                backend: "cpu".into(),
                voice: "none".into(),
            },
        };
        fs::write(
            root.join("installation.json"),
            serde_json::to_vec(&manifest).unwrap(),
        )
        .unwrap();
        assert!(load(&root).is_err());
        fs::remove_file(root.join("installation.json")).unwrap();
        fs::remove_dir(root).unwrap();
    }
    #[test]
    fn a_record_whose_metadata_differs_only_in_line_endings_is_still_this_model() {
        // An early build embedded the metadata with a CRLF at the end, and a
        // checkout with core.autocrlf=true embeds CRLF throughout. The pinned
        // artifacts are the same, so such a record must not lock the voice out.
        let created = std::env::temp_dir().join(format!("speech-test-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&created).unwrap();
        let root = fs::canonicalize(&created).unwrap();
        let text = std::str::from_utf8(METADATA).unwrap();
        let record = |installed: &[u8]| {
            fs::write(root.join("metadata.json"), installed).unwrap();
            let manifest = Manifest {
                version: 1,
                metadata_sha256: format!("{:x}", Sha256::digest(installed)),
                installation: expected(&root, "cpu", &metadata().unwrap()),
            };
            fs::write(
                root.join("installation.json"),
                serde_json::to_vec(&manifest).unwrap(),
            )
            .unwrap();
            // No model is installed here, so loading still fails afterwards;
            // only the record verdict matters to this test.
            load(&root).unwrap_err().to_string()
        };
        let mismatch = "読み上げのインストール記録が選択されたモデルと一致しません。";
        for installed in [
            format!("{}\r\n", text.trim_end_matches('\n')),
            text.replace('\n', "\r\n"),
        ] {
            let verdict = record(installed.as_bytes());
            assert!(
                !verdict.contains(mismatch),
                "rejected as another model: {verdict}"
            );
        }
        let other_model = text.replacen(
            "e237dc3eefe469b8bba87872beacf12fb7fe00dbdf85767edca8ec04ab910d13",
            &"0".repeat(64),
            1,
        );
        assert_ne!(other_model, text);
        assert!(record(other_model.as_bytes()).contains(mismatch));
        fs::remove_dir_all(&created).unwrap();
    }
    #[test]
    fn pending_bytes_are_the_runtime_model_and_notices_that_are_not_saved() {
        let data = metadata().unwrap();
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        let notices: u64 = data.notices.iter().map(|n| n.bytes).sum();
        let runtime = data.runtime.artifact.bytes;
        let model = data.model.artifact.bytes;
        assert_eq!(pending_download_bytes(root), runtime + model + notices);
        let installation = expected(root, "vulkan", &data);
        fs::create_dir_all(installation.model.parent().unwrap()).unwrap();
        fs::File::create(&installation.model)
            .unwrap()
            .set_len(model)
            .unwrap();
        assert_eq!(pending_download_bytes(root), runtime + notices);
        fs::create_dir_all(installation.executable.parent().unwrap()).unwrap();
        assert_eq!(pending_download_bytes(root), notices);
        let folder = root.join("notices");
        fs::create_dir_all(&folder).unwrap();
        for notice in &data.notices {
            fs::File::create(folder.join(&notice.name))
                .unwrap()
                .set_len(notice.bytes)
                .unwrap();
        }
        assert_eq!(pending_download_bytes(root), 0);
    }
}
