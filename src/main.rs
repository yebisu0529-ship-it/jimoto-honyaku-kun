#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use anyhow::Result;
use eframe::egui::{self, Color32, RichText, Vec2};
use plamo_translate::{
    bridge,
    engine::{self, TranslationOptions, Translator},
    journal::{Journal, Record},
    platform,
    subtitles::{self, Cue},
};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::{Path, PathBuf},
    process::{Child, Command},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, SyncSender},
    },
    thread,
    time::{Duration, Instant},
};

mod preview;
mod readout;
mod services;
mod setup;
mod ui;

const ACCENT: Color32 = Color32::from_rgb(60, 191, 164);

#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
struct Settings {
    translation: TranslationOptions,
    asr_endpoint: String,
    asr_backend: String,
    asr_model: String,
    python: String,
    llama_server: String,
    model: String,
    bridge_token: String,
    audio_seconds: f32,
    show_original: bool,
    auto_scroll: bool,
    font_size: f32,
    template: String,
    backend: String,
    gpu_layers: u32,
    asr_device: String,
    model_store: String,
    auto_read: bool,
}
impl Default for Settings {
    fn default() -> Self {
        let root = app_root();
        let venv = root.join(".venv/Scripts/python.exe");
        Self {
            translation: TranslationOptions::default(),
            asr_endpoint: "http://127.0.0.1:17861".into(),
            asr_backend: "qwen".into(),
            asr_model: "Qwen/Qwen3-ASR-0.6B-hf".into(),
            python: if venv.exists() {
                venv.to_string_lossy().into()
            } else {
                "python.exe".into()
            },
            llama_server: root
                .join("runtime/llama-b12119/llama-server.exe")
                .to_string_lossy()
                .into(),
            model: root
                .join("models/plamo-3-translate-2b-Q4_K_M.gguf")
                .to_string_lossy()
                .into(),
            bridge_token: uuid::Uuid::new_v4().to_string(),
            audio_seconds: 3.0,
            show_original: true,
            auto_scroll: true,
            font_size: 20.0,
            template: root
                .join("models/chat_template.jinja")
                .to_string_lossy()
                .into(),
            backend: "cuda".into(),
            gpu_layers: 99,
            asr_device: "auto".into(),
            model_store: String::new(),
            auto_read: false,
        }
    }
}
fn app_root() -> PathBuf {
    if let Some(root) = std::env::var_os("PLAMO_APP_DIR") {
        return root.into();
    }
    if let Ok(exe) = std::env::current_exe()
        && let Some(dir) = exe.parent()
    {
        if dir
            .file_name()
            .is_some_and(|n| n == "release" || n == "debug")
            && let Some(target) = dir.parent()
            && target.file_name().is_some_and(|n| n == "target")
            && let Some(project) = target.parent()
            && project.join("Cargo.toml").exists()
        {
            return project.into();
        }
        return dir.into();
    }
    std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
}
fn data_root() -> PathBuf {
    plamo_translate::storage::data_root()
}
fn config_path() -> PathBuf {
    data_root().join("settings.json")
}
fn save_settings(settings: &Settings) -> Result<()> {
    let path = config_path();
    fs::create_dir_all(path.parent().unwrap())?;
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, serde_json::to_vec_pretty(settings)?)?;
    fs::rename(tmp, path)?;
    Ok(())
}
fn command_hidden(command: &mut Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Tab {
    Text,
    Live,
    Files,
    Settings,
    Setup,
}
#[derive(Clone, Copy, PartialEq)]
enum Lane {
    Manual,
    Live,
}
enum Event {
    Partial {
        id: u64,
        text: String,
    },
    Done {
        id: u64,
        lane: Lane,
        result: std::result::Result<String, String>,
        elapsed: f32,
        generation: u64,
    },
    Ocr(std::result::Result<String, String>),
    Screenshot(std::result::Result<(platform::Screen, image::RgbaImage), String>),
    Transcript {
        generation: u64,
        start_ms: u64,
        end_ms: u64,
        result: std::result::Result<String, String>,
    },
    FileDone(std::result::Result<String, String>),
    Info(String),
    Hardware(std::result::Result<plamo_translate::hardware::Hardware, String>),
    InstallProgress(plamo_translate::installer::Progress),
    Installed(std::result::Result<plamo_translate::installer::InstallResult, String>),
    ServiceStatus {
        translation: std::result::Result<String, String>,
        asr: Option<services::AsrStatus>,
    },
    AudioPrepared(std::result::Result<(), String>),
    Readout(readout::ReadoutEvent),
}
struct Row {
    record: Record,
    pending: bool,
    error: Option<String>,
    generation: u64,
}
struct Selection {
    screen: platform::Screen,
    image: image::RgbaImage,
    texture: egui::TextureHandle,
    start: Option<egui::Pos2>,
}
struct AudioSession {
    stop: Arc<AtomicBool>,
    rx: Receiver<platform::AudioChunk>,
    started: Instant,
}
#[derive(Clone, Copy, Debug, PartialEq)]
enum LiveKind {
    Audio,
    Caption,
}
impl LiveKind {
    fn from_record(kind: &str) -> Option<Self> {
        match kind {
            "音声" => Some(Self::Audio),
            "YouTube" => Some(Self::Caption),
            _ => None,
        }
    }
}
struct PendingTranscript {
    text: String,
    start_ms: u64,
    end_ms: u64,
    generation: u64,
}
const MAX_AUDIO_AGE_MS: u64 = 8000;
struct App {
    settings: Settings,
    settings_load_failed: bool,
    tab: Tab,
    input: String,
    output: String,
    status: String,
    rows: Vec<Row>,
    journal: Option<Journal>,
    journal_error: String,
    next_id: u64,
    tx: SyncSender<Event>,
    rx: Receiver<Event>,
    bridge_rx: Receiver<bridge::Caption>,
    stop: Arc<AtomicBool>,
    screens: Vec<platform::Screen>,
    screen_index: usize,
    selection: Option<Selection>,
    capture_busy: bool,
    manual_busy: Option<u64>,
    pending_ocr: Option<String>,
    live_busy: Option<u64>,
    asr_busy: bool,
    asr_caption_suppressed: bool,
    audio: Option<AudioSession>,
    caption_enabled: bool,
    audio_enabled: bool,
    audio_generation: u64,
    caption_generation: u64,
    audio_offset_ms: u64,
    pending_audio: Option<platform::AudioChunk>,
    pending_transcript: Option<PendingTranscript>,
    video_id: String,
    caption_epoch: u64,
    caption_paused: bool,
    last_caption: Option<Instant>,
    pending_caption: Option<bridge::Caption>,
    last_live_text: String,
    last_live_id: Option<u64>,
    overlay: bool,
    overlay_text: String,
    overlay_source: String,
    overlay_time: Option<Instant>,
    overlay_kind: Option<LiveKind>,
    glossary_input: String,
    search: String,
    children: Vec<Child>,
    hotkeys: Option<global_hotkey::GlobalHotKeyManager>,
    file_busy: bool,
    file_cancel: Arc<AtomicBool>,
    setup: setup::SetupState,
    backend_ready: bool,
    backend_loading: bool,
    asr_ready: bool,
    audio_start_requested: bool,
    service_error: String,
    health_check_running: bool,
    last_health_check: Instant,
    backend_started_at: Option<Instant>,
    asr_started_at: Option<Instant>,
    startup_decided: bool,
    expect_selected_model: bool,
    backend_pid: Option<u32>,
    asr_pid: Option<u32>,
    audio_confirmation: bool,
    audio_installing: bool,
    audio_cancel: Arc<AtomicBool>,
    readout: readout::ReadoutState,
    native_window: Option<isize>,
}

impl App {
    fn inert() -> Self {
        let (tx, rx) = mpsc::sync_channel(256);
        let (_, bridge_rx) = mpsc::sync_channel(8);
        App {
            settings: Settings::default(),
            settings_load_failed: true,
            tab: Tab::Live,
            input: String::new(),
            output: String::new(),
            status: String::new(),
            rows: Vec::new(),
            journal: None,
            journal_error: String::new(),
            next_id: 1,
            tx,
            rx,
            bridge_rx,
            stop: Arc::new(AtomicBool::new(false)),
            screens: Vec::new(),
            screen_index: 0,
            selection: None,
            capture_busy: false,
            manual_busy: None,
            pending_ocr: None,
            live_busy: None,
            asr_busy: false,
            asr_caption_suppressed: false,
            audio: None,
            caption_enabled: false,
            audio_enabled: false,
            audio_generation: 7,
            caption_generation: 11,
            audio_offset_ms: 0,
            pending_audio: None,
            pending_transcript: None,
            video_id: "video-a".into(),
            caption_epoch: 2,
            caption_paused: true,
            last_caption: None,
            pending_caption: None,
            last_live_text: String::new(),
            last_live_id: None,
            overlay: false,
            overlay_text: String::new(),
            overlay_source: String::new(),
            overlay_time: None,
            overlay_kind: None,
            glossary_input: String::new(),
            search: String::new(),
            children: Vec::new(),
            hotkeys: None,
            file_busy: false,
            file_cancel: Arc::new(AtomicBool::new(false)),
            setup: setup::SetupState::default(),
            backend_ready: false,
            backend_loading: false,
            asr_ready: false,
            audio_start_requested: false,
            service_error: String::new(),
            health_check_running: false,
            last_health_check: Instant::now().checked_sub(Duration::from_secs(10)).unwrap(),
            backend_started_at: None,
            asr_started_at: None,
            startup_decided: false,
            expect_selected_model: false,
            backend_pid: None,
            asr_pid: None,
            audio_confirmation: false,
            audio_installing: false,
            audio_cancel: Arc::new(AtomicBool::new(false)),
            readout: readout::ReadoutState::default(),
            native_window: None,
        }
    }
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        ui::configure_theme(&cc.egui_ctx);
        let (tx, rx) = mpsc::sync_channel(256);
        let (bridge_tx, bridge_rx) = mpsc::sync_channel(8);
        let stop = Arc::new(AtomicBool::new(false));
        let (settings, load_error) = match fs::read(config_path()) {
            Ok(bytes) => match serde_json::from_slice::<Settings>(&bytes) {
                Ok(s) => (s, String::new()),
                Err(e) => (
                    Settings::default(),
                    format!("設定の読み込みに失敗しました: {e}"),
                ),
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                (Settings::default(), String::new())
            }
            Err(e) => (
                Settings::default(),
                format!("設定の読み込みに失敗しました: {e}"),
            ),
        };
        let token = settings.bridge_token.clone();
        let bridge_stop = stop.clone();
        let info = tx.clone();
        thread::spawn(move || {
            if let Err(e) = bridge::run(token, bridge_stop, bridge_tx) {
                let _ = info.send(Event::Info(e.to_string()));
            }
        });
        let (journal, records, journal_error) =
            match Journal::open_latest(&data_root().join("sessions")) {
                Ok((j, r)) => (Some(j), r, String::new()),
                Err(e) => (None, Vec::new(), e.to_string()),
            };
        let next_id = records.iter().map(|r| r.id).max().unwrap_or(0) + 1;
        let audio_offset_ms = max_audio_end(&records);
        let rows = records
            .into_iter()
            .map(|record| Row {
                record,
                pending: false,
                error: None,
                generation: 0,
            })
            .collect();
        let mut hotkeys = None;
        if let Ok(manager) = global_hotkey::GlobalHotKeyManager::new() {
            use global_hotkey::hotkey::{Code, HotKey, Modifiers};
            let _ = manager.register(HotKey::new(
                Some(Modifiers::CONTROL | Modifiers::ALT),
                Code::KeyT,
            ));
            let _ = manager.register(HotKey::new(
                Some(Modifiers::CONTROL | Modifiers::ALT),
                Code::KeyL,
            ));
            hotkeys = Some(manager);
        }
        let glossary_input = settings
            .translation
            .glossary
            .iter()
            .map(|(a, b)| format!("{a}\t{b}"))
            .collect::<Vec<_>>()
            .join("\n");
        let mut app = Self {
            settings,
            settings_load_failed: !load_error.is_empty(),
            tab: Tab::Text,
            input: String::new(),
            output: String::new(),
            status: if load_error.is_empty() {
                "画面・文章・再生音声を手元で翻訳します。".into()
            } else {
                load_error
            },
            rows,
            journal,
            journal_error,
            next_id,
            tx,
            rx,
            bridge_rx,
            stop,
            screens: platform::screens().unwrap_or_default(),
            screen_index: 0,
            selection: None,
            capture_busy: false,
            manual_busy: None,
            pending_ocr: None,
            live_busy: None,
            asr_busy: false,
            asr_caption_suppressed: false,
            audio: None,
            caption_enabled: false,
            audio_enabled: false,
            audio_generation: 1,
            caption_generation: 1,
            audio_offset_ms,
            pending_audio: None,
            pending_transcript: None,
            video_id: String::new(),
            caption_epoch: 0,
            caption_paused: true,
            last_caption: None,
            pending_caption: None,
            last_live_text: String::new(),
            last_live_id: None,
            overlay: false,
            overlay_text: String::new(),
            overlay_source: String::new(),
            overlay_time: None,
            overlay_kind: None,
            glossary_input,
            search: String::new(),
            children: Vec::new(),
            hotkeys,
            file_busy: false,
            file_cancel: Arc::new(AtomicBool::new(false)),
            setup: setup::SetupState::default(),
            backend_ready: false,
            backend_loading: false,
            asr_ready: false,
            audio_start_requested: false,
            service_error: String::new(),
            health_check_running: false,
            last_health_check: Instant::now().checked_sub(Duration::from_secs(10)).unwrap(),
            backend_started_at: None,
            asr_started_at: None,
            startup_decided: false,
            expect_selected_model: false,
            backend_pid: None,
            asr_pid: None,
            audio_confirmation: false,
            audio_installing: false,
            audio_cancel: Arc::new(AtomicBool::new(false)),
            readout: readout::ReadoutState::default(),
            native_window: None,
        };
        app.initialize_services();
        app.initialize_readout();
        app
    }
    fn translate(
        &mut self,
        text: String,
        lane: Lane,
        start_ms: u64,
        end_ms: u64,
        kind: &str,
        reuse: Option<u64>,
    ) {
        if text.trim().is_empty() {
            return;
        }
        let generation = self.source_generation(kind);
        let id = reuse.unwrap_or_else(|| {
            let id = self.next_id;
            self.next_id += 1;
            id
        });
        if let Some(row) = self.rows.iter_mut().find(|r| r.record.id == id) {
            row.record.source = text.clone();
            row.record.end_ms = end_ms;
            row.record.translation.clear();
            row.pending = true;
            row.error = None;
            row.generation = generation;
        } else {
            self.rows.push(Row {
                record: Record {
                    id,
                    start_ms,
                    end_ms,
                    source: text.clone(),
                    translation: String::new(),
                    kind: kind.into(),
                    video_id: if kind == "YouTube" {
                        self.video_id.clone()
                    } else {
                        String::new()
                    },
                    created_at: chrono::Local::now().to_rfc3339(),
                },
                pending: true,
                error: None,
                generation,
            });
        }
        if lane == Lane::Manual {
            self.manual_busy = Some(id);
            self.output.clear();
        } else {
            self.live_busy = Some(id);
            if kind == "YouTube" {
                self.last_live_id = Some(id);
                self.last_live_text = text.clone();
            }
        }
        let options = self.settings.translation.clone();
        let tx = self.tx.clone();
        thread::spawn(move || {
            let start = Instant::now();
            let result = Translator::new(options)
                .and_then(|t| {
                    t.translate(&text, |s| {
                        let _ = tx.try_send(Event::Partial { id, text: s.into() });
                    })
                })
                .map_err(|e| format!("{e:#}"));
            let _ = tx.send(Event::Done {
                id,
                lane,
                result,
                elapsed: start.elapsed().as_secs_f32(),
                generation,
            });
        });
    }
    fn manual_translate(&mut self) {
        if !self.backend_ready {
            self.status = "翻訳モデルを準備してからお試しください。".into();
            return;
        }
        if self.manual_busy.is_some() {
            return;
        }
        self.translate(self.input.clone(), Lane::Manual, 0, 0, "文章", None);
    }
    fn capture(&mut self, ctx: &egui::Context) {
        if self.capture_busy || self.selection.is_some() {
            return;
        }
        let Some(screen) = self.screens.get(self.screen_index).cloned() else {
            self.status = "取得できる画面がありません。".into();
            return;
        };
        self.capture_busy = true;
        let tx = self.tx.clone();
        let ctx = ctx.clone();
        let owner = self.native_window;
        thread::spawn(move || {
            let result = std::panic::catch_unwind(|| {
                owner.map_or_else(
                    || platform::capture_screen(screen.id),
                    |owner| platform::capture_for_selection(screen.id, owner),
                )
            })
            .unwrap_or_else(|_| {
                Err(anyhow::anyhow!(
                    "画面を取得できませんでした。もう一度お試しください。"
                ))
            })
            .map(|im| (screen, im))
            .map_err(|e| e.to_string());
            let _ = tx.send(Event::Screenshot(result));
            ctx.request_repaint();
        });
    }
    fn do_ocr(&mut self, image: image::RgbaImage) {
        self.status = "画像から文字を読み取っています…".into();
        let language = self.settings.translation.source_lang.clone();
        let tx = self.tx.clone();
        thread::spawn(move || {
            let result = platform::ocr(&image, &language).map_err(|e| format!("{e:#}"));
            let _ = tx.send(Event::Ocr(result));
        });
    }
    fn source_generation(&self, kind: &str) -> u64 {
        match LiveKind::from_record(kind) {
            Some(LiveKind::Audio) => self.audio_generation,
            Some(LiveKind::Caption) => self.caption_generation,
            None => 0,
        }
    }
    fn captions_fresh(&self) -> bool {
        self.caption_enabled
            && !self.caption_paused
            && self
                .last_caption
                .is_some_and(|i| i.elapsed() < Duration::from_secs(5))
    }
    fn overlay_allowed(&self, kind: &str, generation: u64) -> bool {
        match LiveKind::from_record(kind) {
            Some(LiveKind::Audio) => {
                self.audio_enabled && generation == self.audio_generation && !self.captions_fresh()
            }
            Some(LiveKind::Caption) => {
                self.caption_enabled
                    && !self.caption_paused
                    && generation == self.caption_generation
            }
            None => false,
        }
    }
    fn clear_overlay(&mut self, kind: Option<LiveKind>) {
        if kind.is_none() || self.overlay_kind == kind {
            self.overlay_text.clear();
            self.overlay_source.clear();
            self.overlay_time = None;
            self.overlay_kind = None;
        }
    }
    fn audio_clock_ms(&self) -> u64 {
        self.audio_offset_ms.saturating_add(
            self.audio
                .as_ref()
                .map(|a| a.started.elapsed().as_millis() as u64)
                .unwrap_or(0),
        )
    }
    fn stop_audio(&mut self) {
        self.audio_offset_ms = self
            .audio_clock_ms()
            .max(max_audio_end(self.rows.iter().map(|r| &r.record)));
        self.audio_enabled = false;
        if let Some(audio) = self.audio.take() {
            audio.stop.store(true, Ordering::Relaxed);
        }
        self.audio_generation += 1;
        self.pending_audio = None;
        self.pending_transcript = None;
        self.clear_overlay(Some(LiveKind::Audio));
    }
    fn set_caption_enabled(&mut self, enabled: bool) {
        if self.caption_enabled == enabled {
            return;
        }
        self.caption_enabled = enabled;
        self.caption_generation += 1;
        self.caption_paused = true;
        self.last_caption = None;
        self.pending_caption = None;
        self.last_live_text.clear();
        self.last_live_id = None;
        self.clear_overlay(Some(LiveKind::Caption));
    }
    fn accept_caption(&mut self, caption: bridge::Caption) {
        if !self.caption_enabled {
            return;
        }
        if caption.video_id != self.video_id || caption.epoch != self.caption_epoch {
            self.caption_generation += 1;
            self.clear_overlay(Some(LiveKind::Caption));
            self.pending_caption = None;
            self.last_live_id = None;
            self.last_live_text.clear();
            self.last_caption = None;
            self.video_id = caption.video_id.clone();
            self.caption_epoch = caption.epoch;
        }
        self.caption_paused = caption.paused || caption.text.trim().is_empty();
        if self.caption_paused {
            self.pending_caption = None;
            self.clear_overlay(Some(LiveKind::Caption));
        } else {
            self.last_caption = Some(Instant::now());
            self.pending_caption = Some(caption);
            self.pending_transcript = None;
            if self.asr_busy {
                self.asr_caption_suppressed = true;
            }
            self.clear_overlay(Some(LiveKind::Audio));
        }
    }
    fn accept_ocr(&mut self, text: String) {
        // Keep a single latest request while a manual translation is running.
        self.pending_ocr = Some(text);
        if self.tab != Tab::Live {
            self.tab = Tab::Text;
        }
    }
    fn take_ready_ocr(&mut self) -> Option<String> {
        if self.manual_busy.is_none() {
            self.pending_ocr.take()
        } else {
            None
        }
    }
    fn take_ready_transcript(&mut self) -> Option<PendingTranscript> {
        if self.captions_fresh() {
            self.pending_transcript = None;
            return None;
        }
        if self.pending_transcript.as_ref().is_some_and(|p| {
            p.generation != self.audio_generation
                || self.audio_clock_ms().saturating_sub(p.end_ms) > MAX_AUDIO_AGE_MS
        }) {
            self.pending_transcript = None;
        }
        if self.audio_enabled && self.live_busy.is_none() {
            self.pending_transcript.take()
        } else {
            None
        }
    }
    fn take_ready_audio(&mut self) -> Option<platform::AudioChunk> {
        if self.readout.suppress.load(Ordering::Acquire) {
            self.pending_audio = None;
            return None;
        }
        if self.pending_audio.as_ref().is_some_and(|p| {
            self.audio_clock_ms()
                .saturating_sub(self.audio_offset_ms.saturating_add(p.end_ms))
                > MAX_AUDIO_AGE_MS
        }) {
            self.pending_audio = None;
        }
        if self.audio_enabled && !self.captions_fresh() && !self.asr_busy {
            self.pending_audio.take()
        } else {
            None
        }
    }
    fn invalidate_timeline(&mut self) {
        self.audio_generation += 1;
        self.caption_generation += 1;
        self.pending_audio = None;
        self.pending_transcript = None;
        self.pending_caption = None;
        self.last_live_id = None;
        self.last_live_text.clear();
        self.last_caption = None;
        self.caption_paused = true;
        self.clear_overlay(None);
    }
    fn toggle_audio(&mut self) {
        if !self.audio_enabled && !self.asr_ready {
            self.audio_start_requested = true;
            self.prepare_audio();
            return;
        }
        if self.audio_enabled {
            self.stop_audio();
            self.status = "音声認識を停止しました。保存済みの訳文は残っています。".into();
            return;
        }
        let (tx, rx) = mpsc::sync_channel(1);
        let stop = Arc::new(AtomicBool::new(false));
        let capture_stop = stop.clone();
        let events = self.tx.clone();
        let seconds = self.settings.audio_seconds.clamp(2.0, 4.0);
        let suppress = self.readout.suppress.clone();
        thread::spawn(move || {
            if let Err(e) =
                platform::capture_loopback_suppressing(capture_stop.clone(), tx, seconds, suppress)
            {
                let _ = events.send(Event::Info(format!("音声の取得を停止しました: {e:#}")));
            }
            capture_stop.store(true, Ordering::Relaxed);
        });
        self.audio = Some(AudioSession {
            stop,
            rx,
            started: Instant::now(),
        });
        self.audio_offset_ms = self
            .audio_offset_ms
            .max(max_audio_end(self.rows.iter().map(|r| &r.record)));
        self.audio_enabled = true;
        self.audio_start_requested = false;
        self.audio_generation += 1;
        self.pending_audio = None;
        self.pending_transcript = None;
        self.tab = Tab::Live;
        self.status = "再生中の音声を認識しています。マイクは使用しません。".into();
    }
    fn process(&mut self, ctx: &egui::Context) {
        while let Ok(event) = self.rx.try_recv() {
            match event {
                Event::Partial { id, text } => {
                    if self.manual_busy == Some(id) {
                        self.output = text.clone();
                    }
                    if let Some(row) = self.rows.iter_mut().find(|r| r.record.id == id) {
                        row.record.translation = text;
                    }
                }
                Event::Done {
                    id,
                    lane,
                    result,
                    elapsed,
                    generation,
                } => {
                    if lane == Lane::Manual && self.manual_busy == Some(id) {
                        self.manual_busy = None;
                    }
                    if lane == Lane::Live && self.live_busy == Some(id) {
                        self.live_busy = None;
                    }
                    let show_overlay = lane == Lane::Live
                        && self
                            .rows
                            .iter()
                            .find(|r| r.record.id == id)
                            .is_some_and(|r| self.overlay_allowed(&r.record.kind, generation));
                    let mut read_text = None;
                    if let Some(row) = self.rows.iter_mut().find(|r| r.record.id == id) {
                        row.pending = false;
                        match result {
                            Ok(text) => {
                                if self.settings.auto_read && (lane == Lane::Manual || show_overlay)
                                {
                                    read_text = Some(text.clone());
                                }
                                row.record.translation = text.clone();
                                if lane == Lane::Manual {
                                    self.output = text.clone();
                                }
                                if show_overlay {
                                    self.overlay_text = text;
                                    self.overlay_source = row.record.source.clone();
                                    self.overlay_time = Some(Instant::now());
                                    self.overlay_kind = LiveKind::from_record(&row.record.kind);
                                }
                                if let Some(journal) = &self.journal
                                    && let Err(e) = journal.append(&row.record)
                                {
                                    self.journal_error = e.to_string();
                                }
                                self.status = format!("翻訳完了 · {elapsed:.1}秒");
                            }
                            Err(error) => {
                                row.error = Some(error.clone());
                                self.status = error;
                            }
                        }
                    }
                    if let Some(text) = read_text {
                        self.read_translation(text);
                    }
                }
                Event::Ocr(result) => match result {
                    Ok(text) if !text.trim().is_empty() => {
                        self.accept_ocr(text);
                    }
                    Ok(_) => {
                        self.status =
                            "文字が見つかりませんでした。範囲や原文言語を確認してください。".into()
                    }
                    Err(e) => self.status = e,
                },
                Event::Screenshot(result) => {
                    self.capture_busy = false;
                    match result {
                        Ok((screen, image)) => {
                            let color = egui::ColorImage::from_rgba_unmultiplied(
                                [image.width() as usize, image.height() as usize],
                                image.as_raw(),
                            );
                            let texture = ctx.load_texture(
                                "screen-capture",
                                color,
                                egui::TextureOptions::LINEAR,
                            );
                            self.selection = Some(Selection {
                                screen,
                                image,
                                texture,
                                start: None,
                            });
                        }
                        Err(e) => self.status = e,
                    }
                }
                Event::Transcript {
                    generation,
                    start_ms,
                    end_ms,
                    result,
                } => {
                    self.asr_busy = false;
                    let caption_suppressed = std::mem::take(&mut self.asr_caption_suppressed);
                    if generation != self.audio_generation
                        || !self.audio_enabled
                        || caption_suppressed
                    {
                        continue;
                    }
                    match result {
                        Ok(text) if !text.trim().is_empty() => {
                            self.pending_transcript = Some(PendingTranscript {
                                text,
                                start_ms,
                                end_ms,
                                generation,
                            });
                        }
                        Ok(_) => {}
                        Err(e) => self.status = format!("音声認識: {e}"),
                    }
                }
                Event::FileDone(result) => {
                    self.file_busy = false;
                    self.status = result.unwrap_or_else(|e| format!("ファイル翻訳: {e}"));
                }
                Event::Info(info) => self.status = info,
                Event::Readout(event) => self.accept_readout_event(event),
                Event::Hardware(result) => {
                    self.setup.detecting = false;
                    match result {
                        Ok(hardware) => {
                            self.setup.hardware = Some(hardware);
                            self.recalculate_recommendation();
                        }
                        Err(error) => self.setup.error = error,
                    }
                }
                Event::InstallProgress(progress) => self.setup.progress = Some(progress),
                Event::Installed(result) => {
                    self.setup.installing = false;
                    match result {
                        Ok(result) => self.adopt_installation(result),
                        Err(_) if self.setup.cancel.load(Ordering::Relaxed) => {
                            self.setup.cancelled = true;
                            self.setup.error.clear();
                            self.setup.progress = None;
                        }
                        Err(error) => {
                            self.setup.error = error;
                            self.setup.progress = None;
                        }
                    }
                }
                Event::ServiceStatus { translation, asr } => {
                    self.accept_service_status(translation, asr)
                }
                Event::AudioPrepared(result) => {
                    self.audio_installing = false;
                    match result {
                        Ok(()) => {
                            self.audio_confirmation = false;
                            if let Err(error) = self.load_audio_installation() {
                                self.service_error = error.to_string();
                                self.audio_start_requested = false;
                            } else {
                                self.start_asr();
                            }
                        }
                        Err(error) => {
                            self.service_error = error;
                            self.audio_start_requested = false;
                        }
                    }
                }
            }
        }
        if let Some(text) = self.take_ready_ocr() {
            self.input = text;
            self.translate(self.input.clone(), Lane::Manual, 0, 0, "画面OCR", None);
        }
        let mut latest = None;
        while let Ok(caption) = self.bridge_rx.try_recv() {
            latest = Some(caption);
        }
        if let Some(caption) = latest {
            self.accept_caption(caption);
        }
        let captions_fresh = self.captions_fresh();
        if captions_fresh {
            // A transcript completed before this frame's caption message must
            // also yield priority to the newly active captions.
            self.pending_transcript = None;
        }
        if self.backend_ready
            && self.caption_enabled
            && !self.caption_paused
            && self.live_busy.is_none()
            && let Some(caption) = self.pending_caption.take()
        {
            if caption.text != self.last_live_text {
                let reuse = self.last_live_id.filter(|id| {
                    self.rows
                        .iter()
                        .find(|r| r.record.id == *id)
                        .is_some_and(|r| {
                            r.record.kind == "YouTube"
                                && r.generation == self.caption_generation
                                && caption.text.starts_with(&r.record.source)
                        })
                });
                let start = (caption.position * 1000.0) as u64;
                self.translate(
                    caption.text,
                    Lane::Live,
                    start,
                    start + 3000,
                    "YouTube",
                    reuse,
                );
            } else if let Some(row) = self
                .last_live_id
                .and_then(|id| self.rows.iter().find(|r| r.record.id == id))
                .filter(|r| {
                    !r.pending
                        && r.error.is_none()
                        && r.generation == self.caption_generation
                        && !r.record.translation.is_empty()
                })
            {
                // Resuming the same caption reuses its completed translation.
                self.overlay_text = row.record.translation.clone();
                self.overlay_source = row.record.source.clone();
                self.overlay_time = Some(Instant::now());
                self.overlay_kind = Some(LiveKind::Caption);
            }
        }
        if self.audio_enabled {
            let suppressed = self.readout.suppress.load(Ordering::Acquire);
            if suppressed {
                self.pending_audio = None;
            }
            let mut capture_stopped = false;
            if let Some(audio) = &self.audio {
                // Always drain capture into one persistent latest slot. Translation
                // latency must not discard the only waiting audio chunk.
                while let Ok(value) = audio.rx.try_recv() {
                    if !suppressed {
                        self.pending_audio = Some(value);
                    }
                }
                capture_stopped = audio.stop.load(Ordering::Relaxed);
            }
            if capture_stopped {
                self.stop_audio();
            }
        }
        if self.audio_enabled && self.backend_ready {
            if let Some(pending) = self.take_ready_transcript() {
                self.translate(
                    pending.text,
                    Lane::Live,
                    pending.start_ms,
                    pending.end_ms,
                    "音声",
                    None,
                );
            }
            // ASR and the live translator are independent pipeline stages.
            if let Some(chunk) = self.take_ready_audio() {
                self.asr_busy = true;
                self.asr_caption_suppressed = false;
                let tx = self.tx.clone();
                let endpoint = self.settings.asr_endpoint.clone();
                let lang = self.settings.translation.source_lang.clone();
                let generation = self.audio_generation;
                let start_ms = self.audio_offset_ms.saturating_add(chunk.start_ms);
                let end_ms = self.audio_offset_ms.saturating_add(chunk.end_ms);
                thread::spawn(move || {
                    let result = platform::pcm_to_wav(&chunk.samples, chunk.sample_rate)
                        .and_then(|wav| engine::transcribe(&endpoint, wav, &lang))
                        .map_err(|e| format!("{e:#}"));
                    let _ = tx.send(Event::Transcript {
                        generation,
                        start_ms,
                        end_ms,
                        result,
                    });
                });
            }
        }
        if self
            .overlay_time
            .is_some_and(|i| i.elapsed() > Duration::from_secs(12))
        {
            self.clear_overlay(None);
        }
        use global_hotkey::hotkey::{Code, HotKey, Modifiers};
        while let Ok(event) = global_hotkey::GlobalHotKeyEvent::receiver().try_recv() {
            if event.state == global_hotkey::HotKeyState::Pressed {
                if event.id
                    == HotKey::new(Some(Modifiers::CONTROL | Modifiers::ALT), Code::KeyT).id()
                {
                    self.capture(ctx);
                }
                if event.id
                    == HotKey::new(Some(Modifiers::CONTROL | Modifiers::ALT), Code::KeyL).id()
                {
                    self.toggle_audio();
                }
            }
        }
    }
    fn open_image(&mut self) {
        if let Some(path) = rfd::FileDialog::new()
            .add_filter("画像", &["png", "jpg", "jpeg", "webp", "bmp"])
            .pick_file()
        {
            match image::open(path) {
                Ok(image) => self.do_ocr(image.to_rgba8()),
                Err(e) => self.status = e.to_string(),
            }
        }
    }
    fn save_text(&mut self) {
        if let Some(path) = rfd::FileDialog::new()
            .set_file_name("translation.txt")
            .save_file()
        {
            match fs::write(path, &self.output) {
                Ok(()) => self.status = "訳文を保存しました。".into(),
                Err(e) => self.status = e.to_string(),
            }
        }
    }
    fn export_history(&mut self) {
        let Some(path) = rfd::FileDialog::new()
            .add_filter("字幕", &["srt"])
            .add_filter("対訳テキスト", &["txt"])
            .add_filter("JSON", &["json"])
            .set_file_name("timeline.txt")
            .save_file()
        else {
            return;
        };
        let records: Vec<_> = self
            .rows
            .iter()
            .filter(|r| !r.pending && r.error.is_none() && !r.record.translation.is_empty())
            .map(|r| r.record.clone())
            .collect();
        let ext = path.extension().and_then(|s| s.to_str()).unwrap_or("txt");
        let content = match ext {
            "json" => serde_json::to_string_pretty(&records).unwrap_or_default(),
            "srt" => match timeline_srt(&records) {
                Ok(content) => content,
                Err(e) => {
                    self.status = e.to_string();
                    return;
                }
            },
            _ => records
                .iter()
                .map(|r| {
                    format!(
                        "[{} · {}] {}\n{}\n{}\n",
                        clock(r.start_ms),
                        r.kind,
                        r.created_at,
                        r.source,
                        r.translation
                    )
                })
                .collect::<Vec<_>>()
                .join("\n"),
        };
        match fs::write(path, content) {
            Ok(()) => self.status = "タイムラインを書き出しました。".into(),
            Err(e) => self.status = e.to_string(),
        }
    }
    fn translate_file(&mut self) {
        let Some(input) = rfd::FileDialog::new()
            .add_filter("テキスト・字幕", &["txt", "md", "srt", "vtt"])
            .pick_file()
        else {
            return;
        };
        let ext = input
            .extension()
            .and_then(|s| s.to_str())
            .unwrap_or("txt")
            .to_lowercase();
        let filename = format!(
            "{}-translated.{}",
            input.file_stem().unwrap_or_default().to_string_lossy(),
            ext
        );
        let Some(output) = rfd::FileDialog::new().set_file_name(filename).save_file() else {
            return;
        };
        if input == output {
            self.status = "元のファイルを保つため、別のファイル名を選んでください。".into();
            return;
        }
        self.file_busy = true;
        self.file_cancel = Arc::new(AtomicBool::new(false));
        let cancel = self.file_cancel.clone();
        let tx = self.tx.clone();
        let options = self.settings.translation.clone();
        thread::spawn(move || {
            let result = (|| -> Result<String> {
                let metadata = fs::metadata(&input)?;
                anyhow::ensure!(
                    metadata.len() <= 10_000_000,
                    "10MB以下のファイルを選んでください。"
                );
                let source = fs::read_to_string(&input)?;
                let translator = Translator::new(options)?;
                let content = if ext == "srt" || ext == "vtt" {
                    let mut cues = if ext == "srt" {
                        subtitles::parse_srt(&source)?
                    } else {
                        subtitles::parse_vtt(&source)?
                    };
                    let total = cues.len();
                    for (index, cue) in cues.iter_mut().enumerate() {
                        anyhow::ensure!(
                            !cancel.load(Ordering::Relaxed),
                            "中止しました。元のファイルと出力先は変更していません。"
                        );
                        cue.text = translator.translate(&cue.text, |_| {})?;
                        let _ = tx.try_send(Event::Info(format!(
                            "字幕を翻訳中 {}/{}",
                            index + 1,
                            total
                        )));
                    }
                    if ext == "srt" {
                        subtitles::to_srt(&cues)
                    } else {
                        subtitles::to_vtt(&cues)
                    }
                } else {
                    let parts = subtitles::split_text(&source, 1500);
                    let mut result = String::new();
                    for (index, part) in parts.iter().enumerate() {
                        anyhow::ensure!(!cancel.load(Ordering::Relaxed), "中止しました。");
                        if part.trim().is_empty() {
                            result.push_str(part);
                        } else {
                            result.push_str(&translator.translate(part, |_| {})?);
                            result.push_str("\n\n");
                        }
                        let _ = tx.try_send(Event::Info(format!(
                            "文章を翻訳中 {}/{}",
                            index + 1,
                            parts.len()
                        )));
                    }
                    result
                };
                anyhow::ensure!(!cancel.load(Ordering::Relaxed), "中止しました。");
                fs::write(&output, content)?;
                Ok(format!("保存しました: {}", output.display()))
            })();
            let _ = tx.send(Event::FileDone(result.map_err(|e| format!("{e:#}"))));
        });
    }
    fn selection_ui(&mut self, ctx: &egui::Context) {
        let Some(selection) = &mut self.selection else {
            return;
        };
        let result = Arc::new(Mutex::new(None::<Option<egui::Rect>>));
        let response = result.clone();
        let screen = &selection.screen;
        let builder = egui::ViewportBuilder::default()
            .with_title("翻訳範囲を選択 · Escでキャンセル")
            .with_decorations(false)
            .with_position([
                screen.x as f32 / screen.scale,
                screen.y as f32 / screen.scale,
            ])
            .with_inner_size([
                screen.width as f32 / screen.scale,
                screen.height as f32 / screen.scale,
            ])
            .with_always_on_top()
            .with_resizable(false);
        ctx.show_viewport_immediate(
            egui::ViewportId::from_hash_of("screen-selection"),
            builder,
            |ctx, _| {
                egui::CentralPanel::default()
                    .frame(egui::Frame::NONE)
                    .show(ctx, |ui| {
                        let rect = ui.max_rect();
                        ui.painter().image(
                            selection.texture.id(),
                            rect,
                            egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                            Color32::WHITE,
                        );
                        let interaction =
                            ui.interact(rect, ui.id().with("drag-select"), egui::Sense::drag());
                        if interaction.drag_started() {
                            selection.start = interaction.interact_pointer_pos();
                        }
                        if let (Some(start), Some(pos)) =
                            (selection.start, interaction.interact_pointer_pos())
                        {
                            let chosen = egui::Rect::from_two_pos(start, pos).intersect(rect);
                            ui.painter().rect_stroke(
                                chosen,
                                0.0,
                                egui::Stroke::new(2.5_f32, ACCENT),
                                egui::StrokeKind::Inside,
                            );
                            if interaction.drag_stopped() {
                                let normalized = egui::Rect::from_min_max(
                                    egui::pos2(
                                        (chosen.min.x - rect.min.x) / rect.width(),
                                        (chosen.min.y - rect.min.y) / rect.height(),
                                    ),
                                    egui::pos2(
                                        (chosen.max.x - rect.min.x) / rect.width(),
                                        (chosen.max.y - rect.min.y) / rect.height(),
                                    ),
                                );
                                *response.lock().unwrap() = Some(Some(normalized));
                                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                            }
                        }
                        ui.painter().text(
                            rect.min + Vec2::new(24.0, 24.0),
                            egui::Align2::LEFT_TOP,
                            "翻訳したい範囲をドラッグ · Escでキャンセル",
                            egui::FontId::proportional(20.0),
                            ACCENT,
                        );
                        if ctx.input(|i| {
                            i.key_pressed(egui::Key::Escape) || i.viewport().close_requested()
                        }) {
                            *response.lock().unwrap() = Some(None);
                            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                        }
                    });
            },
        );
        let selected = result.lock().unwrap().take();
        if let Some(rect) = selected {
            let selection = self.selection.take().unwrap();
            if let Some(rect) = rect {
                let width = selection.image.width();
                let height = selection.image.height();
                let x = (rect.min.x * width as f32).floor().max(0.0) as u32;
                let y = (rect.min.y * height as f32).floor().max(0.0) as u32;
                let w = ((rect.width() * width as f32).ceil() as u32).min(width.saturating_sub(x));
                let h =
                    ((rect.height() * height as f32).ceil() as u32).min(height.saturating_sub(y));
                if w >= 5 && h >= 5 {
                    self.do_ocr(image::imageops::crop_imm(&selection.image, x, y, w, h).to_image());
                } else {
                    self.status = "もう少し広い範囲を選択してください。".into();
                }
            }
        }
    }
    fn overlay_ui(&mut self, ctx: &egui::Context) {
        if !self.overlay {
            return;
        }
        let text = self.overlay_text.clone();
        let source = self.overlay_source.clone();
        let show_original = self.settings.show_original;
        let mut close = false;
        ctx.show_viewport_immediate(
            egui::ViewportId::from_hash_of("subtitle-overlay"),
            egui::ViewportBuilder::default()
                .with_title("地元翻訳くん · ライブ字幕")
                .with_inner_size([760.0, 170.0])
                .with_always_on_top()
                .with_resizable(true),
            |ctx, _| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    if show_original && !source.is_empty() {
                        ui.label(RichText::new(&source).size(15.0).color(Color32::GRAY));
                    }
                    ui.label(
                        RichText::new(if text.is_empty() {
                            "字幕を待っています…"
                        } else {
                            &text
                        })
                        .size(26.0),
                    );
                    if ctx.input(|i| i.viewport().close_requested()) {
                        close = true;
                    }
                });
            },
        );
        if close {
            self.overlay = false;
        }
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        if self.native_window.is_none()
            && let Ok(handle) = frame.window_handle()
            && let RawWindowHandle::Win32(handle) = handle.as_raw()
        {
            self.native_window = Some(handle.hwnd.get());
        }
        self.process(ctx);
        self.poll_services();
        self.poll_readout();
        self.render_ui(ctx);
        self.audio_setup_ui(ctx);
        self.readout_setup_ui(ctx);
        self.selection_ui(ctx);
        self.overlay_ui(ctx);
        if ctx.input(|i| i.modifiers.ctrl && i.key_pressed(egui::Key::Q)) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        if ctx.input(|i| !i.raw.dropped_files.is_empty()) {
            let files = ctx.input(|i| i.raw.dropped_files.clone());
            if let Some(path) = files.first().and_then(|f| f.path.clone()) {
                match image::open(&path) {
                    Ok(image) => self.do_ocr(image.to_rgba8()),
                    Err(_) => match fs::read_to_string(&path) {
                        Ok(text) => {
                            self.input = text;
                            self.tab = Tab::Text;
                        }
                        Err(e) => self.status = e.to_string(),
                    },
                }
            }
        }
        self.children.retain_mut(|child| match child.try_wait() {
            Ok(Some(status)) => {
                if self.backend_pid == Some(child.id()) {
                    self.backend_pid = None;
                    self.backend_loading = false;
                    self.backend_ready = false;
                    self.backend_started_at = None;
                }
                if self.asr_pid == Some(child.id()) {
                    self.asr_pid = None;
                    self.asr_started_at = None;
                    self.asr_ready = false;
                    self.audio_start_requested = false;
                }
                if !status.success() {
                    self.status =
                        "モデルの起動に失敗しました。保存先のlogsを確認してください。".into();
                    self.service_error = self.status.clone();
                }
                false
            }
            _ => true,
        });
        ctx.request_repaint_after(Duration::from_millis(
            if self.audio_enabled
                || self.caption_enabled
                || self.manual_busy.is_some()
                || self.live_busy.is_some()
            {
                50
            } else {
                200
            },
        ));
    }
}
impl Drop for App {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        self.setup.cancel.store(true, Ordering::Relaxed);
        self.audio_cancel.store(true, Ordering::Relaxed);
        self.readout.cancel.store(true, Ordering::Relaxed);
        self.file_cancel.store(true, Ordering::Relaxed);
        if let Some(audio) = &self.audio {
            audio.stop.store(true, Ordering::Relaxed);
        }
        for child in &mut self.children {
            services::kill_owned_child(child);
        }
        if self.journal_error.is_empty() && !self.settings_load_failed {
            let _ = save_settings(&self.settings);
        }
        let _ = &self.hotkeys;
    }
}
fn language_label(code: &str) -> String {
    if code == "auto" {
        "自動判定".into()
    } else {
        engine::LANGUAGES
            .iter()
            .find(|(c, _)| *c == code)
            .map(|(_, label)| (*label).into())
            .unwrap_or_else(|| code.into())
    }
}
fn clock(ms: u64) -> String {
    format!(
        "{:02}:{:02}:{:02}",
        ms / 3_600_000,
        (ms / 60_000) % 60,
        (ms / 1000) % 60
    )
}
fn max_audio_end<'a>(records: impl IntoIterator<Item = &'a Record>) -> u64 {
    records
        .into_iter()
        .filter(|r| r.kind == "音声")
        .map(|r| r.end_ms)
        .max()
        .unwrap_or(0)
}
fn timeline_srt(records: &[Record]) -> Result<String> {
    let timed: Vec<_> = records.iter().filter(|r| r.end_ms > r.start_ms).collect();
    anyhow::ensure!(
        !timed.is_empty(),
        "SRTに書き出せる時刻付きの字幕がありません。TXTかJSONを選んでください。"
    );
    let first = timed[0];
    let same_source = timed.iter().all(|r| {
        r.kind == first.kind
            && (r.kind == "音声"
                || (r.kind == "YouTube" && !r.video_id.is_empty() && r.video_id == first.video_id))
    });
    anyhow::ensure!(
        same_source,
        "音声とYouTube、または複数の動画が混在する履歴はSRTに書き出せません。TXTかJSONを選んでください。"
    );
    Ok(subtitles::to_srt(
        &timed
            .into_iter()
            .map(|r| Cue {
                start_ms: r.start_ms,
                end_ms: r.end_ms,
                text: r.translation.clone(),
            })
            .collect::<Vec<_>>(),
    ))
}
fn install_fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    fonts.font_data.insert(
        "japanese".into(),
        egui::FontData::from_static(include_bytes!("../assets/fonts/NotoSansJP-Regular.ttf"))
            .into(),
    );
    fonts
        .families
        .entry(egui::FontFamily::Proportional)
        .or_default()
        .insert(0, "japanese".into());
    fonts
        .families
        .entry(egui::FontFamily::Monospace)
        .or_default()
        .push("japanese".into());
    for (index, path) in [
        "C:/Windows/Fonts/malgun.ttf",
        "C:/Windows/Fonts/msyh.ttc",
        "C:/Windows/Fonts/NotoSansArabic-Regular.ttf",
        "C:/Windows/Fonts/NotoSansHebrew-Regular.ttf",
        "C:/Windows/Fonts/NotoSansDevanagari-Regular.ttf",
    ]
    .iter()
    .enumerate()
    {
        if let Ok(data) = fs::read(path) {
            let name = format!("language-fallback-{index}");
            fonts
                .font_data
                .insert(name.clone(), egui::FontData::from_owned(data).into());
            fonts
                .families
                .entry(egui::FontFamily::Proportional)
                .or_default()
                .push(name);
        }
    }
    ctx.set_fonts(fonts);
}
fn render_demo_views(folder: &Path) -> Result<()> {
    fs::create_dir_all(folder)?;
    for (viewport_width, viewport_height) in [(1280, 880), (860, 620)] {
        for (name, tab, filled) in [
            ("01-text-empty", Tab::Text, false),
            ("02-text-filled", Tab::Text, true),
            ("03-live", Tab::Live, true),
            ("04-settings", Tab::Settings, false),
            ("05-first-run", Tab::Setup, false),
        ] {
            let ctx = egui::Context::default();
            ui::configure_theme(&ctx);
            let mut app = App::inert();
            app.tab = tab;
            app.backend_ready = tab != Tab::Setup;
            app.asr_ready = tab == Tab::Live;
            app.startup_decided = true;
            app.status = "プレビュー · 接続や保存は行いません".into();
            app.settings.bridge_token = "preview-token".into();
            app.settings.model = "models/plamo-3-translate-2b-Q4_K_M.gguf".into();
            app.settings.llama_server = "runtime/llama-server.exe".into();
            app.settings.python = ".venv/Scripts/python.exe".into();
            if tab == Tab::Setup {
                app.setup.hardware = Some(plamo_translate::hardware::Hardware {
                    os_arch: "windows-x86_64".into(),
                    cpu_logical: 16,
                    ram_total_bytes: Some(32 * 1_073_741_824),
                    ram_available_bytes: Some(20 * 1_073_741_824),
                    disk_free_bytes: Some(120 * 1_073_741_824),
                    disk_target: "C:/".into(),
                    gpus: vec![plamo_translate::hardware::Gpu {
                        name: "NVIDIA GeForce RTX 4060".into(),
                        vendor: "nvidia".into(),
                        dedicated_vram_bytes: Some(8 * 1_073_741_824),
                        free_vram_bytes: Some(7 * 1_073_741_824),
                        driver_version: Some("570.00".into()),
                        cuda_eligible: true,
                        vulkan_eligible: true,
                        is_software: false,
                        memory_telemetry: "nvidia-smi".into(),
                    }],
                    warnings: vec![],
                });
                app.recalculate_recommendation();
            }
            if filled {
                app.input = "The meeting has been rescheduled for next Monday.\nPlease open the project dashboard.".into();
                app.output =
                "会議は来週の月曜日に変更になりました。\nプロジェクトの管理画面を開いてください。"
                    .into();
            }
            if tab == Tab::Live {
                app.caption_enabled = true;
                app.audio_enabled = true;
                for (index, source, translation) in [
                    (
                        0,
                        "Welcome back. Today we're going to build a small application.",
                        "おかえりなさい。今日は小さなアプリを作ります。",
                    ),
                    (
                        1,
                        "You can translate text on your screen while listening to a video.",
                        "動画を聴きながら、画面に表示された文章も翻訳できます。",
                    ),
                    (
                        2,
                        "Your translations are saved, so you can come back to them later.",
                        "翻訳は保存されるので、あとから読み返せます。",
                    ),
                ] {
                    app.rows.push(Row {
                        record: Record {
                            id: index + 1,
                            start_ms: index * 5000,
                            end_ms: index * 5000 + 4000,
                            source: source.into(),
                            translation: translation.into(),
                            kind: "YouTube".into(),
                            video_id: "preview".into(),
                            created_at: "2026-11-05T12:00:00+09:00".into(),
                        },
                        pending: false,
                        error: None,
                        generation: 0,
                    });
                }
            }
            preview::render(
                &ctx,
                &folder.join(if viewport_width == 1280 {
                    format!("{name}.png")
                } else {
                    format!("{name}-{viewport_width}x{viewport_height}.png")
                }),
                viewport_width,
                viewport_height,
                |ctx| app.render_ui(ctx),
            )?;
        }
    }
    Ok(())
}

