//! Persist Irodori codec latents once; real-time synthesis only loads the small voicefile.
use crate::{installer, speech_setup::SpeechInstallation};
use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    fs::{self, File, OpenOptions},
    io::{BufReader, Read, Write},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

const MAX_PROFILE: u64 = 2 * 1024 * 1024;
const MAX_HEADER: u64 = 32 * 1024 * 1024;
const DEFAULT_SEED: u64 = 2_026_110_501;
const DEFAULT_TEXT: &str = "こんにちは、地元翻訳くんです、画面の文章や再生中の音声をこのパソコンの中で翻訳し、自然な声でお伝えします。";

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct VoiceProfile {
    pub id: String,
    /// Native voice alias, never `none`.
    pub name: String,
    pub path: PathBuf,
    pub bytes: u64,
    pub sha256: String,
    pub codec_sha256: String,
    pub seed: u64,
    pub source: String,
}

#[derive(Serialize, Deserialize)]
struct Manifest {
    version: u32,
    profile: VoiceProfile,
}

fn cancelled(cancel: &AtomicBool) -> Result<()> {
    ensure!(
        !cancel.load(Ordering::Acquire),
        "固定声の準備を中止しました。"
    );
    Ok(())
}

fn sha256(path: &Path) -> Result<String> {
    let mut file = File::open(path)?;
    let mut hash = Sha256::new();
    let mut bytes = [0; 64 * 1024];
    loop {
        let n = file.read(&mut bytes)?;
        if n == 0 {
            break;
        }
        hash.update(&bytes[..n]);
    }
    Ok(format!("{:x}", hash.finalize()))
}

fn hex_hash(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
}

fn alias(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value != "none"
        && value
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'_' | b'-'))
}

/// Validate hashes, voicefile structure and codec compatibility without loading a model.
pub fn validate_profile(profile: &VoiceProfile, config: &SpeechInstallation) -> Result<()> {
    ensure!(
        alias(&profile.id)
            && alias(&profile.name)
            && profile.seed < (1_u64 << 53)
            && matches!(
                profile.source.as_str(),
                "generated-default" | "reference-import"
            )
            && hex_hash(&profile.sha256)
            && hex_hash(&profile.codec_sha256),
        "固定声の保存情報が不正です。"
    );
    ensure!(
        profile.path.is_absolute(),
        "固定声の保存先は絶対パスで指定してください。"
    );
    installer::safe_file(&profile.path)?;
    ensure!(
        (1..=MAX_PROFILE).contains(&profile.bytes)
            && fs::metadata(&profile.path)?.len() == profile.bytes,
        "固定声ファイルのサイズが一致しません。"
    );
    ensure!(
        sha256(&profile.path)? == profile.sha256,
        "固定声ファイルの照合に失敗しました。"
    );
    let metadata = gguf_metadata(&profile.path, true)?;
    ensure!(
        metadata.get("irodori-tts-voice.codec_sha256") == Some(&profile.codec_sha256)
            && model_codec(config)? == profile.codec_sha256,
        "この固定声は現在の音声モデルと互換性がありません。"
    );
    Ok(())
}

/// Read only a committed profile. A missing manifest never selects an orphan file.
pub fn load_profile(dir: &Path, config: &SpeechInstallation) -> Result<Option<VoiceProfile>> {
    if !dir.exists() {
        return Ok(None);
    }
    installer::safe_components(dir, false)?;
    let root = fs::canonicalize(dir)?;
    let manifest = root.join("profile.json");
    installer::safe_file(&manifest)?;
    if !manifest.exists() {
        return Ok(None);
    }
    ensure!(
        fs::metadata(&manifest)?.len() <= 64 * 1024,
        "固定声の保存情報が大きすぎます。"
    );
    let manifest: Manifest = serde_json::from_slice(&fs::read(manifest)?)?;
    ensure!(
        manifest.version == 1 && manifest.profile.path.parent() == Some(root.as_path()),
        "固定声の保存情報が保存先と一致しません。"
    );
    let name = manifest
        .profile
        .path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("");
    ensure!(
        name.starts_with("voice-") && name.ends_with(".gguf") && !name.contains(['/', '\\']),
        "固定声のファイル名が不正です。"
    );
    validate_profile(&manifest.profile, config)?;
    Ok(Some(manifest.profile))
}

