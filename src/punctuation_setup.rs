//! Isolated, opt-in, SHA-pinned native punctuation installation.
use crate::{
    hardware,
    installer::{self, Artifact, Progress, RuntimeFile, Saved},
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
const PATHS_MISMATCH: &str = "Punctuation paths do not match the isolated installation";

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct PunctuationInstallation {
    pub version: u32,
    pub root: PathBuf,
    pub library: PathBuf,
    pub model: PathBuf,
    pub runtime_version: String,
}
impl PunctuationInstallation {
    /// This record is `expected`, written while the folder was somewhere else.
    /// Only the path fields are left out of the comparison.
    fn moved_to(&self, expected: &Self) -> bool {
        let pinned = Self {
            root: expected.root.clone(),
            library: expected.library.clone(),
            model: expected.model.clone(),
            ..self.clone()
        };
        pinned == *expected
            && self.root.is_absolute()
            && installer::relocated(&expected.root, &expected.library, &self.library)
            && installer::relocated(&expected.root, &expected.model, &self.model)
    }
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
    metadata_of(METADATA)
}
fn metadata_of(release: &[u8]) -> Result<Metadata> {
    let data: Metadata = serde_json::from_slice(release)?;
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
fn digest(release: &[u8]) -> String {
    format!("{:x}", Sha256::digest(release))
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
    read_saved(METADATA, root)?.current(PATHS_MISMATCH)
}

fn read_saved(release: &[u8], root: &Path) -> Result<Saved<PunctuationInstallation>> {
    if !root.exists() {
        return Ok(Saved::Absent);
    }
    installer::safe_components(root, false)?;
    let root = fs::canonicalize(root)?;
    let manifest = root.join("installation.json");
    installer::safe_file(&manifest)?;
    if !manifest.exists() {
        return Ok(Saved::Absent);
    }
    let saved: Manifest = serde_json::from_slice(&read_small(&manifest)?)?;
    let data = metadata_of(release)?;
    ensure!(
        saved.version == 1 && saved.metadata_sha256 == digest(release),
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
    let current = saved.installation == expected;
    ensure!(
        current || saved.installation.moved_to(&expected),
        PATHS_MISMATCH
    );
    Ok(if current {
        Saved::Current(expected)
    } else {
        Saved::Moved
    })
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

/// What `install` would download now: each package unless it is already
/// unpacked, and the license notices that are not saved.
pub fn pending_download_bytes(root: &Path) -> u64 {
    let Ok(data) = metadata() else { return 0 };
    let mut files = Vec::new();
    for (kind, spec) in [("runtime", &data.runtime), ("models", &data.model)] {
        if !package_root(root, kind, spec).exists() {
            files.push((spec, root.join("downloads").join(&spec.name)));
        }
    }
    files.extend(
        data.notices
            .iter()
            .map(|notice| (notice, root.join("licenses").join(&notice.name))),
    );
    installer::remaining_total(&files)
}

pub fn install(
    root: &Path,
    cancel: Arc<AtomicBool>,
    on_progress: impl FnMut(Progress),
) -> Result<PunctuationInstallation> {
    install_release(METADATA, root, cancel, on_progress)
}

/// `install` for the pinned `release`, which tests replace with a small one.
fn install_release(
    release: &[u8],
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
    // A record of a moved folder is not returned. The checks below run again
    // at this folder's paths, and the record is written for them.
    if let Saved::Current(existing) = read_saved(release, &root)? {
        return Ok(existing);
    }
    let data = metadata_of(release)?;
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
    installer::atomic_write(&root.join("metadata.json"), release)?;
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
            metadata_sha256: digest(release),
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
    use crate::installer::fixtures::{self, Folder};

    const LICENSE: &[u8] = b"test licence text";
    const MODEL_NOTICE: &str = "test model notice";

    fn release() -> Vec<u8> {
        serde_json::to_vec_pretty(&serde_json::json!({
            "version": 1,
            "runtime_version": "9.9.9",
            "runtime": fixtures::artifact("runtime.tar.bz2", b"runtime archive"),
            "model": fixtures::artifact("model.tar.bz2", b"model archive"),
            "notices": [fixtures::artifact("test-LICENSE.txt", LICENSE)],
            "model_license_notice": MODEL_NOTICE,
        }))
        .unwrap()
    }
    /// What a real install leaves under `root`: the packages unpacked and the
    /// notices saved, then the record.
    fn installed(root: &Path, release: &[u8]) -> PunctuationInstallation {
        let data = metadata_of(release).unwrap();
        for (kind, spec, paths) in [
            (
                "runtime",
                &data.runtime,
                ["lib/sherpa-onnx-c-api.dll", "lib/onnxruntime.dll"],
            ),
            ("models", &data.model, ["model.int8.onnx", "tokens.txt"]),
        ] {
            let package = package_root(root, kind, spec);
            let files = paths
                .iter()
                .map(|path| fixtures::package_file(&package, path, path.as_bytes()))
                .collect();
            let index = Package {
                artifact: spec.clone(),
                files,
            };
            fixtures::write(
                &package.join("package.json"),
                &serde_json::to_vec(&index).unwrap(),
            );
        }
        for notice in &data.notices {
            fixtures::write(&root.join("licenses").join(&notice.name), LICENSE);
        }
        install_at(release, root).0.unwrap()
    }
    /// The install, and the stages it reported.
    fn install_at(release: &[u8], root: &Path) -> (Result<PunctuationInstallation>, Vec<String>) {
        let mut stages = Vec::new();
        let result = install_release(
            release,
            root,
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
    fn moved(folder: &Folder, release: &[u8]) -> (PathBuf, PathBuf, PunctuationInstallation) {
        let before = folder.path.join("before").join("text-punctuation");
        let after = folder.path.join("after").join("text-punctuation");
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
            PunctuationInstallation {
                root: after.clone(),
                library: here(&was.library),
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
        let source = folder.path.join("source").join("text-punctuation");
        let copy = folder.path.join("copy").join("text-punctuation");
        installed(&source, &release);
        fixtures::copy_tree(&source, &copy);
        let files = fixtures::snapshot(&source);
        assert!(matches!(read_saved(&release, &copy), Ok(Saved::Moved)));
        install_at(&release, &copy).0.unwrap();
        assert_eq!(fixtures::snapshot(&source), files);
        assert!(matches!(
            read_saved(&release, &copy),
            Ok(Saved::Current(now)) if now.root == copy
        ));
    }
    #[test]
    fn a_moved_record_of_another_pinned_release_still_blocks_the_install() {
        let release = release();
        let metadata = "Punctuation installation metadata mismatch";
        assert_record_refused(&release, metadata, |_, m| {
            m.metadata_sha256 = "0".repeat(64)
        });
        assert_record_refused(&release, metadata, |_, m| m.version = 2);
        assert_record_refused(&release, PATHS_MISMATCH, |_, m| {
            m.installation.runtime_version = "0.0.1".into()
        });
        assert_record_refused(&release, PATHS_MISMATCH, |_, m| m.installation.version = 2);
    }
    #[test]
    fn a_record_that_names_other_files_than_the_folder_holds_is_not_a_moved_one() {
        let release = release();
        assert_record_refused(&release, PATHS_MISMATCH, |_, m| {
            m.installation.model.set_file_name("other.onnx")
        });
        assert_record_refused(&release, PATHS_MISMATCH, |root, m| {
            m.installation.library = root.join("sherpa-onnx-c-api.dll")
        });
        assert_record_refused(&release, PATHS_MISMATCH, |_, m| {
            m.installation.root = PathBuf::from("text-punctuation")
        });
    }
    #[test]
    fn a_moved_installation_with_a_damaged_file_is_refused_and_the_old_record_kept() {
        let release = release();
        let data = metadata_of(&release).unwrap();
        let runtime = |root: &Path| package_root(root, "runtime", &data.runtime);
        let model = |root: &Path| package_root(root, "models", &data.model);
        assert_refused(
            &release,
            "SHA256が一致しません: lib/onnxruntime.dll",
            |root| fixtures::corrupt(&runtime(root).join("lib/onnxruntime.dll")),
        );
        assert_refused(
            &release,
            "SHA256が一致しません: model.int8.onnx",
            |root| fixtures::corrupt(&model(root).join("model.int8.onnx")),
        );
        assert_refused(
            &release,
            "SHA256が一致しません: test-LICENSE.txt",
            |root| fixtures::corrupt(&root.join("licenses/test-LICENSE.txt")),
        );
        assert_refused(&release, "Model license notice mismatch", |root| {
            fs::write(root.join("licenses/model-NOTICE.txt"), "changed").unwrap()
        });
    }
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
    #[test]
    fn pending_bytes_are_the_packages_and_notices_that_are_not_saved() {
        let data = metadata().unwrap();
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        let notices: u64 = data.notices.iter().map(|n| n.bytes).sum();
        assert_eq!(
            pending_download_bytes(root),
            data.runtime.bytes + data.model.bytes + notices
        );
        for (kind, spec) in [("runtime", &data.runtime), ("models", &data.model)] {
            fs::create_dir_all(package_root(root, kind, spec)).unwrap();
        }
        assert_eq!(pending_download_bytes(root), notices);
        let licenses = root.join("licenses");
        fs::create_dir_all(&licenses).unwrap();
        for notice in &data.notices {
            File::create(licenses.join(&notice.name))
                .unwrap()
                .set_len(notice.bytes)
                .unwrap();
        }
        assert_eq!(pending_download_bytes(root), 0);
    }
}