fn main() -> eframe::Result {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if let Some(index) = args.iter().position(|arg| arg == "--render-preview") {
        let folder = args
            .get(index + 1)
            .map(PathBuf::from)
            .unwrap_or_else(|| app_root().join("verification/ui-before"));
        if let Err(error) = render_demo_views(&folder) {
            eprintln!("Preview failed: {error:#}");
            std::process::exit(1);
        }
        return Ok(());
    }
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_icon(
                eframe::icon_data::from_png_bytes(include_bytes!("../assets/icons/icon-256.png"))
                    .expect("Bundled application icon"),
            )
            .with_title("地元翻訳くん | translated by PLaMo")
            .with_inner_size([1180.0, 800.0])
            .with_min_inner_size([860.0, 620.0]),
        ..Default::default()
    };
    eframe::run_native(
        "地元翻訳くん",
        options,
        Box::new(|cc| Ok(Box::new(App::new(cc)))),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app() -> App {
        App::inert()
    }
    fn caption(text: &str, paused: bool) -> bridge::Caption {
        bridge::Caption {
            text: text.into(),
            video_id: "video-a".into(),
            position: 1.0,
            paused,
            epoch: 2,
        }
    }
    fn record(id: u64, kind: &str, video: &str, start: u64, end: u64) -> Record {
        Record {
            id,
            kind: kind.into(),
            video_id: video.into(),
            start_ms: start,
            end_ms: end,
            source: "source".into(),
            translation: "translation".into(),
            created_at: String::new(),
        }
    }
    fn chunk(start_ms: u64, end_ms: u64) -> platform::AudioChunk {
        platform::AudioChunk {
            samples: vec![0.0; 16],
            sample_rate: 16000,
            start_ms,
            end_ms,
        }
    }
    #[test]
    fn source_toggles_only_invalidate_their_own_generation_and_overlay() {
        let mut a = app();
        a.audio_enabled = true;
        a.overlay_kind = Some(LiveKind::Audio);
        a.overlay_text = "audio stays".into();
        a.set_caption_enabled(true);
        assert_eq!(a.audio_generation, 7);
        assert_eq!(a.caption_generation, 12);
        assert_eq!(a.overlay_text, "audio stays");
        a.overlay_kind = Some(LiveKind::Caption);
        a.overlay_text = "caption stays".into();
        a.stop_audio();
        assert_eq!(a.audio_generation, 8);
        assert_eq!(a.caption_generation, 12);
        assert_eq!(a.overlay_text, "caption stays");
        a.caption_paused = false;
        assert!(a.overlay_allowed("YouTube", 12));
        assert!(!a.overlay_allowed("YouTube", 11));
        assert!(!a.overlay_allowed("音声", 8));
    }
    #[test]
    fn paused_and_empty_captions_preserve_identity_and_dedup_without_blocking_audio() {
        let mut a = app();
        a.caption_enabled = true;
        a.audio_enabled = true;
        a.last_live_id = Some(42);
        a.last_live_text = "same caption".into();
        a.last_caption = Some(Instant::now());
        a.caption_paused = false;
        a.overlay_kind = Some(LiveKind::Caption);
        a.overlay_text = "old caption".into();
        let last = a.last_caption;
        a.accept_caption(caption("same caption", true));
        assert_eq!(a.caption_generation, 11);
        assert_eq!(a.last_live_id, Some(42));
        assert_eq!(a.last_live_text, "same caption");
        assert_eq!(a.last_caption, last);
        assert!(!a.captions_fresh());
        assert!(a.overlay_text.is_empty());
        assert!(!a.overlay_allowed("YouTube", 11));
        assert!(a.overlay_allowed("音声", 7));
        a.overlay_kind = Some(LiveKind::Audio);
        a.overlay_text = "audio".into();
        a.accept_caption(caption("", false));
        assert_eq!(a.overlay_text, "audio");
        assert_eq!(a.caption_generation, 11);
        a.accept_caption(caption("same caption", false));
        assert!(a.captions_fresh());
        assert_eq!(a.last_live_id, Some(42));
        assert_eq!(a.last_live_text, "same caption");
        assert!(!a.overlay_allowed("音声", 7));
    }
    #[test]
    fn latest_audio_persists_while_busy_and_asr_does_not_wait_for_translation() {
        let mut a = app();
        let (tx, rx) = mpsc::sync_channel(3);
        a.audio_enabled = true;
        a.asr_busy = true;
        a.live_busy = Some(99);
        a.audio = Some(AudioSession {
            stop: Arc::new(AtomicBool::new(false)),
            rx,
            started: Instant::now(),
        });
        tx.send(chunk(0, 1000)).unwrap();
        tx.send(chunk(1000, 2000)).unwrap();
        a.process(&egui::Context::default());
        assert_eq!(a.pending_audio.as_ref().unwrap().end_ms, 2000);
        a.process(&egui::Context::default());
        assert_eq!(a.pending_audio.as_ref().unwrap().end_ms, 2000);
        assert!(a.take_ready_audio().is_none());
        a.asr_busy = false;
        assert_eq!(a.take_ready_audio().unwrap().end_ms, 2000);
        assert_eq!(a.live_busy, Some(99));
    }
    #[test]
    fn transcript_waits_for_live_lane_and_active_captions_discard_it() {
        let mut a = app();
        a.audio_enabled = true;
        a.asr_busy = true;
        a.live_busy = Some(99);
        a.tx.send(Event::Transcript {
            generation: 7,
            start_ms: 0,
            end_ms: 1000,
            result: Ok("first".into()),
        })
        .unwrap();
        a.process(&egui::Context::default());
        assert_eq!(a.pending_transcript.as_ref().unwrap().text, "first");
        assert!(a.take_ready_transcript().is_none());
        a.live_busy = None;
        assert_eq!(a.take_ready_transcript().unwrap().text, "first");
        a.pending_transcript = Some(PendingTranscript {
            text: "obsolete".into(),
            start_ms: 0,
            end_ms: 1000,
            generation: 7,
        });
        a.caption_enabled = true;
        a.asr_busy = true;
        a.accept_caption(caption("new caption", false));
        assert!(a.pending_transcript.is_none());
        // The old ASR stays suppressed even if the caption expires before it completes.
        a.last_caption = Some(Instant::now() - Duration::from_secs(6));
        a.pending_caption = None;
        a.tx.send(Event::Transcript {
            generation: 7,
            start_ms: 0,
            end_ms: 1000,
            result: Ok("old in-flight audio".into()),
        })
        .unwrap();
        a.process(&egui::Context::default());
        assert!(a.pending_transcript.is_none());
        assert!(a.rows.is_empty());
    }
    #[test]
    fn pending_transcript_expires_and_new_timeline_invalidates_both_sources() {
        let mut a = app();
        a.audio_enabled = true;
        a.audio_offset_ms = 10000;
        a.pending_transcript = Some(PendingTranscript {
            text: "old".into(),
            start_ms: 0,
            end_ms: 1,
            generation: 7,
        });
        assert!(a.take_ready_transcript().is_none());
        a.pending_audio = Some(chunk(0, 1));
        a.pending_caption = Some(caption("caption", false));
        a.invalidate_timeline();
        assert_eq!(a.audio_generation, 8);
        assert_eq!(a.caption_generation, 12);
        assert!(a.pending_audio.is_none());
        assert!(a.pending_caption.is_none());
    }
    #[test]
    fn ocr_keeps_live_tab_and_latest_request_until_manual_lane_is_free() {
        let mut a = app();
        a.manual_busy = Some(1);
        a.accept_ocr("first".into());
        a.accept_ocr("latest".into());
        assert!(a.tab == Tab::Live);
        assert!(a.take_ready_ocr().is_none());
        a.manual_busy = None;
        assert_eq!(a.take_ready_ocr().as_deref(), Some("latest"));
    }
    #[test]
    fn srt_rejects_mixed_source_clocks_and_videos() {
        let audio = record(1, "音声", "", 0, 1000);
        let video_a = record(2, "YouTube", "a", 0, 1000);
        let video_b = record(3, "YouTube", "b", 1000, 2000);
        assert!(timeline_srt(&[audio.clone(), video_a.clone()]).is_err());
        assert!(timeline_srt(&[video_a.clone(), video_b]).is_err());
        assert!(timeline_srt(&[video_a]).is_ok());
        let audio_next = record(4, "音声", "", 1000, 2000);
        assert!(timeline_srt(&[audio, audio_next]).is_ok());
    }
    #[test]
    fn audio_clock_high_water_ignores_video_clock_and_survives_stop() {
        let mut a = app();
        a.rows.push(Row {
            record: record(1, "音声", "", 1000, 4000),
            pending: false,
            error: None,
            generation: 0,
        });
        a.rows.push(Row {
            record: record(2, "YouTube", "a", 90000, 99000),
            pending: false,
            error: None,
            generation: 0,
        });
        assert_eq!(max_audio_end(a.rows.iter().map(|r| &r.record)), 4000);
        a.audio_enabled = true;
        a.audio_offset_ms = 5000;
        let (_, rx) = mpsc::sync_channel(1);
        a.audio = Some(AudioSession {
            stop: Arc::new(AtomicBool::new(false)),
            rx,
            started: Instant::now() - Duration::from_secs(2),
        });
        a.stop_audio();
        assert!(a.audio_offset_ms >= 7000);
        assert!(a.audio_clock_ms() >= 7000);
    }
    #[test]
    fn own_readout_discards_waiting_audio_and_capture_can_resume() {
        let mut a = app();
        a.audio_enabled = true;
        a.pending_audio = Some(chunk(0, 1000));
        a.readout.suppress.store(true, Ordering::Release);
        assert!(a.take_ready_audio().is_none());
        assert!(a.pending_audio.is_none());
        a.readout.suppress.store(false, Ordering::Release);
        a.pending_audio = Some(chunk(0, 1000));
        assert!(a.take_ready_audio().is_some());
        assert!(a.audio_enabled);
    }
}
