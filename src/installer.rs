//! Transactional first-run installer. Only pinned public PFN/GitHub assets are fetched.
//! Model/runtime files become selectable only after verification and manifest commit.
use anyhow::{Context, Result, bail, ensure};
use reqwest::{StatusCode, Url, blocking::Client, header, redirect::Policy};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Component, Path, PathBuf},
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};

const RELEASE: &str = "b12119";
const MAX_UNPACKED: u64 = 2 * 1024 * 1024 * 1024;
const TEMPLATE: &[u8] = include_bytes!("../scripts/plamo_template.jinja");
const TEMPLATE_REVISION: &str = "f2b4599dff9b3e45c62567eac15e83186b4cb64e";
const TEMPLATE_SHA256: &str = "c9139ab4d8be258257f56c6ce6a0b4848f812fe495a86657c1e139d6b0d5819a";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct InstallRequest {
    pub repo: String,
    pub revision: String,
    pub filename: String,
    pub bytes: u64,
    pub sha256: String,
    /// cpu, vulkan, or cuda (the pinned CUDA 12.4 runtime).
    pub backend: String,
    pub install_root: PathBuf,
}

#[derive(Clone, Debug)]
pub struct Progress {
    pub stage: String,
    pub downloaded_bytes: u64,
    pub total_bytes: Option<u64>,
    pub message: String,
}

#[derive(Clone, Debug)]
pub struct InstallResult {
    pub model_path: PathBuf,
    pub llama_server: PathBuf,
    pub template_path: PathBuf,
    pub backend: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct InstallationManifest {
    pub version: u32,
    pub repo: String,
    pub revision: String,
    pub filename: String,
    pub bytes: u64,
    pub sha256: String,
    pub backend: String,
    pub model_path: PathBuf,
    pub llama_server: PathBuf,
    pub template_path: PathBuf,
    pub runtime_version: String,
    pub template_revision: String,
    pub template_sha256: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub(crate) struct Artifact {
    pub(crate) name: String,
    pub(crate) url: String,
    pub(crate) bytes: u64,
    pub(crate) sha256: String,
}
#[derive(Debug, Serialize, Deserialize)]
struct RuntimeManifest {
    version: u32,
    release: String,
    backend: String,
    assets: Vec<Artifact>,
    files: Vec<RuntimeFile>,
}
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct RuntimeFile {
    pub(crate) path: String,
    pub(crate) bytes: u64,
    pub(crate) sha256: String,
}

pub(crate) fn cancelled(cancel: &AtomicBool) -> Result<()> {
    ensure!(
        !cancel.load(Ordering::Relaxed),
        "準備を中止しました。次回は取得途中のデータから再開できます。"
    );
    Ok(())
}
fn valid_hex(s: &str, len: usize) -> bool {
    s.len() == len && s.bytes().all(|c| c.is_ascii_hexdigit())
}
fn validate_request(request: &InstallRequest) -> Result<()> {
    ensure!(
        matches!(
            request.repo.as_str(),
            "pfnet/plamo-3-translate-0.8b-GGUF"
                | "pfnet/plamo-3-translate-2b-GGUF"
                | "pfnet/plamo-3-translate-8b-GGUF"
                | "pfnet/plamo-3-translate-31b-GGUF"
        ),
        "公式PLaMo 3 Translate GGUFリポジトリを指定してください。"
    );
    ensure!(
        valid_hex(&request.revision, 40),
        "モデルの取得先には固定コミットが必要です。"
    );
    ensure!(valid_hex(&request.sha256, 64), "モデルのSHA256が不正です。");
    ensure!(
        request.bytes > 0 && request.bytes <= 128 * 1024 * 1024 * 1024,
        "モデルの固定ファイルサイズが不正です。"
    );
    ensure!(
        !request.filename.is_empty()
            && request.filename.ends_with(".gguf")
            && request
                .filename
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || b"-_.".contains(&c))
            && request.filename.len() < 160
            && !request.filename.starts_with('.'),
        "モデルのファイル名が不正です。"
    );
    ensure!(
        matches!(request.backend.as_str(), "cpu" | "vulkan" | "cuda"),
        "未対応の実行バックエンドです。"
    );
    ensure!(
        request.install_root.is_absolute(),
        "保存先には絶対パスが必要です。"
    );
    Ok(())
}

fn progress(
    callback: &mut impl FnMut(Progress),
    stage: &str,
    current: u64,
    total: Option<u64>,
    message: impl Into<String>,
) {
    callback(Progress {
        stage: stage.into(),
        downloaded_bytes: current,
        total_bytes: total,
        message: message.into(),
    });
}

fn reparse(metadata: &fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        metadata.file_attributes() & 0x400 != 0
    }
    #[cfg(not(windows))]
    {
        metadata.file_type().is_symlink()
    }
}

/// Check every component before creating anything below it; never follow junctions.
pub(crate) fn safe_components(path: &Path, create: bool) -> Result<()> {
    ensure!(path.is_absolute(), "保存先は絶対パスで指定してください。");
    let mut cursor = PathBuf::new();
    for component in path.components() {
        ensure!(
            !matches!(component, Component::ParentDir | Component::CurDir),
            "保存先に相対移動は指定できません。"
        );
        cursor.push(component.as_os_str());
        if matches!(component, Component::Prefix(_)) {
            continue;
        }
        match fs::symlink_metadata(&cursor) {
            Ok(metadata) => {
                ensure!(
                    !reparse(&metadata),
                    "保存先にシンボリックリンクまたはジャンクションがあります: {}",
                    cursor.display()
                );
                ensure!(
                    metadata.is_dir(),
                    "保存先の途中にディレクトリ以外があります: {}",
                    cursor.display()
                );
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound && create => {
                fs::create_dir(&cursor)
                    .with_context(|| format!("保存先を作成できません: {}", cursor.display()))?;
            }
            Err(error) => {
                return Err(error)
                    .with_context(|| format!("保存先を確認できません: {}", cursor.display()));
            }
        }
    }
    Ok(())
}
pub(crate) fn safe_file(path: &Path) -> Result<()> {
    safe_components(
        path.parent()
            .context("保存先の親フォルダーがありません。")?,
        false,
    )?;
    match fs::symlink_metadata(path) {
        Ok(metadata) => ensure!(
            metadata.is_file() && !reparse(&metadata),
            "保存先が通常ファイルではありません: {}",
            path.display()
        ),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    Ok(())
}
fn within(root: &Path, path: &Path) -> Result<()> {
    ensure!(
        path.is_absolute() && path.starts_with(root) && path != root,
        "インストール記録のパスが保存先の外を指しています。"
    );
    safe_file(path)
}

pub(crate) struct RootLock {
    file: Option<File>,
    path: PathBuf,
}
pub(crate) struct StagingGuard {
    pub(crate) path: PathBuf,
    pub(crate) parent: PathBuf,
}
impl Drop for StagingGuard {
    fn drop(&mut self) {
        // The path was uniquely created by this installer. Resolve and bound it
        // again before recursive cleanup; never delete an install root/runtime.
        if safe_components(&self.path, false).is_ok()
            && let (Ok(path), Ok(parent)) =
                (fs::canonicalize(&self.path), fs::canonicalize(&self.parent))
            && path.starts_with(&parent)
            && path != parent
        {
            let _ = fs::remove_dir_all(path);
        }
    }
}
impl RootLock {
    pub(crate) fn acquire(root: &Path) -> Result<Self> {
        let path = root.join(".installer.lock");
        safe_file(&path)?;
        let mut options = OpenOptions::new();
        options.read(true).write(true);
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            // Exclusive Windows handle; stale files after a crash do not block resume.
            options.create(true).share_mode(0);
        }
        #[cfg(not(windows))]
        {
            options.create_new(true);
        }
        let file = options
            .open(&path)
            .context("別のインストール処理が動作中か、保存先に書き込めません。")?;
        Ok(Self {
            file: Some(file),
            path,
        })
    }
}
impl Drop for RootLock {
    fn drop(&mut self) {
        drop(self.file.take());
        let _ = fs::remove_file(&self.path);
    }
}

pub(crate) fn atomic_write(path: &Path, bytes: &[u8]) -> Result<()> {
    safe_file(path)?;
    let temp = path.with_file_name(format!(".write-{}.tmp", uuid::Uuid::new_v4()));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temp)?;
    let written = (|| -> Result<()> {
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        fs::rename(&temp, path)
            .with_context(|| format!("インストール記録を確定できません: {}", path.display()))?;
        Ok(())
    })();
    if written.is_err() {
        let _ = fs::remove_file(&temp);
    }
    written
}
fn hash_file(path: &Path, cancel: &AtomicBool) -> Result<String> {
    safe_file(path)?;
    let mut file = File::open(path)?;
    let mut hash = Sha256::new();
    let mut buffer = vec![0; 1024 * 1024];
    loop {
        cancelled(cancel)?;
        let n = file.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        hash.update(&buffer[..n]);
    }
    Ok(format!("{:x}", hash.finalize()))
}
pub(crate) fn verify_file(path: &Path, artifact: &Artifact, cancel: &AtomicBool) -> Result<()> {
    safe_file(path)?;
    ensure!(
        fs::metadata(path)?.len() == artifact.bytes,
        "ファイルサイズが一致しません: {}",
        artifact.name
    );
    ensure!(
        hash_file(path, cancel)?.eq_ignore_ascii_case(&artifact.sha256),
        "SHA256が一致しません: {}",
        artifact.name
    );
    Ok(())
}

/// What the installation record in a folder says, once the files it describes
/// have passed their checks at the places this folder gives them.
pub(crate) enum Saved<T> {
    Absent,
    Current(T),
    /// The pinned release and the files agree, but the record still names the
    /// folder they were in before they were moved or copied.
    Moved,
}

impl<T> Saved<T> {
    /// What a reader may use. Only an install rewrites a moved record, so a
    /// reader gets `mismatch` as the error.
    pub(crate) fn current(self, mismatch: &str) -> Result<Option<T>> {
        match self {
            Self::Absent => Ok(None),
            Self::Current(installation) => Ok(Some(installation)),
            Self::Moved => bail!("{mismatch}"),
        }
    }
}

