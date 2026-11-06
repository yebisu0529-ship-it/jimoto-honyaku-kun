//! Isolated, opt-in, SHA-pinned native punctuation installation.
use crate::{
    hardware,
    installer::{self, Artifact, Progress, RuntimeFile},
};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::{Arc, atomic::AtomicBool},
};

const METADATA: &[u8] = include_bytes!("../scripts/punctuation-runtime.json");
const UNPACK_LIMIT: u64 = 512 * 1024 * 1024;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct PunctuationInstallation {
    pub version: u32,
    pub root: PathBuf,
    pub library: PathBuf,
    pub model: PathBuf,
    pub runtime_version: String,
}
#[derive(Deserialize)]
struct Metadata {
    version: u32,
    runtime_version: String,
    runtime: Artifact,
    model: Artifact,
    notices: Vec<Artifact>,
    model_license_notice: String,
}
#[derive(Serialize, Deserialize)]
struct Package {
    artifact: Artifact,
    files: Vec<RuntimeFile>,
}
#[derive(Serialize, Deserialize)]
struct Manifest {
    version: u32,
    metadata_sha256: String,
    installation: PunctuationInstallation,
}
fn metadata() -> Result<Metadata> {
    let data: Metadata = serde_json::from_slice(METADATA)?;
    ensure!(data.version == 1, "Unsupported punctuation metadata");
    Ok(data)
}
pub(crate) fn downloads() -> Result<Vec<Artifact>> {
    let data = metadata()?;
    Ok([data.runtime, data.model]
        .into_iter()
        .chain(data.notices)
        .collect())
}
fn digest() -> String {
    format!("{:x}", Sha256::digest(METADATA))
}
fn package_root(root: &Path, kind: &str, spec: &Artifact) -> PathBuf {
    root.join(kind).join(&spec.sha256[..16])
}
fn read_small(path: &Path) -> Result<Vec<u8>> {
    installer::safe_file(path)?;
    ensure!(
        fs::metadata(path)?.len() <= 4 * 1024 * 1024,
        "Punctuation installation index exceeds bound"
    );
    Ok(fs::read(path)?)
}

fn verify_package(
    root: &Path,
    spec: &Artifact,
    cancel: &AtomicBool,
    wanted: &str,
) -> Result<PathBuf> {
    installer::safe_components(root, false)?;
    let index: Package = serde_json::from_slice(&read_small(&root.join("package.json"))?)?;
    ensure!(
        index.artifact == *spec && !index.files.is_empty() && index.files.len() <= 2_000,
        "Punctuation package metadata mismatch"
    );
    let mut found = None;
    for file in index.files {
        let relative = installer::archive_path(&file.path)?;
        let path = root.join(&relative);
        installer::verify_file(
            &path,
            &Artifact {
                name: file.path.clone(),
                url: String::new(),
                bytes: file.bytes,
                sha256: file.sha256,
            },
            cancel,
        )?;
        if relative.file_name().is_some_and(|n| n == wanted) {
            ensure!(found.is_none(), "Ambiguous punctuation package entry");
            found = Some(path);
        }
    }
    found.context("Required punctuation package entry missing")
}

/// Hash every installed model/DLL against the committed package inventory.
pub fn load(root: &Path) -> Result<Option<PunctuationInstallation>> {
    if !root.exists() {
        return Ok(None);
    }
    installer::safe_components(root, false)?;
    let root = fs::canonicalize(root)?;
    let manifest = root.join("installation.json");
    installer::safe_file(&manifest)?;
    if !manifest.exists() {
        return Ok(None);
    }
    let saved: Manifest = serde_json::from_slice(&read_small(&manifest)?)?;
    let data = metadata()?;
    ensure!(
        saved.version == 1 && saved.metadata_sha256 == digest(),
        "Punctuation installation metadata mismatch"
    );
    let cancel = AtomicBool::new(false);
    let library = verify_package(
        &package_root(&root, "runtime", &data.runtime),
        &data.runtime,
        &cancel,
        "sherpa-onnx-c-api.dll",
    )?;
    let model = verify_package(
        &package_root(&root, "models", &data.model),
        &data.model,
        &cancel,
        "model.int8.onnx",
    )?;
    for notice in &data.notices {
        installer::verify_file(&root.join("licenses").join(&notice.name), notice, &cancel)?;
    }
    let model_notice = root.join("licenses/model-NOTICE.txt");
    installer::safe_file(&model_notice)?;
    ensure!(
        fs::read(model_notice)? == data.model_license_notice.as_bytes(),
        "Model license notice mismatch"
    );
    let expected = PunctuationInstallation {
        version: 1,
        root,
        library,
        model,
        runtime_version: data.runtime_version,
    };
    ensure!(
        saved.installation == expected,
        "Punctuation paths do not match the isolated installation"
    );
    Ok(Some(expected))
}

