//! Owned NVIDIA NeMo-Speech.cpp server and reusable, bounded PCM16 WebSocket.
//! Only confirmed final segments may be translated or persisted by the caller.
use crate::nemotron_setup::NemotronInstallation;
use anyhow::{Context, Result, bail, ensure};
use reqwest::blocking::{Client, multipart};
use serde_json::{Value, json};
use std::{
    collections::VecDeque,
    fs::{File, OpenOptions},
    io::{Read, Write},
    net::{SocketAddr, TcpListener, TcpStream},
    process::{Child, Command, Stdio},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc::{self, Receiver, SyncSender, TrySendError},
    },
    thread,
    time::{Duration, Instant},
};
use tungstenite::Message;

const PORT: u16 = 17864;
const HTTP: &str = "http://127.0.0.1:17864";
const WS: &str = "ws://127.0.0.1:17864/v1/audio/transcriptions/realtime";
const MAX_FRAME: usize = 3200;
const EVENT_LIMIT: usize = 128;

#[derive(Clone, Debug)]
pub enum NemotronEvent {
    Starting,
    Ready {
        model: String,
        backend: String,
    },
    Partial {
        generation: u64,
        text: String,
        audio_processed_ms: u64,
    },
    Final {
        generation: u64,
        text: String,
        start_ms: u64,
        end_ms: u64,
    },
    Cleared {
        generation: u64,
    },
    Error(String),
    Stopped,
}
enum Input {
    Audio { generation: u64, pcm: Vec<i16> },
    Finish { generation: u64 },
}
struct Shared {
    stop: AtomicBool,
    ready: AtomicBool,
    generation: AtomicU64,
    events: Mutex<VecDeque<NemotronEvent>>,
    child: Mutex<Option<Child>>,
    log_threads: Mutex<Vec<thread::JoinHandle<()>>>,
    log_path: Mutex<Option<std::path::PathBuf>>,
}
impl Shared {
    fn emit(&self, event: NemotronEvent) {
        let mut queue = self.events.lock().unwrap_or_else(|e| e.into_inner());
        // Interim hypotheses are replaceable. Confirmed results are never silently lost.
        if matches!(&event, NemotronEvent::Partial { .. }) {
            queue.retain(|e| !matches!(e, NemotronEvent::Partial { .. }));
            if queue.len() >= EVENT_LIMIT {
                return;
            }
        } else if queue.len() >= EVENT_LIMIT {
            self.generation.fetch_add(1, Ordering::AcqRel);
            queue.clear();
            queue.push_back(NemotronEvent::Error(
                "音声認識の結果が滞留したため、認識をリセットしました。".into(),
            ));
            return;
        }
        queue.push_back(event);
    }
    fn reap(&self) {
        if let Some(mut child) = self.child.lock().unwrap_or_else(|e| e.into_inner()).take() {
            let _ = child.kill();
            let _ = child.wait();
        }
        let threads =
            std::mem::take(&mut *self.log_threads.lock().unwrap_or_else(|e| e.into_inner()));
        for thread in threads {
            let _ = thread.join();
        }
    }
    fn owned_pid(&self) -> Result<u32> {
        let mut owned = self.child.lock().unwrap_or_else(|e| e.into_inner());
        let child = owned
            .as_mut()
            .context("Nemotronのプロセスは終了しています。")?;
        ensure!(
            child.try_wait()?.is_none(),
            "Nemotronのプロセスが終了しました。"
        );
        Ok(child.id())
    }
}

