#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use anyhow::Result;
use eframe::egui::{self, Color32, RichText, Vec2};
use plamo_translate::{
    bridge, credit,
    engine::{self, TranslationOptions, Translator},
    journal::{Journal, Record},
    platform,
    subtitles::{self, Cue},
};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use serde::{Deserialize, Serialize};
use std::{
    collections::VecDeque,
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

mod chat;
mod document;
mod file_output;
mod mini;
mod paragraph_tools;
mod picker;
mod preview;
mod readout;
mod screen_feed;
mod screen_ocr;
mod services;
mod setup;
mod setup_chain;
mod streaming;
mod subtitle_window;
mod theme;
mod ui;
mod window_mode;

use theme::ACCENT;

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
    nemotron_context: u32,
    show_original: bool,
    live_show_original: bool,
    auto_scroll: bool,
    font_size: f32,
    template: String,
    backend: String,
    gpu_layers: u32,
    asr_device: String,
    model_store: String,
    /// Readout is a hidden feature: its controls, its status and automatic
    /// reading stay out of the app until this is turned on in settings.
    readout_enabled: bool,
    auto_read: bool,
    tts_speed: f32,
    tts_catch_up: bool,
    tts_max_speed: f32,
    tts_voice_profile: String,
    ocr_backend: String,
    ocr_device: String,
    english_punctuation: bool,
    live_chunk: document::ChunkLength,
    screen_interval_ms: u64,
}
impl Default for Settings {
    fn default() -> Self {
        let root = app_root();
        let venv = root.join(".venv/Scripts/python.exe");
        Self {
            translation: TranslationOptions::default(),
            asr_endpoint: "http://127.0.0.1:17864".into(),
            asr_backend: "nemotron".into(),
            asr_model: "nvidia/nemotron-speech-streaming-en-0.6b".into(),
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
            nemotron_context: 6,
            show_original: true,
            live_show_original: false,
            auto_scroll: true,
            font_size: theme::READING_DEFAULT,
            template: root
                .join("models/chat_template.jinja")
                .to_string_lossy()
                .into(),
            backend: "cuda".into(),
            gpu_layers: 99,
            asr_device: "auto".into(),
            model_store: String::new(),
            readout_enabled: false,
            auto_read: false,
            tts_speed: 1.0,
            tts_catch_up: true,
            tts_max_speed: 1.8,
            tts_voice_profile: "default-v1".into(),
            ocr_backend: "windows".into(),
            ocr_device: "auto".into(),
            english_punctuation: true,
            live_chunk: document::ChunkLength::default(),
            screen_interval_ms: 1_500,
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

#[derive(Clone, Copy, Default, PartialEq)]
enum Tab {
    Text,
    /// The chat. It is the home screen, and the app opens on it.
    #[default]
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
    ParagraphTools(paragraph_tools::ToolsEvent),
    DocumentPunctuated {
        row_id: u64,
        request: plamo_translate::live_text::Request,
        options: TranslationOptions,
        result: std::result::Result<plamo_translate::punctuation::PunctuationResult, String>,
    },
    DocumentDone {
        row_id: u64,
        request: plamo_translate::live_text::Request,
        options: TranslationOptions,
        result: std::result::Result<document::LiveTranslation, String>,
        elapsed: f32,
    },
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
    Ocr {
        generation: u64,
        result: std::result::Result<String, String>,
    },
    OcrSetup(screen_ocr::OcrSetupEvent),
    Screenshot(std::result::Result<(platform::Screen, image::RgbaImage), String>),
    ScreenFrame {
        epoch: u64,
        result: std::result::Result<screen_feed::ScreenFrame, screen_feed::ScreenError>,
    },
    Transcript {
        generation: u64,
        start_ms: u64,
        end_ms: u64,
        result: std::result::Result<String, String>,
    },
    FileDone(std::result::Result<String, String>),
    Info(String),
    Hardware(std::result::Result<plamo_translate::hardware::Hardware, String>),
    GpuMemory(Vec<plamo_translate::hardware::Gpu>),
    InstallProgress(plamo_translate::installer::Progress),
    Installed(std::result::Result<plamo_translate::installer::InstallResult, String>),
    ServiceStatus {
        translation: std::result::Result<String, String>,
        asr: Option<services::AsrStatus>,
    },
    AudioPrepared {
        backend: String,
        result: std::result::Result<(), String>,
    },
    AudioProgress {
        backend: String,
        progress: plamo_translate::installer::Progress,
    },
    Readout(readout::ReadoutEvent),
}
#[derive(Clone)]
struct Row {
    record: Record,
    pending: bool,
    error: Option<String>,
    generation: u64,
}
impl Row {
    fn recovered(mut record: Record) -> Self {
        if let Some(paragraph) = &mut record.paragraph
            && matches!(
                paragraph.state,
                plamo_translate::journal::ParagraphState::Open
                    | plamo_translate::journal::ParagraphState::Closing
            )
        {
            paragraph.state = plamo_translate::journal::ParagraphState::Interrupted;
            paragraph.close_reason = Some("restarted".into());
        }
        Self {
            record,
            pending: false,
            error: None,
            generation: 0,
        }
    }
    fn is_provisional(&self) -> bool {
        self.record.paragraph.as_ref().is_some_and(|paragraph| {
            matches!(
                paragraph.state,
                plamo_translate::journal::ParagraphState::Open
                    | plamo_translate::journal::ParagraphState::Closing
            )
        })
    }
    fn is_confirmed(&self) -> bool {
        !self.pending
            && self.error.is_none()
            && !self.record.translation.trim().is_empty()
            && self.record.paragraph.as_ref().is_none_or(|paragraph| {
                paragraph.state == plamo_translate::journal::ParagraphState::Final
            })
    }
}
struct Selection {
    screen: platform::Screen,
    image: image::RgbaImage,
    texture: egui::TextureHandle,
    start: Option<egui::Pos2>,
    focus_requested: bool,
    /// Chosen for the 画面 live input rather than a single reading.
    continuous: bool,
}
struct AudioSession {
    stop: Arc<AtomicBool>,
    rx: AudioReceiver,
    started: Instant,
}
enum AudioReceiver {
    Batch(Receiver<platform::AudioChunk>),
    Streaming(Receiver<platform::StreamingAudioFrame>),
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
            "YouTube" | "画面" => Some(Self::Caption),
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
    screen_feed: Option<screen_feed::ScreenFeed>,
    screen_area_requested: bool,
    screen_epoch: u64,
    capture_protected: bool,
    capture_busy: bool,
    manual_busy: Option<u64>,
    pending_ocr: Option<String>,
    live_busy: Option<u64>,
    asr_busy: bool,
    asr_caption_suppressed: bool,
    audio: Option<AudioSession>,
    nemotron: Option<plamo_translate::nemotron::NemotronEngine>,
    nemotron_epoch: u64,
    nemotron_origin_ms: u64,
    nemotron_needs_origin: bool,
    nemotron_suspended: bool,
    asr_partial: String,
    streaming_transcripts: VecDeque<PendingTranscript>,
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
    current_live_id: Option<u64>,
    live_chat: chat::State,
    window_mode: window_mode::MiniWindowState,
    mini: mini::MiniState,
    close_after_restore: Option<Instant>,
    close_sent: bool,
    document: document::DocumentState,
    paragraph_tools: paragraph_tools::ToolsState,
    subtitle_window: subtitle_window::State,
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
    /// What 「聞く」 turns on. It is not saved: every start of the app begins with 自動.
    listen_input: ui::LiveSourceMode,
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
    audio_progress: Option<plamo_translate::installer::Progress>,
    audio_cancel: Arc<AtomicBool>,
    readout: readout::ReadoutState,
    ocr: screen_ocr::OcrState,
    /// The 状態の詳細 window is open.
    details_open: bool,
    native_window: Option<isize>,
    #[cfg(test)]
    ui_probe: ui::UiProbe,
}

impl App {
    fn inert() -> Self {
        let (tx, rx) = mpsc::sync_channel(256);
        let (_, bridge_rx) = mpsc::sync_channel(8);
        App {
            settings: Settings::default(),
            settings_load_failed: true,
            tab: Tab::default(),
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
            screen_feed: None,
            screen_area_requested: false,
            screen_epoch: 0,
            capture_protected: false,
            capture_busy: false,
            manual_busy: None,
            pending_ocr: None,
            live_busy: None,
            asr_busy: false,
            asr_caption_suppressed: false,
            audio: None,
            nemotron: None,
            nemotron_epoch: 0,
            nemotron_origin_ms: 0,
            nemotron_needs_origin: true,
            nemotron_suspended: false,
            asr_partial: String::new(),
            streaming_transcripts: VecDeque::new(),
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
            current_live_id: None,
            live_chat: chat::State::default(),
            window_mode: window_mode::MiniWindowState::default(),
            mini: mini::MiniState::default(),
            close_after_restore: None,
            close_sent: false,
            document: document::DocumentState::default(),
            paragraph_tools: paragraph_tools::ToolsState::default(),
            subtitle_window: subtitle_window::State::default(),
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
            listen_input: ui::LiveSourceMode::default(),
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
            audio_progress: None,
            audio_cancel: Arc::new(AtomicBool::new(false)),
            readout: readout::ReadoutState::default(),
            ocr: screen_ocr::OcrState::default(),
            details_open: false,
            native_window: None,
            #[cfg(test)]
            ui_probe: ui::UiProbe::default(),
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
        let rows = records.into_iter().map(Row::recovered).collect();
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
            tab: Tab::default(),
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
            screen_feed: None,
            screen_area_requested: false,
            screen_epoch: 0,
            capture_protected: false,
            capture_busy: false,
            manual_busy: None,
            pending_ocr: None,
            live_busy: None,
            asr_busy: false,
            asr_caption_suppressed: false,
            audio: None,
            nemotron: None,
            nemotron_epoch: 0,
            nemotron_origin_ms: 0,
            nemotron_needs_origin: true,
            nemotron_suspended: false,
            asr_partial: String::new(),
            streaming_transcripts: VecDeque::new(),
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
            current_live_id: None,
            live_chat: chat::State::default(),
            window_mode: window_mode::MiniWindowState::default(),
            mini: mini::MiniState::default(),
            close_after_restore: None,
            close_sent: false,
            document: document::DocumentState::default(),
            paragraph_tools: paragraph_tools::ToolsState::default(),
            subtitle_window: subtitle_window::State::default(),
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
            listen_input: ui::LiveSourceMode::default(),
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
            audio_progress: None,
            audio_cancel: Arc::new(AtomicBool::new(false)),
            readout: readout::ReadoutState::default(),
            ocr: screen_ocr::OcrState::default(),
            details_open: false,
            native_window: None,
            #[cfg(test)]
            ui_probe: ui::UiProbe::default(),
        };
        app.initialize_services();
        app.initialize_readout();
        app.initialize_screen_ocr();
        app.initialize_paragraph_tools();
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
                    standard_translation: None,
                    kind: kind.into(),
                    video_id: if kind == "YouTube" {
                        self.video_id.clone()
                    } else {
                        String::new()
                    },
                    created_at: chrono::Local::now().to_rfc3339(),
                    paragraph: None,
                },
                pending: true,
                error: None,
                generation,
            });
        }
        if lane == Lane::Manual {
            self.manual_busy = Some(id);
            self.output.clear();
            self.mini.clear_manual_failure();
        } else {
            self.live_busy = Some(id);
            self.current_live_id = Some(id);
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
    fn manual_output_ready(&self) -> bool {
        self.manual_busy.is_none() && !self.output.is_empty()
    }
    fn enter_mini_view(&mut self, ctx: &egui::Context) {
        if self.window_mode.is_mini() || self.close_after_restore.is_some() {
            return;
        }
        if !self.backend_ready || self.tab == Tab::Setup {
            self.status = "モデルの準備ができると、ミニ表示を使えます。".into();
            return;
        }
        match self.tab {
            Tab::Text => self.mini.activate_manual(&self.rows),
            _ => self.mini.show_live(),
        }
        let info = ctx.input(|input| input.viewport().clone());
        self.window_mode.observe_with_zoom(&info, ctx.zoom_factor());
        for command in self.window_mode.enter(&info) {
            ctx.send_viewport_cmd(command);
        }
        ctx.request_repaint();
    }
    fn restore_normal_view(&mut self, ctx: &egui::Context) {
        let was_mini = self.window_mode.is_mini();
        let info = ctx.input(|input| input.viewport().clone());
        self.window_mode.observe_with_zoom(&info, ctx.zoom_factor());
        for command in self.window_mode.leave(&info) {
            ctx.send_viewport_cmd(command);
        }
        if was_mini && self.mini.is_manual() {
            self.tab = Tab::Text;
        }
        ctx.request_repaint();
    }
    fn request_app_close(&mut self, ctx: &egui::Context) {
        if self.close_after_restore.is_some() {
            if !self.close_sent {
                ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            }
            return;
        }
        if self.window_mode.is_mini() {
            // Restore normal geometry before eframe persists window placement.
            // No extra window or hidden application survives a native close.
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.restore_normal_view(ctx);
            self.set_caption_enabled(false);
            self.audio_start_requested = false;
            self.stop_audio();
            self.stop_readout();
            self.document_shutdown();
            self.close_after_restore = Some(Instant::now());
        } else {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }
    fn update_window_mode(&mut self, ctx: &egui::Context) {
        let info = ctx.input(|input| input.viewport().clone());
        self.window_mode.observe_with_zoom(&info, ctx.zoom_factor());
        self.mini.observe_sources(
            self.caption_enabled,
            self.audio_enabled || self.audio_start_requested,
        );
        if info.close_requested() {
            if self.close_after_restore.is_some() && !self.close_sent {
                ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            } else if self.window_mode.is_mini() {
                self.request_app_close(ctx);
            }
        }
        if let Some(started) = self.close_after_restore {
            if !self.close_sent
                && (self.window_mode.normal_geometry_restored()
                    || started.elapsed() >= Duration::from_secs(1))
            {
                self.close_sent = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            } else {
                ctx.request_repaint_after(Duration::from_millis(16));
            }
        }
    }
    fn capture(&mut self, ctx: &egui::Context) {
        if self.capture_busy || self.selection.is_some() {
            return;
        }
        self.mini.clear_manual_failure();
        let Some(screen) = self.screens.get(self.screen_index).cloned() else {
            self.status = "取得できる画面がありません。".into();
            self.mini.manual_failed();
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
    fn archive_current_live(&mut self, kind: Option<LiveKind>) {
        if kind.is_none()
            || self.current_live_id.is_some_and(|id| {
                self.rows
                    .iter()
                    .find(|r| r.record.id == id)
                    .is_some_and(|r| LiveKind::from_record(&r.record.kind) == kind)
            })
        {
            self.current_live_id = None;
        }
    }
    fn archive_completed_audio_for_draft(&mut self) {
        if !self.asr_partial.is_empty()
            && self.current_live_id.is_some_and(|id| {
                self.rows
                    .iter()
                    .any(|r| r.record.id == id && r.record.kind == "音声" && r.is_confirmed())
            })
        {
            self.current_live_id = None;
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
        self.document_close(plamo_translate::live_text::SourceKind::Audio);
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
        self.clear_streaming_audio();
        self.clear_overlay(Some(LiveKind::Audio));
        self.archive_current_live(Some(LiveKind::Audio));
    }
    fn set_caption_enabled(&mut self, enabled: bool) {
        if self.caption_enabled == enabled {
            return;
        }
        if !enabled {
            self.screen_feed = None;
            self.document_close(plamo_translate::live_text::SourceKind::Caption);
        }
        self.caption_enabled = enabled;
        self.caption_generation += 1;
        self.caption_paused = true;
        self.document.caption_media_paused = true;
        self.last_caption = None;
        self.pending_caption = None;
        self.last_live_text.clear();
        self.last_live_id = None;
        self.clear_overlay(Some(LiveKind::Caption));
        self.archive_current_live(Some(LiveKind::Caption));
    }
    fn accept_caption(&mut self, caption: bridge::Caption) {
        if !self.caption_enabled {
            return;
        }
        if caption.video_id != self.video_id || caption.epoch != self.caption_epoch {
            self.caption_generation += 1;
            self.clear_overlay(Some(LiveKind::Caption));
            self.archive_current_live(Some(LiveKind::Caption));
            self.pending_caption = None;
            self.last_live_id = None;
            self.last_live_text.clear();
            self.last_caption = None;
            self.video_id = caption.video_id.clone();
            self.caption_epoch = caption.epoch;
            self.document_source_changed(plamo_translate::live_text::SourceKind::Caption);
        }
        self.caption_paused = caption.paused || caption.text.trim().is_empty();
        self.document.caption_media_paused = caption.paused;
        if self.caption_paused {
            // Pausing a player does not complete an unfinished sentence. Keep
            // the paragraph for resume; explicit source OFF still flushes it.
            self.pending_caption = None;
            self.clear_overlay(Some(LiveKind::Caption));
        } else {
            self.document_close(plamo_translate::live_text::SourceKind::Audio);
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
        // The chat does not show the reading, so the window opens the screen
        // that does. The mini window shows it itself and picks the screen on
        // the way back.
        if !self.window_mode.is_mini() {
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
            self.streaming_transcripts.clear();
            return None;
        }
        if !self.audio_enabled {
            return None;
        }
        while self
            .streaming_transcripts
            .front()
            .is_some_and(|p| p.generation != self.audio_generation)
        {
            self.streaming_transcripts.pop_front();
        }
        if self
            .pending_transcript
            .as_ref()
            .is_some_and(|p| p.generation != self.audio_generation)
        {
            self.pending_transcript = None;
        }
        // Accepted words are retained while translation runs. Only interim
        // translations coalesce; recognized speech is not aged out of the prose.
        self.streaming_transcripts
            .pop_front()
            .or_else(|| self.pending_transcript.take())
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
        self.document_reset();
        self.audio_generation += 1;
        self.caption_generation += 1;
        self.pending_audio = None;
        self.pending_transcript = None;
        self.clear_streaming_audio();
        self.pending_caption = None;
        self.last_live_id = None;
        self.last_live_text.clear();
        self.last_caption = None;
        self.caption_paused = true;
        self.clear_overlay(None);
        self.archive_current_live(None);
        self.live_chat.clear();
        // A new timeline clears only the subtitle content, preserving the
        // independent window and its presentation preferences.
        let kind = self
            .subtitle_window
            .scope()
            .map(|stamp| stamp.kind)
            .unwrap_or(plamo_translate::live_text::SourceKind::Caption);
        let stamp = self.document_stamp(kind);
        self.subtitle_window
            .update(Some(&stamp), None, subtitle_window::Playback::Stopped);
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
        if self.settings.asr_backend == "nemotron" {
            self.start_streaming_audio();
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
            rx: AudioReceiver::Batch(rx),
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
                Event::ParagraphTools(event) => self.accept_paragraph_tools(event),
                Event::DocumentPunctuated {
                    row_id,
                    request,
                    options,
                    result,
                } => {
                    self.document_punctuated(row_id, request, options, result);
                }
                Event::DocumentDone {
                    row_id,
                    request,
                    options,
                    result,
                    elapsed,
                } => {
                    self.document_done(row_id, request, options, result, elapsed);
                }
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
                    let mut manual_completed = false;
                    let mut manual_failed = false;
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
                                    manual_completed = true;
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
                                row.record.translation.clear();
                                if lane == Lane::Manual {
                                    self.output.clear();
                                }
                                row.error = Some(error.clone());
                                self.status = error;
                                manual_failed = lane == Lane::Manual;
                            }
                        }
                    }
                    if manual_completed {
                        self.mini.clear_manual_failure();
                        if self.window_mode.is_mini() {
                            self.mini.activate_manual(&self.rows);
                        }
                    } else if manual_failed {
                        self.mini.manual_failed();
                    }
                    if let Some(text) = read_text {
                        // read_text is populated for Live only after the
                        // existing generation/freshness gate show_overlay.
                        self.auto_read_translation(text, lane == Lane::Live);
                    }
                    self.archive_completed_audio_for_draft();
                }
                Event::Ocr { generation, result } => {
                    if self.ocr.busy == Some(generation) {
                        self.ocr.busy = None;
                        if result.is_err() {
                            self.discard_screen_ocr_runtime();
                        }
                    }
                    if generation != self.ocr.generation {
                        continue;
                    }
                    match result {
                        Ok(text) if !text.trim().is_empty() => {
                            self.accept_ocr(text);
                        }
                        Ok(_) => {
                            self.status =
                                "文字が見つかりませんでした。範囲や原文言語を確認してください。"
                                    .into();
                            self.mini.manual_failed();
                        }
                        Err(e) => {
                            self.status = e;
                            self.mini.manual_failed();
                        }
                    }
                }
                Event::OcrSetup(event) => self.accept_ocr_setup(event),
                Event::ScreenFrame { epoch, result } => self.accept_screen_frame(epoch, result),
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
                                focus_requested: false,
                                continuous: std::mem::take(&mut self.screen_area_requested),
                            });
                        }
                        Err(e) => {
                            self.screen_area_requested = false;
                            self.status = e;
                            self.mini.manual_failed();
                        }
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
                            self.setup.gpu_polled_at = Instant::now();
                            self.refresh_store_state();
                            self.recalculate_recommendation();
                        }
                        Err(error) => self.setup.error = error,
                    }
                }
                Event::GpuMemory(gpus) => self.accept_gpu_memory(gpus),
                Event::InstallProgress(progress) => self.setup.progress = Some(progress),
                Event::Installed(result) => self.translation_installed(result),
                Event::ServiceStatus { translation, asr } => {
                    self.accept_service_status(translation, asr)
                }
                Event::AudioProgress { backend, progress } => {
                    if backend == self.settings.asr_backend {
                        self.audio_progress = Some(progress);
                    }
                }
                Event::AudioPrepared { backend, result } => self.audio_prepared(backend, result),
            }
        }
        if let Some(text) = self.take_ready_ocr() {
            self.input = text;
            self.translate(self.input.clone(), Lane::Manual, 0, 0, "画面OCR", None);
        }
        self.kick_screen_ocr();
        while let Ok(caption) = self.bridge_rx.try_recv() {
            // The 画面 input owns the caption source while it runs.
            if self.screen_feed.is_some() {
                continue;
            }
            self.accept_caption(caption);
            self.document_take_caption();
        }
        self.document_take_caption();
        let captions_fresh = self.captions_fresh();
        self.poll_nemotron();
        if captions_fresh {
            // A transcript completed before this frame's caption message must
            // also yield priority to the newly active captions.
            self.pending_transcript = None;
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
                if let AudioReceiver::Batch(rx) = &audio.rx {
                    while let Ok(value) = rx.try_recv() {
                        if !suppressed {
                            self.pending_audio = Some(value);
                        }
                    }
                }
                capture_stopped = audio.stop.load(Ordering::Relaxed);
            }
            if capture_stopped {
                self.stop_audio();
            }
            self.process_streaming_audio();
        }
        while let Some(transcript) = self.take_ready_transcript() {
            self.document_ingest_audio(transcript);
        }
        if self.audio_enabled && self.backend_ready {
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
        while let Ok(event) = global_hotkey::GlobalHotKeyEvent::receiver().try_recv() {
            if event.state == global_hotkey::HotKeyState::Pressed {
                self.press_hotkey(ctx, event.id);
            }
        }
        self.document_tick();
    }
    /// A global shortcut was pressed: Ctrl+Alt+T reads a range, and
    /// Ctrl+Alt+L is the 聞く and 止める button.
    fn press_hotkey(&mut self, ctx: &egui::Context, id: u32) {
        use global_hotkey::hotkey::{Code, HotKey, Modifiers};
        let ctrl_alt = |code| HotKey::new(Some(Modifiers::CONTROL | Modifiers::ALT), code).id();
        if id == ctrl_alt(Code::KeyT) {
            self.capture(ctx);
        }
        if id == ctrl_alt(Code::KeyL) {
            self.toggle_listening(ctx);
        }
    }
    fn open_dropped_file(&mut self, ctx: &egui::Context) {
        let files = ctx.input(|i| i.raw.dropped_files.clone());
        let Some(path) = files.first().and_then(|f| f.path.clone()) else {
            return;
        };
        match image::open(&path) {
            Ok(image) => self.do_ocr(image.to_rgba8()),
            Err(_) => match fs::read_to_string(&path) {
                Ok(text) => {
                    self.input = text;
                    self.tab = Tab::Text;
                    self.restore_normal_view(ctx);
                }
                Err(e) => self.status = e.to_string(),
            },
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
            match fs::write(path, credit::published(&self.output)) {
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
            .filter(|r| {
                r.record.paragraph.is_some()
                    || (!r.pending && r.error.is_none() && !r.record.translation.trim().is_empty())
            })
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
            _ => credit::published(&timeline_txt(&records)),
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
        if let Err(error) = file_output::check_destination(&input, &output) {
            self.status = error.to_string();
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
                        cue.text = translator.translate_live(&cue.text, &cancel, |_| {})?;
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
                            result.push_str(&translator.translate_live(part, &cancel, |_| {})?);
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
                // 字幕は形式を崩すため、クレジットは本文のファイルだけに付ける
                let content = if ext == "srt" || ext == "vtt" {
                    content
                } else {
                    credit::published(&content)
                };
                file_output::write_translation(&input, &output, &content, &cancel)?;
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
        let scale = screen.scale * ctx.zoom_factor();
        let builder = egui::ViewportBuilder::default()
            .with_title("翻訳範囲を選択 · Escでキャンセル")
            .with_decorations(false)
            .with_position([screen.x as f32 / scale, screen.y as f32 / scale])
            .with_inner_size([screen.width as f32 / scale, screen.height as f32 / scale])
            .with_always_on_top()
            .with_resizable(false);
        ctx.show_viewport_immediate(
            egui::ViewportId::from_hash_of("screen-selection"),
            builder,
            |ctx, _| {
                if !selection.focus_requested {
                    // Opened by Ctrl+Alt+T while another app holds the keyboard,
                    // the overlay would otherwise leave Esc going to that app.
                    selection.focus_requested = true;
                    ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
                }
                ctx.set_cursor_icon(egui::CursorIcon::Crosshair);
                ctx.send_viewport_cmd(egui::ViewportCommand::CursorVisible(true));
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
                        let interaction = ui
                            .interact(rect, ui.id().with("drag-select"), egui::Sense::drag())
                            .on_hover_and_drag_cursor(egui::CursorIcon::Crosshair);
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
                            egui::FontId::proportional(theme::SIZE_TITLE),
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
                if w >= 5 && h >= 5 && selection.continuous {
                    self.start_screen_feed(selection.screen.id, (x, y, w, h));
                } else if w >= 5 && h >= 5 {
                    self.do_ocr(image::imageops::crop_imm(&selection.image, x, y, w, h).to_image());
                } else {
                    self.status = "もう少し広い範囲を選択してください。".into();
                }
            }
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
        self.update_window_mode(ctx);
        self.process(ctx);
        self.run_chain();
        self.tick_screen_feed(ctx);
        self.sync_capture_protection(ctx);
        self.poll_services();
        self.poll_readout();
        self.poll_gpu_memory(ctx);
        if self.window_mode.is_mini()
            && (self.tab == Tab::Setup
                || self.audio_confirmation
                || self.audio_installing
                || self.readout.confirm
                || self.readout.preparing
                || self.screen_ocr_needs_normal_view())
        {
            self.restore_normal_view(ctx);
        }
        self.render_ui(ctx);
        self.run_chain();
        self.audio_setup_ui(ctx);
        self.readout_setup_ui(ctx);
        self.screen_ocr_setup_ui(ctx);
        self.document_update_subtitle_window();
        self.sync_subtitle_moments();
        self.subtitle_window.show(ctx);
        self.selection_ui(ctx);
        if ctx.input(|i| i.modifiers.ctrl && i.key_pressed(egui::Key::Q)) {
            self.request_app_close(ctx);
        }
        if ctx.input(|i| !i.raw.dropped_files.is_empty()) {
            self.open_dropped_file(ctx);
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
        self.document_shutdown();
        self.paragraph_tools.cancel.store(true, Ordering::Release);
        if let Some(worker) = self.paragraph_tools.worker.take() {
            let _ = worker.join();
        }
        self.stop.store(true, Ordering::Relaxed);
        self.setup.cancel.store(true, Ordering::Relaxed);
        self.audio_cancel.store(true, Ordering::Relaxed);
        self.readout.cancel.store(true, Ordering::Relaxed);
        self.readout.voice_cancel.store(true, Ordering::Relaxed);
        if let Some(worker) = self.readout.voice_worker.take() {
            let _ = worker.join();
        }
        self.ocr.cancel.store(true, Ordering::Relaxed);
        if let Some(worker) = self.ocr.setup_worker.take() {
            let _ = worker.join();
        }
        if let Some(control) = &self.ocr.control {
            control.stop();
        }
        self.file_cancel.store(true, Ordering::Relaxed);
        if let Some(audio) = &self.audio {
            audio.stop.store(true, Ordering::Relaxed);
        }
        self.stop_nemotron();
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
fn timeline_txt(records: &[Record]) -> String {
    use plamo_translate::journal::ParagraphState;
    records
        .iter()
        .map(|r| {
            let state = match r.paragraph.as_ref().map(|p| p.state) {
                Some(ParagraphState::Open) => "継続中 · 訳文は未確定",
                Some(ParagraphState::Closing) => "文章を確定中 · 訳文は未確定",
                Some(ParagraphState::Interrupted) => "中断 · 訳文は未確定",
                Some(ParagraphState::Failed) => "翻訳できなかった文章 · 訳文は未確定",
                Some(ParagraphState::Final) | None => "確定訳",
            };
            format!(
                "[{} · {} · {}] {}\n{}\n{}\n",
                clock(r.start_ms),
                r.kind,
                state,
                r.created_at,
                r.source,
                r.translation
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}
fn timeline_srt(records: &[Record]) -> Result<String> {
    let timed: Vec<_> = records
        .iter()
        .filter(|r| {
            r.end_ms > r.start_ms
                && !r.translation.trim().is_empty()
                && r.paragraph
                    .as_ref()
                    .is_none_or(|p| p.state == plamo_translate::journal::ParagraphState::Final)
        })
        .collect();
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

/// One saved row of a preview talk, said at `clock` on 7 November.
fn demo_row(
    id: u64,
    (kind, clock, start_ms, source, translation): (&str, &str, u64, &str, &str),
    state: plamo_translate::journal::ParagraphState,
) -> Row {
    use plamo_translate::journal::ParagraphMeta;
    Row {
        record: Record {
            id,
            start_ms,
            end_ms: start_ms + 4_000,
            source: source.into(),
            translation: translation.into(),
            standard_translation: None,
            kind: kind.into(),
            video_id: if kind == "YouTube" {
                "preview".into()
            } else {
                String::new()
            },
            created_at: format!("2026-11-07T{clock}+09:00"),
            paragraph: Some(ParagraphMeta {
                revision: 3,
                state,
                close_reason: None,
            }),
        },
        pending: false,
        error: None,
        generation: 0,
    }
}

/// A short talk for the live chat previews: a YouTube video with a change of
/// speaker, then PC audio, ending in a paragraph that is still forming. The
/// last row is the forming one.
fn demo_talk(rows: usize) -> Vec<Row> {
    use plamo_translate::journal::ParagraphState;
    const FIXED: [(&str, &str, u64, &str, &str); 9] = [
        (
            "YouTube",
            "21:46:10",
            0,
            "Welcome back. Today we're going to build a small application.",
            "おかえりなさい。今日は小さなアプリを作ります。",
        ),
        (
            "YouTube",
            "21:46:38",
            8_000,
            ">> You can translate text on your screen while listening to a video. >> That is really useful for me.",
            ">> 動画を聴きながら、画面に表示された文章も翻訳できます。>> それは私にはとても助かります。",
        ),
        (
            "YouTube",
            "21:47:02",
            754_000,
            "Your translations are saved, so you can come back to them later.",
            "翻訳は保存されるので、あとから読み返せます。",
        ),
        (
            "音声",
            "21:48:20",
            0,
            "Each description has about 150 words in English and a Japanese translation. A shared glossary keeps the names of local places consistent.",
            "各説明文は英語で約150語で構成され、日本語訳が添えられています。共通の用語集で地域の地名を統一します。",
        ),
        (
            "音声",
            "21:48:52",
            0,
            "When a direct translation is unclear, staff will explain the meaning in ordinary language.",
            "直訳では分かりにくい場合は、スタッフが平易な言葉で意味を説明します。",
        ),
        (
            "音声",
            "21:49:10",
            0,
            "No name or contact address is required.",
            "お名前や連絡先は必要ありません。",
        ),
        (
            "音声",
            "21:49:30",
            0,
            "Each Friday staff will review the comments.",
            "毎週金曜日にスタッフがご意見を確認します。",
        ),
        (
            "音声",
            "21:49:55",
            0,
            "The trial will last six weeks.",
            "試行期間は6週間です。",
        ),
        (
            "音声",
            "21:50:20",
            0,
            "Please ask the staff if you have any questions.",
            "ご質問があれば、スタッフにお声がけください。",
        ),
    ];
    let Some(fixed) = rows.checked_sub(1) else {
        return Vec::new();
    };
    let mut talk: Vec<Row> = FIXED
        .iter()
        .take(fixed)
        .zip(1..)
        .map(|(entry, id)| demo_row(id, *entry, ParagraphState::Final))
        .collect();
    talk.push(demo_row(
        rows as u64,
        (
            "音声",
            "21:50:40",
            0,
            "Feedback cards ask three questions about memorable details, confusing passages and possible next steps.",
            "フィードバックカードでは、印象に残った内容、分かりにくい箇所、",
        ),
        ParagraphState::Open,
    ));
    talk
}

/// The part still forming in the subtitle window previews, with its original:
/// long, two lines at the smallest window, and one line at the largest text.
const FORMING_LONG: (&str, &str) = (
    "フィードバックカードでは、印象に残った内容、分かりにくい箇所、",
    "Feedback cards ask three questions about memorable details, confusing passages",
);
const FORMING_SHORT: (&str, &str) = ("お名前は必要ありません。", "No name is required.");
const FORMING_TINY: (&str, &str) = ("はい。", "Yes.");

/// A finished paragraph of seven sentences: its translation, then its original.
const LONG_PARAGRAPH: (&str, &str) = (
    "とてもすごいですね。皆さん、こんにちは。今日のセッションへようこそ。今日は、自分のコンピューターでローカル翻訳がどのように動くのかについて話します。仕組みは簡単です。クラウドには何も送られません。すべて自分のコンピューターの中に残ります。",
    "That is very impressive. Hello, everyone. Welcome to today's session. Today I will talk about how local translation works on your own computer. It is simple. Nothing is sent to the cloud. Everything stays on your own computer.",
);

/// A short talk for the chat previews that ends in `LONG_PARAGRAPH`.
fn demo_long_talk() -> Vec<Row> {
    use plamo_translate::journal::ParagraphState;
    vec![
        demo_row(
            1,
            (
                "音声",
                "21:49:30",
                0,
                "Each Friday staff will review the comments.",
                "毎週金曜日にスタッフがご意見を確認します。",
            ),
            ParagraphState::Final,
        ),
        demo_row(
            2,
            ("音声", "21:50:20", 0, LONG_PARAGRAPH.1, LONG_PARAGRAPH.0),
            ParagraphState::Final,
        ),
    ]
}

/// A subtitle window holding `fixed` parts and one still forming, filled the
/// way the app fills it. A YouTube window gets a change of speaker and video
/// times.
fn demo_subtitle(youtube: bool, fixed: usize, forming: (&str, &str)) -> subtitle_window::State {
    use plamo_translate::live_text::{SourceKind, SourceStamp};
    use subtitle_window::{Phase, Playback, View};
    const PARTS: [(&str, &str); 7] = [
        (
            "来館者はいつでも展示リストに戻れます。",
            "Visitors can return to the exhibit list whenever they wish.",
        ),
        (
            "画面のタップ数を数えるだけでなく、同じ資料をもう一度見つけられるかどうかを確認します。",
            "We will observe whether they can find an item again rather than simply count screen taps.",
        ),
        (
            "共通の用語集で地域の地名を統一します。",
            "A shared glossary keeps the names of local places consistent.",
        ),
        (
            "直訳では分かりにくい場合は、スタッフが平易な言葉で意味を説明します。",
            "When a direct translation is unclear, staff will explain the meaning in ordinary language.",
        ),
        ("試行期間は6週間です。", "The trial will last six weeks."),
        (
            "お名前や連絡先は必要ありません。",
            "No name or contact address is required.",
        ),
        LONG_PARAGRAPH,
    ];
    let stamp = SourceStamp {
        kind: if youtube {
            SourceKind::Caption
        } else {
            SourceKind::Audio
        },
        generation: 1,
        video_id: if youtube {
            "preview".into()
        } else {
            String::new()
        },
        epoch: 0,
    };
    let source = if youtube {
        chat::Source::YouTube
    } else {
        chat::Source::PcAudio
    };
    let view = |key: u64, target: &str, original: &str, phase| View {
        stamp: stamp.clone(),
        paragraph_key: key,
        revision: 1,
        source: original.into(),
        target: target.into(),
        phase,
    };
    let mut history: Vec<View> = PARTS
        .iter()
        .take(fixed)
        .zip(1..)
        .map(|((target, original), key)| view(key, target, original, Phase::Final))
        .collect();
    if youtube && let Some(second) = history.get_mut(1) {
        second.target = "戻ったあとも、続きから読めますか。>> はい、続きから読めます。".into();
        second.source = "Can I keep reading afterwards? >> Yes, you can.".into();
    }
    let forming = view(fixed as u64 + 1, forming.0, forming.1, Phase::Updating);
    let mut state = subtitle_window::State::default();
    state.update(Some(&stamp), Some(&forming), Playback::Running);
    state.set_history(history);
    state.set_moments(
        (1..=fixed as u64 + 1)
            .map(|key| {
                // The part still forming comes minutes after the fixed ones.
                let clock = if key > fixed as u64 {
                    "21:49:52".to_owned()
                } else {
                    format!("21:46:{:02}", 10 + key * 12)
                };
                (
                    key,
                    chat::Moment::from_record(
                        &format!("2026-11-07T{clock}+09:00"),
                        754_000 + key * 8_000,
                        source,
                    ),
                )
            })
            .collect(),
    );
    state
}

/// The model setup screen on several PCs, at the smallest window and at 4K,
/// then the 「はじめる」 run on one of them: step 2 of 3, a failure at step 3, and
/// the live screen it opens. Memory figures and what is saved are fixtures:
/// previews never measure the machine or read its folders.
fn render_setup_views(folder: &Path) -> Result<()> {
    use plamo_translate::{
        hardware::{Gpu, Hardware},
        model_catalog,
    };
    const GIB: u64 = 1_073_741_824;
    let pc = |gpu: Option<(&str, u64, u64)>| Hardware {
        os_arch: "windows-x86_64".into(),
        cpu_logical: 16,
        ram_total_bytes: Some(32 * GIB),
        ram_available_bytes: Some(20 * GIB),
        disk_free_bytes: Some(120 * GIB),
        disk_target: "C:/".into(),
        gpus: gpu
            .into_iter()
            .map(|(name, total, used)| Gpu {
                name: name.into(),
                vendor: "nvidia".into(),
                dedicated_vram_bytes: Some(total),
                free_vram_bytes: Some(total - used),
                driver_version: Some("570.00".into()),
                cuda_eligible: true,
                vulkan_eligible: true,
                is_software: false,
                memory_telemetry: "nvidia-smi".into(),
            })
            .collect(),
        warnings: vec![],
    };
    let catalog = model_catalog::bundled()?;
    let model = |id: &str| catalog.models.iter().find(|m| m.id == id).cloned().unwrap();
    let loaded = model("plamo3-8b-q3_k_m");
    let (big, small) = ("NVIDIA GeForce RTX 4090", "NVIDIA GeForce RTX 4060");
    let nothing_saved = tempfile::tempdir()?;
    let fixture = |hardware: &Hardware, in_use: Option<&model_catalog::Model>, selected| {
        let mut app = App::inert();
        app.tab = Tab::Setup;
        app.startup_decided = true;
        app.status = "プレビュー · 接続や保存は行いません".into();
        app.setup.hardware = Some(hardware.clone());
        if let Some(model) = in_use {
            app.settings.model = format!("models/{}", model.filename);
            app.settings.gpu_layers = 99;
            app.backend_ready = true;
            app.setup.in_use = Some(model.clone());
            app.setup.saved = ["plamo3-0.8b-q4_k_m", "plamo3-2b-q4_k_m", &model.id]
                .map(String::from)
                .into();
        }
        app.recalculate_recommendation();
        if let Some(id) = selected {
            app.setup.selected_id = id;
        }
        app.seed_plan(&setup_chain::Roots::under(nothing_saved.path()));
        if in_use.is_some()
            && let Some((_, plan)) = &mut app.setup.plan
        {
            plan.steps[0].1 = 0;
        }
        app
    };
    for (name, hardware, in_use, selected) in [
        (
            "setup-a-24gb",
            pc(Some((big, 24 * GIB, 5 * GIB))),
            None,
            None,
        ),
        (
            "setup-b-8gb-busy",
            pc(Some((small, 8 * GIB, 13 * GIB / 2))),
            None,
            None,
        ),
        (
            "setup-b2-8gb",
            pc(Some((small, 8 * GIB, 3 * GIB))),
            None,
            None,
        ),
        ("setup-c-cpu", pc(None), None, None),
        (
            "setup-d-loaded",
            pc(Some((
                big,
                24 * GIB,
                5 * GIB + model_catalog::inference_bytes(&loaded),
            ))),
            Some(&loaded),
            None,
        ),
        (
            "setup-e-no-longer-fits",
            pc(Some((small, 8 * GIB, 3 * GIB))),
            None,
            Some("plamo3-8b-q4_k_m"),
        ),
    ] {
        for (width, height) in [(860, 620), (2560, 1369)] {
            let ctx = egui::Context::default();
            ui::configure_theme(&ctx);
            let mut app = fixture(&hardware, in_use, selected.map(String::from));
            preview::render(
                &ctx,
                &folder.join(format!("{name}-{width}x{height}.png")),
                width,
                height,
                |ctx| app.render_ui(ctx),
            )?;
        }
    }
    let roomy = pc(Some((big, 24 * GIB, 5 * GIB)));
    // The chain after `done` steps, with the next one started.
    fn chain_after(app: &mut App, done: usize) -> setup_chain::Chain {
        let mut chain = setup_chain::Chain::new(app.download_plan().unwrap().steps());
        for _ in 0..done {
            let step = chain.take_next().unwrap();
            chain.complete(step);
        }
        chain.take_next();
        chain
    }
    type Change = fn(&mut App);
    let runs: [(&str, Change); 3] = [
        ("setup-f-progress", |app| {
            app.setup.chain = Some(chain_after(app, 1));
            app.backend_loading = true;
            app.paragraph_tools.progress = Some(plamo_translate::installer::Progress {
                stage: "punctuation_download".into(),
                downloaded_bytes: 41_000_000,
                total_bytes: Some(64_717_756),
                message: "sherpa-onnx-punct-ct-transformer-zh-en-vocab272727-2024-04-12-int8.tar.bz2を取得中".into(),
            });
        }),
        ("setup-g-failed", |app| {
            let mut chain = chain_after(app, 2);
            chain.fail(
                setup_chain::Step::Speech,
                "nemotron-speech-streaming-en-0.6b.q8_0.ggufの取得に失敗しました（HTTP 503 Service Unavailable）。".into(),
            );
            app.setup.chain = Some(chain);
            app.backend_ready = true;
        }),
        ("setup-h-finished", |app| {
            app.tab = Tab::Live;
            app.backend_ready = true;
            app.asr_ready = true;
            app.status = setup_chain::READY_STATUS.into();
        }),
    ];
    for (name, change) in runs {
        for (width, height) in [(860, 620), (2560, 1369)] {
            let ctx = egui::Context::default();
            ui::configure_theme(&ctx);
            let mut app = fixture(&roomy, None, None);
            change(&mut app);
            preview::render(
                &ctx,
                &folder.join(format!("{name}-{width}x{height}.png")),
                width,
                height,
                |ctx| app.render_ui(ctx),
            )?;
        }
    }
    Ok(())
}

/// What a preview of the main window shows over its screen, or changes on it.
enum Overlay {
    Nothing,
    Menu,
    /// The menu, with the submenu of the row 入力の切り替え open.
    InputMenu,
    Details,
    NoModel,
    Readout,
    Error,
}

/// What the live inputs are doing in a preview.
#[derive(Clone, Copy)]
enum Listening {
    Stopped,
    PcAudio,
    Captions,
    Both,
}

fn render_demo_views(folder: &Path) -> Result<()> {
    fs::create_dir_all(folder)?;
    render_long_paragraph_views(folder)?;
    render_setup_views(folder)?;
    let nothing_saved = tempfile::tempdir()?;
    for (viewport_width, viewport_height) in [(1280, 880), (860, 620), (2560, 1369)] {
        for (name, tab, filled, overlay, listening) in [
            (
                "01-text-empty",
                Tab::Text,
                false,
                Overlay::Nothing,
                Listening::Stopped,
            ),
            (
                "02-text-filled",
                Tab::Text,
                true,
                Overlay::Nothing,
                Listening::Stopped,
            ),
            (
                "03-live",
                Tab::Live,
                true,
                Overlay::Nothing,
                Listening::Both,
            ),
            (
                "04-settings",
                Tab::Settings,
                false,
                Overlay::Nothing,
                Listening::Stopped,
            ),
            (
                "05-first-run",
                Tab::Setup,
                false,
                Overlay::Nothing,
                Listening::Stopped,
            ),
            (
                "18-history",
                Tab::Files,
                true,
                Overlay::Nothing,
                Listening::Both,
            ),
            ("27-menu", Tab::Live, true, Overlay::Menu, Listening::Both),
            (
                "28-details",
                Tab::Live,
                true,
                Overlay::Details,
                Listening::Both,
            ),
            (
                "29-no-model",
                Tab::Live,
                true,
                Overlay::NoModel,
                Listening::Stopped,
            ),
            (
                "30-live-stopped",
                Tab::Live,
                true,
                Overlay::Nothing,
                Listening::Stopped,
            ),
            (
                "31-live-pc-audio",
                Tab::Live,
                true,
                Overlay::Nothing,
                Listening::PcAudio,
            ),
            (
                "32-live-captions",
                Tab::Live,
                true,
                Overlay::Nothing,
                Listening::Captions,
            ),
            (
                "33-input-menu",
                Tab::Live,
                true,
                Overlay::InputMenu,
                Listening::PcAudio,
            ),
            (
                "34-live-readout",
                Tab::Live,
                true,
                Overlay::Readout,
                Listening::PcAudio,
            ),
            (
                "35-live-error",
                Tab::Live,
                true,
                Overlay::Error,
                Listening::Stopped,
            ),
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
                app.seed_plan(&setup_chain::Roots::under(nothing_saved.path()));
            }
            if filled {
                app.input = "The meeting has been rescheduled for next Monday.\nPlease open the project dashboard.".into();
                app.output =
                "会議は来週の月曜日に変更になりました。\nプロジェクトの管理画面を開いてください。"
                    .into();
            }
            match listening {
                Listening::Stopped => {}
                Listening::PcAudio => app.audio_enabled = true,
                Listening::Captions => {
                    app.caption_enabled = true;
                    app.caption_paused = false;
                    app.last_caption = Some(Instant::now());
                }
                Listening::Both => {
                    app.caption_enabled = true;
                    app.audio_enabled = true;
                }
            }
            if tab == Tab::Files {
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
                            standard_translation: None,
                            kind: "YouTube".into(),
                            video_id: "preview".into(),
                            created_at: "2026-11-05T12:00:00+09:00".into(),
                            paragraph: None,
                        },
                        pending: false,
                        error: None,
                        generation: 0,
                    });
                }
                app.current_live_id = Some(3);
            }
            if tab == Tab::Live {
                app.rows = demo_talk(5);
                app.current_live_id = Some(5);
                app.live_chat.toggle_original(3, 0);
            }
            let mut pointer = None;
            match overlay {
                Overlay::Nothing => {}
                Overlay::Menu => egui::Popup::open_id(&ctx, egui::Id::new(ui::MORE_MENU)),
                Overlay::InputMenu => {
                    let open_menu = |ctx: &egui::Context| {
                        egui::Popup::open_id(ctx, egui::Id::new(ui::MORE_MENU))
                    };
                    // A context of its own finds the row, so that the pointer can
                    // rest on it and open the submenu. Without the row, the menu
                    // is drawn as it is.
                    let probe = egui::Context::default();
                    ui::configure_theme(&probe);
                    open_menu(&probe);
                    pointer = preview::locate(
                        &probe,
                        viewport_width,
                        viewport_height,
                        "入力の切り替え",
                        |ctx| app.render_ui(ctx),
                    );
                    open_menu(&ctx);
                }
                Overlay::Details => {
                    // The window fades in; the picture shows it after it has.
                    ctx.style_mut(|style| style.animation_time = 0.0);
                    app.settings.readout_enabled = true;
                    app.details_open = true;
                }
                Overlay::NoModel => app.backend_ready = false,
                Overlay::Readout => {
                    app.settings.readout_enabled = true;
                    app.settings.auto_read = true;
                }
                Overlay::Error => {
                    app.service_error = "音声認識を読み込めませんでした。設定で音声認識の方式を確認し、準備し直してください。".into();
                }
            }
            preview::render_hovering(
                &ctx,
                &folder.join(if viewport_width == 1280 {
                    format!("{name}.png")
                } else {
                    format!("{name}-{viewport_width}x{viewport_height}.png")
                }),
                viewport_width,
                viewport_height,
                pointer,
                |ctx| app.render_ui(ctx),
            )?;
        }
    }
    let mini_default = window_mode::MiniWindowState::default().mini_size();
    for (name, size, draft, manual, ready) in [
        ("06-mini-live", mini_default, false, false, true),
        ("07-mini-draft", mini_default, true, false, true),
        ("08-mini-screen", mini_default, false, true, true),
        (
            "09-mini-minimum",
            Vec2::new(400.0, 260.0),
            true,
            false,
            true,
        ),
        ("10-mini-service-error", mini_default, false, false, false),
        ("11-mini-translating", mini_default, false, false, true),
    ] {
        let ctx = egui::Context::default();
        ui::configure_theme(&ctx);
        let mut app = App::inert();
        app.backend_ready = ready;
        app.caption_enabled = true;
        app.audio_enabled = true;
        app.tab = Tab::Live;
        app.startup_decided = true;
        app.window_mode.enter(&egui::ViewportInfo::default());
        app.rows.push(Row {
            record: Record {
                id: 1,
                start_ms: 12500,
                end_ms: 16000,
                source: "The translations stay on your computer, so you can read them again later.".into(),
                translation: "翻訳はこのPCに保存されるので、あとから読み返せます。画面を小さくしても、翻訳と保存は続きます。".into(),
                standard_translation: None,
                kind: "YouTube".into(),
                video_id: "preview".into(),
                created_at: "2026-11-05T12:00:00+09:00".into(),
                paragraph: None,
            },
            pending: false,
            error: None,
            generation: 0,
        });
        app.current_live_id = Some(1);
        if name == "11-mini-translating" {
            app.rows.push(Row {
                record: Record {
                    id: 2,
                    start_ms: 16500,
                    end_ms: 20000,
                    source: "The next sentence is still being translated.".into(),
                    translation: "次の文は、まだ翻訳の".into(),
                    standard_translation: None,
                    kind: "音声".into(),
                    video_id: String::new(),
                    created_at: String::new(),
                    paragraph: None,
                },
                pending: true,
                error: None,
                generation: 0,
            });
            app.current_live_id = Some(2);
            app.live_busy = Some(2);
        }
        if draft {
            app.asr_partial =
                "The next sentence is being recognized while you read the previous translation."
                    .into();
        }
        if manual {
            app.mini.activate_manual(&app.rows);
            app.input =
                "The dashboard shows the schedule. Pause the video, then play it again.".into();
            app.output = "管理画面にスケジュールが表示されます。動画を一時停止してから、再度再生してください。".into();
        }
        if !ready {
            app.service_error = "翻訳モデルへの接続が切れました。".into();
        }
        preview::render(
            &ctx,
            &folder.join(format!("{name}.png")),
            size.x as u32,
            size.y as u32,
            |ctx| app.render_ui(ctx),
        )?;
    }
    for (name, size, interrupted) in [
        ("12-continuous-live", Vec2::new(1280.0, 880.0), false),
        ("13-continuous-mini", mini_default, false),
        ("14-continuous-mini-minimum", Vec2::new(400.0, 260.0), false),
        ("15-continuous-interrupted", Vec2::new(1280.0, 880.0), true),
    ] {
        use plamo_translate::journal::{ParagraphMeta, ParagraphState};
        let ctx = egui::Context::default();
        ui::configure_theme(&ctx);
        let mut app = App::inert();
        app.backend_ready = true;
        app.audio_enabled = !interrupted;
        app.asr_ready = true;
        app.tab = Tab::Live;
        app.startup_decided = true;
        app.settings.show_original = false;
        app.status = "プレビュー · 公開用の架空例文です".into();
        for (id, source, target, state, start, end) in [
            (
                1,
                "Each description has about 150 words in English and a Japanese translation A shared glossary keeps the names of local places consistent When a direct translation is unclear staff will explain the meaning in ordinary language The trial will last six weeks",
                "各説明文は英語で約150語で構成され、日本語訳が添えられています。共通の用語集で地域の地名を統一します。直訳では分かりにくい場合は、スタッフが平易な言葉で意味を説明します。試行期間は6週間です。",
                ParagraphState::Final,
                12_500,
                28_000,
            ),
            (
                2,
                "Visitors can return to the exhibit list whenever they wish We will observe whether they can find an item again rather than simply count screen taps Feedback cards ask three questions about memorable details confusing passages and possible next steps",
                "来館者はいつでも展示リストに戻れます。画面のタップ数を数えるだけでなく、同じ資料をもう一度見つけられるかどうかを確認します。フィードバックカードでは、印象に残った内容、分かりにくい箇所、今後の改善について3つの質問をします。",
                if interrupted {
                    ParagraphState::Interrupted
                } else {
                    ParagraphState::Open
                },
                28_000,
                40_000,
            ),
        ] {
            app.rows.push(Row {
                record: Record {
                    id,
                    start_ms: start,
                    end_ms: end,
                    source: source.into(),
                    translation: target.into(),
                    standard_translation: None,
                    kind: "音声".into(),
                    video_id: String::new(),
                    created_at: String::new(),
                    paragraph: Some(ParagraphMeta {
                        revision: 7,
                        state,
                        close_reason: None,
                    }),
                },
                pending: false,
                error: None,
                generation: 0,
            });
        }
        app.current_live_id = if interrupted { None } else { Some(2) };
        if !interrupted {
            app.asr_partial =
                "No name or contact address is required Each Friday staff will review the comments"
                    .into();
        }
        if size.x < 860.0 {
            app.window_mode.enter(&egui::ViewportInfo::default());
        }
        preview::render(
            &ctx,
            &folder.join(format!("{name}.png")),
            size.x as u32,
            size.y as u32,
            |ctx| app.render_ui(ctx),
        )?;
    }
    // The live chat in its other states.
    for (name, rows, font_size) in [
        ("20-live-new-messages", 10, theme::READING_DEFAULT),
        ("21-live-empty", 0, theme::READING_DEFAULT),
        ("22-live-large-text", 5, theme::READING_MAX),
    ] {
        let ctx = egui::Context::default();
        ui::configure_theme(&ctx);
        let mut app = App::inert();
        app.tab = Tab::Live;
        app.backend_ready = true;
        app.asr_ready = true;
        app.startup_decided = true;
        app.caption_enabled = true;
        app.audio_enabled = true;
        app.status = "プレビュー · 接続や保存は行いません".into();
        app.settings.font_size = font_size;
        app.rows = demo_talk(rows);
        app.current_live_id = app.rows.last().map(|row| row.record.id);
        if name == "20-live-new-messages" {
            app.live_chat.reading_from_start(7);
        }
        preview::render(
            &ctx,
            &folder.join(format!("{name}-860x620.png")),
            860,
            620,
            |ctx| app.render_ui(ctx),
        )?;
    }
    // Independent subtitle window, drawn with its real paint path.
    for (name, size, mut state) in [
        (
            "16-subtitle-updating",
            [720, 360],
            demo_subtitle(false, 3, FORMING_LONG),
        ),
        (
            "17-subtitle-source",
            [720, 360],
            demo_subtitle(false, 2, FORMING_LONG),
        ),
        (
            "19-subtitle-minimum",
            [360, 220],
            demo_subtitle(false, 2, FORMING_SHORT),
        ),
        (
            "23-subtitle-new-messages",
            [720, 360],
            demo_subtitle(false, 6, FORMING_LONG),
        ),
        (
            "24-subtitle-youtube",
            [720, 600],
            demo_subtitle(true, 2, FORMING_LONG),
        ),
        (
            "25-subtitle-empty",
            [720, 360],
            subtitle_window::State::default(),
        ),
        (
            "26-subtitle-large-text",
            [360, 220],
            demo_subtitle(false, 1, FORMING_TINY),
        ),
    ] {
        state.open = true;
        match name {
            "17-subtitle-source" => state.show_source = true,
            "23-subtitle-new-messages" => state.chat.reading_from_start(4),
            "24-subtitle-youtube" => state.chat.toggle_original(2, 1),
            "26-subtitle-large-text" => state.font_size = theme::SUBTITLE_READING_MAX,
            _ => {}
        }
        let ctx = egui::Context::default();
        ui::configure_theme(&ctx);
        preview::render(
            &ctx,
            &folder.join(format!("{name}.png")),
            size[0],
            size[1],
            |ctx| {
                subtitle_window::State::panel().show(ctx, |ui| {
                    state.paint(ui);
                });
            },
        )?;
    }
    Ok(())
}

/// `LONG_PARAGRAPH` in the chat and in the subtitle window, as it is shown
/// and with its original opened, at the smallest window and at 4K.
fn render_long_paragraph_views(folder: &Path) -> Result<()> {
    for (width, height) in [(860, 620), (2560, 1369)] {
        for (name, original) in [("long-paragraph", false), ("long-original", true)] {
            let ctx = egui::Context::default();
            ui::configure_theme(&ctx);
            let mut app = App::inert();
            app.tab = Tab::Live;
            app.backend_ready = true;
            app.asr_ready = true;
            app.startup_decided = true;
            app.audio_enabled = true;
            app.status = "プレビュー · 接続や保存は行いません".into();
            app.rows = demo_long_talk();
            app.current_live_id = app.rows.last().map(|row| row.record.id);
            if original {
                app.live_chat.toggle_original(2, 0);
            }
            preview::render(
                &ctx,
                &folder.join(format!("36-live-{name}-{width}x{height}.png")),
                width,
                height,
                |ctx| app.render_ui(ctx),
            )?;

            let ctx = egui::Context::default();
            ui::configure_theme(&ctx);
            let mut window = demo_subtitle(false, 7, FORMING_SHORT);
            window.open = true;
            if original {
                window.chat.toggle_original(7, 0);
            }
            preview::render(
                &ctx,
                &folder.join(format!("37-subtitle-{name}-{width}x{height}.png")),
                width,
                height,
                |ctx| {
                    subtitle_window::State::panel().show(ctx, |ui| {
                        window.paint(ui);
                    });
                },
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
        persistence_path: Some(data_root().join("window-state.ron")),
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
            standard_translation: None,
            created_at: String::new(),
            paragraph: None,
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
    fn regression_failed_manual_stream_discards_the_unfinished_translation() {
        let mut a = app();
        let ctx = egui::Context::default();
        let id = 42;
        a.manual_busy = Some(id);
        a.rows.push(Row {
            record: record(id, "文章", "", 0, 0),
            pending: true,
            error: None,
            generation: 0,
        });
        a.tx.send(Event::Partial {
            id,
            text: "unfinished translation".into(),
        })
        .unwrap();
        a.process(&ctx);
        assert_eq!(a.output, "unfinished translation");
        assert!(!a.manual_output_ready());
        a.tx.send(Event::Done {
            id,
            lane: Lane::Manual,
            result: Err("stream interrupted".into()),
            elapsed: 0.0,
            generation: 0,
        })
        .unwrap();
        a.process(&ctx);
        assert!(a.output.is_empty());
        assert!(!a.manual_output_ready());
        assert!(a.rows[0].record.translation.is_empty());
        assert_eq!(a.rows[0].record.source, "source");
        assert_eq!(a.rows[0].error.as_deref(), Some("stream interrupted"));
        assert!(a.manual_busy.is_none());
    }
    #[test]
    fn successful_manual_stream_enables_output_only_after_completion() {
        let mut a = app();
        let ctx = egui::Context::default();
        let id = 42;
        a.manual_busy = Some(id);
        a.rows.push(Row {
            record: record(id, "文章", "", 0, 0),
            pending: true,
            error: None,
            generation: 0,
        });
        a.tx.send(Event::Partial {
            id,
            text: "draft".into(),
        })
        .unwrap();
        a.process(&ctx);
        assert!(!a.manual_output_ready());
        a.tx.send(Event::Done {
            id,
            lane: Lane::Manual,
            result: Ok("complete translation".into()),
            elapsed: 0.0,
            generation: 0,
        })
        .unwrap();
        a.process(&ctx);
        assert!(a.manual_output_ready());
        assert_eq!(a.output, "complete translation");
        assert_eq!(a.rows[0].record.translation, a.output);
        assert!(a.rows[0].error.is_none());
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
    fn active_live_result_is_archived_only_for_its_source_or_a_new_audio_draft() {
        let mut a = app();
        a.rows.push(Row {
            record: record(42, "YouTube", "video-a", 0, 1000),
            pending: false,
            error: None,
            generation: a.caption_generation,
        });
        a.current_live_id = Some(42);
        a.asr_partial = "new audio draft".into();
        a.archive_completed_audio_for_draft();
        assert_eq!(a.current_live_id, Some(42));
        a.stop_audio();
        assert_eq!(a.current_live_id, Some(42));
        a.caption_enabled = true;
        a.set_caption_enabled(false);
        assert_eq!(a.current_live_id, None);
        assert_eq!(a.rows.len(), 1);
        assert_eq!(a.rows[0].record.translation, "translation");

        a.rows[0].record.kind = "音声".into();
        a.rows[0].pending = true;
        a.current_live_id = Some(42);
        a.asr_partial = "next draft".into();
        a.archive_completed_audio_for_draft();
        assert_eq!(a.current_live_id, Some(42));
        a.rows[0].pending = false;
        a.archive_completed_audio_for_draft();
        assert_eq!(a.current_live_id, None);
        assert_eq!(a.rows.len(), 1);
    }
    #[test]
    fn mini_mode_preserves_work_and_cannot_hide_initial_setup() {
        let mut a = app();
        let ctx = egui::Context::default();
        a.tab = Tab::Setup;
        a.enter_mini_view(&ctx);
        assert!(!a.window_mode.is_mini());
        a.backend_ready = true;
        a.tab = Tab::Text;
        a.input = "source".into();
        a.output = "translation".into();
        a.caption_enabled = true;
        a.audio_enabled = true;
        let settings = serde_json::to_value(&a.settings).unwrap();
        let generations = (a.caption_generation, a.audio_generation);
        a.enter_mini_view(&ctx);
        assert!(a.window_mode.is_mini());
        a.restore_normal_view(&ctx);
        assert!(!a.window_mode.is_mini());
        assert!(a.tab == Tab::Text);
        assert!(a.caption_enabled && a.audio_enabled);
        assert_eq!((a.caption_generation, a.audio_generation), generations);
        assert_eq!(a.input, "source");
        assert_eq!(a.output, "translation");
        assert_eq!(serde_json::to_value(&a.settings).unwrap(), settings);
    }
    #[test]
    fn closing_mini_stops_runtime_without_changing_saved_preferences_and_has_a_bound() {
        let mut a = app();
        let ctx = egui::Context::default();
        a.backend_ready = true;
        a.caption_enabled = true;
        a.audio_enabled = true;
        a.settings.auto_read = true;
        let settings = serde_json::to_value(&a.settings).unwrap();
        a.enter_mini_view(&ctx);
        a.request_app_close(&ctx);
        assert!(!a.window_mode.is_mini());
        assert!(!a.caption_enabled && !a.audio_enabled);
        assert!(a.readout.voice_cancel.load(Ordering::Acquire));
        assert_eq!(serde_json::to_value(&a.settings).unwrap(), settings);
        a.close_after_restore = Some(Instant::now() - Duration::from_secs(2));
        let output = ctx.run(egui::RawInput::default(), |ctx| a.update_window_mode(ctx));
        assert!(output.viewport_output.values().any(|viewport| {
            viewport
                .commands
                .iter()
                .any(|command| matches!(command, egui::ViewportCommand::Close))
        }));
        // Closing remains a terminal UI state; another shortcut cannot reopen mini.
        a.enter_mini_view(&ctx);
        assert!(!a.window_mode.is_mini());
    }
    #[test]
    fn repeated_close_requests_wait_and_dispatch_only_once() {
        let mut a = app();
        let ctx = egui::Context::default();
        a.backend_ready = true;
        a.enter_mini_view(&ctx);
        a.request_app_close(&ctx);
        let close_input = || {
            let mut input = egui::RawInput::default();
            input
                .viewports
                .get_mut(&egui::ViewportId::ROOT)
                .unwrap()
                .events
                .push(egui::ViewportEvent::Close);
            input
        };
        let waiting = ctx.run(close_input(), |ctx| {
            a.request_app_close(ctx);
            a.request_app_close(ctx);
            a.update_window_mode(ctx);
        });
        let commands = &waiting.viewport_output[&egui::ViewportId::ROOT].commands;
        assert!(commands.contains(&egui::ViewportCommand::CancelClose));
        assert!(!commands.contains(&egui::ViewportCommand::Close));
        a.close_after_restore = Some(Instant::now() - Duration::from_secs(2));
        let ready = ctx.run(close_input(), |ctx| a.update_window_mode(ctx));
        assert_eq!(
            ready.viewport_output[&egui::ViewportId::ROOT]
                .commands
                .iter()
                .filter(|c| **c == egui::ViewportCommand::Close)
                .count(),
            1
        );
        // egui-winit turns our Close command into a Close event on the next pass.
        let sent = ctx.run(close_input(), |ctx| {
            a.update_window_mode(ctx);
            a.request_app_close(ctx);
        });
        let commands = &sent.viewport_output[&egui::ViewportId::ROOT].commands;
        assert!(!commands.contains(&egui::ViewportCommand::CancelClose));
        assert!(!commands.contains(&egui::ViewportCommand::Close));
    }
    #[test]
    fn ocr_dispatch_keeps_live_view_until_the_manual_worker_succeeds() {
        for succeeds in [true, false] {
            let mut a = app();
            let ctx = egui::Context::default();
            a.backend_ready = true;
            a.tab = Tab::Live;
            a.enter_mini_view(&ctx);
            a.manual_busy = Some(42);
            a.rows.push(Row {
                record: record(42, "画面OCR", "", 0, 0),
                pending: true,
                error: None,
                generation: 0,
            });
            a.accept_ocr("screen text".into());
            assert!(!a.mini.is_manual());
            // Inject the response of an already-started worker, without HTTP/OCR.
            a.pending_ocr = None;
            a.tx.send(Event::Partial {
                id: 42,
                text: "途中の訳".into(),
            })
            .unwrap();
            a.process(&ctx);
            assert!(!a.mini.is_manual());
            a.tx.send(Event::Done {
                id: 42,
                lane: Lane::Manual,
                result: if succeeds {
                    Ok("完成した訳".into())
                } else {
                    Err("test failure".into())
                },
                elapsed: 0.1,
                generation: 0,
            })
            .unwrap();
            a.process(&ctx);
            assert_eq!(a.mini.is_manual(), succeeds);
            assert!(a.manual_busy.is_none());
        }
    }
    #[test]
    fn selection_cursor_stays_visible_and_crosshair_before_and_during_drag() {
        let mut a = app();
        let ctx = egui::Context::default();
        a.selection = Some(Selection {
            screen: platform::Screen {
                id: 1,
                name: "fixture".into(),
                x: 0,
                y: 0,
                width: 640,
                height: 480,
                scale: 1.0,
            },
            image: image::RgbaImage::new(640, 480),
            texture: ctx.load_texture(
                "fixture",
                egui::ColorImage::filled([2, 2], Color32::WHITE),
                egui::TextureOptions::LINEAR,
            ),
            start: None,
            focus_requested: false,
            continuous: false,
        });
        for events in [
            vec![egui::Event::PointerMoved(egui::pos2(120.0, 120.0))],
            vec![egui::Event::PointerButton {
                pos: egui::pos2(120.0, 120.0),
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::NONE,
            }],
            vec![egui::Event::PointerMoved(egui::pos2(240.0, 240.0))],
        ] {
            let output = ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(640.0, 480.0),
                    )),
                    events,
                    ..Default::default()
                },
                |ctx| a.selection_ui(ctx),
            );
            assert_eq!(
                output.platform_output.cursor_icon,
                egui::CursorIcon::Crosshair
            );
            assert!(output.viewport_output.values().any(|v| {
                v.commands
                    .contains(&egui::ViewportCommand::CursorVisible(true))
            }));
        }
        a.selection = None;
        let output = ctx.run(egui::RawInput::default(), |ctx| a.render_ui(ctx));
        assert_ne!(
            output.platform_output.cursor_icon,
            egui::CursorIcon::Crosshair
        );
    }
    #[test]
    fn paused_and_empty_captions_preserve_identity_and_dedup_without_blocking_audio() {
        let mut a = app();
        a.caption_enabled = true;
        a.audio_enabled = true;
        a.last_live_id = Some(42);
        a.current_live_id = Some(42);
        a.last_live_text = "same caption".into();
        a.last_caption = Some(Instant::now());
        a.caption_paused = false;
        a.overlay_kind = Some(LiveKind::Caption);
        a.overlay_text = "old caption".into();
        let last = a.last_caption;
        a.accept_caption(caption("same caption", true));
        assert_eq!(a.caption_generation, 11);
        assert_eq!(a.last_live_id, Some(42));
        assert_eq!(a.current_live_id, Some(42));
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
            rx: AudioReceiver::Batch(rx),
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
    fn transcript_accumulates_during_translation_and_active_captions_suppress_late_audio() {
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
        assert!(a.pending_transcript.is_none());
        assert_eq!(a.rows.len(), 1);
        assert_eq!(a.rows[0].record.source, "first");
        assert!(a.rows[0].is_provisional());
        assert!(!a.rows[0].is_confirmed());
        assert_eq!(a.live_busy, Some(99));
        assert!(a.take_ready_transcript().is_none());
        a.live_busy = None;
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
        assert_eq!(a.rows.len(), 1);
        assert_eq!(a.rows[0].record.source, "first");
    }
    #[test]
    fn accepted_transcript_is_not_aged_out_and_new_timeline_invalidates_both_sources() {
        let mut a = app();
        a.audio_enabled = true;
        a.audio_offset_ms = 10000;
        a.pending_transcript = Some(PendingTranscript {
            text: "old".into(),
            start_ms: 0,
            end_ms: 1,
            generation: 7,
        });
        assert_eq!(a.take_ready_transcript().unwrap().text, "old");
        a.pending_audio = Some(chunk(0, 1));
        a.pending_caption = Some(caption("caption", false));
        a.invalidate_timeline();
        assert_eq!(a.audio_generation, 8);
        assert_eq!(a.caption_generation, 12);
        assert!(a.pending_audio.is_none());
        assert!(a.pending_caption.is_none());
    }
    #[test]
    fn ocr_opens_text_screen_and_latest_request_waits_until_manual_lane_is_free() {
        let mut a = app();
        assert!(a.tab == Tab::Live);
        a.caption_enabled = true;
        let generation = a.caption_generation;
        a.manual_busy = Some(1);
        a.accept_ocr("first".into());
        a.accept_ocr("latest".into());
        assert!(a.tab == Tab::Text, "the chat does not show a reading");
        assert!(
            a.caption_enabled && a.caption_generation == generation,
            "the live input runs on behind the other screen"
        );
        assert!(a.take_ready_ocr().is_none());
        a.manual_busy = None;
        assert_eq!(a.take_ready_ocr().as_deref(), Some("latest"));

        let mut a = app();
        a.window_mode.enter(&egui::ViewportInfo::default());
        assert!(a.window_mode.is_mini());
        a.accept_ocr("read in the mini window".into());
        assert!(
            a.tab == Tab::Live,
            "the mini window shows the reading itself"
        );
    }
    #[test]
    fn a_file_dropped_on_the_chat_ends_on_the_text_screen_and_the_live_input_runs_on() {
        let folder = tempfile::tempdir().unwrap();
        let note = folder.path().join("note.txt");
        fs::write(&note, "A dropped note.").unwrap();
        let picture = folder.path().join("picture.png");
        image::RgbaImage::new(2, 2).save(&picture).unwrap();
        let drop = |a: &mut App, path: &Path| {
            let input = egui::RawInput {
                dropped_files: vec![egui::DroppedFile {
                    path: Some(path.to_path_buf()),
                    ..Default::default()
                }],
                ..Default::default()
            };
            let _ = egui::Context::default().run(input, |ctx| a.open_dropped_file(ctx));
        };
        let chat = || {
            let mut a = app();
            assert!(a.tab == Tab::Live);
            a.caption_enabled = true;
            a
        };

        let mut a = chat();
        drop(&mut a, &note);
        assert!(a.tab == Tab::Text);
        assert_eq!(a.input, "A dropped note.");
        assert!(a.caption_enabled);

        // A picture is read first. An unprepared recogniser keeps it waiting,
        // and the manual lane is busy, so nothing runs in this test.
        let mut a = chat();
        a.settings.ocr_backend = "indra".into();
        a.manual_busy = Some(1);
        let before = a.ocr.generation;
        drop(&mut a, &picture);
        assert_ne!(a.ocr.generation, before, "the picture is sent to be read");
        a.tx.send(Event::Ocr {
            generation: a.ocr.generation,
            result: Ok("読み取った文字".into()),
        })
        .unwrap();
        a.process(&egui::Context::default());
        assert!(a.tab == Tab::Text);
        assert_eq!(a.pending_ocr.as_deref(), Some("読み取った文字"));
        assert!(a.caption_enabled);
    }
    #[test]
    fn unfinished_paragraphs_recover_and_export_as_unfinished_until_final_commit() {
        use plamo_translate::journal::{ParagraphMeta, ParagraphState};
        let mut records = Vec::new();
        for (index, state) in [
            ParagraphState::Open,
            ParagraphState::Closing,
            ParagraphState::Interrupted,
            ParagraphState::Failed,
            ParagraphState::Final,
        ]
        .into_iter()
        .enumerate()
        {
            let mut item = record(
                index as u64 + 1,
                "音声",
                "",
                index as u64 * 1_000,
                (index as u64 + 1) * 1_000,
            );
            item.source = format!("original paragraph {index}");
            item.translation = format!("訳文 {index}");
            item.paragraph = Some(ParagraphMeta {
                revision: 3,
                state,
                close_reason: None,
            });
            let recovered = Row::recovered(item.clone());
            if matches!(state, ParagraphState::Open | ParagraphState::Closing) {
                assert!(!recovered.is_confirmed() && !recovered.is_provisional());
                let meta = recovered.record.paragraph.as_ref().unwrap();
                assert_eq!(meta.state, ParagraphState::Interrupted);
                assert_eq!(meta.close_reason.as_deref(), Some("restarted"));
                assert_eq!(recovered.record.source, item.source);
                assert_eq!(recovered.record.translation, item.translation);
            }
            records.push(item);
        }
        let text = timeline_txt(&records);
        assert_eq!(text.matches("訳文は未確定").count(), 4);
        for item in &records {
            assert!(text.contains(&item.source));
        }
        let srt = subtitles::parse_srt(&timeline_srt(&records).unwrap()).unwrap();
        assert_eq!(srt.len(), 1);
        assert_eq!(srt[0].text, "訳文 4");
        assert_eq!((srt[0].start_ms, srt[0].end_ms), (4_000, 5_000));
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
            rx: AudioReceiver::Batch(rx),
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
    #[test]
    fn range_selection_takes_keyboard_focus_once_so_esc_can_cancel_it() {
        let ctx = egui::Context::default();
        let mut a = app();
        let screen = platform::Screen {
            id: 0,
            name: "test".into(),
            x: 0,
            y: 0,
            width: 800,
            height: 600,
            scale: 1.0,
        };
        a.tx.send(Event::Screenshot(Ok((screen, image::RgbaImage::new(8, 8)))))
            .unwrap();
        let frame = |a: &mut App, events: Vec<egui::Event>| {
            let output = ctx.run(
                egui::RawInput {
                    events,
                    ..Default::default()
                },
                |ctx| {
                    a.process(ctx);
                    a.selection_ui(ctx);
                },
            );
            output
                .viewport_output
                .values()
                .any(|viewport| viewport.commands.contains(&egui::ViewportCommand::Focus))
        };
        // Ctrl+Alt+T opens the overlay while another app holds the keyboard.
        assert!(frame(&mut a, vec![]), "the overlay asks for keyboard focus");
        assert!(!frame(&mut a, vec![]), "it asks only once");
        frame(
            &mut a,
            vec![egui::Event::Key {
                key: egui::Key::Escape,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            }],
        );
        assert!(a.selection.is_none(), "Esc cancels the selection");
    }
    #[test]
    fn automatic_readout_never_opens_the_voice_preparation() {
        let mut a = app();
        let ctx = egui::Context::default();
        a.settings.readout_enabled = true;
        a.settings.auto_read = true;
        let id = 43;
        a.manual_busy = Some(id);
        a.rows.push(Row {
            record: record(id, "文章", "", 0, 0),
            pending: true,
            error: None,
            generation: 0,
        });
        a.tx.send(Event::Done {
            id,
            lane: Lane::Manual,
            result: Ok("完成した訳文です。".into()),
            elapsed: 0.0,
            generation: 0,
        })
        .unwrap();
        a.process(&ctx);
        assert_eq!(a.output, "完成した訳文です。", "the translation finished");
        assert!(
            !a.readout.confirm,
            "with no voice, automatic readout stays quiet instead of asking again"
        );
        a.read_translation(a.output.clone());
        assert!(
            a.readout.confirm,
            "a click on 読み上げ still offers to prepare"
        );
    }
}
