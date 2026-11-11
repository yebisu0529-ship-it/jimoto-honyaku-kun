//! Optional Indra CPU installation. Every network asset is pinned and resumable;
//! Python/uv/package installation runs from verified local files, offline.
use crate::{
    hardware,
    installer::{self, Artifact, Progress, RuntimeFile},
    ocr_runtime::ProcessControl,
};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{Arc, atomic::AtomicBool},
    thread,
    time::{Duration, Instant},
};

const METADATA: &[u8] = include_bytes!("../scripts/indra-runtime.json");
const REQUIREMENTS: &[u8] = include_bytes!("../scripts/indra-requirements.lock");
const SETUP: &[u8] = include_bytes!("../scripts/setup-indra.ps1");
const SERVER: &[u8] = include_bytes!("../scripts/indra_server.py");

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct IndraInstallation {
    pub version: u32,
    pub python_path: PathBuf,
    pub model_path: PathBuf,
    pub sdk_version: String,
    pub model_revision: String,
}
#[derive(Deserialize)]
struct VersionedArtifact {
    #[serde(flatten)]
    artifact: Artifact,
    version: String,
}
#[derive(Deserialize)]
struct Metadata {
    version: u32,
    sdk_version: String,
    model_revision: String,
    uv: VersionedArtifact,
    python: VersionedArtifact,
    models: Vec<Artifact>,
    wheels: Vec<Artifact>,
    notices: Vec<Artifact>,
    requirements_sha256: String,
}
#[derive(Serialize, Deserialize)]
struct Manifest {
    metadata_sha256: String,
    setup_sha256: String,
    server_sha256: String,
    python_sha256: String,
    installation: IndraInstallation,
}
#[derive(Serialize, Deserialize)]
struct RuntimeIndex {
    artifact: Artifact,
    files: Vec<RuntimeFile>,
}
pub(crate) fn metadata_hash() -> String {
    hash(METADATA)
}
fn hash(data: &[u8]) -> String {
    format!("{:x}", Sha256::digest(data))
}
fn metadata() -> Result<Metadata> {
    let data: Metadata = serde_json::from_slice(METADATA)?;
    ensure!(
        data.version == 1
            && data.models.len() == 9
            && data.wheels.len() == 23
            && hash(REQUIREMENTS) == data.requirements_sha256,
        "Indraの固定配布情報が不正です。"
    );
    Ok(data)
}
pub(crate) fn downloads() -> Result<Vec<Artifact>> {
    let data = metadata()?;
    Ok([data.uv.artifact, data.python.artifact]
        .into_iter()
        .chain(data.wheels)
        .chain(data.models)
        .chain(data.notices)
        .collect())
}
fn expected(root: &Path, data: &Metadata) -> IndraInstallation {
    IndraInstallation {
        version: 1,
        python_path: root.join(".venv/Scripts/python.exe"),
        model_path: root.join("models").join(&data.model_revision),
        sdk_version: data.sdk_version.clone(),
        model_revision: data.model_revision.clone(),
    }
}
fn read_small(path: &Path) -> Result<Vec<u8>> {
    installer::safe_file(path)?;
    ensure!(
        fs::metadata(path)?.len() <= 4 * 1024 * 1024,
        "Indraのインストール記録が大きすぎます。"
    );
    Ok(fs::read(path)?)
}
fn verify_bytes(path: &Path, bytes: &[u8]) -> Result<()> {
    ensure!(
        read_small(path)? == bytes,
        "保存済みのIndra実行情報が一致しません。"
    );
    Ok(())
}
pub(crate) fn ordinary_path(path: &Path) -> Result<PathBuf> {
    let text = path
        .to_str()
        .context("Indraの保存先をUnicodeで読み取れません。")?;
    Ok(PathBuf::from(text.strip_prefix("\\\\?\\").unwrap_or(text)))
}
pub(crate) fn installation_root(config: &IndraInstallation) -> Result<PathBuf> {
    let root = config
        .model_path
        .parent()
        .and_then(Path::parent)
        .context("Indraモデルの配置が不正です。")?;
    ensure!(
        load_verified(root)?.as_ref() == Some(config),
        "Indraのインストール記録が選択された環境と一致しません。"
    );
    Ok(root.into())
}