/// Call on a background worker before real-time reading. The original standard
/// voice is synthesized once, encoded once on CPU, and only its small latent is kept.
pub fn prepare_default_profile(
    config: &SpeechInstallation,
    dir: &Path,
    cancel: Arc<AtomicBool>,
) -> Result<VoiceProfile> {
    prepare(config, dir, None, cancel)
}

/// Import a caller-selected, authorized reference. Its original file is read only;
/// neither its path nor its waveform is copied into the committed profile.
pub fn import_reference_profile(
    config: &SpeechInstallation,
    dir: &Path,
    reference: &Path,
    cancel: Arc<AtomicBool>,
) -> Result<VoiceProfile> {
    prepare(config, dir, Some(reference), cancel)
}

fn prepare(
    config: &SpeechInstallation,
    dir: &Path,
    reference: Option<&Path>,
    cancel: Arc<AtomicBool>,
) -> Result<VoiceProfile> {
    cancelled(&cancel)?;
    ensure!(
        dir.is_absolute(),
        "固定声の保存先は絶対パスで指定してください。"
    );
    installer::safe_components(dir, true)?;
    let root = fs::canonicalize(dir)?;
    let _lock = installer::RootLock::acquire(&root)?;
    if let Some(profile) = load_profile(&root, config)? {
        // Importing a new reference into an existing profile would silently
        // select the wrong voice. Each import needs its own destination.
        ensure!(
            reference.is_none(),
            "別の固定声が保存済みです。新しい保存先を選んでください。"
        );
        return Ok(profile);
    }
    ensure!(
        config.executable.is_absolute()
            && config.executable.is_file()
            && matches!(config.backend.as_str(), "cpu" | "vulkan"),
        "音声モデルと実行環境を先に準備してください。"
    );
    installer::safe_file(&config.executable)?;
    let codec = model_codec(config)?;
    let generated = if reference.is_none() {
        Some(TemporaryFile::new(&root, "reference", "wav")?)
    } else {
        None
    };
    let source = if let Some(reference) = reference {
        installer::safe_file(reference)?;
        reference
    } else {
        let generated = generated.as_ref().unwrap();
        let device = preparation_device(config);
        let mut command = native_command(config);
        command
            .arg("tts")
            .arg(&config.model)
            .args([
                "--voice",
                "none",
                "--seed",
                &DEFAULT_SEED.to_string(),
                "--speed",
                "1",
                "--device",
                device,
            ])
            .args([
                "--instructions",
                "明瞭で落ち着いた自然な声で、穏やかに読み上げてください。",
            ])
            .arg("-o")
            .arg(&generated.path);
        run_command(command, Some(DEFAULT_TEXT.as_bytes()), &cancel)?;
        &generated.path
    };
    validate_reference(source)?;
    cancelled(&cancel)?;
    let voice = TemporaryFile::new(&root, "pending-voice", "gguf")?;
    let mut command = native_command(config);
    // CPU encodes only the codec encoder. No HTTP port, reference re-encoding
    // at playback, or second full GPU synthesis model is required here.
    command
        .arg("voice")
        .arg(&config.model)
        .arg(source)
        .arg(&voice.path)
        .args(["--device", "cpu"]);
    run_command(command, None, &cancel)?;
    if let Some(generated) = &generated {
        // A successful preparation must not leave the original generated WAV.
        // Drop retries cleanup on every error/cancellation path as well.
        generated.remove()?;
    }
    cancelled(&cancel)?;
    ensure!(
        fs::metadata(&voice.path)?.len() <= MAX_PROFILE,
        "固定声ファイルが大きすぎます。"
    );
    let metadata = gguf_metadata(&voice.path, true)?;
    ensure!(
        metadata.get("irodori-tts-voice.codec_sha256") == Some(&codec),
        "作成した固定声の互換性を確認できませんでした。"
    );
    let id = if reference.is_none() {
        "jimoto-default-v1".to_owned()
    } else {
        format!("import-{}", uuid::Uuid::new_v4())
    };
    let profile = VoiceProfile {
        id,
        name: "jimoto_fixed_v1".into(),
        path: root.join(format!("voice-{}.gguf", uuid::Uuid::new_v4())),
        bytes: fs::metadata(&voice.path)?.len(),
        sha256: sha256(&voice.path)?,
        codec_sha256: codec,
        seed: DEFAULT_SEED,
        source: if reference.is_none() {
            "generated-default"
        } else {
            "reference-import"
        }
        .into(),
    };
    // Immutable voicefiles are committed before the atomic manifest. A crash
    // between these steps leaves only a harmless orphan, never a half profile.
    OpenOptions::new()
        .write(true)
        .open(&voice.path)?
        .sync_all()?;
    fs::rename(&voice.path, &profile.path)?;
    cancelled(&cancel)?;
    installer::atomic_write(
        &root.join("profile.json"),
        &serde_json::to_vec_pretty(&Manifest {
            version: 1,
            profile: profile.clone(),
        })?,
    )?;
    // `generated` drops here on every success/error/cancellation path. Imported
    // reference paths never enter TemporaryFile, so they cannot be removed.
    Ok(profile)
}