/// Whether `saved` is the path `expected` under `root`, as it was written in the
/// folder these files were in before: absolute, with the same part below it.
pub(crate) fn relocated(root: &Path, expected: &Path, saved: &Path) -> bool {
    saved.is_absolute()
        && expected
            .strip_prefix(root)
            .is_ok_and(|below| saved.ends_with(below))
}

#[derive(Clone, Copy)]
enum NetworkPolicy {
    Official,
    #[cfg(test)]
    LocalTest,
}
impl NetworkPolicy {
    fn permits(self, url: &Url) -> bool {
        if !url.username().is_empty() || url.password().is_some() || url.fragment().is_some() {
            return false;
        }
        match self {
            Self::Official => {
                url.scheme() == "https"
                    && url.port_or_known_default() == Some(443)
                    && url.host_str().is_some_and(|host| {
                        host == "huggingface.co"
                            || host.ends_with(".huggingface.co")
                            || host.ends_with(".hf.co")
                            || matches!(
                                host,
                                "github.com"
                                    | "raw.githubusercontent.com"
                                    | "files.pythonhosted.org"
                                    | "release-assets.githubusercontent.com"
                                    | "objects.githubusercontent.com"
                                    | "github-releases.githubusercontent.com"
                            )
                    })
            }
            #[cfg(test)]
            Self::LocalTest => url.scheme() == "http" && url.host_str() == Some("127.0.0.1"),
        }
    }
    fn client(self) -> Result<Client> {
        Client::builder()
            .no_proxy()
            .user_agent("PLaMo-Translate-Installer/1")
            .connect_timeout(Duration::from_secs(15))
            .timeout(Duration::from_secs(30 * 60))
            .redirect(Policy::custom(move |attempt| {
                if attempt.previous().len() >= 10 || !self.permits(attempt.url()) {
                    attempt.error("公式配布先以外へのリダイレクトは許可されません。")
                } else {
                    attempt.follow()
                }
            }))
            .build()
            .context("ダウンロード接続を初期化できません。")
    }
}

enum Packet {
    Headers(StatusCode, header::HeaderMap),
    Data(Vec<u8>),
    End,
    Error(String),
}
// Network reads are isolated so cancellation stops disk writes immediately even
// while a remote server is stalled. At most two 128 KiB packets are buffered.
fn start_stream(client: Client, url: String, offset: u64) -> mpsc::Receiver<Packet> {
    let (tx, rx) = mpsc::sync_channel(2);
    thread::spawn(move || {
        let result = (|| -> Result<()> {
            let mut request = client.get(url).header(header::ACCEPT_ENCODING, "identity");
            if offset > 0 {
                request = request.header(header::RANGE, format!("bytes={offset}-"));
            }
            let mut response = request.send()?;
            if tx
                .send(Packet::Headers(
                    response.status(),
                    response.headers().clone(),
                ))
                .is_err()
            {
                return Ok(());
            }
            let mut buffer = vec![0; 128 * 1024];
            loop {
                let n = response.read(&mut buffer)?;
                if n == 0 {
                    break;
                }
                if tx.send(Packet::Data(buffer[..n].to_vec())).is_err() {
                    return Ok(());
                }
            }
            let _ = tx.send(Packet::End);
            Ok(())
        })();
        if let Err(error) = result {
            let _ = tx.send(Packet::Error(format!("{error:#}")));
        }
    });
    rx
}
fn receive(rx: &mpsc::Receiver<Packet>, cancel: &AtomicBool) -> Result<Packet> {
    loop {
        cancelled(cancel)?;
        match rx.recv_timeout(Duration::from_millis(100)) {
            Ok(packet) => return Ok(packet),
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                bail!("ダウンロード接続が途中で切れました。再実行で続きから取得できます。")
            }
        }
    }
}
fn range_response(headers: &header::HeaderMap, offset: u64, total: u64) -> Result<()> {
    bounded_range_response(headers, offset, total - 1, total)
}
fn bounded_range_response(
    headers: &header::HeaderMap,
    offset: u64,
    expected_end: u64,
    total: u64,
) -> Result<()> {
    let value = headers
        .get(header::CONTENT_RANGE)
        .context("再開応答にContent-Rangeがありません。")?
        .to_str()?;
    let value = value
        .strip_prefix("bytes ")
        .context("Content-Range形式が不正です。")?;
    let (bounds, declared_total) = value
        .split_once('/')
        .context("Content-Range形式が不正です。")?;
    let (start, end) = bounds
        .split_once('-')
        .context("Content-Range形式が不正です。")?;
    ensure!(
        start.parse::<u64>()? == offset
            && declared_total.parse::<u64>()? == total
            && end.parse::<u64>()? == expected_end,
        "サーバーの再開位置またはサイズが一致しません。"
    );
    Ok(())
}

const PARALLEL_THRESHOLD: u64 = 64 * 1024 * 1024;
const SEGMENT_BYTES: u64 = 8 * 1024 * 1024;
const DOWNLOAD_WORKERS: usize = 8;

type SegmentJob = Box<dyn FnOnce() + Send + 'static>;

// One bounded pool across retries: cancelled, stalled sockets cannot accumulate
// new network threads or buffers each time the user restarts the installer.
fn submit_segment(job: impl FnOnce() + Send + 'static, cancel: &AtomicBool) -> Result<()> {
    static QUEUE: OnceLock<mpsc::SyncSender<SegmentJob>> = OnceLock::new();
    let queue = QUEUE.get_or_init(|| {
        let (tx, rx) = mpsc::sync_channel::<SegmentJob>(DOWNLOAD_WORKERS);
        let rx = Arc::new(Mutex::new(rx));
        for _ in 0..DOWNLOAD_WORKERS {
            let rx = rx.clone();
            thread::spawn(move || {
                loop {
                    let job = rx.lock().unwrap().recv();
                    match job {
                        Ok(job) => job(),
                        Err(_) => break,
                    }
                }
            });
        }
        tx
    });
    let mut job: SegmentJob = Box::new(job);
    loop {
        cancelled(cancel)?;
        match queue.try_send(job) {
            Ok(()) => return Ok(()),
            Err(mpsc::TrySendError::Full(pending)) => {
                job = pending;
                thread::sleep(Duration::from_millis(20));
            }
            Err(mpsc::TrySendError::Disconnected(_)) => {
                bail!("ダウンロード接続が途中で切れました。再実行で続きから取得できます。")
            }
        }
    }
}

// Workers never own files. Dropping the coordinator immediately stops all disk
// writes; a blocked socket can finish independently without holding the UI open.
struct StopDownloads(Arc<AtomicBool>);
impl Drop for StopDownloads {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Relaxed);
    }
}
fn fetch_segment(
    client: &Client,
    artifact: &Artifact,
    start: u64,
    end: u64,
    stop: &AtomicBool,
    received: &AtomicU64,
) -> Result<Option<Vec<u8>>> {
    cancelled(stop)?;
    let mut response = client
        .get(&artifact.url)
        .header(header::ACCEPT_ENCODING, "identity")
        .header(header::RANGE, format!("bytes={start}-{end}"))
        .timeout(Duration::from_secs(180))
        .send()?;
    if response.status() == StatusCode::OK {
        return Ok(None); // Only an ignored Range permits the sequential fallback.
    }
    ensure!(
        response.status() == StatusCode::PARTIAL_CONTENT,
        "{}の取得に失敗しました（HTTP {}）。",
        artifact.name,
        response.status()
    );
    bounded_range_response(response.headers(), start, end, artifact.bytes)?;
    let expected = end - start + 1;
    ensure!(
        response
            .headers()
            .get(header::CONTENT_LENGTH)
            .context("分割応答にContent-Lengthがありません。")?
            .to_str()?
            .parse::<u64>()?
            == expected,
        "配布サーバーのContent-Lengthが固定サイズと一致しません。"
    );
    if let Some(encoding) = response.headers().get(header::CONTENT_ENCODING) {
        ensure!(
            encoding == "identity",
            "圧縮されたダウンロード応答は使用できません。"
        );
    }
    let mut bytes = Vec::with_capacity(expected as usize);
    let mut buffer = [0; 128 * 1024];
    loop {
        cancelled(stop)?;
        let count = response.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        ensure!(
            bytes.len() as u64 + count as u64 <= expected,
            "固定サイズを超えるデータを受信しました。ファイルは確定していません。"
        );
        bytes.extend_from_slice(&buffer[..count]);
        received.fetch_add(count as u64, Ordering::Relaxed);
    }
    ensure!(
        bytes.len() as u64 == expected,
        "取得データが途中で終了しました。次回再開できます。"
    );
    Ok(Some(bytes))
}
fn receive_segment(
    rx: &mpsc::Receiver<Result<Option<Vec<u8>>>>,
    cancel: &AtomicBool,
    mut tick: impl FnMut(),
) -> Result<Option<Vec<u8>>> {
    loop {
        cancelled(cancel)?;
        match rx.recv_timeout(Duration::from_millis(100)) {
            Ok(result) => return result,
            Err(mpsc::RecvTimeoutError::Timeout) => tick(),
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                bail!("ダウンロード接続が途中で切れました。再実行で続きから取得できます。")
            }
        }
    }
}