pub struct NemotronEngine {
    shared: Arc<Shared>,
    input: SyncSender<Input>,
    worker: Option<thread::JoinHandle<()>>,
}
impl NemotronEngine {
    /// Starts loading on a worker. Wait for Ready before sending mono 16kHz PCM16.
    pub fn start(installation: NemotronInstallation, right_context: u32) -> Result<Self> {
        ensure!(
            matches!(right_context, 0 | 1 | 6 | 13),
            "Nemotronの右コンテキストは0、1、6、13から選んでください。"
        );
        ensure!(
            matches!(installation.backend.as_str(), "cpu" | "vulkan"),
            "Nemotronの実行方式はCPUまたはVulkanを選んでください。"
        );
        ensure!(
            installation.model.is_absolute()
                && installation.executable.is_absolute()
                && installation.model.is_file()
                && installation.executable.is_file(),
            "Nemotronの実行ファイルまたはモデルがありません。音声認識の準備を実行してください。"
        );
        let shared = Arc::new(Shared {
            stop: AtomicBool::new(false),
            ready: AtomicBool::new(false),
            generation: AtomicU64::new(1),
            events: Mutex::new(VecDeque::new()),
            child: Mutex::new(None),
            log_threads: Mutex::new(Vec::new()),
            log_path: Mutex::new(None),
        });
        let (input, receiver) = mpsc::sync_channel(16);
        let state = shared.clone();
        let worker = thread::Builder::new()
            .name("nemotron-stream".into())
            .spawn(move || {
                state.emit(NemotronEvent::Starting);
                if let Err(error) = run(&installation, right_context, &state, receiver)
                    && !state.stop.load(Ordering::Acquire)
                {
                    let log = state
                        .log_path
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .as_ref()
                        .map(|p| format!(" 実行ログ: {}", p.display()))
                        .unwrap_or_default();
                    state.emit(NemotronEvent::Error(format!(
                        "Nemotronの音声認識に失敗しました。{error:#}{log}"
                    )));
                }
                state.ready.store(false, Ordering::Release);
                state.reap();
                state.emit(NemotronEvent::Stopped);
            })?;
        Ok(Self {
            shared,
            input,
            worker: Some(worker),
        })
    }
    pub fn generation(&self) -> u64 {
        self.shared.generation.load(Ordering::Acquire)
    }
    pub fn is_ready(&self) -> bool {
        self.shared.ready.load(Ordering::Acquire)
    }
    /// Maximum 200ms per frame. Overflow clears the stream and returns an error.
    pub fn push_pcm16(&self, pcm: &[i16]) -> Result<()> {
        ensure!(self.is_ready(), "Nemotronの読み込みが完了していません。");
        ensure!(
            !pcm.is_empty() && pcm.len() <= MAX_FRAME,
            "音声認識には200ミリ秒以内の16kHzモノラルPCM16を送ってください。"
        );
        self.enqueue(Input::Audio {
            generation: self.generation(),
            pcm: pcm.to_vec(),
        })
    }
    fn enqueue(&self, input: Input) -> Result<()> {
        match self.input.try_send(input) {
            Ok(()) => Ok(()),
            Err(TrySendError::Full(_)) => {
                self.clear();
                bail!("音声認識の入力が滞留したため、認識をリセットしました。")
            }
            Err(TrySendError::Disconnected(_)) => bail!("Nemotronへの音声接続が終了しています。"),
        }
    }
    /// Discard all old audio/results immediately; clear acknowledgement is asynchronous.
    pub fn clear(&self) -> u64 {
        self.shared.generation.fetch_add(1, Ordering::AcqRel) + 1
    }
    /// Flush the current utterance; the same connection remains usable afterwards.
    pub fn finish(&self) -> Result<()> {
        ensure!(self.is_ready(), "Nemotronの読み込みが完了していません。");
        self.enqueue(Input::Finish {
            generation: self.generation(),
        })
    }
    pub fn poll(&self) -> Vec<NemotronEvent> {
        let generation = self.generation();
        self.shared
            .events
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .drain(..)
            .filter(|event| match event {
                NemotronEvent::Partial { generation: g, .. }
                | NemotronEvent::Final { generation: g, .. }
                | NemotronEvent::Cleared { generation: g } => *g == generation,
                _ => true,
            })
            .collect()
    }
    /// Diagnostic/batch REST path to this owned, already loaded local server.
    pub fn transcribe_wav(&self, wav: &[u8]) -> Result<String> {
        ensure!(self.is_ready(), "Nemotronの読み込みが完了していません。");
        ensure!(
            wav.len() <= 4 * 1024 * 1024
                && wav.starts_with(b"RIFF")
                && wav.get(8..12) == Some(b"WAVE"),
            "4MiB以内のWAV音声を指定してください。"
        );
        verify_owner(&self.shared)?;
        let form = multipart::Form::new()
            .part(
                "file",
                multipart::Part::bytes(wav.to_vec())
                    .file_name("audio.wav")
                    .mime_str("audio/wav")?,
            )
            .text("language", "en")
            .text("response_format", "json");
        let response = http_client(Duration::from_secs(60))?
            .post(format!("{HTTP}/v1/audio/transcriptions"))
            .multipart(form)
            .send()?
            .error_for_status()?;
        let value: Value = response.json()?;
        let text = value["text"]
            .as_str()
            .context("Nemotronの音声認識応答に本文がありません。")?
            .trim();
        ensure!(!text.is_empty(), "Nemotronの音声認識結果が空です。");
        Ok(text.into())
    }
    /// Kill and wait only for our child, then join the bounded-time worker.
    pub fn stop(&mut self) {
        self.shared.stop.store(true, Ordering::Release);
        self.shared.ready.store(false, Ordering::Release);
        self.clear();
        self.shared.reap();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}
impl Drop for NemotronEngine {
    fn drop(&mut self) {
        self.stop();
    }
}
fn http_client(timeout: Duration) -> Result<Client> {
    Ok(Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(2))
        .timeout(timeout)
        .build()?)
}
fn verify_owner(shared: &Shared) -> Result<()> {
    let pid = shared.owned_pid()?;
    ensure!(
        listener_owner(PORT)? == Some(pid),
        "Nemotronへのローカル接続の所有者を確認できません。"
    );
    Ok(())
}
fn model_id(value: &Value) -> Option<String> {
    let models = value["data"].as_array()?;
    if models.len() != 1 {
        return None;
    }
    let id = models[0]["id"].as_str()?;
    let lower = id.to_ascii_lowercase();
    matches!(
        lower.as_str(),
        "nemotron-en"
            | "nemotron-speech-streaming-en-0.6b"
            | "nemotron-speech-streaming-en-0.6b.q8_0"
            | "nemotron-speech-streaming-en-0.6b.q8_0.gguf"
            | ".nemotron-speech-streaming-en-0.6b.q8_0.gguf"
    )
    .then(|| id.into())
}
struct NativeLog {
    file: File,
    bytes: usize,
}
fn prepare_log(config: &NemotronInstallation, shared: &Shared) -> Result<Arc<Mutex<NativeLog>>> {
    let models = config
        .model
        .parent()
        .and_then(|p| p.parent())
        .context("Nemotronモデルの保存先を確認できません。")?;
    ensure!(
        models.file_name().is_some_and(|n| n == "models"),
        "Nemotronモデルの配置が不正です。"
    );
    let directory = models.parent().unwrap().join("logs");
    crate::installer::safe_components(&directory, true)?;
    let path = directory.join("latest.log");
    crate::installer::safe_file(&path)?;
    if path.exists() {
        ensure!(
            std::fs::metadata(&path)?.len() <= 1024 * 1024,
            "Nemotronの実行ログが大きすぎます。"
        );
        crate::installer::atomic_write(&directory.join("previous.log"), &std::fs::read(&path)?)?;
    }
    crate::installer::atomic_write(&path, b"")?;
    let file = OpenOptions::new().append(true).open(&path)?;
    *shared.log_path.lock().unwrap_or_else(|e| e.into_inner()) = Some(path);
    Ok(Arc::new(Mutex::new(NativeLog { file, bytes: 0 })))
}
fn log_reader(
    mut input: impl Read + Send + 'static,
    log: Arc<Mutex<NativeLog>>,
) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        let mut buffer = [0; 8192];
        while let Ok(n) = input.read(&mut buffer) {
            if n == 0 {
                break;
            }
            let mut log = log.lock().unwrap_or_else(|e| e.into_inner());
            let count = n.min((1024 * 1024usize).saturating_sub(log.bytes));
            if count > 0 {
                let _ = log.file.write_all(&buffer[..count]);
                log.bytes += count;
            }
        }
    })
}
fn run(
    config: &NemotronInstallation,
    right_context: u32,
    shared: &Shared,
    input: Receiver<Input>,
) -> Result<()> {
    let probe = TcpListener::bind(SocketAddr::from(([127, 0, 0, 1], PORT)))
        .context("音声認識用ポート17864が使用中です。ほかのNemotronの処理を確認してください。")?;
    drop(probe);
    let log = prepare_log(config, shared)?;
    let mut command = Command::new(&config.executable);
    command
        .args(["serve", "--asr-model"])
        .arg(&config.model)
        .args([
            "--host",
            "127.0.0.1",
            "--port",
            "17864",
            "--no-ui",
            "--device",
            &config.backend,
            "--asr.backend.gpu",
            if config.backend == "cpu" { "-1" } else { "0" },
            "--asr.streaming.rnnt_right_context",
            &right_context.to_string(),
            "--asr.endpointing.enable=true",
            "--asr.endpointing.stop_history_eou_ms",
            "800",
            "--asr.batching.enabled=false",
        ])
        .current_dir(config.executable.parent().unwrap())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    crate::platform::hide_console(&mut command);
    // Hold the lock while spawning so stop() cannot miss a newly created process.
    {
        let mut child = shared.child.lock().unwrap_or_else(|e| e.into_inner());
        if shared.stop.load(Ordering::Acquire) {
            return Ok(());
        }
        let mut process = command.spawn().context(
            "Nemotronを起動できませんでした。GPUドライバーと保存済みの実行環境を確認してください。",
        )?;
        let mut readers = shared.log_threads.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(stdout) = process.stdout.take() {
            readers.push(log_reader(stdout, log.clone()));
        }
        if let Some(stderr) = process.stderr.take() {
            readers.push(log_reader(stderr, log.clone()));
        }
        *child = Some(process);
    }
    let client = http_client(Duration::from_secs(2))?;
    let deadline = Instant::now() + Duration::from_secs(90);
    let model = loop {
        if shared.stop.load(Ordering::Acquire) {
            return Ok(());
        }
        shared.owned_pid()?;
        if let Ok(response) = client.get(format!("{HTTP}/ready")).send()
            && response.status().is_success()
        {
            verify_owner(shared)?;
            let ready: Value = response.json()?;
            let device = ready["device"].as_str().unwrap_or("");
            ensure!(
                device == config.backend || device.starts_with(&format!("{}:", config.backend)),
                "Nemotronの実行デバイスが選択された方式と一致しません。"
            );
            let models: Value = client
                .get(format!("{HTTP}/v1/models"))
                .send()?
                .error_for_status()?
                .json()?;
            break model_id(&models)
                .context("起動したNemotron English 0.6Bモデルを確認できませんでした。")?;
        }
        ensure!(
            Instant::now() < deadline,
            "Nemotronの読み込みが時間内に完了しませんでした。"
        );
        thread::sleep(Duration::from_millis(100));
    };
    let stream = TcpStream::connect_timeout(
        &SocketAddr::from(([127, 0, 0, 1], PORT)),
        Duration::from_secs(2),
    )?;
    stream.set_read_timeout(Some(Duration::from_secs(3)))?;
    stream.set_write_timeout(Some(Duration::from_secs(2)))?;
    let socket_config = tungstenite::protocol::WebSocketConfig::default()
        .max_message_size(Some(1024 * 1024))
        .max_frame_size(Some(1024 * 1024));
    let (mut socket, _) = tungstenite::client::client_with_config(WS, stream, Some(socket_config))
        .map_err(|e| anyhow::anyhow!("Nemotronのストリームに接続できません。{e}"))?;
    socket
        .get_mut()
        .set_read_timeout(Some(Duration::from_millis(20)))?;
    socket.send(Message::Text(json!({"type":"session.update","session":{"sample_rate":16000,"language":"en","automatic_punctuation":true,"endpointing_ms":800}}).to_string().into()))?;
    let mut state = StreamState::new(shared.generation.load(Ordering::Acquire));
    let mut clear_acks = VecDeque::new();
    let mut initialized = false;
    let mut last_ping = Instant::now();
    let mut reset_sent_at = None;
    let session_deadline = Instant::now() + Duration::from_secs(5);
    while !shared.stop.load(Ordering::Acquire) {
        let generation = shared.generation.load(Ordering::Acquire);
        if generation != state.generation {
            socket.send(Message::Text(
                json!({"type":"input_audio_buffer.clear"})
                    .to_string()
                    .into(),
            ))?;
            state = StreamState::new(generation);
            clear_acks.push_back(generation);
            reset_sent_at.get_or_insert_with(Instant::now);
            ensure!(
                clear_acks.len() <= 32,
                "Nemotronのリセット応答が滞留しています。"
            );
        }
        if clear_acks.is_empty() && initialized {
            // Limit each pass so server events and cancellation cannot starve.
            for _ in 0..4 {
                let Ok(command) = input.try_recv() else { break };
                match command {
                    Input::Audio { generation, pcm } if generation == state.generation => {
                        let bytes: Vec<u8> = pcm.iter().flat_map(|s| s.to_le_bytes()).collect();
                        socket.send(Message::Binary(bytes.into()))?;
                        state.sent_samples += pcm.len() as u64;
                        // Reset the server's cumulative upload budget periodically.
                        // The socket/model remain resident across this flush.
                        if state.sent_samples - state.last_commit_samples >= 16000 * 120 {
                            state.commit_sent()?;
                            socket.send(Message::Text(
                                json!({"type":"input_audio_buffer.commit"})
                                    .to_string()
                                    .into(),
                            ))?;
                        }
                    }
                    Input::Finish { generation } if generation == state.generation => {
                        state.commit_sent()?;
                        socket.send(Message::Text(
                            json!({"type":"input_audio_buffer.commit"})
                                .to_string()
                                .into(),
                        ))?
                    }
                    _ => {}
                }
            }
        } else {
            // Until the session is ready and every reset is acknowledged, input
            // waits in the queue (bounded to 16 frames). Input of an older
            // generation is dropped above, when it is read.
        }
        match socket.read() {
            Ok(Message::Text(text)) => {
                let event: Value = serde_json::from_str(&text)?;
                match event["type"].as_str().unwrap_or("") {
                    "session.updated" => {
                        initialized = true;
                        shared.ready.store(true, Ordering::Release);
                        shared.emit(NemotronEvent::Ready {
                            model: model.clone(),
                            backend: config.backend.clone(),
                        });
                    }
                    "input_audio_buffer.cleared" => {
                        if let Some(generation) = clear_acks.pop_front()
                            && clear_acks.is_empty()
                            && generation == state.generation
                        {
                            shared.emit(NemotronEvent::Cleared { generation });
                            reset_sent_at = None;
                        }
                    }
                    "error" => bail!(
                        "Nemotronのストリームエラー: {}",
                        event.get("error").unwrap_or(&event)
                    ),
                    _ if clear_acks.is_empty() && generation == state.generation => {
                        if let Some(result) = state.event(&event)? {
                            shared.emit(result)
                        }
                    }
                    _ => {}
                }
            }
            Ok(Message::Close(_)) => bail!("Nemotronのストリームが閉じられました。"),
            Ok(_) => {}
            Err(tungstenite::Error::Io(e))
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) => {}
            Err(error) => return Err(error.into()),
        }
        ensure!(
            initialized || Instant::now() < session_deadline,
            "Nemotronのストリーム設定が時間内に完了しませんでした。"
        );
        ensure!(
            reset_sent_at.is_none_or(|sent: Instant| sent.elapsed() < Duration::from_secs(5)),
            "Nemotronのリセットが時間内に完了しませんでした。"
        );
        if last_ping.elapsed() >= Duration::from_secs(10) {
            socket.send(Message::Ping(Vec::new().into()))?;
            last_ping = Instant::now();
        }
    }
    let _ = socket.close(None);
    Ok(())
}
struct StreamState {
    generation: u64,
    partial: String,
    base_ms: u64,
    last_end_ms: u64,
    segment_start_ms: u64,
    sent_samples: u64,
    last_commit_samples: u64,
    commits: VecDeque<u64>,
}
impl StreamState {
    fn new(generation: u64) -> Self {
        Self {
            generation,
            partial: String::new(),
            base_ms: 0,
            last_end_ms: 0,
            segment_start_ms: 0,
            sent_samples: 0,
            last_commit_samples: 0,
            commits: VecDeque::new(),
        }
    }
    fn commit_sent(&mut self) -> Result<()> {
        ensure!(
            self.commits.len() < 16,
            "Nemotronの確定応答が滞留しています。"
        );
        self.last_commit_samples = self.sent_samples;
        self.commits.push_back(self.sent_samples / 16);
        Ok(())
    }
    fn event(&mut self, event: &Value) -> Result<Option<NemotronEvent>> {
        let processed = event["audio_processed"].as_f64().unwrap_or(0.0);
        ensure!(
            processed.is_finite() && (0.0..=86400.0).contains(&processed),
            "Nemotronの音声時刻が不正です。"
        );
        let end = self.base_ms + (processed * 1000.0).round() as u64;
        match event["type"].as_str().unwrap_or("") {
            "conversation.item.input_audio_transcription.delta" => {
                let delta = event["delta"]
                    .as_str()
                    .context("Nemotronの暫定応答に本文がありません。")?;
                ensure!(
                    self.partial.len() + delta.len() <= 64 * 1024,
                    "Nemotronの暫定結果が長すぎます。"
                );
                // v0.3.0 does not flag full-hypothesis revisions separately from
                // appended deltas. Interim text is advisory; completed always
                // supplies the authoritative full transcript.
                self.partial.push_str(delta);
                Ok(Some(NemotronEvent::Partial {
                    generation: self.generation,
                    text: self.partial.clone(),
                    audio_processed_ms: end,
                }))
            }
            "conversation.item.input_audio_transcription.completed" => {
                let text = event["transcript"]
                    .as_str()
                    .context("Nemotronの確定応答に本文がありません。")?
                    .trim()
                    .to_owned();
                ensure!(text.len() <= 64 * 1024, "Nemotronの確定結果が長すぎます。");
                let start = self.segment_start_ms;
                self.last_end_ms = self.last_end_ms.max(end);
                self.segment_start_ms = self.last_end_ms;
                self.partial.clear();
                Ok((!text.is_empty()).then_some(NemotronEvent::Final {
                    generation: self.generation,
                    text,
                    start_ms: start,
                    end_ms: self.last_end_ms,
                }))
            }
            "input_audio_buffer.committed" => {
                self.base_ms = self
                    .commits
                    .pop_front()
                    .unwrap_or(self.last_end_ms)
                    .max(self.last_end_ms);
                self.partial.clear();
                Ok(None)
            }
            _ => Ok(None),
        }
    }
}

