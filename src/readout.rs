//! Optional local reading of translated Japanese. Audio capture never starts here.
use super::setup_chain::Step;
use super::*;
use plamo_translate::{
    installer::Progress,
    speech_setup::{self, SpeechInstallation},
    tts::{ReadoutOptions, TtsEngine, TtsEvent},
    voice_profile::{self, VoiceProfile},
};

pub(super) const AUTO_READ_HINT: &str = "新しい日本語の訳文を自動で読み上げます。押すたびにオンとオフが切り替わり、オフにすると読み上げ中の文も止まります";

pub(super) struct ReadoutState {
    pub ready: Option<SpeechInstallation>,
    pub install_checked: bool,
    pub engine: Option<TtsEngine>,
    pub preparing: bool,
    pub confirm: bool,
    pub progress: Option<Progress>,
    pub error: String,
    pub status: String,
    pub current_rate: f32,
    rate_active: bool,
    pub suppress: Arc<AtomicBool>,
    pub cancel: Arc<AtomicBool>,
    pub voices: Vec<(String, VoiceProfile)>,
    pub voice_preparing: bool,
    pub voice_cancel: Arc<AtomicBool>,
    pub voice_worker: Option<thread::JoinHandle<()>>,
    engine_voice: String,
    pending_voice_read: Option<(String, bool, Instant)>,
}
impl Default for ReadoutState {
    fn default() -> Self {
        Self {
            ready: None,
            install_checked: false,
            engine: None,
            preparing: false,
            confirm: false,
            progress: None,
            error: String::new(),
            status: String::new(),
            current_rate: 1.0,
            rate_active: false,
            suppress: Arc::new(AtomicBool::new(false)),
            cancel: Arc::new(AtomicBool::new(false)),
            voices: Vec::new(),
            voice_preparing: false,
            voice_cancel: Arc::new(AtomicBool::new(false)),
            voice_worker: None,
            engine_voice: String::new(),
            pending_voice_read: None,
        }
    }
}
pub(super) enum ReadoutEvent {
    Progress(Progress),
    Installed(std::result::Result<SpeechInstallation, String>),
    VoicePrepared {
        key: String,
        result: std::result::Result<VoiceProfile, String>,
    },
}

/// Manual actions always use the configured base rate. Only fresh automatic
/// Live output opts into the native worker's adaptive catch-up policy.
fn readout_options(settings: &Settings, live: bool) -> ReadoutOptions {
    let speed = if settings.tts_speed.is_finite() {
        settings.tts_speed
    } else {
        1.0
    }
    .clamp(0.8, 1.6);
    let max_speed = if settings.tts_max_speed.is_finite() {
        settings.tts_max_speed
    } else {
        1.8
    }
    .clamp(1.2, 2.0)
    .max(speed);
    ReadoutOptions {
        speed,
        catch_up: live && settings.tts_catch_up,
        max_speed,
    }
}

