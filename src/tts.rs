//! Native speech.cpp synthesis, latest-only pending text, and owned Windows audio.
//! Text/audio stays in memory and is sent only to our own loopback child.
use crate::speech_setup::SpeechInstallation;
use anyhow::{Context, Result, bail};
use reqwest::blocking::Client;
use serde_json::{Value, json};
use std::{
    io::Read,
    net::{SocketAddr, TcpListener},
    process::{Child, Command, Stdio},
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc::{self, Receiver, Sender},
    },
    thread,
    time::{Duration, Instant},
};

const ENDPOINT: &str = "http://127.0.0.1:17863";
const MAX_TEXT: usize = 8_000;
const MAX_CHUNK: usize = 80;
const MAX_WAV: u64 = 16 * 1024 * 1024;
const TAIL: Duration = Duration::from_millis(180);

#[derive(Clone, Debug)]
pub enum TtsEvent {
    Status(String),
    Started,
    Finished,
    Error(String),
}

struct Job {
    text: String,
    epoch: u64,
}

struct Shared {
    pending: Mutex<Option<Job>>,
    wake: Condvar,
    quit: AtomicBool,
    epoch: AtomicU64,
    busy: AtomicBool,
    child: Mutex<Option<Child>>,
    // A waveOut handle, not the process-global PlaySound API.
    playback: Mutex<Option<usize>>,
    suppress: Arc<AtomicBool>,
    events: Sender<TtsEvent>,
}

pub struct TtsEngine {
    shared: Arc<Shared>,
    events: Receiver<TtsEvent>,
    worker: Option<thread::JoinHandle<()>>,
}

impl TtsEngine {
    /// Start a lightweight worker. Native model loading is deferred until speak().
    pub fn start(config: SpeechInstallation, suppress_audio: Arc<AtomicBool>) -> Result<Self> {
        anyhow::ensure!(cfg!(windows), "この読み上げ再生機能はWindows専用です。");
        anyhow::ensure!(
            config.executable.is_absolute()
                && config.executable.is_file()
                && config.model.is_absolute()
                && config.model.is_file(),
            "読み上げ用の実行ファイルまたはモデルがありません。読み上げの準備を実行してください。"
        );
        anyhow::ensure!(
            config
                .model
                .extension()
                .is_some_and(|s| s.eq_ignore_ascii_case("gguf")),
            "読み上げには取得済みのGGUFモデルを指定してください。"
        );
        anyhow::ensure!(
            matches!(config.backend.as_str(), "cpu" | "vulkan") && config.voice == "none",
            "読み上げの実行方式または声の設定を確認してください。"
        );
        let (tx, events) = mpsc::channel();
        let shared = Arc::new(Shared {
            pending: Mutex::new(None),
            wake: Condvar::new(),
            quit: AtomicBool::new(false),
            epoch: AtomicU64::new(0),
            busy: AtomicBool::new(false),
            child: Mutex::new(None),
            playback: Mutex::new(None),
            suppress: suppress_audio,
            events: tx,
        });
        let state = shared.clone();
        let worker = thread::Builder::new()
            .name("native-irodori-tts".into())
            .spawn(move || worker(config, state))
            .context("読み上げ処理を開始できませんでした。")?;
        Ok(Self {
            shared,
            events,
            worker: Some(worker),
        })
    }

    /// Keep only the newest waiting utterance. An active utterance finishes
    /// normally; stop() cancels both the active and waiting utterances.
    pub fn speak(&self, text: String) {
        let text = text.trim().to_owned();
        if text.is_empty() {
            return;
        }
        if text.chars().count() > MAX_TEXT {
            let _ = self.shared.events.send(TtsEvent::Error(
                "読み上げる文章は8000文字以内にしてください。".into(),
            ));
            return;
        }
        let mut pending = self
            .shared
            .pending
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        *pending = Some(Job {
            text,
            epoch: self.shared.epoch.load(Ordering::Acquire),
        });
        self.shared.busy.store(true, Ordering::Release);
        self.shared.wake.notify_one();
    }