fn unpack(archive: &Path, destination: &Path, cancel: &AtomicBool) -> Result<Vec<RuntimeFile>> {
    let input = bzip2::read::BzDecoder::new(File::open(archive)?);
    let mut tar = tar::Archive::new(input);
    let mut files = Vec::new();
    let mut total = 0u64;
    let mut entries = 0usize;
    for item in tar.entries()? {
        installer::cancelled(cancel)?;
        entries += 1;
        ensure!(entries <= 2_000, "Too many punctuation archive entries");
        let mut entry = item?;
        let relative = installer::archive_path(
            entry
                .path()?
                .to_str()
                .context("Non-UTF8 punctuation archive path")?,
        )?;
        let target = destination.join(&relative);
        let kind = entry.header().entry_type();
        if kind.is_dir() {
            installer::safe_components(&target, true)?;
            continue;
        }
        ensure!(
            kind == tar::EntryType::Regular,
            "Links and special punctuation archive entries are forbidden"
        );
        let size = entry.header().size()?;
        total = total
            .checked_add(size)
            .context("Punctuation archive size overflow")?;
        ensure!(
            total <= UNPACK_LIMIT,
            "Punctuation archive exceeds expansion bound"
        );
        installer::safe_components(target.parent().context("Archive parent missing")?, true)?;
        installer::safe_file(&target)?;
        let mut out = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&target)?;
        let mut hash = Sha256::new();
        let mut copied = 0u64;
        let mut buffer = [0u8; 128 * 1024];
        loop {
            installer::cancelled(cancel)?;
            let n = entry.read(&mut buffer)?;
            if n == 0 {
                break;
            }
            copied += n as u64;
            ensure!(copied <= size, "Archive size mismatch");
            out.write_all(&buffer[..n])?;
            hash.update(&buffer[..n]);
        }
        ensure!(copied == size, "Truncated punctuation archive entry");
        out.sync_all()?;
        files.push(RuntimeFile {
            path: relative.to_string_lossy().replace('\\', "/"),
            bytes: size,
            sha256: format!("{:x}", hash.finalize()),
        });
    }
    ensure!(!files.is_empty(), "Empty punctuation package");
    Ok(files)
}

fn install_package(
    root: &Path,
    kind: &str,
    spec: &Artifact,
    cancel: &AtomicBool,
    callback: &mut impl FnMut(Progress),
    wanted: &str,
) -> Result<PathBuf> {
    let destination = package_root(root, kind, spec);
    if destination.exists() {
        return verify_package(&destination, spec, cancel, wanted);
    }
    let archive = root.join("downloads").join(&spec.name);
    installer::download_official(spec, &archive, cancel, "punctuation_download", callback)?;
    installer::safe_components(destination.parent().unwrap(), true)?;
    let staging = destination
        .parent()
        .unwrap()
        .join(format!(".staging-{}", uuid::Uuid::new_v4()));
    installer::safe_components(&staging, true)?;
    let _guard = installer::StagingGuard {
        path: staging.clone(),
        parent: destination.parent().unwrap().into(),
    };
    let files = unpack(&archive, &staging, cancel)?;
    installer::atomic_write(
        &staging.join("package.json"),
        &serde_json::to_vec_pretty(&Package {
            artifact: spec.clone(),
            files,
        })?,
    )?;
    verify_package(&staging, spec, cancel, wanted)?;
    installer::cancelled(cancel)?;
    ensure!(
        !destination.exists(),
        "Punctuation package destination was created concurrently"
    );
    fs::rename(&staging, &destination)?;
    verify_package(&destination, spec, cancel, wanted)
}