impl App {
    /// Reading the saved install hashes the runtime and every voice, so it
    /// waits until 「読み上げを使う」 is on: at start, or when it is turned on.
    pub(super) fn initialize_readout(&mut self) {
        if !self.settings.readout_enabled || self.readout.install_checked {
            return;
        }
        self.readout.install_checked = true;
        match speech_setup::load(&data_root().join("speech")) {
            Ok(installation) => self.readout.ready = installation,
            Err(error) => {
                self.readout.error = format!("読み上げの保存情報を確認できません: {error}")
            }
        }
        self.refresh_readout_voices();
    }
    pub(super) fn readout_busy(&self) -> bool {
        self.readout.voice_preparing || self.readout.engine.as_ref().is_some_and(TtsEngine::is_busy)
    }
    pub(super) fn read_translation(&mut self, text: String) {
        self.read_translation_mode(text, false);
    }
    /// Reads a finished translation because 自動読み上げ is on. Only a click
    /// offers to prepare a missing voice; asking after every translation
    /// would bring the dialog back each time it is declined.
    pub(super) fn auto_read_translation(&mut self, text: String, live: bool) {
        if self.settings.readout_enabled && self.readout.ready.is_some() {
            self.read_translation_mode(text, live);
        }
    }
    fn read_translation_mode(&mut self, text: String, live: bool) {
        if text.trim().is_empty() {
            return;
        }
        if self.settings.translation.target_lang != "ja" {
            self.readout.error = "読み上げは日本語の訳文に対応しています。".into();
            return;
        }
        self.read_japanese_text(text, live);
    }
    fn read_japanese_text(&mut self, text: String, live: bool) {
        let Some(config) = self.readout.ready.clone() else {
            self.readout.confirm = true;
            return;
        };
        self.readout.error.clear();
        let Some(profile) = self
            .readout
            .voices
            .iter()
            .find(|(key, _)| *key == self.settings.tts_voice_profile)
            .map(|(_, profile)| profile.clone())
        else {
            if self.settings.tts_voice_profile != "default-v1" {
                self.readout.error =
                    "選択した固定声がありません。設定の「その他の機能」で声を選び直してください。"
                        .into();
                return;
            }
            self.readout.pending_voice_read = Some((text, live, Instant::now()));
            self.prepare_readout_voice(None);
            return;
        };
        if self.readout.engine_voice != self.settings.tts_voice_profile {
            self.readout.engine.take();
        }
        if self.readout.engine.is_none() {
            match TtsEngine::start_with_voice(config, self.readout.suppress.clone(), profile) {
                Ok(engine) => {
                    self.readout.engine = Some(engine);
                    self.readout.engine_voice = self.settings.tts_voice_profile.clone();
                }
                Err(error) => {
                    self.readout.error = error.to_string();
                    return;
                }
            }
        }
        if let Some(engine) = &self.readout.engine {
            if !engine.is_busy() {
                self.readout.rate_active = false;
            }
            engine.speak_with_options(text, readout_options(&self.settings, live));
        }
    }
    pub(super) fn stop_readout(&mut self) {
        self.readout.voice_cancel.store(true, Ordering::Release);
        self.readout.pending_voice_read = None;
        if let Some(engine) = &self.readout.engine {
            engine.stop();
        }
        self.readout.rate_active = false;
        self.readout.status = "読み上げを停止しました。".into();
    }
    /// Every 自動読み上げ control calls this, so they all behave alike.
    pub(super) fn toggle_auto_read(&mut self, ctx: &egui::Context) {
        if self.settings.auto_read {
            self.settings.auto_read = false;
            self.stop_readout();
        } else if self.readout.ready.is_some() {
            self.settings.auto_read = true;
        } else {
            if self.window_mode.is_mini() {
                self.restore_normal_view(ctx);
            }
            self.open_other_setting(ui::OtherSetting::Readout);
            self.readout.confirm = true;
        }
    }
    pub(super) fn poll_readout(&mut self) {
        let events = self
            .readout
            .engine
            .as_ref()
            .map(TtsEngine::poll)
            .unwrap_or_default();
        for event in events {
            match event {
                TtsEvent::Status(status) => self.readout.status = status,
                TtsEvent::Started => self.readout.status = "読み上げ中です。".into(),
                TtsEvent::Rate(rate) => {
                    if rate.is_finite() && rate > 0.0 {
                        self.readout.current_rate = rate;
                        self.readout.rate_active = self.readout_busy();
                    }
                }
                TtsEvent::Finished => {
                    self.readout.rate_active = false;
                    self.readout.status = "読み上げが終わりました。".into();
                }
                TtsEvent::Error(error) => {
                    self.readout.rate_active = false;
                    self.readout.error = error;
                }
            }
        }
        if self.readout.voice_preparing {
            self.readout.status = "読み上げる声を固定しています。初回だけ準備が必要です。".into();
        } else if self.readout_busy() && self.readout.rate_active {
            self.readout.status = format!("読み上げ ×{:.1}", self.readout.current_rate);
        } else if !self.readout_busy() {
            self.readout.rate_active = false;
            if self.readout.status.starts_with("読み上げ ×") {
                self.readout.status = "読み上げが終わりました。".into();
            }
        }
    }
    fn refresh_readout_voices(&mut self) {
        self.readout.voices.clear();
        let Some(config) = &self.readout.ready else {
            return;
        };
        let root = data_root().join("speech/voices");
        let Ok(entries) = fs::read_dir(root) else {
            return;
        };
        for entry in entries.flatten() {
            let key = entry.file_name().to_string_lossy().into_owned();
            if key.is_empty()
                || key.len() > 64
                || !key.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-')
            {
                continue;
            }
            match voice_profile::load_profile(&entry.path(), config) {
                Ok(Some(profile)) => self.readout.voices.push((key, profile)),
                Err(error) => self.readout.error = format!("固定声を読み込めません: {error}"),
                _ => {}
            }
        }
        self.readout
            .voices
            .sort_by_key(|(key, _)| (key != "default-v1", key.clone()));
    }
    pub(super) fn prepare_readout_voice(&mut self, reference: Option<PathBuf>) {
        if self.readout.voice_preparing {
            return;
        }
        let Some(config) = self.readout.ready.clone() else {
            self.readout.confirm = true;
            return;
        };
        self.readout.voice_preparing = true;
        self.readout.voice_cancel = Arc::new(AtomicBool::new(false));
        self.readout.error.clear();
        let key = if reference.is_some() {
            format!("custom-{}", uuid::Uuid::new_v4())
        } else {
            "default-v1".into()
        };
        let dir = data_root().join("speech/voices").join(&key);
        let cancel = self.readout.voice_cancel.clone();
        let tx = self.tx.clone();
        let shutdown = self.stop.clone();
        self.readout.voice_worker = Some(thread::spawn(move || {
            let result = match reference {
                Some(path) => voice_profile::import_reference_profile(&config, &dir, &path, cancel),
                None => voice_profile::prepare_default_profile(&config, &dir, cancel),
            }
            .map_err(|e| format!("{e:#}"));
            let mut event = Event::Readout(ReadoutEvent::VoicePrepared { key, result });
            loop {
                match tx.try_send(event) {
                    Ok(()) => break,
                    Err(mpsc::TrySendError::Full(value)) if !shutdown.load(Ordering::Acquire) => {
                        event = value;
                        thread::sleep(Duration::from_millis(5));
                    }
                    Err(_) => break,
                }
            }
        }));
    }
    pub(super) fn begin_readout_install(&mut self) {
        if self.readout.preparing {
            return;
        }
        self.readout.preparing = true;
        self.readout.progress = None;
        self.readout.error.clear();
        self.readout.cancel = Arc::new(AtomicBool::new(false));
        let cancel = self.readout.cancel.clone();
        let tx = self.tx.clone();
        let backend = if self
            .setup
            .hardware
            .as_ref()
            .is_some_and(|h| h.gpus.iter().any(|g| g.vulkan_eligible))
        {
            "vulkan"
        } else {
            "cpu"
        };
        let root = data_root().join("speech");
        thread::spawn(move || {
            let result = speech_setup::install(&root, backend, cancel, |progress| {
                let _ = tx.try_send(Event::Readout(ReadoutEvent::Progress(progress)));
            })
            .map_err(|e| format!("{e:#}"));
            let _ = tx.send(Event::Readout(ReadoutEvent::Installed(result)));
        });
    }
    pub(super) fn accept_readout_event(&mut self, event: ReadoutEvent) {
        match event {
            ReadoutEvent::Progress(progress) => self.readout.progress = Some(progress),
            ReadoutEvent::Installed(result) => {
                self.readout.preparing = false;
                self.readout.progress = None;
                match result {
                    Ok(config) => {
                        self.readout.ready = Some(config);
                        self.setup.plan = None;
                        self.readout.confirm = false;
                        self.readout.error.clear();
                        self.readout.status = "読み上げの準備ができました。".into();
                        self.refresh_readout_voices();
                        if self.chain_drives(Step::Voice) {
                            self.continue_voice();
                        }
                    }
                    Err(_) if self.readout.cancel.load(Ordering::Relaxed) => {
                        self.readout.error.clear();
                        self.readout.status =
                            "準備を中止しました。取得済みデータから再開できます。".into();
                        self.stop_step(Step::Voice, true, String::new());
                    }
                    Err(error) => {
                        self.readout.error = error.clone();
                        self.stop_step(Step::Voice, false, error);
                    }
                }
            }
            ReadoutEvent::VoicePrepared { key, result } => {
                if let Some(worker) = self.readout.voice_worker.take() {
                    let _ = worker.join();
                }
                self.readout.voice_preparing = false;
                match result {
                    Ok(_) => {
                        self.readout.engine.take();
                        self.settings.tts_voice_profile = key;
                        self.refresh_readout_voices();
                        self.readout.status = "読み上げる声を固定しました。".into();
                        if !self.readout.voice_cancel.load(Ordering::Acquire)
                            && let Some((text, live, received)) =
                                self.readout.pending_voice_read.take()
                            && (!live || received.elapsed() < Duration::from_secs(8))
                        {
                            self.read_japanese_text(text, live);
                        }
                        self.finish_step(Step::Voice);
                    }
                    Err(_) if self.readout.voice_cancel.load(Ordering::Acquire) => {
                        self.readout.status = "固定声の準備を中止しました。".into();
                        self.stop_step(Step::Voice, true, String::new());
                    }
                    Err(error) => {
                        self.readout.error = error.clone();
                        self.stop_step(Step::Voice, false, error);
                    }
                }
            }
        }
    }
    pub(super) fn readout_settings_ui(&mut self, ui: &mut egui::Ui) {
        ui.label(RichText::new("訳文の読み上げ").strong());
        let was_used = self.settings.readout_enabled;
        ui::toggle(ui, &mut self.settings.readout_enabled, "読み上げを使う");
        if was_used && !self.settings.readout_enabled {
            self.readout.cancel.store(true, Ordering::Relaxed);
            self.stop_readout();
            self.readout.engine.take();
            self.readout.status.clear();
            self.readout.error.clear();
        }
        if !was_used && self.settings.readout_enabled {
            self.initialize_readout();
        }
        if !self.settings.readout_enabled {
            ui.label(theme::caption(
                "オンにすると、訳文の「読み上げ」と「自動読み上げ」が画面に出ます。",
            ));
            return;
        }
        ui.label(theme::label("日本語の訳文を、このPC内で音声にします。"));
        ui.label(theme::caption("Irodori-TTS v4.1 Small-MF / speech.cpp"));
        let old_voice = self.settings.tts_voice_profile.clone();
        ui.add_enabled_ui(!self.readout.voice_preparing, |ui| {
            egui::ComboBox::from_id_salt("fixed-readout-voice")
                .selected_text(if self.settings.tts_voice_profile == "default-v1" {
                    "標準の固定声"
                } else {
                    "追加した固定声"
                })
                .show_ui(ui, |ui| {
                    ui.selectable_value(
                        &mut self.settings.tts_voice_profile,
                        "default-v1".into(),
                        "標準の固定声",
                    );
                    for (index, (key, _)) in self
                        .readout
                        .voices
                        .iter()
                        .filter(|(key, _)| key != "default-v1")
                        .enumerate()
                    {
                        ui.selectable_value(
                            &mut self.settings.tts_voice_profile,
                            key.clone(),
                            format!("追加した固定声 {}", index + 1),
                        );
                    }
                });
            ui.horizontal_wrapped(|ui| {
                if ui.button("固定声を準備").clicked() {
                    self.prepare_readout_voice(None);
                }
                if ui.button("参照音声から登録").clicked()
                    && let Some(path) = rfd::FileDialog::new()
                        .add_filter("WAV", &["wav"])
                        .pick_file()
                {
                    self.prepare_readout_voice(Some(path));
                }
            });
        });
        if old_voice != self.settings.tts_voice_profile {
            self.readout.pending_voice_read = None;
            self.readout.engine.take();
        }
        ui.label(theme::caption("声の特徴を一度だけ保存し、字幕が変わっても同じ声で読み上げます。生成音声ファイルは残しません。"));
        let options = readout_options(&self.settings, false);
        self.settings.tts_speed = options.speed;
        self.settings.tts_max_speed = options.max_speed;
        theme::slider(
            ui,
            egui::Slider::new(&mut self.settings.tts_speed, 0.8..=1.6)
                .step_by(0.1)
                .fixed_decimals(1)
                .suffix("倍")
                .text("基準速度"),
        );
        ui::toggle(
            ui,
            &mut self.settings.tts_catch_up,
            "ライブ字幕の遅れに合わせて速度を上げる",
        );
        theme::slider(
            ui,
            egui::Slider::new(&mut self.settings.tts_max_speed, 1.2..=2.0)
                .step_by(0.1)
                .fixed_decimals(1)
                .suffix("倍")
                .text("自動調整の上限"),
        );
        self.settings.tts_max_speed = self.settings.tts_max_speed.max(self.settings.tts_speed);
        let was_enabled = self.settings.auto_read;
        ui.add_enabled_ui(self.readout.ready.is_some(), |ui| {
            ui::toggle(
                ui,
                &mut self.settings.auto_read,
                "日本語の訳文を自動で読み上げる",
            );
        });
        if was_enabled && !self.settings.auto_read {
            self.stop_readout();
        }
        ui.label(theme::caption(
            "読み上げ中は、再生音声の聞き取りを一時停止します。",
        ));
        ui.horizontal_wrapped(|ui| {
            if self.readout.ready.is_none() {
                if ui.button("読み上げを準備").clicked() {
                    self.readout.confirm = true;
                }
            } else if ui.button("声を試す").clicked() {
                self.read_japanese_text(
                    "こんにちは。トーク風ライブ翻訳の読み上げテストです。".into(),
                    false,
                );
            }
            if ui
                .add_enabled(self.readout_busy(), egui::Button::new("読み上げを停止"))
                .clicked()
            {
                self.stop_readout();
            }
        });
        if !self.readout.status.is_empty() {
            ui.label(&self.readout.status);
        }
        if !self.readout.error.is_empty() {
            ui.colored_label(theme::ERROR, &self.readout.error);
        }
    }
    pub(super) fn readout_setup_ui(&mut self, ctx: &egui::Context) {
        if self.chain_drives(Step::Voice) || (!self.readout.confirm && !self.readout.preparing) {
            return;
        }
        egui::Window::new("読み上げの準備")
            .collapsible(false)
            .resizable(false)
            .default_width(470.0)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.label("小型の日本語音声モデルとspeech.cppを追加します。Pythonは不要です。");
                ui.label("初回は約1.95GBを取得します。WindowsではVulkan対応ドライバーが必要です。");
                ui.label("展開用を含め、約4.4GBの空き容量を用意してください。");
                ui.label("準備だけでは読み上げや音声の取得は始まりません。");
                if self.readout.preparing {
                    if let Some(progress) = &self.readout.progress {
                        ui.label(&progress.message);
                        if let Some(total) = progress.total_bytes.filter(|t| *t > 0) {
                            ui.add(
                                egui::ProgressBar::new(
                                    (progress.downloaded_bytes as f64 / total as f64)
                                        .clamp(0.0, 1.0) as f32,
                                )
                                .text(format!(
                                    "{} / {}",
                                    setup::bytes(progress.downloaded_bytes),
                                    setup::bytes(total)
                                )),
                            );
                        } else {
                            ui.spinner();
                        }
                    } else {
                        ui.spinner();
                    }
                    if ui.button("中止").clicked() {
                        self.readout.cancel.store(true, Ordering::Relaxed);
                    }
                } else {
                    ui.horizontal_wrapped(|ui| {
                        if ui.add(ui::primary("ダウンロードして準備")).clicked() {
                            self.begin_readout_install();
                        }
                        if ui.button("今はしない").clicked() {
                            self.readout.confirm = false;
                        }
                    });
                }
                if !self.readout.error.is_empty() {
                    ui.colored_label(theme::ERROR, &self.readout.error);
                }
            });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn manual_and_live_options_keep_catch_up_separate() {
        let mut settings = Settings {
            tts_speed: 1.3,
            tts_max_speed: 1.8,
            ..Settings::default()
        };
        let manual = readout_options(&settings, false);
        let live = readout_options(&settings, true);
        assert_eq!(manual.speed, 1.3);
        assert_eq!(live.speed, manual.speed);
        assert!(!manual.catch_up && live.catch_up);
        assert_eq!(live.max_speed, 1.8);
        settings.tts_catch_up = false;
        assert!(!readout_options(&settings, true).catch_up);
        settings.tts_speed = 1.6;
        settings.tts_max_speed = 1.2;
        assert_eq!(readout_options(&settings, true).max_speed, 1.6);
    }
    #[test]
    fn older_settings_keep_defaults_and_existing_values() {
        let settings: Settings =
            serde_json::from_str(r#"{"auto_read":true,"font_size":22.0}"#).unwrap();
        assert!(settings.auto_read);
        assert!(
            !settings.readout_enabled,
            "settings saved before the switch existed keep readout hidden"
        );
        assert_eq!(settings.font_size, 22.0);
        assert_eq!(settings.tts_speed, 1.0);
        assert!(settings.tts_catch_up);
        assert_eq!(settings.tts_max_speed, 1.8);
        let defaults = Settings::default();
        assert!(!defaults.auto_read);
        assert_eq!(ReadoutState::default().current_rate, 1.0);
    }
    #[test]
    fn readout_is_optional_and_preparation_does_not_start_capture() {
        let mut app = App::inert();
        assert!(!app.settings.auto_read && app.readout.engine.is_none());
        app.read_translation("日本語の訳文です。".into());
        assert!(app.readout.confirm);
        assert!(!app.audio_enabled && app.audio.is_none() && app.readout.engine.is_none());
    }
    #[test]
    fn unsupported_translation_language_does_not_speak() {
        let mut app = App::inert();
        app.settings.translation.target_lang = "en".into();
        app.read_translation("Hello".into());
        assert!(!app.readout.confirm && app.readout.engine.is_none());
        assert!(!app.readout.error.is_empty());
    }
}
