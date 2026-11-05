//! Optional local reading of translated Japanese. Audio capture never starts here.
use super::*;
use plamo_translate::{
    installer::Progress,
    speech_setup::{self, SpeechInstallation},
    tts::{ReadoutOptions, TtsEngine, TtsEvent},
};

pub(super) struct ReadoutState {
    pub ready: Option<SpeechInstallation>,
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
}
impl Default for ReadoutState {
    fn default() -> Self {
        Self {
            ready: None,
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
        }
    }
}
pub(super) enum ReadoutEvent {
    Progress(Progress),
    Installed(std::result::Result<SpeechInstallation, String>),
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
    pub(super) fn initialize_readout(&mut self) {
        match speech_setup::load(&data_root().join("speech")) {
            Ok(installation) => self.readout.ready = installation,
            Err(error) => {
                self.readout.error = format!("読み上げの保存情報を確認できません: {error}")
            }
        }
    }
    pub(super) fn readout_busy(&self) -> bool {
        self.readout.engine.as_ref().is_some_and(TtsEngine::is_busy)
    }
    pub(super) fn read_translation(&mut self, text: String) {
        self.read_translation_mode(text, false);
    }
    pub(super) fn read_live_translation(&mut self, text: String) {
        self.read_translation_mode(text, true);
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
        if self.readout.engine.is_none() {
            match TtsEngine::start(config, self.readout.suppress.clone()) {
                Ok(engine) => self.readout.engine = Some(engine),
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
        if let Some(engine) = &self.readout.engine {
            engine.stop();
        }
        self.readout.rate_active = false;
        self.readout.status = "読み上げを停止しました。".into();
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
        if self.readout_busy() && self.readout.rate_active {
            self.readout.status = format!("読み上げ ×{:.1}", self.readout.current_rate);
        } else if !self.readout_busy() {
            self.readout.rate_active = false;
            if self.readout.status.starts_with("読み上げ ×") {
                self.readout.status = "読み上げが終わりました。".into();
            }
        }
    }
    fn begin_readout_install(&mut self) {
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
                        self.readout.confirm = false;
                        self.readout.error.clear();
                        self.readout.status = "読み上げの準備ができました。".into();
                    }
                    Err(_) if self.readout.cancel.load(Ordering::Relaxed) => {
                        self.readout.error.clear();
                        self.readout.status =
                            "準備を中止しました。取得済みデータから再開できます。".into();
                    }
                    Err(error) => self.readout.error = error,
                }
            }
        }
    }
    pub(super) fn readout_settings_ui(&mut self, ui: &mut egui::Ui) {
        ui.label(RichText::new("訳文の読み上げ").strong());
        ui.label("Irodori-TTS v4.1 Small-MF / speech.cpp");
        ui.label("日本語の訳文を、このPC内で音声にします。");
        let options = readout_options(&self.settings, false);
        self.settings.tts_speed = options.speed;
        self.settings.tts_max_speed = options.max_speed;
        ui.add(
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
        ui.add(
            egui::Slider::new(&mut self.settings.tts_max_speed, 1.2..=2.0)
                .step_by(0.1)
                .fixed_decimals(1)
                .suffix("倍")
                .text("自動調整の上限"),
        );
        self.settings.tts_max_speed = self.settings.tts_max_speed.max(self.settings.tts_speed);
        ui.add_enabled_ui(self.readout.ready.is_some(), |ui| {
            ui::toggle(
                ui,
                &mut self.settings.auto_read,
                "日本語の訳文を自動で読み上げる",
            );
        });
        ui.label(RichText::new("読み上げ中は、再生音声の聞き取りを一時停止します。").size(12.0));
        ui.horizontal_wrapped(|ui| {
            if self.readout.ready.is_none() {
                if ui.button("読み上げを準備").clicked() {
                    self.readout.confirm = true;
                }
            } else if ui.button("声を試す").clicked() {
                self.read_japanese_text(
                    "こんにちは。地元翻訳くんの読み上げテストです。".into(),
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
            ui.colored_label(Color32::from_rgb(240, 122, 122), &self.readout.error);
        }
    }
    pub(super) fn readout_setup_ui(&mut self, ctx: &egui::Context) {
        if !self.readout.confirm && !self.readout.preparing {
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
                ui.label("展開用を含め、約4.4GiBの空き容量を用意してください。");
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
                    ui.colored_label(Color32::from_rgb(240, 122, 122), &self.readout.error);
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