pub fn install(
    root: &Path,
    cancel: Arc<AtomicBool>,
    mut on_progress: impl FnMut(Progress),
) -> Result<PunctuationInstallation> {
    installer::cancelled(&cancel)?;
    ensure!(
        cfg!(all(windows, target_arch = "x86_64")),
        "Native punctuation supports Windows x64 only"
    );
    ensure!(
        root.is_absolute(),
        "Punctuation storage path must be absolute"
    );
    installer::safe_components(root, true)?;
    let root = fs::canonicalize(root)?;
    let _lock = installer::RootLock::acquire(&root)?;
    if let Some(existing) = load(&root)? {
        return Ok(existing);
    }
    let data = metadata()?;
    installer::safe_components(&root.join("downloads"), true)?;
    installer::safe_components(&root.join("licenses"), true)?;
    let free = hardware::detect(&root)?
        .disk_free_bytes
        .context("Cannot determine punctuation storage capacity")?;
    ensure!(
        free >= 512 * 1024 * 1024,
        "Punctuation setup requires at least 512 MiB free storage"
    );
    let library = install_package(
        &root,
        "runtime",
        &data.runtime,
        &cancel,
        &mut on_progress,
        "sherpa-onnx-c-api.dll",
    )?;
    let model = install_package(
        &root,
        "models",
        &data.model,
        &cancel,
        &mut on_progress,
        "model.int8.onnx",
    )?;
    for notice in &data.notices {
        installer::download_official(
            notice,
            &root.join("licenses").join(&notice.name),
            &cancel,
            "punctuation_notices",
            &mut on_progress,
        )?;
    }
    installer::atomic_write(
        &root.join("licenses/model-NOTICE.txt"),
        data.model_license_notice.as_bytes(),
    )?;
    installer::atomic_write(&root.join("metadata.json"), METADATA)?;
    let installation = PunctuationInstallation {
        version: 1,
        root: root.clone(),
        library,
        model,
        runtime_version: data.runtime_version,
    };
    installer::cancelled(&cancel)?;
    installer::atomic_write(
        &root.join("installation.json"),
        &serde_json::to_vec_pretty(&Manifest {
            version: 1,
            metadata_sha256: digest(),
            installation: installation.clone(),
        })?,
    )?;
    on_progress(Progress {
        stage: "complete".into(),
        downloaded_bytes: data.model.bytes + data.runtime.bytes,
        total_bytes: Some(data.model.bytes + data.runtime.bytes),
        message: "文章区切りの補助を準備しました。".into(),
    });
    Ok(installation)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pinned_artifacts() {
        let data = metadata().unwrap();
        assert_eq!(data.runtime.bytes, 20_962_760);
        assert_eq!(data.model.bytes, 64_717_756);
        assert_eq!(data.notices.len(), 3);
    }
    #[test]
    fn cancelled_install_preserves_existing_manifest() {
        let root = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
        fs::create_dir(&root).unwrap();
        fs::write(root.join("installation.json"), b"unmodified").unwrap();
        assert!(install(&root, Arc::new(AtomicBool::new(true)), |_| {}).is_err());
        assert_eq!(
            fs::read(root.join("installation.json")).unwrap(),
            b"unmodified"
        );
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn tar_links_are_rejected() {
        let root = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
        fs::create_dir(&root).unwrap();
        let archive = root.join("bad.tar.bz2");
        {
            let out = File::create(&archive).unwrap();
            let mut builder = tar::Builder::new(bzip2::write::BzEncoder::new(
                out,
                bzip2::Compression::default(),
            ));
            let mut header = tar::Header::new_gnu();
            header.set_entry_type(tar::EntryType::Symlink);
            header.set_size(0);
            header.set_mode(0o777);
            header.set_cksum();
            builder
                .append_link(&mut header, "link", "../escape")
                .unwrap();
            builder.into_inner().unwrap().finish().unwrap();
        }
        let destination = root.join("stage");
        fs::create_dir(&destination).unwrap();
        assert!(unpack(&archive, &destination, &AtomicBool::new(false)).is_err());
        fs::remove_dir_all(root).unwrap();
    }
}