    pub fn stop(&self) {
        self.shared.epoch.fetch_add(1, Ordering::AcqRel);
        *self
            .shared
            .pending
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = None;
        self.shared.busy.store(false, Ordering::Release);
        reset_playback(&self.shared);
        // Disconnecting stops speech.cpp's request. Killing only our own child
        // also interrupts model loading / a blocking HTTP generation immediately.
        if let Some(child) = self
            .shared
            .child
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_mut()
        {
            let _ = child.kill();
        }
        let _ = self
            .shared
            .events
            .send(TtsEvent::Status("読み上げを停止しました。".into()));
        self.shared.wake.notify_one();
    }

    pub fn poll(&self) -> Vec<TtsEvent> {
        self.events.try_iter().collect()
    }
    pub fn is_busy(&self) -> bool {
        self.shared.busy.load(Ordering::Acquire)
    }
}

impl Drop for TtsEngine {
    fn drop(&mut self) {
        self.shared.quit.store(true, Ordering::Release);
        self.stop();
        self.shared.wake.notify_one();
        // The worker owns native buffers until waveOutReset/unprepare finishes.
        // Joining here keeps those buffers alive and prevents a detached child.
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

fn cancelled(shared: &Shared, epoch: u64) -> bool {
    shared.quit.load(Ordering::Acquire) || shared.epoch.load(Ordering::Acquire) != epoch
}

fn worker(config: SpeechInstallation, shared: Arc<Shared>) {
    let client = match Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(2))
        .timeout(Duration::from_secs(180))
        .build()
    {
        Ok(client) => client,
        Err(_) => {
            let _ = shared.events.send(TtsEvent::Error(
                "読み上げ用のローカル接続を準備できませんでした。".into(),
            ));
            return;
        }
    };
    let mut model_id = None;
    loop {
        let job = {
            let mut pending = shared.pending.lock().unwrap_or_else(|e| e.into_inner());
            while pending.is_none() && !shared.quit.load(Ordering::Acquire) {
                pending = shared.wake.wait(pending).unwrap_or_else(|e| e.into_inner());
            }
            if shared.quit.load(Ordering::Acquire) {
                break;
            }
            pending.take().unwrap()
        };
        if cancelled(&shared, job.epoch) {
            continue;
        }
        let mut played = false;
        // Reuse a seed across chunks so "none" does not choose anew every request.
        let seed = (uuid::Uuid::new_v4().as_u128() as u64) & ((1u64 << 53) - 1);
        let result = (|| -> Result<()> {
            if !owned_child_alive(&shared) {
                model_id = None;
            }
            if model_id.is_none() {
                model_id = Some(start_server(&config, &shared, &client, job.epoch)?);
            }
            for chunk in chunks(&job.text) {
                if cancelled(&shared, job.epoch) {
                    return Ok(());
                }
                let owned_pid = shared
                    .child
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .as_ref()
                    .map(Child::id);
                anyhow::ensure!(
                    owned_child_alive(&shared)
                        && owned_pid.is_some()
                        && listener_owner(17863)? == owned_pid,
                    "彩りTTSへのローカル接続が切れました。"
                );
                let _ = shared
                    .events
                    .send(TtsEvent::Status("読み上げ音声を作っています。".into()));
                let bytes = synthesize(
                    &client,
                    model_id.as_ref().unwrap(),
                    &config.voice,
                    &chunk,
                    seed,
                )?;
                if cancelled(&shared, job.epoch) {
                    return Ok(());
                }
                play_wav(&bytes, &shared, job.epoch, !played)?;
                played = true;
            }
            Ok(())
        })();
        if let Err(error) = result {
            if !cancelled(&shared, job.epoch) {
                let _ = shared.events.send(TtsEvent::Error(format!(
                    "読み上げに失敗しました。{error:#}"
                )));
            }
            model_id = None;
            reap_child(&shared);
        }
        if played {
            let _ = shared.events.send(TtsEvent::Finished);
        }
        let pending = shared.pending.lock().unwrap_or_else(|e| e.into_inner());
        shared.busy.store(pending.is_some(), Ordering::Release);
    }
    reset_playback(&shared);
    reap_child(&shared);
    shared.suppress.store(false, Ordering::Release);
    shared.busy.store(false, Ordering::Release);
}

fn owned_child_alive(shared: &Shared) -> bool {
    let mut owned = shared.child.lock().unwrap_or_else(|e| e.into_inner());
    owned
        .as_mut()
        .is_some_and(|child| matches!(child.try_wait(), Ok(None)))
}

fn reap_child(shared: &Shared) {
    // v0.10.1 tools/server/served-models.cpp loads the native model in-process;
    // tools/server/audio-speech.cpp synthesizes on std::thread, not a subprocess.
    let child = shared
        .child
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .take();
    if let Some(mut child) = child {
        let _ = child.kill();
        let limit = Instant::now() + Duration::from_secs(3);
        while matches!(child.try_wait(), Ok(None)) && Instant::now() < limit {
            thread::sleep(Duration::from_millis(10));
        }
    }
}

fn start_server(
    config: &SpeechInstallation,
    shared: &Shared,
    client: &Client,
    epoch: u64,
) -> Result<String> {
    reap_child(shared);
    // Never attach to or stop an unrelated server, even one advertising speech.cpp.
    let probe = TcpListener::bind(SocketAddr::from(([127, 0, 0, 1], 17863)))
        .context("読み上げ用ポート17863が使用中です。ほかの読み上げ処理を確認してください。")?;
    drop(probe);
    let _ = shared
        .events
        .send(TtsEvent::Status("彩りTTSを読み込んでいます。".into()));
    // This runs only on the background worker, once per owned server start.
    // The other models can have consumed VRAM since initial installation.
    let free_vram = if config.backend == "vulkan" {
        config
            .model
            .parent()
            .and_then(|path| crate::hardware::detect(path).ok())
            .and_then(|hardware| {
                hardware
                    .gpus
                    .iter()
                    .filter(|gpu| !gpu.is_software && gpu.vulkan_eligible)
                    .filter_map(|gpu| Some(gpu.free_vram_bytes?.min(gpu.dedicated_vram_bytes?)))
                    .max()
            })
    } else {
        None
    };
    let device = startup_device(&config.backend, free_vram);
    if config.backend == "vulkan" && device == "cpu" {
        let _ = shared.events.send(TtsEvent::Status(
            "GPUメモリーの不足を避けるため、CPUで読み上げます。".into(),
        ));
    }
    let mut command = Command::new(&config.executable);
    command
        .args(["serve"])
        .arg(&config.model)
        .args(["--host", "127.0.0.1", "--port", "17863", "--device", device])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    if let Some(parent) = config.executable.parent() {
        command.current_dir(parent);
    }
    if let Some(parent) = config.model.parent() {
        command.env("SPEECH_MODEL_DIR", parent);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    {
        let mut owned = shared.child.lock().unwrap_or_else(|e| e.into_inner());
        if cancelled(shared, epoch) {
            bail!("読み上げを停止しました。");
        }
        *owned = Some(
            command
                .spawn()
                .context("speech.cppを起動できませんでした。")?,
        );
    }
    let deadline = Instant::now() + Duration::from_secs(180);
    while Instant::now() < deadline {
        if cancelled(shared, epoch) {
            bail!("読み上げを停止しました。");
        }
        anyhow::ensure!(
            owned_child_alive(shared),
            "彩りTTSの起動に失敗しました。GPU・モデルの設定を確認してください。"
        );
        let owned_pid = shared
            .child
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
            .map(Child::id);
        match listener_owner(17863)? {
            Some(pid) if Some(pid) == owned_pid => {}
            Some(_) => bail!("読み上げ用ポート17863を別の処理が使用しています。"),
            None => {
                thread::sleep(Duration::from_millis(100));
                continue;
            }
        }
        let health = client
            .get(format!("{ENDPOINT}/health"))
            .timeout(Duration::from_secs(1))
            .send();
        if health.is_ok_and(|response| response.status().is_success()) {
            let response = client
                .get(format!("{ENDPOINT}/v1/models"))
                .timeout(Duration::from_secs(2))
                .send()?;
            let mut body = Vec::new();
            response.take(128 * 1024).read_to_end(&mut body)?;
            let models: Value = serde_json::from_slice(&body)?;
            let id = irodori_model_id(&models)
                .context("起動した読み上げモデルを確認できませんでした。")?;
            anyhow::ensure!(owned_child_alive(shared), "彩りTTSが起動中に終了しました。");
            return Ok(id);
        }
        thread::sleep(Duration::from_millis(100));
    }
    bail!("彩りTTSの読み込みが時間内に完了しませんでした。")
}

fn startup_device(configured: &str, free_vram: Option<u64>) -> &'static str {
    if configured == "vulkan" && free_vram.is_some_and(|free| free >= 4 * 1_073_741_824) {
        "gpu"
    } else {
        "cpu"
    }
}

#[cfg(windows)]
fn listener_owner(port: u16) -> Result<Option<u32>> {
    // Match the loopback socket to our actual child PID before sending text.
    // This also closes the free-port-check / slow model-load binding race.
    #[link(name = "iphlpapi")]
    unsafe extern "system" {
        #[link_name = "GetExtendedTcpTable"]
        fn tcp_table(
            table: *mut std::ffi::c_void,
            size: *mut u32,
            order: i32,
            family: u32,
            class: i32,
            reserved: u32,
        ) -> u32;
    }
    let mut size = 0;
    let _ = unsafe { tcp_table(std::ptr::null_mut(), &mut size, 0, 2, 3, 0) };
    for _ in 0..3 {
        anyhow::ensure!(
            (4..=1_048_576).contains(&size),
            "読み上げの接続先を確認できませんでした。"
        );
        let mut table = vec![0u32; (size as usize).div_ceil(4)];
        let result = unsafe { tcp_table(table.as_mut_ptr().cast(), &mut size, 0, 2, 3, 0) };
        if result == 122 {
            continue;
        }
        anyhow::ensure!(result == 0, "読み上げの接続先を確認できませんでした。");
        let count = table[0] as usize;
        anyhow::ensure!(
            count <= (table.len() - 1) / 6,
            "読み上げの接続先情報が不正です。"
        );
        for row in table[1..].as_chunks::<6>().0.iter().take(count) {
            if u16::from_be(row[2] as u16) == port
                && (row[1] == 0 || row[1] == u32::from_ne_bytes([127, 0, 0, 1]))
            {
                return Ok(Some(row[5]));
            }
        }
        return Ok(None);
    }
    bail!("読み上げの接続先を確認できませんでした。")
}

#[cfg(not(windows))]
fn listener_owner(_port: u16) -> Result<Option<u32>> {
    Ok(None)
}

fn irodori_model_id(models: &Value) -> Option<String> {
    let data = models.get("data")?.as_array()?;
    if data.len() != 1 {
        return None;
    }
    let model = &data[0];
    let id = model.get("id")?.as_str()?;
    let speech = model.get("speech")?;
    (id.to_ascii_lowercase().contains("irodori") && speech.get("task")?.as_str()? == "synthesis")
        .then(|| id.to_owned())
}

fn synthesize(client: &Client, model: &str, voice: &str, text: &str, seed: u64) -> Result<Vec<u8>> {
    let response = client
        .post(format!("{ENDPOINT}/v1/audio/speech"))
        .json(&json!({"model": model, "input": text, "voice": voice, "response_format": "wav", "seed": seed}))
        .send()
        .context("彩りTTSへのローカル接続が切れました。")?;
    anyhow::ensure!(
        response.status().is_success(),
        "音声を生成できませんでした（HTTP {}）。",
        response.status().as_u16()
    );
    if let Some(reason) = response.headers().get("X-Speech-Stop") {
        anyhow::ensure!(reason == "complete", "音声生成が上限に達しました。");
    }
    anyhow::ensure!(
        response.content_length().is_none_or(|n| n <= MAX_WAV),
        "生成音声のサイズが上限を超えました。"
    );
    let mut bytes = Vec::new();
    response.take(MAX_WAV + 1).read_to_end(&mut bytes)?;
    anyhow::ensure!(
        bytes.len() as u64 <= MAX_WAV,
        "生成音声のサイズが上限を超えました。"
    );
    Ok(bytes)
}

fn chunks(text: &str) -> Vec<String> {
    let mut result = Vec::new();
    let mut current = String::new();
    let mut count = 0;
    for character in text.chars() {
        current.push(character);
        count += 1;
        if count >= MAX_CHUNK || matches!(character, '。' | '！' | '？' | '\n') {
            if !current.trim().is_empty() {
                result.push(current.trim().to_owned());
            }
            current.clear();
            count = 0;
        }
    }
    if !current.trim().is_empty() {
        result.push(current.trim().to_owned());
    }
    result
}

#[cfg(windows)]
fn reset_playback(shared: &Shared) {
    use windows::Win32::Media::Audio::{HWAVEOUT, waveOutReset};
    let playing = shared.playback.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(handle) = *playing {
        let _ = unsafe { waveOutReset(HWAVEOUT(handle as *mut _)) };
    }
}

#[cfg(not(windows))]
fn reset_playback(_shared: &Shared) {}

#[cfg(windows)]
fn play_wav(bytes: &[u8], shared: &Shared, epoch: u64, announce: bool) -> Result<()> {
    use windows::{
        Win32::Media::Audio::{
            CALLBACK_NULL, HWAVEOUT, WAVE_MAPPER, WAVEFORMATEX, WAVEHDR, WHDR_DONE, waveOutClose,
            waveOutOpen, waveOutPrepareHeader, waveOutReset, waveOutUnprepareHeader, waveOutWrite,
        },
        core::PSTR,
    };
    let mut reader = hound::WavReader::new(std::io::Cursor::new(bytes))
        .context("生成音声がWAV形式ではありません。")?;
    let spec = reader.spec();
    anyhow::ensure!(
        spec.channels == 1
            && spec.bits_per_sample == 16
            && spec.sample_format == hound::SampleFormat::Int
            && (8_000..=96_000).contains(&spec.sample_rate)
            && reader.duration() > 0
            && reader.duration() <= spec.sample_rate * 60,
        "生成音声の形式または長さを確認できませんでした。"
    );
    let mut pcm: Vec<i16> = reader
        .samples::<i16>()
        .collect::<std::result::Result<_, _>>()?;
    let format = WAVEFORMATEX {
        wFormatTag: 1,
        nChannels: 1,
        nSamplesPerSec: spec.sample_rate,
        nAvgBytesPerSec: spec.sample_rate * 2,
        nBlockAlign: 2,
        wBitsPerSample: 16,
        cbSize: 0,
    };
    let mut header = Box::new(WAVEHDR {
        lpData: PSTR(pcm.as_mut_ptr().cast()),
        dwBufferLength: (pcm.len() * 2) as u32,
        ..Default::default()
    });
    let mut handle = HWAVEOUT::default();
    let size = std::mem::size_of::<WAVEHDR>() as u32;
    {
        let mut playing = shared.playback.lock().unwrap_or_else(|e| e.into_inner());
        if cancelled(shared, epoch) {
            return Ok(());
        }
        let result = unsafe {
            waveOutOpen(
                Some(&mut handle),
                WAVE_MAPPER,
                &format,
                None,
                None,
                CALLBACK_NULL,
            )
        };
        anyhow::ensure!(result == 0, "音声出力を開けませんでした（{}）。", result);
        *playing = Some(handle.0 as usize);
    }
    let result = (|| -> Result<()> {
        let result = unsafe { waveOutPrepareHeader(handle, &mut *header, size) };
        anyhow::ensure!(result == 0, "音声の再生準備に失敗しました（{}）。", result);
        {
            let _playing = shared.playback.lock().unwrap_or_else(|e| e.into_inner());
            if cancelled(shared, epoch) {
                return Ok(());
            }
            shared.suppress.store(true, Ordering::Release);
            let result = unsafe { waveOutWrite(handle, &mut *header, size) };
            anyhow::ensure!(result == 0, "音声を再生できませんでした（{}）。", result);
            if announce {
                let _ = shared.events.send(TtsEvent::Started);
            }
        }
        let deadline = Instant::now() + Duration::from_secs(65);
        loop {
            if cancelled(shared, epoch) {
                break;
            }
            let flags = unsafe { std::ptr::read_volatile(std::ptr::addr_of!(header.dwFlags)) };
            if flags & WHDR_DONE != 0 {
                break;
            }
            anyhow::ensure!(
                Instant::now() < deadline,
                "音声の再生が時間内に終了しませんでした。"
            );
            thread::sleep(Duration::from_millis(10));
        }
        Ok(())
    })();
    {
        let mut playing = shared.playback.lock().unwrap_or_else(|e| e.into_inner());
        let _ = unsafe { waveOutReset(handle) };
        let _ = unsafe { waveOutUnprepareHeader(handle, &mut *header, size) };
        let _ = unsafe { waveOutClose(handle) };
        if *playing == Some(handle.0 as usize) {
            *playing = None;
        }
    }
    if shared.suppress.load(Ordering::Acquire) {
        thread::sleep(TAIL);
        shared.suppress.store(false, Ordering::Release);
    }
    result
}

#[cfg(not(windows))]
fn play_wav(_bytes: &[u8], _shared: &Shared, _epoch: u64, _announce: bool) -> Result<()> {
    bail!("この読み上げ再生機能はWindows専用です。")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_startup_requires_confirmed_four_gib_free_for_gpu() {
        const FOUR_GIB: u64 = 4 * 1_073_741_824;
        assert_eq!(startup_device("cpu", Some(24 * 1_073_741_824)), "cpu");
        assert_eq!(startup_device("cpu", None), "cpu");
        assert_eq!(startup_device("vulkan", None), "cpu");
        assert_eq!(startup_device("vulkan", Some(FOUR_GIB - 1)), "cpu");
        assert_eq!(startup_device("vulkan", Some(FOUR_GIB)), "gpu");
        assert_eq!(startup_device("vulkan", Some(24 * 1_073_741_824)), "gpu");
    }
    #[test]
    fn text_chunks_are_unicode_safe_and_bounded() {
        let text = format!("最初の文章。{}最後です！", "日本語".repeat(90));
        let parts = chunks(&text);
        assert!(parts.iter().all(|s| s.chars().count() <= MAX_CHUNK));
        assert_eq!(parts.concat(), text);
        assert!(chunks(" \n ").is_empty());
        let long = "あ".repeat(MAX_TEXT);
        let parts = chunks(&long);
        assert_eq!(parts.len(), 100);
        assert!(parts.iter().all(|part| part.chars().count() == MAX_CHUNK));
        assert_eq!(parts.concat(), long);
    }
    #[test]
    fn model_identity_requires_exactly_one_irodori_synthesis() {
        let model = json!({"id":"Irodori-TTS-866M-MF-v4.1","speech":{"task":"synthesis"}});
        assert!(irodori_model_id(&json!({"data":[model.clone()]})).is_some());
        assert!(irodori_model_id(&json!({"data":[model.clone(), model]})).is_none());
        assert!(
            irodori_model_id(&json!({"data":[{"id":"irodori","speech":{"task":"recognition"}}]}))
                .is_none()
        );
        assert!(
            irodori_model_id(&json!({"data":[{"id":"unrelated","speech":{"task":"synthesis"}}]}))
                .is_none()
        );
    }
    #[test]
    fn stopped_epoch_never_plays_a_late_result() {
        let (tx, _) = mpsc::channel();
        let state = Shared {
            pending: Mutex::new(None),
            wake: Condvar::new(),
            quit: AtomicBool::new(false),
            epoch: AtomicU64::new(3),
            busy: AtomicBool::new(false),
            child: Mutex::new(None),
            playback: Mutex::new(None),
            suppress: Arc::new(AtomicBool::new(false)),
            events: tx,
        };
        assert!(!cancelled(&state, 3));
        state.epoch.fetch_add(1, Ordering::AcqRel);
        assert!(cancelled(&state, 3));
        assert!(!state.suppress.load(Ordering::Acquire));
    }

    #[cfg(windows)]
    #[test]
    fn loopback_identity_matches_the_actual_listener_pid() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        assert_eq!(listener_owner(port).unwrap(), Some(std::process::id()));
        drop(listener);
    }
}