fn preparation_device(config: &SpeechInstallation) -> &'static str {
    let free = config
        .model
        .parent()
        .and_then(|p| crate::hardware::detect(p).ok())
        .and_then(|h| {
            h.gpus
                .into_iter()
                .filter(|g| !g.is_software && g.vulkan_eligible)
                .filter_map(|g| Some(g.free_vram_bytes?.min(g.dedicated_vram_bytes?)))
                .max()
        });
    if config.backend == "vulkan" && free.is_some_and(|n| n >= 4 * 1_073_741_824) {
        "gpu"
    } else {
        "cpu"
    }
}

fn native_command(config: &SpeechInstallation) -> Command {
    let mut command = Command::new(&config.executable);
    command.stdout(Stdio::null()).stderr(Stdio::null());
    if let Some(parent) = config.executable.parent() {
        command.current_dir(parent);
    }
    if let Some(parent) = config.model.parent() {
        command.env("SPEECH_MODEL_DIR", parent);
    }
    crate::platform::hide_console(&mut command);
    command
}

struct OwnedChild(Child);
impl Drop for OwnedChild {
    fn drop(&mut self) {
        if self.0.try_wait().ok().flatten().is_none() {
            let _ = self.0.kill();
        }
        let _ = self.0.wait();
    }
}

fn run_command(mut command: Command, input: Option<&[u8]>, cancel: &AtomicBool) -> Result<()> {
    cancelled(cancel)?;
    command.stdin(if input.is_some() {
        Stdio::piped()
    } else {
        Stdio::null()
    });
    let mut child = OwnedChild(
        command
            .spawn()
            .context("固定声の作成を開始できませんでした。")?,
    );
    if let Some(input) = input {
        let mut stdin = child
            .0
            .stdin
            .take()
            .context("固定声用の文章を渡せませんでした。")?;
        stdin.write_all(input)?;
        stdin.write_all(b"\n")?;
    }
    let deadline = Instant::now() + Duration::from_secs(300);
    loop {
        cancelled(cancel)?;
        if let Some(status) = child.0.try_wait()? {
            ensure!(
                status.success(),
                "固定声の準備に失敗しました（終了コード {:?}）。",
                status.code()
            );
            return Ok(());
        }
        ensure!(
            Instant::now() < deadline,
            "固定声の準備が時間内に完了しませんでした。"
        );
        thread::sleep(Duration::from_millis(25));
    }
}