/// Returns false only when the server ignores Range. The existing sequential
/// path can then resume from the contiguous prefix; no sparse files are created.
fn download_parallel(
    artifact: &Artifact,
    partial: &Path,
    client: Client,
    cancel: &AtomicBool,
    stage: &str,
    callback: &mut impl FnMut(Progress),
    segment_bytes: u64,
) -> Result<bool> {
    ensure!(
        segment_bytes > 0 && segment_bytes <= SEGMENT_BYTES,
        "分割ダウンロードのサイズが不正です。"
    );
    let mut offset = fs::metadata(partial).map(|m| m.len()).unwrap_or(0);
    progress(
        callback,
        stage,
        offset,
        Some(artifact.bytes),
        format!("{}を取得中", artifact.name),
    );
    cancelled(cancel)?;
    let stop = StopDownloads(Arc::new(AtomicBool::new(false)));
    let received = Arc::new(AtomicU64::new(offset));
    // The tiny probe prevents a non-Range server from sending eight whole models.
    let (tx, rx) = mpsc::sync_channel(1);
    let worker_client = client.clone();
    let worker_artifact = artifact.clone();
    let worker_stop = stop.0.clone();
    submit_segment(
        move || {
            let result = fetch_segment(
                &worker_client,
                &worker_artifact,
                offset,
                offset,
                &worker_stop,
                &AtomicU64::new(0),
            );
            let _ = tx.send(result);
        },
        cancel,
    )?;
    if receive_segment(&rx, cancel, || {})?.is_none() {
        return Ok(false);
    }
    let mut file = OpenOptions::new().create(true).append(true).open(partial)?;
    while offset < artifact.bytes {
        cancelled(cancel)?;
        let mut receivers = Vec::new();
        let mut next = offset;
        for _ in 0..DOWNLOAD_WORKERS {
            if next == artifact.bytes {
                break;
            }
            let start = next;
            let end = (start + segment_bytes).min(artifact.bytes) - 1;
            next = end + 1;
            let (tx, rx) = mpsc::sync_channel(1);
            let client = client.clone();
            let artifact = artifact.clone();
            let stop = stop.0.clone();
            let received = received.clone();
            submit_segment(
                move || {
                    let result = fetch_segment(&client, &artifact, start, end, &stop, &received);
                    let _ = tx.send(result);
                },
                cancel,
            )?;
            receivers.push(rx);
        }
        for rx in receivers {
            let bytes = receive_segment(&rx, cancel, || {
                progress(
                    callback,
                    stage,
                    received.load(Ordering::Relaxed),
                    Some(artifact.bytes),
                    format!("{}を取得中", artifact.name),
                );
            })?;
            let Some(bytes) = bytes else {
                return Ok(false);
            };
            // Only complete, validated, consecutive segments reach disk. A
            // cancellation callback is checked again before every bounded write.
            for chunk in bytes.chunks(128 * 1024) {
                cancelled(cancel)?;
                file.write_all(chunk)?;
                offset += chunk.len() as u64;
            }
            progress(
                callback,
                stage,
                received.load(Ordering::Relaxed),
                Some(artifact.bytes),
                format!("{}を取得中", artifact.name),
            );
        }
    }
    cancelled(cancel)?;
    file.sync_all()?;
    Ok(true)
}
fn quarantine(path: &Path) -> Result<()> {
    if path.exists() {
        safe_file(path)?;
        fs::rename(
            path,
            path.with_file_name(format!(".rejected-{}", uuid::Uuid::new_v4())),
        )?;
    }
    Ok(())
}

fn download_verified(
    artifact: &Artifact,
    target: &Path,
    policy: NetworkPolicy,
    cancel: &AtomicBool,
    stage: &str,
    callback: &mut impl FnMut(Progress),
) -> Result<()> {
    cancelled(cancel)?;
    ensure!(
        policy.permits(&Url::parse(&artifact.url)?),
        "公式配布先以外からは取得できません。"
    );
    safe_file(target)?;
    if target.exists() {
        progress(
            callback,
            "verify",
            artifact.bytes,
            Some(artifact.bytes),
            format!("保存済みの{}を検証中", artifact.name),
        );
        return verify_file(target, artifact, cancel).context(
            "既存ファイルを保全しました。別の保存先を選ぶか、既存ファイルを確認してください。",
        );
    }
    let partial = target.with_file_name(format!(
        "{}.partial",
        target.file_name().unwrap().to_string_lossy()
    ));
    let meta = target.with_file_name(format!(
        "{}.partial.json",
        target.file_name().unwrap().to_string_lossy()
    ));
    safe_file(&partial)?;
    safe_file(&meta)?;
    let previous = fs::metadata(&meta)
        .ok()
        .filter(|m| m.len() <= 64 * 1024)
        .and_then(|_| fs::read(&meta).ok())
        .and_then(|v| serde_json::from_slice::<Artifact>(&v).ok());
    if partial.exists()
        && (previous.as_ref() != Some(artifact) || fs::metadata(&partial)?.len() > artifact.bytes)
    {
        quarantine(&partial)?;
    }
    atomic_write(&meta, &serde_json::to_vec(artifact)?)?;
    let mut offset = fs::metadata(&partial).map(|m| m.len()).unwrap_or(0);
    if offset < artifact.bytes && artifact.bytes >= PARALLEL_THRESHOLD {
        download_parallel(
            artifact,
            &partial,
            policy.client()?,
            cancel,
            stage,
            callback,
            SEGMENT_BYTES,
        )?;
        offset = fs::metadata(&partial).map(|m| m.len()).unwrap_or(0);
    }
    if offset < artifact.bytes {
        let client = policy.client()?;
        let rx = start_stream(client, artifact.url.clone(), offset);
        let (status, headers) = match receive(&rx, cancel)? {
            Packet::Headers(status, headers) => (status, headers),
            Packet::Error(error) => bail!("取得に失敗しました: {error}"),
            _ => bail!("ダウンロード応答が不正です。"),
        };
        ensure!(
            status == StatusCode::OK || status == StatusCode::PARTIAL_CONTENT,
            "{}の取得に失敗しました（HTTP {status}）。",
            artifact.name
        );
        if status == StatusCode::PARTIAL_CONTENT {
            range_response(&headers, offset, artifact.bytes)?;
        } else {
            offset = 0;
        } // A server may legitimately ignore Range; restart safely.
        if let Some(length) = headers.get(header::CONTENT_LENGTH) {
            ensure!(
                length.to_str()?.parse::<u64>()? == artifact.bytes - offset,
                "配布サーバーのContent-Lengthが固定サイズと一致しません。"
            );
        }
        if let Some(encoding) = headers.get(header::CONTENT_ENCODING) {
            ensure!(
                encoding == "identity",
                "圧縮されたダウンロード応答は使用できません。"
            );
        }
        let mut file = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(offset == 0)
            .open(&partial)?;
        if offset > 0 {
            use std::io::{Seek, SeekFrom};
            file.seek(SeekFrom::Start(offset))?;
        }
        progress(
            callback,
            stage,
            offset,
            Some(artifact.bytes),
            format!("{}を取得中", artifact.name),
        );
        let mut last = Instant::now();
        loop {
            match receive(&rx, cancel)? {
                Packet::Data(bytes) => {
                    ensure!(
                        offset.saturating_add(bytes.len() as u64) <= artifact.bytes,
                        "固定サイズを超えるデータを受信しました。ファイルは確定していません。"
                    );
                    file.write_all(&bytes)?;
                    offset += bytes.len() as u64;
                    if last.elapsed() >= Duration::from_millis(100) {
                        progress(
                            callback,
                            stage,
                            offset,
                            Some(artifact.bytes),
                            format!("{}を取得中", artifact.name),
                        );
                        last = Instant::now();
                    }
                }
                Packet::End => break,
                Packet::Error(error) => bail!("取得が中断されました。次回再開できます: {error}"),
                Packet::Headers(_, _) => bail!("ダウンロード応答が不正です。"),
            }
        }
        file.sync_all()?;
        drop(file);
        ensure!(
            offset == artifact.bytes,
            "取得データが途中で終了しました。次回再開できます。"
        );
    }
    progress(
        callback,
        "verify",
        offset,
        Some(artifact.bytes),
        format!("{}のSHA256を検証中", artifact.name),
    );
    if let Err(error) = verify_file(&partial, artifact, cancel) {
        if !cancel.load(Ordering::Relaxed) {
            quarantine(&partial)?;
        }
        return Err(error);
    }
    cancelled(cancel)?;
    ensure!(
        !target.exists(),
        "確定先が他の処理で作成されました。既存ファイルは変更しません。"
    );
    fs::rename(&partial, target)?;
    let _ = fs::remove_file(meta);
    progress(
        callback,
        stage,
        artifact.bytes,
        Some(artifact.bytes),
        format!("{}の取得と検証が完了", artifact.name),
    );
    Ok(())
}

/// Shared verified downloader for additional pinned, opt-in speech assets.
/// The official HTTPS and redirect allowlist is identical to the main installer.
pub(crate) fn download_official(
    artifact: &Artifact,
    target: &Path,
    cancel: &AtomicBool,
    stage: &str,
    callback: &mut impl FnMut(Progress),
) -> Result<()> {
    download_verified(
        artifact,
        target,
        NetworkPolicy::Official,
        cancel,
        stage,
        callback,
    )
}

pub(crate) fn hugging_face_url(repo: &str, revision: &str, filename: &str) -> String {
    format!("https://huggingface.co/{repo}/resolve/{revision}/{filename}")
}

/// Asks pinned files for their first byte with the installer's own client, so a
/// refused redirect, an error status or a changed size is found before release.
pub(crate) struct OfficialProbe(Client);
impl OfficialProbe {
    pub(crate) fn new() -> Result<Self> {
        NetworkPolicy::Official.client().map(Self)
    }
    /// Returns the host that served the byte.
    pub(crate) fn first_byte(&self, artifact: &Artifact) -> Result<String> {
        probe(&self.0, NetworkPolicy::Official, artifact)
    }
}
fn probe(client: &Client, policy: NetworkPolicy, artifact: &Artifact) -> Result<String> {
    let url = Url::parse(&artifact.url)?;
    ensure!(policy.permits(&url), "公式配布先以外のURLです。");
    let response = client
        .get(url)
        .header(header::ACCEPT_ENCODING, "identity")
        .header(header::RANGE, "bytes=0-0")
        .timeout(Duration::from_secs(60))
        .send()
        .map_err(|error| {
            let host = error
                .url()
                .and_then(Url::host_str)
                .unwrap_or("?")
                .to_owned();
            anyhow::Error::new(error.without_url()).context(host)
        })?;
    let host = response.url().host_str().unwrap_or("?").to_owned();
    served_as_pinned(response.status(), response.headers(), artifact.bytes)
        .context(host.clone())?;
    Ok(host)
}
// A pinned file answers with its first byte and pinned size, or with the whole
// pinned size when the server ignores Range, as download_verified requires.
fn served_as_pinned(status: StatusCode, headers: &header::HeaderMap, bytes: u64) -> Result<()> {
    let shown = |name: header::HeaderName| {
        headers.get(name).map_or_else(
            || "なし".to_owned(),
            |value| String::from_utf8_lossy(value.as_bytes()).into_owned(),
        )
    };
    let length = match status {
        StatusCode::PARTIAL_CONTENT => {
            bounded_range_response(headers, 0, 0, bytes)
                .with_context(|| format!("Content-Range: {}", shown(header::CONTENT_RANGE)))?;
            1
        }
        StatusCode::OK => bytes,
        status => bail!("HTTP {status}"),
    };
    ensure!(
        headers.get(header::CONTENT_LENGTH).is_none_or(|value| {
            value.to_str().ok().and_then(|value| value.parse().ok()) == Some(length)
        }),
        "Content-Length（{}）が{length}バイトではありません。",
        shown(header::CONTENT_LENGTH)
    );
    ensure!(
        headers
            .get(header::CONTENT_ENCODING)
            .is_none_or(|encoding| encoding == "identity"),
        "圧縮されたダウンロード応答は使用できません。"
    );
    Ok(())
}