/// Cheap start check of a fully committed installation: its record, the
/// pinned files beside it and the size of every model and notice. No file is
/// hashed here; `installation_root` hashes them when Indra starts. Never
/// installs or downloads.
pub fn load(root: &Path) -> Result<Option<IndraInstallation>> {
    Ok(committed(root)?.map(|(_, manifest)| manifest.installation))
}

/// `load`, then every model, every notice and the Python executable hashed.
fn load_verified(root: &Path) -> Result<Option<IndraInstallation>> {
    let Some((root, manifest)) = committed(root)? else {
        return Ok(None);
    };
    let data = metadata()?;
    let cancel = AtomicBool::new(false);
    for artifact in &data.models {
        installer::verify_file(
            &manifest.installation.model_path.join(&artifact.name),
            artifact,
            &cancel,
        )?;
    }
    for artifact in &data.notices {
        installer::verify_file(
            &root.join("notices").join(&artifact.name),
            artifact,
            &cancel,
        )?;
    }
    ensure!(
        hash(&fs::read(&manifest.installation.python_path)?) == manifest.python_sha256,
        "Indra専用Pythonの実行ファイルが変更されています。"
    );
    Ok(Some(manifest.installation))
}

/// What `load` checks, with the folder and the record the hashes are read against.
fn committed(root: &Path) -> Result<Option<(PathBuf, Manifest)>> {
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
    ensure!(
        manifest.metadata_sha256 == metadata_hash()
            && manifest.server_sha256 == hash(SERVER)
            && manifest.setup_sha256 == hash(SETUP)
            && manifest.installation == expected(&root, &data),
        "Indraのインストール記録が固定版と一致しません。"
    );
    for (path, bytes) in [
        (root.join("metadata.json"), METADATA),
        (root.join("runtime/indra_server.py"), SERVER),
        (root.join("runtime/setup-indra.ps1"), SETUP),
        (root.join("runtime/indra-requirements.lock"), REQUIREMENTS),
    ] {
        verify_bytes(&path, bytes)?;
    }
    for (folder, artifacts) in [
        (&manifest.installation.model_path, &data.models),
        (&root.join("notices"), &data.notices),
    ] {
        for artifact in artifacts {
            let path = folder.join(&artifact.name);
            installer::safe_file(&path)?;
            ensure!(
                fs::metadata(&path)?.len() == artifact.bytes,
                "ファイルサイズが一致しません: {}",
                artifact.name
            );
        }
    }
    installer::safe_file(&manifest.installation.python_path)?;
    ensure!(
        fs::metadata(&manifest.installation.python_path)?.len() <= 64 * 1024 * 1024,
        "Indra専用Pythonの実行ファイルが変更されています。"
    );
    let cfg = String::from_utf8(read_small(&root.join(".venv/pyvenv.cfg"))?)?;
    ensure!(
        !cfg.to_ascii_lowercase()
            .contains("include-system-site-packages = true"),
        "Indra環境にシステムのパッケージが混在しています。"
    );
    Ok(Some((root, manifest)))
}
fn prepare_uv(
    root: &Path,
    spec: &VersionedArtifact,
    cancel: &AtomicBool,
    progress: &mut impl FnMut(Progress),
) -> Result<()> {
    let destination = root.join("tools").join(format!("uv-{}", spec.version));
    if destination.exists() {
        let index: RuntimeIndex =
            serde_json::from_slice(&read_small(&destination.join("runtime.json"))?)?;
        ensure!(
            index.artifact == spec.artifact && index.files.iter().any(|f| f.path == "uv.exe"),
            "保存済みuvの配布情報が一致しません。"
        );
        for file in index.files {
            let relative = installer::archive_path(&file.path)?;
            installer::verify_file(
                &destination.join(relative),
                &Artifact {
                    name: file.path,
                    url: String::new(),
                    bytes: file.bytes,
                    sha256: file.sha256,
                },
                cancel,
            )?;
        }
        return Ok(());
    }
    let staging = root
        .join("tools")
        .join(format!(".staging-{}", uuid::Uuid::new_v4()));
    installer::safe_components(&staging, true)?;
    let _guard = installer::StagingGuard {
        path: staging.clone(),
        parent: root.join("tools"),
    };
    let files = installer::unpack_runtime(
        &[root.join("downloads").join(&spec.artifact.name)],
        &staging,
        cancel,
        progress,
        "uv.exe",
    )?;
    installer::atomic_write(
        &staging.join("runtime.json"),
        &serde_json::to_vec_pretty(&RuntimeIndex {
            artifact: spec.artifact.clone(),
            files,
        })?,
    )?;
    installer::cancelled(cancel)?;
    ensure!(
        !destination.exists(),
        "Indra用uvの保存先がほかの処理で作成されました。"
    );
    fs::rename(staging, destination)?;
    Ok(())
}
fn retain_sdk_notice(root: &Path, wheels: &[Artifact]) -> Result<()> {
    let sdk = wheels
        .iter()
        .find(|a| a.name.starts_with("indra_ocr-"))
        .context("Indra SDKのwheel情報がありません。")?;
    let mut archive = zip::ZipArchive::new(fs::File::open(root.join("wheels").join(&sdk.name))?)?;
    let mut copied = false;
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i)?;
        if entry.name().ends_with(".dist-info/licenses/LICENSE") {
            ensure!(
                entry.size() <= 1024 * 1024,
                "SDKライセンスのサイズが不正です。"
            );
            let mut bytes = Vec::new();
            entry.read_to_end(&mut bytes)?;
            installer::atomic_write(&root.join("notices/SDK-LICENSE.txt"), &bytes)?;
            copied = true;
        }
    }
    ensure!(copied, "Indra SDKのライセンスを確認できません。");
    Ok(())
}
/// Explicit opt-in setup. Call only after the user has seen the separate model terms.
pub fn install(
    root: &Path,
    cancel: Arc<AtomicBool>,
    mut on_progress: impl FnMut(Progress),
) -> Result<IndraInstallation> {
    installer::cancelled(&cancel)?;
    ensure!(
        cfg!(all(windows, target_arch = "x86_64")),
        "Indraの配布版はWindows x64専用です。"
    );
    installer::safe_components(root, true)?;
    let root = fs::canonicalize(root)?;
    let _lock = installer::RootLock::acquire(&root)?;
    if let Some(existing) = load_verified(&root)
        .context("既存のIndra環境を保持しました。インストール記録を確認してください。")?
    {
        return Ok(existing);
    }
    let data = metadata()?;
    let installation = expected(&root, &data);
    for folder in ["downloads", "wheels", "notices", "runtime", "tools", "logs"] {
        installer::safe_components(&root.join(folder), true)?;
    }
    installer::safe_components(&installation.model_path, true)?;
    let mut assets: Vec<(&Artifact, PathBuf, &str)> = Vec::new();
    for artifact in [&data.uv.artifact, &data.python.artifact] {
        assets.push((
            artifact,
            root.join("downloads").join(&artifact.name),
            "indra_runtime_download",
        ));
    }
    for artifact in &data.wheels {
        assets.push((
            artifact,
            root.join("wheels").join(&artifact.name),
            "indra_wheel_download",
        ));
    }
    for artifact in &data.models {
        assets.push((
            artifact,
            installation.model_path.join(&artifact.name),
            "indra_model_download",
        ));
    }
    for artifact in &data.notices {
        assets.push((
            artifact,
            root.join("notices").join(&artifact.name),
            "indra_notices",
        ));
    }
    let mut remaining = 0;
    for (artifact, path, _) in &assets {
        remaining += installer::remaining_bytes(artifact, path)?;
    }
    let required = remaining + 1024 * 1024 * 1024;
    let free = hardware::detect(&root)?
        .disk_free_bytes
        .context("Indraの保存先の空き容量を確認できませんでした。")?;
    ensure!(
        free >= required,
        "Indraの準備に必要な空き容量が足りません（必要 {:.1} GB、空き {:.1} GB）。",
        required as f64 / 1e9,
        free as f64 / 1e9
    );
    for (artifact, path, stage) in assets {
        installer::download_official(artifact, &path, &cancel, stage, &mut on_progress)?;
    }
    prepare_uv(&root, &data.uv, &cancel, &mut on_progress)?;
    retain_sdk_notice(&root, &data.wheels)?;
    for (path, bytes) in [
        (root.join("metadata.json"), METADATA),
        (root.join("runtime/indra_server.py"), SERVER),
        (root.join("runtime/setup-indra.ps1"), SETUP),
        (root.join("runtime/indra-requirements.lock"), REQUIREMENTS),
    ] {
        installer::atomic_write(&path, bytes)?;
    }
    installer::cancelled(&cancel)?;
    on_progress(Progress {
        stage: "indra_environment".into(),
        downloaded_bytes: 0,
        total_bytes: None,
        message: "Indra専用Pythonと固定パッケージをオフラインで準備しています。".into(),
    });
    let log_path = root.join("logs/setup.log");
    installer::safe_file(&log_path)?;
    let log = fs::File::create(&log_path)?;
    let control = ProcessControl::new();
    let mut command = Command::new("powershell.exe");
    command
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-File",
        ])
        .arg(ordinary_path(&root.join("runtime/setup-indra.ps1"))?)
        .arg("-InstallRoot")
        .arg(ordinary_path(&root)?)
        .stdin(Stdio::null())
        .stdout(log.try_clone()?)
        .stderr(log);
    control.spawn(&mut command)?;
    let deadline = Instant::now() + Duration::from_secs(10 * 60);
    loop {
        if cancel.load(std::sync::atomic::Ordering::Acquire) {
            control.stop();
            installer::cancelled(&cancel)?;
        }
        if let Some(status) = control.try_wait()? {
            ensure!(
                status.success(),
                "Indraの環境を準備できませんでした。取得済みファイルは保持しています。実行ログ: {}",
                log_path.display()
            );
            break;
        }
        ensure!(
            Instant::now() < deadline,
            "Indraの環境準備が時間内に完了しませんでした。実行ログ: {}",
            log_path.display()
        );
        thread::sleep(Duration::from_millis(100));
    }
    control.stop();
    installer::cancelled(&cancel)?;
    installer::safe_file(&installation.python_path)?;
    let python_sha256 = hash(&fs::read(&installation.python_path)?);
    installer::atomic_write(
        &root.join("installation.json"),
        &serde_json::to_vec_pretty(&Manifest {
            metadata_sha256: metadata_hash(),
            setup_sha256: hash(SETUP),
            server_sha256: hash(SERVER),
            python_sha256,
            installation: installation.clone(),
        })?,
    )?;
    ensure!(
        load_verified(&root)?.as_ref() == Some(&installation),
        "Indraのインストール完了記録を確認できませんでした。"
    );
    on_progress(Progress {
        stage: "complete".into(),
        downloaded_bytes: 54_007_683,
        total_bytes: Some(54_007_683),
        message: "Indraの日英OCRの準備ができました。".into(),
    });
    Ok(installation)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore = "downloads optional pinned OCR assets into explicit env-selected root"]
    fn indra_installation_opt_in() {
        assert_eq!(std::env::var("INDRA_NATIVE_VERIFY").as_deref(), Ok("1"));
        let root =
            PathBuf::from(std::env::var_os("INDRA_VERIFY_ROOT").expect("dedicated root required"));
        assert!(root.is_absolute());
        let installation = install(&root, Arc::new(AtomicBool::new(false)), |p| {
            eprintln!("{} {} {:?}", p.stage, p.downloaded_bytes, p.total_bytes)
        })
        .unwrap();
        assert_eq!(load(&root).unwrap(), Some(installation));
    }
    #[test]
    fn model_and_all_wheels_are_fixed() {
        let data = metadata().unwrap();
        assert_eq!(data.models.iter().map(|a| a.bytes).sum::<u64>(), 54_007_683);
        assert_eq!(data.python.version, "3.13.14");
        for a in data.wheels {
            assert!(a.name.ends_with(".whl"));
            assert!(
                std::str::from_utf8(REQUIREMENTS)
                    .unwrap()
                    .contains(&a.sha256)
            );
            assert!(a.url.starts_with("https://files.pythonhosted.org/"));
        }
    }
    /// A folder as `install` leaves it, except that every model and notice has
    /// only its recorded size, not its content.
    fn prepared(root: &Path) -> IndraInstallation {
        let root = fs::canonicalize(root).unwrap();
        let data = metadata().unwrap();
        let installation = expected(&root, &data);
        let python = b"python".as_slice();
        for (path, bytes) in [
            (root.join("metadata.json"), METADATA),
            (root.join("runtime/indra_server.py"), SERVER),
            (root.join("runtime/setup-indra.ps1"), SETUP),
            (root.join("runtime/indra-requirements.lock"), REQUIREMENTS),
            (
                root.join(".venv/pyvenv.cfg"),
                b"home = C:\\Python\n".as_slice(),
            ),
            (installation.python_path.clone(), python),
        ] {
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, bytes).unwrap();
        }
        for (folder, artifacts) in [
            (installation.model_path.clone(), &data.models),
            (root.join("notices"), &data.notices),
        ] {
            fs::create_dir_all(&folder).unwrap();
            for artifact in artifacts {
                fs::File::create(folder.join(&artifact.name))
                    .unwrap()
                    .set_len(artifact.bytes)
                    .unwrap();
            }
        }
        let manifest = Manifest {
            metadata_sha256: metadata_hash(),
            setup_sha256: hash(SETUP),
            server_sha256: hash(SERVER),
            python_sha256: hash(python),
            installation: installation.clone(),
        };
        fs::write(
            root.join("installation.json"),
            serde_json::to_vec_pretty(&manifest).unwrap(),
        )
        .unwrap();
        installation
    }
    #[test]
    fn the_start_check_reads_sizes_and_only_starting_indra_reads_the_models() {
        let root = std::env::temp_dir().join(format!("indra-start-check-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&root).unwrap();
        let installation = prepared(&root);
        assert_eq!(
            load(&root).unwrap(),
            Some(installation.clone()),
            "the start check accepts a prepared folder without hashing it"
        );
        let refused = installation_root(&installation).unwrap_err().to_string();
        assert!(
            refused.contains("SHA256が一致しません"),
            "starting Indra still refuses a model with the right size and wrong content: {refused}"
        );
        let model = installation
            .model_path
            .join(&metadata().unwrap().models[0].name);
        fs::OpenOptions::new()
            .write(true)
            .open(model)
            .unwrap()
            .set_len(1)
            .unwrap();
        let refused = load(&root).unwrap_err().to_string();
        assert!(
            refused.contains("ファイルサイズが一致しません"),
            "the start check refuses a model of the wrong size: {refused}"
        );
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn cancelled_setup_preserves_other_data() {
        let root = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
        fs::create_dir(&root).unwrap();
        fs::write(root.join("installation.json"), b"keep").unwrap();
        assert!(install(&root, Arc::new(AtomicBool::new(true)), |_| {}).is_err());
        assert_eq!(fs::read(root.join("installation.json")).unwrap(), b"keep");
        fs::remove_dir_all(root).unwrap();
    }
}