struct TemporaryFile {
    path: PathBuf,
    root: PathBuf,
}
impl TemporaryFile {
    fn new(root: &Path, prefix: &str, extension: &str) -> Result<Self> {
        let path = root.join(format!(".{prefix}-{}.{}", uuid::Uuid::new_v4(), extension));
        OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)?;
        Ok(Self {
            path,
            root: root.to_path_buf(),
        })
    }
    fn remove(&self) -> Result<()> {
        ensure!(
            self.path.parent() == Some(self.root.as_path()),
            "一時音声の保存先が不正です。"
        );
        installer::safe_file(&self.path)?;
        match fs::remove_file(&self.path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error).context("一時音声を削除できませんでした。"),
        }
    }
}
impl Drop for TemporaryFile {
    fn drop(&mut self) {
        let _ = self.remove();
    }
}

fn validate_reference(path: &Path) -> Result<()> {
    ensure!(
        fs::metadata(path)?.len() <= 64 * 1024 * 1024,
        "参照音声は64MiB以内のWAVを選んでください。"
    );
    let mut reader =
        hound::WavReader::open(path).context("参照音声をWAVとして読み取れませんでした。")?;
    let spec = reader.spec();
    ensure!(
        (1..=2).contains(&spec.channels) && (8_000..=192_000).contains(&spec.sample_rate),
        "参照音声のチャンネル数またはサンプルレートを確認してください。"
    );
    let seconds = reader.duration() as f64 / spec.sample_rate as f64;
    ensure!(
        (1.0..=120.0).contains(&seconds),
        "参照音声は1秒以上120秒以内のWAVを選んでください。"
    );
    let (mut energy, mut count) = (0.0_f64, 0_u64);
    match spec.sample_format {
        hound::SampleFormat::Float => {
            for sample in reader.samples::<f32>() {
                let sample = sample?;
                ensure!(
                    sample.is_finite(),
                    "参照音声に不正なサンプルが含まれています。"
                );
                energy += (sample as f64).powi(2);
                count += 1;
            }
        }
        hound::SampleFormat::Int => {
            ensure!(
                (1..=32).contains(&spec.bits_per_sample),
                "参照音声のビット深度を確認してください。"
            );
            let scale = 2_f64.powi(spec.bits_per_sample as i32 - 1);
            for sample in reader.samples::<i32>() {
                energy += (sample? as f64 / scale).powi(2);
                count += 1;
            }
        }
    }
    ensure!(
        count > 0 && (energy / count as f64).sqrt() >= 0.00001,
        "参照音声が無音です。声の入ったWAVを選んでください。"
    );
    Ok(())
}

fn model_codec(config: &SpeechInstallation) -> Result<String> {
    ensure!(
        config.model.is_absolute(),
        "音声モデルは絶対パスで指定してください。"
    );
    installer::safe_file(&config.model)?;
    let metadata = gguf_metadata(&config.model, false)?;
    ensure!(
        metadata
            .get("general.architecture")
            .is_some_and(|s| s == "irodori-tts"),
        "Irodori音声モデルを確認できませんでした。"
    );
    let codec = metadata
        .get("irodori-tts.codec.sha256")
        .context("音声モデルにコーデックの照合情報がありません。")?;
    ensure!(
        hex_hash(codec),
        "音声モデルのコーデック照合情報が不正です。"
    );
    Ok(codec.clone())
}

struct HeaderReader {
    inner: BufReader<File>,
    position: u64,
    length: u64,
}
impl HeaderReader {
    fn new(path: &Path) -> Result<Self> {
        let file = File::open(path)?;
        let length = file.metadata()?.len();
        Ok(Self {
            inner: BufReader::new(file),
            position: 0,
            length,
        })
    }
    fn read_exact(&mut self, bytes: &mut [u8]) -> Result<()> {
        let end = self
            .position
            .checked_add(bytes.len() as u64)
            .context("GGUFの長さが不正です。")?;
        ensure!(
            end <= MAX_HEADER && end <= self.length,
            "GGUFの保存情報が途中で切れています。"
        );
        self.inner.read_exact(bytes)?;
        self.position = end;
        Ok(())
    }
    fn skip(&mut self, bytes: u64) -> Result<()> {
        let end = self
            .position
            .checked_add(bytes)
            .context("GGUFの長さが不正です。")?;
        ensure!(
            end <= MAX_HEADER && end <= self.length,
            "GGUFの保存情報が途中で切れています。"
        );
        self.inner.seek_relative(bytes as i64)?;
        self.position = end;
        Ok(())
    }
}