#[cfg(windows)]
fn listener_owner(port: u16) -> Result<Option<u32>> {
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
        ensure!(
            (4..=1_048_576).contains(&size),
            "Nemotronの接続先を確認できません。"
        );
        let mut table = vec![0u32; (size as usize).div_ceil(4)];
        let result = unsafe { tcp_table(table.as_mut_ptr().cast(), &mut size, 0, 2, 3, 0) };
        if result == 122 {
            continue;
        }
        ensure!(result == 0, "Nemotronの接続先を確認できません。");
        let count = table[0] as usize;
        ensure!(
            count <= (table.len() - 1) / 6,
            "Nemotronの接続先情報が不正です。"
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
    bail!("Nemotronの接続先情報が変化したため確認できません。")
}
#[cfg(not(windows))]
fn listener_owner(_port: u16) -> Result<Option<u32>> {
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn test_engine() -> (NemotronEngine, Receiver<Input>) {
        let (input, receiver) = mpsc::sync_channel(16);
        let shared = Arc::new(Shared {
            stop: AtomicBool::new(false),
            ready: AtomicBool::new(true),
            generation: AtomicU64::new(1),
            events: Mutex::new(VecDeque::new()),
            child: Mutex::new(None),
            log_threads: Mutex::new(Vec::new()),
            log_path: Mutex::new(None),
        });
        (
            NemotronEngine {
                shared,
                input,
                worker: None,
            },
            receiver,
        )
    }
    #[test]
    fn overflow_resets_epoch_and_poll_discards_old_final() {
        let (engine, receiver) = test_engine();
        engine.shared.emit(NemotronEvent::Final {
            generation: 1,
            text: "old".into(),
            start_ms: 0,
            end_ms: 100,
        });
        for _ in 0..16 {
            engine.push_pcm16(&[0; 2560]).unwrap();
        }
        assert!(engine.push_pcm16(&[0; 2560]).is_err());
        assert_eq!(engine.generation(), 2);
        assert!(engine.poll().is_empty());
        for _ in 0..16 {
            assert!(matches!(
                receiver.try_recv().unwrap(),
                Input::Audio { generation: 1, .. }
            ));
        }
        engine.push_pcm16(&[0; 2560]).unwrap();
        assert!(matches!(
            receiver.try_recv().unwrap(),
            Input::Audio { generation: 2, .. }
        ));
    }
    #[test]
    fn interim_events_coalesce_and_results_have_a_hard_bound() {
        let (engine, _receiver) = test_engine();
        for _ in 0..300 {
            engine.shared.emit(NemotronEvent::Partial {
                generation: 1,
                text: "interim".into(),
                audio_processed_ms: 10,
            });
        }
        assert_eq!(engine.poll().len(), 1);
        for _ in 0..=EVENT_LIMIT {
            engine.shared.emit(NemotronEvent::Final {
                generation: 1,
                text: "final".into(),
                start_ms: 0,
                end_ms: 10,
            });
        }
        assert_eq!(engine.generation(), 2);
        assert!(matches!(
            engine.poll().as_slice(),
            [NemotronEvent::Error(_)]
        ));
    }
    /// Explicit opt-in native integration test. No downloads in ordinary tests.
    /// Set NEMOTRON_NATIVE_VERIFY=1, NEMOTRON_VERIFY_ROOT (dedicated absolute
    /// installation directory), NEMOTRON_VERIFY_AUDIO (local video/audio input)
    /// and optionally NEMOTRON_VERIFY_REPORT (private JSON evidence destination).
    #[test]
    #[ignore = "downloads pinned runtime/model and runs local inference"]
    fn native_real_validation() {
        assert_eq!(std::env::var("NEMOTRON_NATIVE_VERIFY").as_deref(), Ok("1"));
        let root = std::path::PathBuf::from(
            std::env::var_os("NEMOTRON_VERIFY_ROOT").expect("dedicated installation root required"),
        );
        let audio = std::path::PathBuf::from(
            std::env::var_os("NEMOTRON_VERIFY_AUDIO").expect("local fixture path required"),
        );
        assert!(root.is_absolute() && audio.is_absolute());
        let seek = std::env::var("NEMOTRON_VERIFY_SEEK").unwrap_or_else(|_| "0".into());
        let clip = Command::new("ffmpeg")
            .args(["-v", "error", "-ss", &seek, "-t", "10", "-i"])
            .arg(&audio)
            .args([
                "-map",
                "0:a:0",
                "-ac",
                "1",
                "-ar",
                "16000",
                "-f",
                "s16le",
                "-acodec",
                "pcm_s16le",
                "pipe:1",
            ])
            .output()
            .unwrap();
        assert!(clip.status.success());
        let pcm: Vec<i16> = clip
            .stdout
            .as_chunks::<2>()
            .0
            .iter()
            .map(|s| i16::from_le_bytes([s[0], s[1]]))
            .collect();
        let mut wav = std::io::Cursor::new(Vec::new());
        {
            let mut writer = hound::WavWriter::new(
                &mut wav,
                hound::WavSpec {
                    channels: 1,
                    sample_rate: 16000,
                    bits_per_sample: 16,
                    sample_format: hound::SampleFormat::Int,
                },
            )
            .unwrap();
            for &sample in &pcm {
                writer.write_sample(sample).unwrap();
            }
            writer.finalize().unwrap();
        }
        let wav = wav.into_inner();
        assert!(pcm.len() > 16000);
        let mut report = json!({"version":1,"fixture":audio,"seek_seconds":seek,"sample_rate":16000,"samples":pcm.len(),"runs":[]});
        for backend in ["vulkan", "cpu"] {
            let installation = crate::nemotron_setup::install(
                &root,
                backend,
                Arc::new(AtomicBool::new(false)),
                |p| eprintln!("{} {} / {:?}", p.stage, p.downloaded_bytes, p.total_bytes),
            )
            .unwrap();
            for context in [6, 1] {
                let loading = Instant::now();
                let mut engine = NemotronEngine::start(installation.clone(), context).unwrap();
                let deadline = Instant::now() + Duration::from_secs(100);
                while !engine.is_ready() {
                    for event in engine.poll() {
                        eprintln!("{event:?}");
                        if let NemotronEvent::Error(error) = event {
                            panic!("{error}")
                        }
                    }
                    assert!(Instant::now() < deadline);
                    thread::sleep(Duration::from_millis(20));
                }
                let load_ms = loading.elapsed().as_millis();
                let ready: Value = http_client(Duration::from_secs(2))
                    .unwrap()
                    .get(format!("{HTTP}/ready"))
                    .send()
                    .unwrap()
                    .json()
                    .unwrap();
                let models: Value = http_client(Duration::from_secs(2))
                    .unwrap()
                    .get(format!("{HTTP}/v1/models"))
                    .send()
                    .unwrap()
                    .json()
                    .unwrap();
                let pid = engine.shared.owned_pid().unwrap();
                engine.poll();
                let mut rest = Vec::new();
                for _ in 0..3 {
                    let start = Instant::now();
                    let text = engine.transcribe_wav(&wav).unwrap();
                    assert!(text.split_whitespace().count() >= 3);
                    rest.push(json!({"elapsed_ms":start.elapsed().as_millis(),"text":text}));
                }
                let start = Instant::now();
                let mut events = Vec::new();
                let mut first_partial = None;
                let mut first_final = None;
                let mut last_final_end = 0;
                let mut capture = |engine: &NemotronEngine| {
                    for event in engine.poll() {
                        let wall_ms = start.elapsed().as_millis();
                        match event {
                            NemotronEvent::Partial {
                                generation,
                                text,
                                audio_processed_ms,
                            } => {
                                if first_partial.is_none() {
                                    first_partial = Some(wall_ms);
                                }
                                events.push(json!({"type":"partial","wall_ms":wall_ms,"generation":generation,"text":text,"audio_processed_ms":audio_processed_ms}));
                            }
                            NemotronEvent::Final {
                                generation,
                                text,
                                start_ms,
                                end_ms,
                            } => {
                                if first_final.is_none() {
                                    first_final = Some(wall_ms);
                                }
                                last_final_end = end_ms;
                                events.push(json!({"type":"final","wall_ms":wall_ms,"generation":generation,"text":text,"start_ms":start_ms,"end_ms":end_ms}));
                            }
                            NemotronEvent::Error(error) => panic!("{error}"),
                            _ => {}
                        }
                    }
                };
                for (index, frame) in pcm.chunks(2560).enumerate() {
                    let due = Duration::from_millis(index as u64 * 160);
                    while start.elapsed() < due {
                        capture(&engine);
                        thread::sleep(Duration::from_millis(5));
                    }
                    engine.push_pcm16(frame).unwrap();
                    capture(&engine);
                }
                engine.finish().unwrap();
                let deadline = Instant::now() + Duration::from_secs(10);
                while Instant::now() < deadline {
                    capture(&engine);
                    thread::sleep(Duration::from_millis(10));
                }
                // Ends the mutable borrows held by the closure before the
                // assertions below; the closure itself implements no Drop.
                #[allow(clippy::drop_non_drop)]
                drop(capture);
                assert!(first_final.is_some());
                assert!(last_final_end > 0);
                let reset_start = Instant::now();
                let generation = engine.clear();
                let deadline = Instant::now() + Duration::from_secs(3);
                let mut cleared = false;
                while Instant::now() < deadline && !cleared {
                    cleared = engine.poll().iter().any(
                        |e| matches!(e,NemotronEvent::Cleared{generation:g} if *g==generation),
                    );
                    thread::sleep(Duration::from_millis(10));
                }
                assert!(cleared);
                let reset_ms = reset_start.elapsed().as_millis();
                let stop_start = Instant::now();
                engine.stop();
                let stop_ms = stop_start.elapsed().as_millis();
                let port_closed =
                    TcpListener::bind(SocketAddr::from(([127, 0, 0, 1], PORT))).is_ok();
                assert!(port_closed);
                let row = json!({"backend":backend,"right_context":context,"ready":ready,"models":models,"owned_pid":pid,"load_ms":load_ms,"rest":rest,"first_partial_wall_ms":first_partial,"first_final_wall_ms":first_final,"events":events,"clear_ack_ms":reset_ms,"stop_ms":stop_ms,"port_closed":port_closed});
                eprintln!("NATIVE_REPORT {row}");
                report["runs"].as_array_mut().unwrap().push(row);
                if let Some(path) = std::env::var_os("NEMOTRON_VERIFY_REPORT") {
                    std::fs::write(path, serde_json::to_vec_pretty(&report).unwrap()).unwrap();
                }
            }
        }
        crate::nemotron_setup::install(&root, "vulkan", Arc::new(AtomicBool::new(false)), |_| {})
            .unwrap();
        assert_eq!(
            crate::nemotron_setup::load(&root).unwrap().unwrap().backend,
            "vulkan"
        );
    }
    #[test]
    fn deltas_are_tentative_and_final_is_authoritative() {
        let mut state = StreamState::new(7);
        state.event(&json!({"type":"conversation.item.input_audio_transcription.delta","delta":"Hello ","audio_processed":0.4})).unwrap();
        let partial=state.event(&json!({"type":"conversation.item.input_audio_transcription.delta","delta":"world","audio_processed":0.8})).unwrap().unwrap();
        assert!(matches!(partial,NemotronEvent::Partial{text,..} if text=="Hello world"));
        let final_result=state.event(&json!({"type":"conversation.item.input_audio_transcription.completed","transcript":"Hello, world.","audio_processed":1.2})).unwrap().unwrap();
        assert!(
            matches!(final_result,NemotronEvent::Final{generation:7,start_ms:0,end_ms:1200,text} if text=="Hello, world.")
        );
        assert!(state.partial.is_empty());
    }
    #[test]
    fn commit_keeps_clock_monotonic() {
        let mut state = StreamState::new(1);
        state.event(&json!({"type":"conversation.item.input_audio_transcription.completed","transcript":"one","audio_processed":2.0})).unwrap();
        state
            .event(&json!({"type":"input_audio_buffer.committed"}))
            .unwrap();
        assert!(matches!(state.event(&json!({"type":"conversation.item.input_audio_transcription.completed","transcript":"two","audio_processed":1.0})).unwrap(),Some(NemotronEvent::Final{start_ms:2000,end_ms:3000,..})));
    }
    #[test]
    fn rejects_wrong_model_and_invalid_clock() {
        assert!(model_id(&json!({"data":[{"id":"qwen"}]})).is_none());
        assert!(
            model_id(&json!({"data":[{"id":"nemotron-speech-streaming-en-0.6b.q8_0"}]})).is_some()
        );
        assert!(
            StreamState::new(1)
                .event(&json!({"type":"x","audio_processed":-1}))
                .is_err()
        );
    }
}