pub(crate) fn remaining_bytes(artifact: &Artifact, target: &Path) -> Result<u64> {
    safe_file(target)?;
    if target.is_file() && fs::metadata(target)?.len() == artifact.bytes {
        return Ok(0);
    }
    let partial = target.with_file_name(format!(
        "{}.partial",
        target.file_name().unwrap().to_string_lossy()
    ));
    let meta = target.with_file_name(format!(
        "{}.partial.json",
        target.file_name().unwrap().to_string_lossy()
    ));
    safe_file(&partial)?;
    safe_file(&meta)?;
    let identity = fs::metadata(&meta)
        .ok()
        .filter(|m| m.len() <= 64 * 1024)
        .and_then(|_| fs::read(&meta).ok())
        .and_then(|bytes| serde_json::from_slice::<Artifact>(&bytes).ok());
    let size = fs::metadata(partial).map(|m| m.len()).unwrap_or(0);
    Ok(
        if identity.as_ref() == Some(artifact) && size <= artifact.bytes {
            artifact.bytes - size
        } else {
            artifact.bytes
        },
    )
}

/// What is left to download for these files, each counted from its resumable
/// partial download. A path or record that cannot be checked counts in full.
pub(crate) fn remaining_total(files: &[(&Artifact, PathBuf)]) -> u64 {
    files
        .iter()
        .map(|(artifact, target)| remaining_bytes(artifact, target).unwrap_or(artifact.bytes))
        .sum()
}

fn model_artifact(request: &InstallRequest) -> Artifact {
    Artifact {
        name: request.filename.clone(),
        url: hugging_face_url(&request.repo, &request.revision, &request.filename),
        bytes: request.bytes,
        sha256: request.sha256.to_ascii_lowercase(),
    }
}

/// What `install` would download for this request now: the runtime archives
/// unless the runtime is already unpacked, and the model unless the whole file
/// is saved.
pub fn pending_download_bytes(request: &InstallRequest) -> u64 {
    let root = &request.install_root;
    let runtime = root
        .join("runtime")
        .join(format!("llama-{RELEASE}-{}", request.backend));
    let model = model_artifact(request);
    let mut files = vec![(
        &model,
        root.join("models")
            .join(&request.revision)
            .join(&request.filename),
    )];
    let archives = runtime_assets(&request.backend);
    if !runtime.exists() {
        files.extend(
            archives
                .iter()
                .map(|asset| (asset, root.join("downloads").join(&asset.name))),
        );
    }
    remaining_total(&files)
}

pub(crate) fn runtime_assets(backend: &str) -> Vec<Artifact> {
    let rows: &[(&str, u64, &str)] = match backend {
        "cpu" => &[(
            "llama-b12119-bin-win-cpu-x64.zip",
            24730637,
            "37a6c6c9eae5b7baa2619cba513781fad15ca0f63f7d07362ec83a294cf96f86",
        )],
        "vulkan" => &[(
            "llama-b12119-bin-win-vulkan-x64.zip",
            37684344,
            "da6f3e49f5b2383bc79dd9bfcb17205e69df4aaa8ccf72d3db02da72cdb5e89a",
        )],
        "cuda" => &[
            (
                "llama-b12119-bin-win-cuda-12.4-x64.zip",
                287037856,
                "944fea366abc778026d73213277ea104a7e2413b265f42193fcf2da1f5cdd60f",
            ),
            (
                "cudart-llama-bin-win-cuda-12.4-x64.zip",
                391443627,
                "8c79a9b226de4b3cacfd1f83d24f962d0773be79f1e7b75c6af4ded7e32ae1d6",
            ),
        ],
        _ => &[],
    };
    rows.iter()
        .map(|(name, bytes, sha256)| Artifact {
            name: (*name).into(),
            bytes: *bytes,
            sha256: (*sha256).into(),
            url: format!(
                "https://github.com/ggml-org/llama.cpp/releases/download/{RELEASE}/{name}"
            ),
        })
        .collect()
}

pub(crate) fn archive_path(name: &str) -> Result<PathBuf> {
    ensure!(
        !name.is_empty() && name.len() <= 240 && !name.contains('\0'),
        "ZIPのパスが不正です。"
    );
    let normalized = name.replace('\\', "/");
    ensure!(!normalized.starts_with('/'), "ZIPに絶対パスがあります。");
    let mut result = PathBuf::new();
    for segment in normalized.trim_end_matches('/').split('/') {
        ensure!(
            !segment.is_empty()
                && segment != "."
                && segment != ".."
                && !segment.contains(':')
                && !segment.ends_with(['.', ' '])
                && !segment.chars().any(|c| c.is_control()),
            "ZIPに危険なパスがあります。"
        );
        let stem = segment.split('.').next().unwrap_or("").to_ascii_uppercase();
        ensure!(
            !matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL" | "CLOCK$")
                && !(stem.len() == 4
                    && (stem.starts_with("COM") || stem.starts_with("LPT"))
                    && stem.as_bytes()[3].is_ascii_digit()),
            "ZIPにWindows予約名があります。"
        );
        result.push(segment);
    }
    Ok(result)
}

fn unpack(
    archives: &[PathBuf],
    staging: &Path,
    cancel: &AtomicBool,
    callback: &mut impl FnMut(Progress),
) -> Result<Vec<RuntimeFile>> {
    unpack_runtime(archives, staging, cancel, callback, "llama-server.exe")
}

pub(crate) fn unpack_runtime(
    archives: &[PathBuf],
    staging: &Path,
    cancel: &AtomicBool,
    callback: &mut impl FnMut(Progress),
    executable: &str,
) -> Result<Vec<RuntimeFile>> {
    let mut files: BTreeMap<String, RuntimeFile> = BTreeMap::new();
    let mut total = 0_u64;
    for archive in archives {
        let mut zip = zip::ZipArchive::new(File::open(archive)?)?;
        ensure!(zip.len() <= 10000, "ZIPのファイル数が多すぎます。");
        for i in 0..zip.len() {
            cancelled(cancel)?;
            let mut entry = zip.by_index(i)?;
            let relative = archive_path(entry.name())?;
            let mode = entry.unix_mode().unwrap_or(0) & 0o170000;
            ensure!(
                matches!(mode, 0 | 0o100000 | 0o040000),
                "ZIP内のリンクや特殊ファイルは展開できません。"
            );
            let path = staging.join(&relative);
            if entry.is_dir() {
                safe_components(&path, true)?;
                continue;
            }
            total = total
                .checked_add(entry.size())
                .context("ZIPサイズが不正です。")?;
            ensure!(
                total <= MAX_UNPACKED,
                "ZIPの展開サイズが2GiBを超えています。"
            );
            safe_components(path.parent().unwrap(), true)?;
            safe_file(&path)?;
            let temp = path.with_file_name(format!(".extract-{}", uuid::Uuid::new_v4()));
            let mut output = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&temp)?;
            let mut hash = Sha256::new();
            let mut copied = 0_u64;
            let mut buffer = [0; 128 * 1024];
            loop {
                cancelled(cancel)?;
                let n = entry.read(&mut buffer)?;
                if n == 0 {
                    break;
                }
                copied += n as u64;
                ensure!(copied <= entry.size(), "ZIP内のファイルサイズが不正です。");
                output.write_all(&buffer[..n])?;
                hash.update(&buffer[..n]);
            }
            ensure!(copied == entry.size(), "ZIPの展開が途中で終了しました。");
            output.sync_all()?;
            drop(output);
            let digest = format!("{:x}", hash.finalize());
            let key = relative.to_string_lossy().replace('\\', "/");
            if path.exists() {
                ensure!(
                    fs::metadata(&path)?.len() == copied && hash_file(&path, cancel)? == digest,
                    "ZIP間で異なる同名ファイルがあります。既存ファイルは変更しません。"
                );
                fs::remove_file(temp)?;
            } else {
                fs::rename(temp, &path)?;
            }
            let folded = key.to_ascii_lowercase();
            if let Some(previous) = files.get(&folded) {
                ensure!(
                    previous.path == key && previous.sha256 == digest,
                    "ZIPに大文字小文字が衝突するパスがあります。"
                );
            } else {
                files.insert(
                    folded,
                    RuntimeFile {
                        path: key,
                        bytes: copied,
                        sha256: digest,
                    },
                );
            }
            progress(
                callback,
                "runtime_extract",
                total,
                None,
                "実行環境を安全な一時フォルダーへ展開中",
            );
        }
    }
    ensure!(
        files.contains_key(&executable.to_ascii_lowercase()),
        "ZIPに{executable}がありません。"
    );
    Ok(files.into_values().collect())
}