fn u32_value(file: &mut HeaderReader) -> Result<u32> {
    let mut b = [0; 4];
    file.read_exact(&mut b)?;
    Ok(u32::from_le_bytes(b))
}
fn u64_value(file: &mut HeaderReader) -> Result<u64> {
    let mut b = [0; 8];
    file.read_exact(&mut b)?;
    Ok(u64::from_le_bytes(b))
}
fn string_value(file: &mut HeaderReader) -> Result<String> {
    let size = u64_value(file)?;
    ensure!(size <= 1024 * 1024, "GGUFの文字列が大きすぎます。");
    let mut bytes = vec![0; size as usize];
    file.read_exact(&mut bytes)?;
    Ok(String::from_utf8(bytes)?)
}
fn skip_value(file: &mut HeaderReader, kind: u32, depth: u32) -> Result<()> {
    ensure!(depth <= 2, "GGUFの配列が深すぎます。");
    let bytes = match kind {
        0 | 1 | 7 => 1,
        2 | 3 => 2,
        4..=6 => 4,
        10..=12 => 8,
        8 => u64_value(file)?,
        9 => {
            let kind = u32_value(file)?;
            let count = u64_value(file)?;
            ensure!(count <= 1_000_000, "GGUFの配列が大きすぎます。");
            for _ in 0..count {
                skip_value(file, kind, depth + 1)?;
            }
            return Ok(());
        }
        _ => bail!("GGUFの値の型が不正です。"),
    };
    ensure!(bytes <= MAX_HEADER, "GGUFの値が大きすぎます。");
    file.skip(bytes)
}