fn verify_runtime(
    root: &Path,
    backend: &str,
    assets: &[Artifact],
    cancel: &AtomicBool,
) -> Result<()> {
    let marker = root.join("runtime.json");
    safe_file(&marker)?;
    ensure!(
        fs::metadata(&marker)?.len() < 2 * 1024 * 1024,
        "実行環境の記録が大きすぎます。"
    );
    let manifest: RuntimeManifest = serde_json::from_slice(&fs::read(marker)?)?;
    ensure!(
        manifest.version == 1
            && manifest.release == RELEASE
            && manifest.backend == backend
            && manifest.assets == assets
            && !manifest.files.is_empty(),
        "実行環境の記録が一致しません。"
    );
    let mut saw_server = false;
    for file in manifest.files {
        let relative = archive_path(&file.path)?;
        let path = root.join(relative);
        safe_file(&path)?;
        ensure!(
            fs::metadata(&path)?.len() == file.bytes && hash_file(&path, cancel)? == file.sha256,
            "保存済みの実行環境が破損しています: {}",
            file.path
        );
        saw_server |= file.path == "llama-server.exe";
    }
    ensure!(saw_server, "実行環境にllama-server.exeがありません。");
    Ok(())
}

/// Read the committed installation only. No download, model load, or settings write.
pub fn load_installation(root: &Path) -> Result<Option<InstallationManifest>> {
    if !root.exists() {
        return Ok(None);
    }
    safe_components(root, false)?;
    let root = fs::canonicalize(root)?;
    let path = root.join("installation.json");
    safe_file(&path)?;
    if !path.exists() {
        return Ok(None);
    }
    ensure!(
        fs::metadata(&path)?.len() < 64 * 1024,
        "インストール記録が大きすぎます。"
    );
    let manifest: InstallationManifest = serde_json::from_slice(&fs::read(path)?)
        .context("インストール記録が壊れています。既存モデルは変更していません。")?;
    ensure!(
        manifest.version == 1
            && manifest.runtime_version == RELEASE
            && manifest.template_revision == TEMPLATE_REVISION
            && manifest.template_sha256 == TEMPLATE_SHA256,
        "未対応のインストール記録です。"
    );
    validate_request(&InstallRequest {
        repo: manifest.repo.clone(),
        revision: manifest.revision.clone(),
        filename: manifest.filename.clone(),
        bytes: manifest.bytes,
        sha256: manifest.sha256.clone(),
        backend: manifest.backend.clone(),
        install_root: root.clone(),
    })?;
    for path in [
        &manifest.model_path,
        &manifest.llama_server,
        &manifest.template_path,
    ] {
        within(&root, path)?;
    }
    ensure!(
        manifest.model_path
            == root
                .join("models")
                .join(&manifest.revision)
                .join(&manifest.filename)
            && manifest.llama_server
                == root
                    .join("runtime")
                    .join(format!("llama-{RELEASE}-{}", manifest.backend))
                    .join("llama-server.exe")
            && manifest.template_path
                == root
                    .join("templates")
                    .join(format!("plamo-{TEMPLATE_REVISION}.jinja")),
        "インストール記録の保存先が、選択されたモデルと実行環境に一致しません。"
    );
    ensure!(
        fs::metadata(&manifest.model_path)?.len() == manifest.bytes,
        "保存済みモデルのサイズが変わっています。再度準備してください。"
    );
    ensure!(
        manifest.llama_server.is_file(),
        "保存済みの実行環境が見つかりません。"
    );
    ensure!(
        hash_file(&manifest.template_path, &AtomicBool::new(false))? == TEMPLATE_SHA256,
        "保存済みの翻訳テンプレートが変わっています。"
    );
    Ok(Some(manifest))
}

pub fn install(
    request: InstallRequest,
    cancel: Arc<AtomicBool>,
    mut on_progress: impl FnMut(Progress),
) -> Result<InstallResult> {
    validate_request(&request)?;
    cancelled(&cancel)?;
    safe_components(&request.install_root, true)?;
    let root = fs::canonicalize(&request.install_root)?;
    let _lock = RootLock::acquire(&root)?;
    let downloads = root.join("downloads");
    let runtime_parent = root.join("runtime");
    let model_parent = root.join("models").join(&request.revision);
    let template_parent = root.join("templates");
    for path in [&downloads, &runtime_parent, &model_parent, &template_parent] {
        safe_components(path, true)?;
    }
    let assets = runtime_assets(&request.backend);
    let runtime = runtime_parent.join(format!("llama-{RELEASE}-{}", request.backend));
    if runtime.exists() {
        safe_components(&runtime, false)?;
        progress(
            &mut on_progress,
            "verify",
            0,
            None,
            "保存済みの実行環境を検証中",
        );
        verify_runtime(&runtime, &request.backend, &assets, &cancel)?;
    } else {
        let mut archives = Vec::new();
        for asset in &assets {
            let path = downloads.join(&asset.name);
            download_verified(
                asset,
                &path,
                NetworkPolicy::Official,
                &cancel,
                "runtime_download",
                &mut on_progress,
            )?;
            archives.push(path);
        }
        let staging = runtime_parent.join(format!(".staging-{}", uuid::Uuid::new_v4()));
        safe_components(&staging, true)?;
        let _staging_guard = StagingGuard {
            path: staging.clone(),
            parent: runtime_parent.clone(),
        };
        let files = unpack(&archives, &staging, &cancel, &mut on_progress)?;
        atomic_write(
            &staging.join("runtime.json"),
            &serde_json::to_vec_pretty(&RuntimeManifest {
                version: 1,
                release: RELEASE.into(),
                backend: request.backend.clone(),
                assets,
                files,
            })?,
        )?;
        cancelled(&cancel)?;
        ensure!(
            !runtime.exists(),
            "実行環境の保存先が他の処理で作成されました。"
        );
        fs::rename(&staging, &runtime)?;
    }
    let model_path = model_parent.join(&request.filename);
    let model = model_artifact(&request);
    download_verified(
        &model,
        &model_path,
        NetworkPolicy::Official,
        &cancel,
        "model_download",
        &mut on_progress,
    )?;
    let template_path = template_parent.join(format!("plamo-{TEMPLATE_REVISION}.jinja"));
    ensure!(
        format!("{:x}", Sha256::digest(TEMPLATE)) == TEMPLATE_SHA256,
        "同梱テンプレートの検証に失敗しました。"
    );
    if template_path.exists() {
        ensure!(
            hash_file(&template_path, &cancel)? == TEMPLATE_SHA256,
            "既存テンプレートが異なります。ファイルは変更しません。"
        );
    } else {
        atomic_write(&template_path, TEMPLATE)?;
    }
    cancelled(&cancel)?;
    let result = InstallResult {
        model_path: model_path.clone(),
        llama_server: runtime.join("llama-server.exe"),
        template_path: template_path.clone(),
        backend: request.backend.clone(),
    };
    let manifest = InstallationManifest {
        version: 1,
        repo: request.repo,
        revision: request.revision,
        filename: request.filename,
        bytes: request.bytes,
        sha256: request.sha256.to_ascii_lowercase(),
        backend: request.backend,
        model_path,
        llama_server: result.llama_server.clone(),
        template_path,
        runtime_version: RELEASE.into(),
        template_revision: TEMPLATE_REVISION.into(),
        template_sha256: TEMPLATE_SHA256.into(),
    };
    atomic_write(
        &root.join("installation.json"),
        &serde_json::to_vec_pretty(&manifest)?,
    )?;
    progress(
        &mut on_progress,
        "complete",
        manifest.bytes,
        Some(manifest.bytes),
        "モデルと実行環境の準備が完了しました。",
    );
    Ok(result)
}

/// What the setup modules' tests share to lay out a small installation of a
/// release of their own, since the pinned files cannot be rebuilt.
#[cfg(test)]
pub(crate) mod fixtures {
    use super::*;
    use std::collections::BTreeMap;

    /// A temporary folder under its canonical path, so no junction lies on the way.
    pub(crate) struct Folder {
        _guard: tempfile::TempDir,
        pub(crate) path: PathBuf,
    }
    impl Folder {
        pub(crate) fn new() -> Self {
            let guard = tempfile::tempdir().unwrap();
            let path = fs::canonicalize(guard.path()).unwrap();
            Self {
                _guard: guard,
                path,
            }
        }
    }

    /// A pinned file of a test release. Its URL passes the official-host check,
    /// and a test always has the file on disk already, so nothing is fetched.
    pub(crate) fn artifact(name: &str, content: &[u8]) -> Artifact {
        Artifact {
            name: name.into(),
            url: format!("https://github.com/pinned/test-release/releases/download/v1/{name}"),
            bytes: content.len() as u64,
            sha256: format!("{:x}", Sha256::digest(content)),
        }
    }

    pub(crate) fn write(path: &Path, content: &[u8]) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, content).unwrap();
    }

    /// Writes one file of a package below `directory` and returns its inventory line.
    pub(crate) fn package_file(directory: &Path, path: &str, content: &[u8]) -> RuntimeFile {
        write(&directory.join(path), content);
        RuntimeFile {
            path: path.into(),
            bytes: content.len() as u64,
            sha256: format!("{:x}", Sha256::digest(content)),
        }
    }

    /// Every file below `folder` with its bytes.
    pub(crate) fn snapshot(folder: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
        let mut files = BTreeMap::new();
        let mut pending = vec![folder.to_path_buf()];
        while let Some(directory) = pending.pop() {
            for entry in fs::read_dir(directory).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    pending.push(path);
                } else {
                    files.insert(
                        path.strip_prefix(folder).unwrap().to_path_buf(),
                        fs::read(&path).unwrap(),
                    );
                }
            }
        }
        files
    }

    pub(crate) fn copy_tree(from: &Path, to: &Path) {
        for (relative, bytes) in snapshot(from) {
            write(&to.join(relative), &bytes);
        }
    }

    /// The file keeps its size and loses its content.
    pub(crate) fn corrupt(path: &Path) {
        let mut bytes = fs::read(path).unwrap();
        bytes[0] ^= 0xff;
        fs::write(path, bytes).unwrap();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{Shutdown, TcpListener};
    use zip::write::SimpleFileOptions;

    struct Temp(PathBuf);
    impl Temp {
        fn new() -> Self {
            let path =
                std::env::temp_dir().join(format!("plamo-installer-test-{}", uuid::Uuid::new_v4()));
            fs::create_dir(&path).unwrap();
            Self(fs::canonicalize(path).unwrap())
        }
    }
    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    fn fixture(responses: Vec<Vec<u8>>) -> (String, thread::JoinHandle<Vec<String>>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/asset", listener.local_addr().unwrap());
        let worker = thread::spawn(move || {
            let mut requests = Vec::new();
            for response in responses {
                let (mut socket, _) = listener.accept().unwrap();
                socket
                    .set_read_timeout(Some(Duration::from_secs(3)))
                    .unwrap();
                let mut request = Vec::new();
                let mut byte = [0; 1];
                while !request.ends_with(b"\r\n\r\n") {
                    if socket.read(&mut byte).unwrap() == 0 {
                        break;
                    }
                    request.push(byte[0]);
                }
                requests.push(String::from_utf8(request).unwrap().to_lowercase());
                let _ = socket.write_all(&response);
                let _ = socket.shutdown(Shutdown::Both);
            }
            requests
        });
        (url, worker)
    }
    fn response(status: &str, headers: &str, body: &[u8]) -> Vec<u8> {
        let mut result =
            format!("HTTP/1.1 {status}\r\nConnection: close\r\n{headers}\r\n").into_bytes();
        result.extend_from_slice(body);
        result
    }
    fn artifact(url: String, bytes: &[u8]) -> Artifact {
        Artifact {
            name: "model.gguf".into(),
            url,
            bytes: bytes.len() as u64,
            sha256: format!("{:x}", Sha256::digest(bytes)),
        }
    }
    fn partial(root: &Path, spec: &Artifact, bytes: &[u8]) {
        fs::write(root.join("model.gguf.partial"), bytes).unwrap();
        fs::write(
            root.join("model.gguf.partial.json"),
            serde_json::to_vec(spec).unwrap(),
        )
        .unwrap();
    }
    fn download(spec: &Artifact, target: &Path, cancel: &AtomicBool) -> Result<()> {
        download_verified(
            spec,
            target,
            NetworkPolicy::LocalTest,
            cancel,
            "test",
            &mut |_| {},
        )
    }
    // Serve requests concurrently and deliberately complete later ranges first.
    fn range_fixture(body: Vec<u8>, count: usize) -> (String, thread::JoinHandle<Vec<String>>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/asset", listener.local_addr().unwrap());
        let body = Arc::new(body);
        let worker = thread::spawn(move || {
            let mut workers = Vec::new();
            for _ in 0..count {
                let (mut socket, _) = listener.accept().unwrap();
                let body = body.clone();
                workers.push(thread::spawn(move || {
                    socket
                        .set_read_timeout(Some(Duration::from_secs(3)))
                        .unwrap();
                    let mut request = Vec::new();
                    let mut byte = [0; 1];
                    while !request.ends_with(b"\r\n\r\n") {
                        if socket.read(&mut byte).unwrap() == 0 {
                            break;
                        }
                        request.push(byte[0]);
                    }
                    let request = String::from_utf8(request).unwrap().to_lowercase();
                    let range = request
                        .lines()
                        .find_map(|line| line.strip_prefix("range: bytes="))
                        .unwrap();
                    let (start, end) = range.split_once('-').unwrap();
                    let start = start.parse::<usize>().unwrap();
                    let end = if end.is_empty() {
                        body.len() - 1
                    } else {
                        end.parse::<usize>().unwrap()
                    };
                    if end > start && start == 3 {
                        thread::sleep(Duration::from_millis(80));
                    }
                    let reply = response(
                        "206 Partial Content",
                        &format!(
                            "Content-Length: {}\r\nContent-Range: bytes {start}-{end}/{}\r\n",
                            end - start + 1,
                            body.len()
                        ),
                        &body[start..=end],
                    );
                    let _ = socket.write_all(&reply);
                    request
                }));
            }
            workers
                .into_iter()
                .map(|worker| worker.join().unwrap())
                .collect()
        });
        (url, worker)
    }
    #[test]
    fn parallel_ranges_commit_in_order_and_resume_after_cancel() {
        let temp = Temp::new();
        let body: Vec<u8> = (0..35).collect();
        // Probe + eight 4-byte ranges, followed by the final sequential resume.
        let (url, worker) = range_fixture(body.clone(), 10);
        let spec = artifact(url, &body);
        partial(&temp.0, &spec, &body[..3]);
        let path = temp.0.join("model.gguf.partial");
        let cancel = AtomicBool::new(false);
        let result = download_parallel(
            &spec,
            &path,
            NetworkPolicy::LocalTest.client().unwrap(),
            &cancel,
            "test",
            &mut |p| {
                // Progress reports received bytes, but only the ordered prefix is durable.
                if p.downloaded_bytes > 3 && fs::metadata(&path).unwrap().len() >= 7 {
                    cancel.store(true, Ordering::Relaxed);
                }
            },
            4,
        );
        assert!(result.is_err());
        assert_eq!(fs::read(&path).unwrap(), body[..7]);
        cancel.store(false, Ordering::Relaxed);
        download(&spec, &temp.0.join(&spec.name), &cancel).unwrap();
        assert_eq!(fs::read(temp.0.join(&spec.name)).unwrap(), body);
        let requests = worker.join().unwrap();
        assert!(
            requests
                .iter()
                .any(|request| request.contains("range: bytes=7-"))
        );
        assert!(
            requests
                .iter()
                .any(|request| request.contains("range: bytes=31-34"))
        );
    }
    #[test]
    fn parallel_ranges_complete_out_of_order_with_exact_contents() {
        let temp = Temp::new();
        let body: Vec<u8> = (0..35).collect();
        let (url, worker) = range_fixture(body.clone(), 9);
        let spec = artifact(url, &body);
        partial(&temp.0, &spec, &body[..3]);
        let path = temp.0.join("model.gguf.partial");
        assert!(
            download_parallel(
                &spec,
                &path,
                NetworkPolicy::LocalTest.client().unwrap(),
                &AtomicBool::new(false),
                "test",
                &mut |_| {},
                4
            )
            .unwrap()
        );
        assert_eq!(fs::read(path).unwrap(), body);
        assert_eq!(worker.join().unwrap().len(), 9);
    }
    #[test]
    fn parallel_ranges_reject_forged_bounds_lengths_and_short_bodies() {
        for headers in [
            "Content-Length: 1\r\nContent-Range: bytes 1-1/6\r\n",
            "Content-Length: 1\r\nContent-Range: bytes 0-1/6\r\n",
            "Content-Length: 1\r\nContent-Range: bytes 0-0/7\r\n",
            "Content-Length: 2\r\nContent-Range: bytes 0-0/6\r\n",
            "Content-Range: bytes 0-0/6\r\n",
        ] {
            let temp = Temp::new();
            let (url, worker) = fixture(vec![response("206 Partial Content", headers, b"a")]);
            let spec = artifact(url, b"abcdef");
            assert!(
                download_parallel(
                    &spec,
                    &temp.0.join("model.gguf.partial"),
                    NetworkPolicy::LocalTest.client().unwrap(),
                    &AtomicBool::new(false),
                    "test",
                    &mut |_| {},
                    2
                )
                .is_err()
            );
            assert!(!temp.0.join("model.gguf.partial").exists());
            worker.join().unwrap();
        }
        let temp = Temp::new();
        let (url, worker) = fixture(vec![response(
            "206 Partial Content",
            "Content-Length: 1\r\nContent-Range: bytes 0-0/6\r\n",
            b"",
        )]);
        let spec = artifact(url, b"abcdef");
        assert!(
            download_parallel(
                &spec,
                &temp.0.join("model.gguf.partial"),
                NetworkPolicy::LocalTest.client().unwrap(),
                &AtomicBool::new(false),
                "test",
                &mut |_| {},
                2
            )
            .is_err()
        );
        worker.join().unwrap();
    }
    #[test]
    fn parallel_ignored_range_preserves_partial_for_sequential_fallback() {
        let temp = Temp::new();
        let (url, worker) = fixture(vec![response("200 OK", "Content-Length: 6\r\n", b"abcdef")]);
        let spec = artifact(url, b"abcdef");
        partial(&temp.0, &spec, b"abc");
        let path = temp.0.join("model.gguf.partial");
        assert!(
            !download_parallel(
                &spec,
                &path,
                NetworkPolicy::LocalTest.client().unwrap(),
                &AtomicBool::new(false),
                "test",
                &mut |_| {},
                2
            )
            .unwrap()
        );
        assert_eq!(fs::read(path).unwrap(), b"abc");
        worker.join().unwrap();
    }
    #[test]
    fn parallel_cancel_returns_without_waiting_for_stalled_socket() {
        let temp = Temp::new();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let spec = artifact(
            format!("http://{}/asset", listener.local_addr().unwrap()),
            b"abcdef",
        );
        let cancel = Arc::new(AtomicBool::new(false));
        let server_cancel = cancel.clone();
        let worker = thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            socket
                .set_read_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            let mut request = Vec::new();
            let mut byte = [0; 1];
            while !request.ends_with(b"\r\n\r\n") {
                if socket.read(&mut byte).unwrap() == 0 {
                    break;
                }
                request.push(byte[0]);
            }
            socket.write_all(b"HTTP/1.1 206 Partial Content\r\nConnection: close\r\nContent-Length: 1\r\nContent-Range: bytes 0-0/6\r\n\r\n").unwrap();
            server_cancel.store(true, Ordering::Relaxed);
            thread::sleep(Duration::from_millis(600));
            let _ = socket.write_all(b"a");
        });
        let started = Instant::now();
        let path = temp.0.join("model.gguf.partial");
        assert!(
            download_parallel(
                &spec,
                &path,
                NetworkPolicy::LocalTest.client().unwrap(),
                &cancel,
                "test",
                &mut |_| {},
                2
            )
            .is_err()
        );
        assert!(started.elapsed() < Duration::from_millis(500));
        assert!(!path.exists());
        worker.join().unwrap();
        assert!(!path.exists());
    }
    #[test]
    fn resume_checks_range_and_hash_before_rename() {
        let temp = Temp::new();
        let (url, worker) = fixture(vec![response(
            "206 Partial Content",
            "Content-Length: 3\r\nContent-Range: bytes 3-5/6\r\n",
            b"def",
        )]);
        let spec = artifact(url, b"abcdef");
        partial(&temp.0, &spec, b"abc");
        let path = temp.0.join(&spec.name);
        download(&spec, &path, &AtomicBool::new(false)).unwrap();
        assert_eq!(fs::read(path).unwrap(), b"abcdef");
        assert!(worker.join().unwrap()[0].contains("range: bytes=3-"));
        assert!(!temp.0.join("model.gguf.partial").exists());
    }
    #[test]
    fn ignored_range_restarts_instead_of_appending() {
        let temp = Temp::new();
        let (url, worker) = fixture(vec![response("200 OK", "Content-Length: 6\r\n", b"abcdef")]);
        let spec = artifact(url, b"abcdef");
        partial(&temp.0, &spec, b"abc");
        let path = temp.0.join(&spec.name);
        download(&spec, &path, &AtomicBool::new(false)).unwrap();
        assert_eq!(fs::read(path).unwrap(), b"abcdef");
        worker.join().unwrap();
    }
    #[test]
    fn interrupted_transfer_keeps_verified_resume_identity() {
        let temp = Temp::new();
        let (url, worker) = fixture(vec![
            response("200 OK", "Content-Length: 6\r\n", b"abc"),
            response(
                "206 Partial Content",
                "Content-Length: 3\r\nContent-Range: bytes 3-5/6\r\n",
                b"def",
            ),
        ]);
        let spec = artifact(url, b"abcdef");
        let path = temp.0.join(&spec.name);
        assert!(download(&spec, &path, &AtomicBool::new(false)).is_err());
        assert!(!path.exists());
        assert_eq!(fs::read(temp.0.join("model.gguf.partial")).unwrap(), b"abc");
        download(&spec, &path, &AtomicBool::new(false)).unwrap();
        assert_eq!(fs::read(path).unwrap(), b"abcdef");
        let requests = worker.join().unwrap();
        assert!(requests[1].contains("range: bytes=3-"));
    }
    #[test]
    fn bad_hash_or_range_never_creates_final_file() {
        let temp = Temp::new();
        let (url, worker) = fixture(vec![response("200 OK", "Content-Length: 3\r\n", b"bad")]);
        let spec = artifact(url, b"yes");
        let path = temp.0.join(&spec.name);
        assert!(
            download(&spec, &path, &AtomicBool::new(false))
                .unwrap_err()
                .to_string()
                .contains("SHA256")
        );
        assert!(!path.exists());
        assert!(!temp.0.join("model.gguf.partial").exists());
        worker.join().unwrap();
        let (url, worker) = fixture(vec![response(
            "206 Partial Content",
            "Content-Length: 3\r\nContent-Range: bytes 2-5/6\r\n",
            b"def",
        )]);
        let spec = artifact(url, b"abcdef");
        partial(&temp.0, &spec, b"abc");
        assert!(download(&spec, &path, &AtomicBool::new(false)).is_err());
        assert!(!path.exists());
        assert_eq!(fs::read(temp.0.join("model.gguf.partial")).unwrap(), b"abc");
        worker.join().unwrap();
    }
    #[test]
    fn cancellation_retains_partial_and_does_not_commit() {
        let temp = Temp::new();
        let (url, worker) = fixture(vec![response(
            "206 Partial Content",
            "Content-Length: 3\r\nContent-Range: bytes 3-5/6\r\n",
            b"def",
        )]);
        let spec = artifact(url, b"abcdef");
        partial(&temp.0, &spec, b"abc");
        let cancel = AtomicBool::new(false);
        let result = download_verified(
            &spec,
            &temp.0.join(&spec.name),
            NetworkPolicy::LocalTest,
            &cancel,
            "test",
            &mut |p| {
                if p.stage == "test" {
                    cancel.store(true, Ordering::Relaxed);
                }
            },
        );
        assert!(result.is_err());
        assert_eq!(fs::read(temp.0.join("model.gguf.partial")).unwrap(), b"abc");
        assert!(!temp.0.join(&spec.name).exists());
        worker.join().unwrap();
    }
    #[test]
    fn content_length_overflow_and_existing_file_are_rejected() {
        let temp = Temp::new();
        let (url, worker) = fixture(vec![response("200 OK", "Content-Length: 4\r\n", b"four")]);
        let spec = artifact(url, b"yes");
        let path = temp.0.join(&spec.name);
        assert!(download(&spec, &path, &AtomicBool::new(false)).is_err());
        assert!(!path.exists());
        worker.join().unwrap();
        fs::write(&path, b"old working file").unwrap();
        assert!(download(&spec, &path, &AtomicBool::new(false)).is_err());
        assert_eq!(fs::read(path).unwrap(), b"old working file");
    }
    fn make_zip(path: &Path, name: &str, symlink: bool) {
        let mut writer = zip::ZipWriter::new(File::create(path).unwrap());
        if symlink {
            writer
                .add_symlink(name, "outside", SimpleFileOptions::default())
                .unwrap();
        } else {
            writer
                .start_file(name, SimpleFileOptions::default())
                .unwrap();
            writer.write_all(b"test").unwrap();
        }
        writer.finish().unwrap();
    }
    #[test]
    fn zip_rejects_traversal_windows_devices_and_links() {
        for name in [
            "../evil",
            "..\\evil",
            "/absolute",
            "C:/evil",
            "a/../evil",
            "a:stream",
            "CON.txt",
            "LPT1",
            "a.",
            "a ",
            "a//b",
        ] {
            assert!(archive_path(name).is_err(), "{name}");
        }
        let temp = Temp::new();
        let archive = temp.0.join("bad.zip");
        let staging = temp.0.join("staging");
        fs::create_dir(&staging).unwrap();
        make_zip(&archive, "../escape", false);
        assert!(
            unpack(
                std::slice::from_ref(&archive),
                &staging,
                &AtomicBool::new(false),
                &mut |_| {}
            )
            .is_err()
        );
        assert!(!temp.0.join("escape").exists());
        make_zip(&archive, "link", true);
        assert!(unpack(&[archive], &staging, &AtomicBool::new(false), &mut |_| {}).is_err());
        assert!(!staging.join("link").exists());
    }
    #[test]
    fn zip_runtime_fingerprints_detect_tampering() {
        let temp = Temp::new();
        let archive = temp.0.join("runtime.zip");
        let staging = temp.0.join("runtime");
        fs::create_dir(&staging).unwrap();
        make_zip(&archive, "llama-server.exe", false);
        let files = unpack(&[archive], &staging, &AtomicBool::new(false), &mut |_| {}).unwrap();
        let assets = runtime_assets("cpu");
        atomic_write(
            &staging.join("runtime.json"),
            &serde_json::to_vec(&RuntimeManifest {
                version: 1,
                release: RELEASE.into(),
                backend: "cpu".into(),
                assets: assets.clone(),
                files,
            })
            .unwrap(),
        )
        .unwrap();
        verify_runtime(&staging, "cpu", &assets, &AtomicBool::new(false)).unwrap();
        fs::write(staging.join("llama-server.exe"), b"evil").unwrap();
        assert!(verify_runtime(&staging, "cpu", &assets, &AtomicBool::new(false)).is_err());
    }
    #[test]
    fn pinned_sources_and_bundled_template_are_fixed() {
        assert_eq!(format!("{:x}", Sha256::digest(TEMPLATE)), TEMPLATE_SHA256);
        assert_eq!(runtime_assets("cuda").len(), 2);
        assert_eq!(runtime_assets("cpu")[0].bytes, 24730637);
        assert_eq!(runtime_assets("vulkan")[0].bytes, 37684344);
        for url in [
            "http://huggingface.co/model",
            "https://huggingface.co.evil.com/",
            "https://github.com@evil.com/",
            "https://files.pythonhosted.org.evil.com/package.whl",
            "https://raw.githubusercontent.com.evil.com/LICENSE",
            "http://127.0.0.1/",
        ] {
            assert!(!NetworkPolicy::Official.permits(&Url::parse(url).unwrap()));
        }
        assert!(NetworkPolicy::Official.permits(
            &Url::parse("https://release-assets.githubusercontent.com/asset?sig=1").unwrap()
        ));
        assert!(NetworkPolicy::Official.permits(
            &Url::parse("https://files.pythonhosted.org/packages/pinned/package.whl").unwrap()
        ));
    }
    #[test]
    fn download_check_passes_only_a_file_the_installer_can_fetch() {
        let body = b"pinned";
        let cases: [(&str, &str, &[u8], Option<&str>); 6] = [
            (
                "206 Partial Content",
                "Content-Length: 1\r\nContent-Range: bytes 0-0/6\r\n",
                b"p",
                None,
            ),
            ("200 OK", "Content-Length: 6\r\n", body, None),
            (
                "206 Partial Content",
                "Content-Length: 1\r\nContent-Range: bytes 0-0/7\r\n",
                b"p",
                Some("bytes 0-0/7"),
            ),
            (
                "200 OK",
                "Content-Length: 7\r\n",
                b"pinned!",
                Some("Content-Length（7）"),
            ),
            (
                "403 Forbidden",
                "Content-Length: 0\r\n",
                b"",
                Some("HTTP 403"),
            ),
            (
                "302 Found",
                "Location: http://localhost:9/asset\r\nContent-Length: 0\r\n",
                b"",
                Some("redirect"),
            ),
        ];
        for (status, headers, reply, failure) in cases {
            let (url, worker) = fixture(vec![response(status, headers, reply)]);
            let client = NetworkPolicy::LocalTest.client().unwrap();
            let result = probe(&client, NetworkPolicy::LocalTest, &artifact(url, body));
            match failure {
                None => assert_eq!(result.unwrap(), "127.0.0.1"),
                Some(evidence) => {
                    let message = format!("{:#}", result.unwrap_err());
                    assert!(message.contains(evidence), "{status}: {message}");
                }
            }
            assert!(worker.join().unwrap()[0].contains("range: bytes=0-0"));
        }
    }
    #[test]
    fn cancellation_does_not_replace_an_existing_installation_manifest() {
        let temp = Temp::new();
        fs::write(temp.0.join("installation.json"), b"previous installation").unwrap();
        let request = InstallRequest {
            repo: "pfnet/plamo-3-translate-2b-GGUF".into(),
            revision: "a".repeat(40),
            filename: "model.gguf".into(),
            bytes: 3,
            sha256: "a".repeat(64),
            backend: "cpu".into(),
            install_root: temp.0.clone(),
        };
        assert!(install(request, Arc::new(AtomicBool::new(true)), |_| {}).is_err());
        assert_eq!(
            fs::read(temp.0.join("installation.json")).unwrap(),
            b"previous installation"
        );
    }
    #[test]
    fn atomic_manifest_replace_is_supported() {
        let temp = Temp::new();
        let path = temp.0.join("manifest.json");
        atomic_write(&path, b"old").unwrap();
        atomic_write(&path, b"new").unwrap();
        assert_eq!(fs::read(path).unwrap(), b"new");
    }
    #[cfg(windows)]
    #[test]
    fn a_record_that_cannot_replace_its_file_leaves_no_temporary_file() {
        use std::os::windows::fs::OpenOptionsExt;
        let temp = Temp::new();
        let path = temp.0.join("manifest.json");
        atomic_write(&path, b"old").unwrap();
        // Another program has the record open and shares nothing.
        let held = OpenOptions::new()
            .read(true)
            .share_mode(0)
            .open(&path)
            .unwrap();
        assert!(atomic_write(&path, b"new").is_err());
        drop(held);
        let left: Vec<_> = fs::read_dir(&temp.0)
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        assert_eq!(left, ["manifest.json"]);
        assert_eq!(fs::read(&path).unwrap(), b"old");
    }
    #[test]
    fn a_download_that_cannot_connect_says_why() {
        let temp = Temp::new();
        let closed = TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap();
        let spec = artifact(format!("http://{closed}/asset"), b"abcdef");
        let error = download(&spec, &temp.0.join(&spec.name), &AtomicBool::new(false)).unwrap_err();
        let message = format!("{error:#}");
        assert!(message.contains("tcp connect error"), "{message}");
    }
    #[test]
    fn a_runtime_whose_executable_name_has_capitals_unpacks() {
        let temp = Temp::new();
        let archive = temp.0.join("runtime.zip");
        let staging = temp.0.join("runtime");
        fs::create_dir(&staging).unwrap();
        make_zip(&archive, "Release/Nemo-Speech.exe", false);
        let files = unpack_runtime(
            &[archive],
            &staging,
            &AtomicBool::new(false),
            &mut |_| {},
            "Release/Nemo-Speech.exe",
        )
        .unwrap();
        assert_eq!(
            files
                .iter()
                .map(|file| file.path.as_str())
                .collect::<Vec<_>>(),
            ["Release/Nemo-Speech.exe"]
        );
    }
    #[test]
    fn a_saved_path_is_the_expected_one_only_when_just_the_folder_in_front_differs() {
        let root = Path::new(r"D:\data\nemotron");
        let expected = root.join("models").join("rev").join("model.gguf");
        for (saved, same_place) in [
            (r"D:\data\nemotron\models\rev\model.gguf", true),
            (r"E:\backup\data-2\nemotron\models\rev\model.gguf", true),
            (r"\\?\E:\backup\nemotron\models\rev\model.gguf", true),
            (r"E:\backup\nemotron\models\rev\other.gguf", false),
            (r"E:\backup\nemotron\models\other\model.gguf", false),
            (r"E:\backup\nemotron\model.gguf", false),
            (r"models\rev\model.gguf", false),
            (r"D:\data\nemotron\..\model.gguf", false),
        ] {
            assert_eq!(
                relocated(root, &expected, Path::new(saved)),
                same_place,
                "{saved}"
            );
        }
    }
    #[test]
    fn a_reader_takes_only_a_record_that_names_its_own_folder() {
        assert_eq!(Saved::<u8>::Absent.current("moved").unwrap(), None);
        assert_eq!(Saved::Current(7).current("moved").unwrap(), Some(7));
        assert_eq!(
            Saved::<u8>::Moved.current("moved").unwrap_err().to_string(),
            "moved"
        );
    }
    #[test]
    fn load_only_accepts_committed_contained_selected_paths() {
        let temp = Temp::new();
        assert!(load_installation(&temp.0).unwrap().is_none());
        let revision = "a".repeat(40);
        let model_path = temp.0.join("models").join(&revision).join("model.gguf");
        let llama_server = temp
            .0
            .join("runtime")
            .join(format!("llama-{RELEASE}-cpu"))
            .join("llama-server.exe");
        let template_path = temp
            .0
            .join("templates")
            .join(format!("plamo-{TEMPLATE_REVISION}.jinja"));
        for path in [&model_path, &llama_server, &template_path] {
            safe_components(path.parent().unwrap(), true).unwrap();
        }
        fs::write(&model_path, b"yes").unwrap();
        fs::write(&llama_server, b"exe").unwrap();
        fs::write(&template_path, TEMPLATE).unwrap();
        let mut manifest = InstallationManifest {
            version: 1,
            repo: "pfnet/plamo-3-translate-2b-GGUF".into(),
            revision,
            filename: "model.gguf".into(),
            bytes: 3,
            sha256: format!("{:x}", Sha256::digest(b"yes")),
            backend: "cpu".into(),
            model_path,
            llama_server,
            template_path,
            runtime_version: RELEASE.into(),
            template_revision: TEMPLATE_REVISION.into(),
            template_sha256: TEMPLATE_SHA256.into(),
        };
        let path = temp.0.join("installation.json");
        atomic_write(&path, &serde_json::to_vec(&manifest).unwrap()).unwrap();
        assert!(load_installation(&temp.0).unwrap().is_some());
        // A regular existing file inside the root is still not a selected runtime.
        manifest.llama_server = manifest.model_path.clone();
        atomic_write(&path, &serde_json::to_vec(&manifest).unwrap()).unwrap();
        assert!(load_installation(&temp.0).is_err());
        let outside = Temp::new();
        manifest.model_path = outside.0.join("model.gguf");
        fs::write(&manifest.model_path, b"yes").unwrap();
        atomic_write(&path, &serde_json::to_vec(&manifest).unwrap()).unwrap();
        assert!(load_installation(&temp.0).is_err());
    }
    fn pending_request(root: &Path, backend: &str) -> InstallRequest {
        InstallRequest {
            repo: "pfnet/plamo-3-translate-2b-GGUF".into(),
            revision: "b".repeat(40),
            filename: "plamo-3-translate-2b-Q4_K_M.gguf".into(),
            bytes: 1_700_000_000,
            sha256: "c".repeat(64),
            backend: backend.into(),
            install_root: root.into(),
        }
    }
    fn saved_file(path: &Path, bytes: u64) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::File::create(path).unwrap().set_len(bytes).unwrap();
    }
    #[test]
    fn pending_bytes_are_the_runtime_archives_and_the_model_that_are_not_saved() {
        for backend in ["cpu", "vulkan", "cuda"] {
            let temp = Temp::new();
            let request = pending_request(&temp.0, backend);
            let archives: Vec<_> = runtime_assets(backend);
            let runtime_bytes: u64 = archives.iter().map(|a| a.bytes).sum();
            assert_eq!(
                pending_download_bytes(&request),
                runtime_bytes + request.bytes,
                "{backend}: nothing saved"
            );
            let model = temp
                .0
                .join("models")
                .join(&request.revision)
                .join(&request.filename);
            saved_file(&model, request.bytes);
            assert_eq!(pending_download_bytes(&request), runtime_bytes, "{backend}");
            fs::create_dir_all(
                temp.0
                    .join("runtime")
                    .join(format!("llama-{RELEASE}-{backend}")),
            )
            .unwrap();
            assert_eq!(pending_download_bytes(&request), 0, "{backend}: all saved");
        }
    }
    #[test]
    fn a_model_of_the_wrong_size_or_for_another_runtime_still_counts() {
        let temp = Temp::new();
        let request = pending_request(&temp.0, "cpu");
        let model = temp
            .0
            .join("models")
            .join(&request.revision)
            .join(&request.filename);
        saved_file(&model, request.bytes - 1);
        fs::create_dir_all(
            temp.0
                .join("runtime")
                .join(format!("llama-{RELEASE}-vulkan")),
        )
        .unwrap();
        assert_eq!(
            pending_download_bytes(&request),
            runtime_assets("cpu")[0].bytes + request.bytes,
            "a cut-off file is downloaded again, and the Vulkan runtime is not the CPU one"
        );
    }
    #[test]
    fn a_resumable_partial_download_counts_only_for_what_is_left() {
        let temp = Temp::new();
        let request = pending_request(&temp.0, "cpu");
        fs::create_dir_all(temp.0.join("runtime").join(format!("llama-{RELEASE}-cpu"))).unwrap();
        let model = model_artifact(&request);
        let folder = temp.0.join("models").join(&request.revision);
        fs::create_dir_all(&folder).unwrap();
        let partial = folder.join(format!("{}.partial", request.filename));
        fs::File::create(&partial)
            .unwrap()
            .set_len(500_000_000)
            .unwrap();
        let record = folder.join(format!("{}.partial.json", request.filename));
        fs::write(&record, serde_json::to_vec(&model).unwrap()).unwrap();
        assert_eq!(pending_download_bytes(&request), 1_200_000_000);
        let other = Artifact {
            sha256: "d".repeat(64),
            ..model
        };
        fs::write(&record, serde_json::to_vec(&other).unwrap()).unwrap();
        assert_eq!(
            pending_download_bytes(&request),
            request.bytes,
            "a partial file of another file is thrown away, so all of it is downloaded"
        );
    }
}