fn gguf_metadata(path: &Path, voice: bool) -> Result<HashMap<String, String>> {
    let mut file = HeaderReader::new(path)?;
    let mut magic = [0; 4];
    file.read_exact(&mut magic)?;
    ensure!(
        &magic == b"GGUF" && matches!(u32_value(&mut file)?, 2 | 3),
        "固定声またはモデルがGGUF形式ではありません。"
    );
    let tensors = u64_value(&mut file)?;
    let count = u64_value(&mut file)?;
    ensure!(
        count <= 100_000 && tensors > 0,
        "GGUFの保存情報が不正です。"
    );
    let mut output = HashMap::new();
    let mut alignment = 32_u64;
    let mut layout = 0;
    for _ in 0..count {
        let key = string_value(&mut file)?;
        let kind = u32_value(&mut file)?;
        if matches!(
            key.as_str(),
            "general.architecture"
                | "irodori-tts.codec.sha256"
                | "irodori-tts-voice.codec_sha256"
                | "irodori-tts-voice.source"
        ) {
            ensure!(
                kind == 8 && !output.contains_key(&key),
                "GGUFの照合情報が不正です。"
            );
            output.insert(key, string_value(&mut file)?);
        } else if key == "general.alignment" || key == "speech.layout" {
            ensure!(kind == 4, "GGUFの配置情報が不正です。");
            let value = u32_value(&mut file)?;
            if key == "general.alignment" {
                alignment = value as u64;
            } else {
                layout = value;
            }
        } else {
            skip_value(&mut file, kind, 0)?;
        }
        ensure!(
            file.position <= MAX_HEADER,
            "GGUFの保存情報が大きすぎます。"
        );
    }
    if voice {
        ensure!(
            tensors == 1
                && output
                    .get("general.architecture")
                    .is_some_and(|s| s == "irodori-tts-voice")
                && matches!(layout, 1 | 2)
                && output
                    .get("irodori-tts-voice.source")
                    .is_none_or(|s| s == "references"),
            "参照音声から作成したIrodori固定声ではありません。"
        );
        ensure!(
            string_value(&mut file)? == "latent" && u32_value(&mut file)? == 2,
            "固定声の音声特徴情報が不正です。"
        );
        let width = u64_value(&mut file)?;
        let frames = u64_value(&mut file)?;
        ensure!(
            width == 32 && (1..=12_000).contains(&frames) && u32_value(&mut file)? == 0,
            "固定声の音声特徴形式が不正です。"
        );
        let offset = u64_value(&mut file)?;
        ensure!(
            (1..=4096).contains(&alignment) && alignment.is_power_of_two(),
            "固定声の配置が不正です。"
        );
        let data = file.position.div_ceil(alignment) * alignment;
        let end = data
            .checked_add(offset)
            .and_then(|n| n.checked_add(width * frames * 4))
            .context("固定声の長さが不正です。")?;
        ensure!(end <= file.length, "固定声の音声特徴が途中で切れています。");
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Directory(PathBuf);
    impl Directory {
        fn new() -> Self {
            let path =
                std::env::temp_dir().join(format!("jimoto-voice-test-{}", uuid::Uuid::new_v4()));
            fs::create_dir(&path).unwrap();
            Self(fs::canonicalize(path).unwrap())
        }
    }
    impl Drop for Directory {
        fn drop(&mut self) {
            if let (Ok(path), Ok(temp)) = (
                fs::canonicalize(&self.0),
                fs::canonicalize(std::env::temp_dir()),
            ) && path.starts_with(&temp)
                && path != temp
                && path
                    .file_name()
                    .is_some_and(|n| n.to_string_lossy().starts_with("jimoto-voice-test-"))
            {
                let _ = fs::remove_dir_all(path);
            }
        }
    }
    fn string(bytes: &mut Vec<u8>, text: &str) {
        bytes.extend((text.len() as u64).to_le_bytes());
        bytes.extend(text.as_bytes());
    }
    fn fixture(path: &Path, codec: &str, voice: bool) {
        let mut bytes = b"GGUF".to_vec();
        bytes.extend(3_u32.to_le_bytes());
        bytes.extend(1_u64.to_le_bytes());
        bytes.extend(if voice { 4_u64 } else { 2_u64 }.to_le_bytes());
        for (key, value) in [
            (
                "general.architecture",
                if voice {
                    "irodori-tts-voice"
                } else {
                    "irodori-tts"
                },
            ),
            (
                if voice {
                    "irodori-tts-voice.codec_sha256"
                } else {
                    "irodori-tts.codec.sha256"
                },
                codec,
            ),
        ] {
            string(&mut bytes, key);
            bytes.extend(8_u32.to_le_bytes());
            string(&mut bytes, value);
        }
        if voice {
            string(&mut bytes, "speech.layout");
            bytes.extend(4_u32.to_le_bytes());
            bytes.extend(2_u32.to_le_bytes());
            string(&mut bytes, "irodori-tts-voice.source");
            bytes.extend(8_u32.to_le_bytes());
            string(&mut bytes, "references");
            string(&mut bytes, "latent");
            bytes.extend(2_u32.to_le_bytes());
            bytes.extend(32_u64.to_le_bytes());
            bytes.extend(1_u64.to_le_bytes());
            bytes.extend(0_u32.to_le_bytes());
            bytes.extend(0_u64.to_le_bytes());
            bytes.resize(bytes.len().div_ceil(32) * 32 + 128, 0);
        }
        fs::write(path, bytes).unwrap();
    }
    fn config(root: &Path) -> SpeechInstallation {
        let model = root.join("model.gguf");
        fixture(&model, &"a".repeat(64), false);
        SpeechInstallation {
            executable: std::env::current_exe().unwrap(),
            model,
            backend: "cpu".into(),
            voice: "none".into(),
        }
    }
    fn saved(root: &Path) -> VoiceProfile {
        let path = root.join("voice-fixture.gguf");
        fixture(&path, &"a".repeat(64), true);
        let profile = VoiceProfile {
            id: "jimoto-default-v1".into(),
            name: "jimoto_fixed_v1".into(),
            bytes: fs::metadata(&path).unwrap().len(),
            sha256: sha256(&path).unwrap(),
            path,
            codec_sha256: "a".repeat(64),
            seed: DEFAULT_SEED,
            source: "generated-default".into(),
        };
        fs::write(
            root.join("profile.json"),
            serde_json::to_vec(&Manifest {
                version: 1,
                profile: profile.clone(),
            })
            .unwrap(),
        )
        .unwrap();
        profile
    }
    #[test]
    fn profile_reload_preserves_identity_seed_and_detects_tampering() {
        let dir = Directory::new();
        let config = config(&dir.0);
        let profile = saved(&dir.0);
        assert_eq!(
            load_profile(&dir.0, &config).unwrap(),
            Some(profile.clone())
        );
        let mut bytes = fs::read(&profile.path).unwrap();
        *bytes.last_mut().unwrap() = 1;
        fs::write(&profile.path, bytes).unwrap();
        assert!(
            load_profile(&dir.0, &config)
                .unwrap_err()
                .to_string()
                .contains("照合")
        );
    }
    #[test]
    fn incompatible_codec_and_manifest_paths_are_rejected() {
        let dir = Directory::new();
        let config = config(&dir.0);
        let profile = saved(&dir.0);
        fixture(&config.model, &"b".repeat(64), false);
        assert!(
            validate_profile(&profile, &config)
                .unwrap_err()
                .to_string()
                .contains("互換性")
        );
        let other = Directory::new();
        let mut moved = profile;
        moved.path = other.0.join("voice-other.gguf");
        fs::write(
            dir.0.join("profile.json"),
            serde_json::to_vec(&Manifest {
                version: 1,
                profile: moved,
            })
            .unwrap(),
        )
        .unwrap();
        assert!(
            load_profile(&dir.0, &config)
                .unwrap_err()
                .to_string()
                .contains("保存先")
        );
    }
    #[test]
    fn truncated_voice_and_uncommitted_files_are_not_loaded() {
        let dir = Directory::new();
        let config = config(&dir.0);
        fixture(&dir.0.join("voice-orphan.gguf"), &"a".repeat(64), true);
        assert!(load_profile(&dir.0, &config).unwrap().is_none());
        let profile = saved(&dir.0);
        OpenOptions::new()
            .write(true)
            .open(&profile.path)
            .unwrap()
            .set_len(profile.bytes - 1)
            .unwrap();
        assert!(gguf_metadata(&profile.path, true).is_err());
    }
    #[test]
    fn cancellation_and_temporary_cleanup_preserve_original_reference() {
        let dir = Directory::new();
        let config = config(&dir.0);
        let original = dir.0.join("original.wav");
        fs::write(&original, b"original user data").unwrap();
        let original_hash = sha256(&original).unwrap();
        let temp_path;
        {
            let temporary = TemporaryFile::new(&dir.0, "reference", "wav").unwrap();
            temp_path = temporary.path.clone();
            fs::write(&temp_path, b"generated").unwrap();
        }
        assert!(!temp_path.exists());
        let destination = dir.0.join("not-created");
        assert!(
            import_reference_profile(
                &config,
                &destination,
                &original,
                Arc::new(AtomicBool::new(true))
            )
            .is_err()
        );
        assert!(!destination.exists());
        assert_eq!(sha256(&original).unwrap(), original_hash);
    }
    #[test]
    fn reference_validation_rejects_silence_and_accepts_voiced_pcm() {
        let dir = Directory::new();
        for (name, amplitude, valid) in [("silent.wav", 0.0, false), ("voiced.wav", 0.1, true)] {
            let path = dir.0.join(name);
            let mut wav = hound::WavWriter::create(
                &path,
                hound::WavSpec {
                    channels: 1,
                    sample_rate: 16_000,
                    bits_per_sample: 16,
                    sample_format: hound::SampleFormat::Int,
                },
            )
            .unwrap();
            for i in 0..16_000 {
                wav.write_sample(((i as f32 * 0.1).sin() * amplitude * 32767.0) as i16)
                    .unwrap();
            }
            wav.finalize().unwrap();
            assert_eq!(validate_reference(&path).is_ok(), valid);
        }
    }

    /// Explicit diagnostic only. This creates original audio files but never
    /// opens an audio playback device or touches the app's port 17863.
    #[test]
    #[ignore = "requires explicit installed native model; generates WAV but never plays"]
    fn native_default_profile_and_repeated_voicefile_synthesis() -> Result<()> {
        let data = PathBuf::from(
            std::env::var_os("PLAMO_VOICE_CHECK_DATA")
                .context("Set PLAMO_VOICE_CHECK_DATA explicitly")?,
        );
        let config = crate::speech_setup::load(&data.join("speech"))?
            .context("Native speech installation missing")?;
        let dir = data.join("speech/voices/default-v1");
        let cancel = Arc::new(AtomicBool::new(false));
        let start = Instant::now();
        let profile = prepare_default_profile(&config, &dir, cancel.clone())?;
        let prepare_ms = start.elapsed().as_millis();
        let reload_started = Instant::now();
        ensure!(
            load_profile(&dir, &config)? == Some(profile.clone()),
            "Profile changed after reload"
        );
        let reload_ms = reload_started.elapsed().as_millis();
        ensure!(
            prepare_default_profile(&config, &dir, cancel.clone())? == profile,
            "Preparation did not reuse existing profile"
        );
        let temporary = Directory::new();
        let mut trials = Vec::new();
        for (index, rate) in [1.0, 1.6].into_iter().enumerate() {
            let output = TemporaryFile::new(&temporary.0, "preview", "wav")?;
            let mut command = native_command(&config);
            command
                .arg("tts")
                .arg(&config.model)
                .arg("--add-voice")
                .arg(format!("{}={}", profile.name, profile.path.display()))
                .args([
                    "--voice",
                    &profile.name,
                    "--seed",
                    &profile.seed.to_string(),
                    "--speed",
                    &rate.to_string(),
                    "--device",
                    preparation_device(&config),
                ])
                .arg("-o")
                .arg(&output.path);
            let started = Instant::now();
            run_command(
                command,
                Some("固定した声で、自然に読み上げます。".as_bytes()),
                &cancel,
            )?;
            validate_reference(&output.path)?;
            let wav = hound::WavReader::open(&output.path)?;
            trials.push(serde_json::json!({"index":index,"rate":rate,"elapsed_ms":started.elapsed().as_millis(),"audio_seconds":wav.duration() as f64/wav.spec().sample_rate as f64}));
            // Exercise reference import and prove it does not alter the input.
            if index == 0 {
                let original_sha = sha256(&output.path)?;
                let imported = import_reference_profile(
                    &config,
                    &temporary.0.join("imported"),
                    &output.path,
                    cancel.clone(),
                )?;
                ensure!(
                    imported.source == "reference-import" && sha256(&output.path)? == original_sha,
                    "Import changed reference"
                );
            }
        }
        ensure!(
            load_profile(&dir, &config)? == Some(profile.clone()),
            "Synthesis changed profile"
        );
        ensure!(
            !fs::read_dir(&dir)?
                .any(|entry| entry.is_ok_and(|e| e.path().extension().is_some_and(|s| s == "wav"))),
            "Generated reference remained"
        );
        println!(
            "{}",
            serde_json::to_string_pretty(
                &serde_json::json!({"success":true,"played":false,"profile":profile,"prepare_ms":prepare_ms,"reload_ms":reload_ms,"trials":trials,"reference_cleaned":true,"import_original_preserved":true})
            )?
        );
        Ok(())
    }
}
