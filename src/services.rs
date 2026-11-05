//! Owned model processes and first-run readiness. No capture starts during setup.
use super::*;
use plamo_translate::installer;
use std::process::Stdio;

pub(super) fn kill_owned_child(child: &mut Child) {
    if matches!(child.try_wait(), Ok(Some(_))) {
        return;
    }
    #[cfg(windows)]
    {
        let mut kill = Command::new("taskkill.exe");
        kill.args(["/PID", &child.id().to_string(), "/T", "/F"])
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        command_hidden(&mut kill);
        let _ = kill.status();
    }
    let _ = child.kill();
    let _ = child.wait();
}
pub(super) struct AsrStatus {
    backend: String,
    model: String,
    device: String,
}
impl AsrStatus {
    fn matches(&self, settings: &Settings) -> bool {
        self.backend == settings.asr_backend
            && self.model == settings.asr_model
            && (settings.asr_device == "auto" || self.device == settings.asr_device)
    }
}
fn local_health(endpoint: &str) -> Result<Option<AsrStatus>> {
    let mut url = reqwest::Url::parse(endpoint)?;
    anyhow::ensure!(
        matches!(
            url.host_str(),
            Some("localhost" | "127.0.0.1" | "::1" | "[::1]")
        ),
        "ローカルの接続先を指定してください"
    );
    anyhow::ensure!(
        url.username().is_empty()
            && url.password().is_none()
            && url.query().is_none()
            && url.fragment().is_none(),
        "接続先の形式が不正です"
    );
    url.set_path("/health");
    let client = reqwest::blocking::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(1))
        .timeout(Duration::from_secs(2))
        .build()?;
    let response = client.get(url).send()?;
    if !response.status().is_success() {
        return Ok(None);
    }
    let data: serde_json::Value = response.json()?;
    if data["status"] != "ok" || data["loaded"] != true {
        return Ok(None);
    }
    Ok(Some(AsrStatus {
        backend: data["backend"].as_str().unwrap_or_default().into(),
        model: data["model"].as_str().unwrap_or_default().into(),
        device: data["device"].as_str().unwrap_or_default().into(),
    }))
}
impl App {
    pub(super) fn persist_preferences(&mut self) -> Result<()> {
        if self.settings_load_failed && config_path().exists() {
            let backup = config_path().with_file_name(format!(
                "settings-unreadable-{}.json",
                chrono::Utc::now().format("%Y%m%dT%H%M%S%f")
            ));
            fs::copy(config_path(), backup)?;
        }
        save_settings(&self.settings)?;
        self.settings_load_failed = false;
        Ok(())
    }
    pub(super) fn initialize_services(&mut self) {
        let pointer = data_root().join("selected-engine.json");
        let newer_pointer = fs::metadata(&pointer)
            .and_then(|m| m.modified())
            .ok()
            .is_some_and(|p| {
                fs::metadata(config_path())
                    .and_then(|m| m.modified())
                    .ok()
                    .is_none_or(|c| p > c)
            });
        if newer_pointer
            && let Ok(bytes) = fs::read(&pointer)
            && let Ok(value) = serde_json::from_slice::<serde_json::Value>(&bytes)
            && let Some(root) = value["root"].as_str()
        {
            self.settings.model_store = root.into();
        }
        match installer::load_installation(&self.model_store_root()) {
            Ok(Some(manifest))
                if !Path::new(&self.settings.model).is_file()
                    || !Path::new(&self.settings.llama_server).is_file()
                    || newer_pointer =>
            {
                self.settings.model = manifest.model_path.to_string_lossy().into();
                self.settings.llama_server = manifest.llama_server.to_string_lossy().into();
                self.settings.template = manifest.template_path.to_string_lossy().into();
                self.settings.backend = manifest.backend.clone();
                self.settings.gpu_layers = if manifest.backend == "cpu" { 0 } else { 99 };
            }
            Err(error) => {
                self.service_error = format!("保存済みの準備状況を確認できません: {error}")
            }
            _ => {}
        }
        let _ = self.load_audio_installation();
        if !Path::new(&self.settings.model).is_file()
            || !Path::new(&self.settings.llama_server).is_file()
        {
            self.open_setup();
            self.startup_decided = true;
        }
        self.detect_capacity();
        self.request_service_status();
    }
    pub(super) fn request_service_status(&mut self) {
        if self.health_check_running {
            return;
        }
        self.health_check_running = true;
        self.last_health_check = Instant::now();
        let tx = self.tx.clone();
        let options = self.settings.translation.clone();
        let asr_endpoint = self.settings.asr_endpoint.clone();
        thread::spawn(move || {
            let translation = Translator::new(options)
                .and_then(|t| t.health())
                .map_err(|e| format!("{e:#}"));
            let asr = local_health(&asr_endpoint).unwrap_or(None);
            let _ = tx.send(Event::ServiceStatus { translation, asr });
        });
    }
    pub(super) fn accept_service_status(
        &mut self,
        translation: std::result::Result<String, String>,
        asr: Option<AsrStatus>,
    ) {
        self.health_check_running = false;
        self.asr_ready = asr
            .as_ref()
            .is_some_and(|status| status.matches(&self.settings));
        if self.asr_ready {
            self.asr_started_at = None;
        } else if self
            .asr_started_at
            .is_some_and(|t| t.elapsed() > Duration::from_secs(120))
        {
            self.asr_started_at = None;
            self.audio_start_requested = false;
            self.service_error = "音声認識を読み込めませんでした。設定で音声認識の方式を確認し、準備し直してください。".into();
        }
        if asr.is_some() && !self.asr_ready && self.asr_pid.is_none() && self.audio_start_requested
        {
            self.audio_start_requested = false;
            self.service_error = "別の音声認識環境が動いています。設定を切り替える場合は、その環境を終了してください。".into();
        }
        let translation = translation.and_then(|message| {
            let alias = Path::new(&self.settings.model).file_stem().and_then(|s|s.to_str()).unwrap_or("");
            if self.expect_selected_model && !message.trim().ends_with(alias) {
                Err("別のモデルを使う翻訳環境が動いています。モデルの切替前にその環境を終了してください。".into())
            } else { Ok(message) }
        });
        if let Err(error) = &translation
            && self.expect_selected_model
            && !self.backend_loading
        {
            self.service_error = error.clone();
        }
        self.backend_ready = translation.is_ok();
        if self.backend_ready {
            self.backend_loading = false;
            self.backend_started_at = None;
        }
        if !self.startup_decided {
            self.startup_decided = true;
            if !self.backend_ready
                && Path::new(&self.settings.model).is_file()
                && Path::new(&self.settings.llama_server).is_file()
            {
                self.start_translation_server();
            }
        }
        if !self.backend_ready
            && self
                .backend_started_at
                .is_some_and(|t| t.elapsed() > Duration::from_secs(90))
        {
            self.backend_loading = false;
            self.service_error="モデルを読み込めませんでした。「モデルの準備」で小さいモデルやCPU実行をお試しください。".into();
        }
        if self.audio_start_requested && self.asr_ready && self.backend_ready && !self.audio_enabled
        {
            self.toggle_audio();
        }
    }
    pub(super) fn poll_services(&mut self) {
        let interval = if self.backend_loading || self.audio_start_requested {
            Duration::from_secs(1)
        } else {
            Duration::from_secs(5)
        };
        if self.last_health_check.elapsed() > interval && !self.health_check_running {
            self.request_service_status();
        }
    }
    pub(super) fn start_translation_server(&mut self) {
        if self.backend_loading {
            return;
        }
        if !Path::new(&self.settings.model).is_file()
            || !Path::new(&self.settings.llama_server).is_file()
        {
            self.open_setup();
            return;
        }
        if self.backend_ready && self.backend_pid.is_none() {
            self.service_error="別の翻訳環境が動いています。モデルを切り替える場合は、その環境を終了してから準備してください。".into();
            return;
        }
        if let Some(pid) = self.backend_pid.take()
            && let Some(child) = self.children.iter_mut().find(|c| c.id() == pid)
        {
            kill_owned_child(child);
        }
        let alias = Path::new(&self.settings.model)
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("plamo-3-translate")
            .to_owned();
        let context = if self.settings.model.contains("0.8b") {
            "8192"
        } else if self.settings.model.contains("2b-") {
            "16384"
        } else {
            "32768"
        };
        let mut cmd = Command::new(&self.settings.llama_server);
        cmd.current_dir(app_root()).args([
            "-m",
            &self.settings.model,
            "--host",
            "127.0.0.1",
            "--port",
            "17860",
            "--alias",
            &alias,
            "-c",
            context,
            "--parallel",
            "2",
            "-ngl",
            &self.settings.gpu_layers.to_string(),
            "--flash-attn",
            "auto",
            "--no-webui",
        ]);
        if Path::new(&self.settings.template).is_file() {
            cmd.arg("--chat-template-file")
                .arg(&self.settings.template)
                .arg("--no-prefill-assistant");
        }
        self.backend_ready = false;
        self.expect_selected_model = true;
        self.backend_loading = true;
        self.backend_started_at = Some(Instant::now());
        self.service_error.clear();
        if let Err(error) = self.spawn_service(cmd, "plamo-server") {
            self.backend_loading = false;
            self.service_error = error.to_string();
        }
    }
    pub(super) fn start_asr(&mut self) {
        if self.asr_ready {
            return;
        }
        if !self.audio_environment_ready() {
            self.audio_confirmation = true;
            return;
        }
        if let Some(pid) = self.asr_pid
            && let Some(child) = self.children.iter_mut().find(|c| c.id() == pid)
            && matches!(child.try_wait(), Ok(None))
        {
            if self.asr_started_at.is_some() {
                return;
            }
            kill_owned_child(child);
        }
        let mut cmd = Command::new(&self.settings.python);
        cmd.current_dir(app_root())
            .arg(app_root().join("scripts/asr_server.py"))
            .args([
                "--backend",
                &self.settings.asr_backend,
                "--model",
                &self.settings.asr_model,
                "--device",
                &self.settings.asr_device,
                "--port",
                "17861",
            ]);
        if let Err(error) = self.spawn_service(cmd, "asr-server") {
            self.service_error = error.to_string();
            self.audio_start_requested = false;
        }
    }
    fn spawn_service(&mut self, mut cmd: Command, name: &str) -> Result<()> {
        let dir = data_root().join("logs");
        fs::create_dir_all(&dir)?;
        let log = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(dir.join(format!("{name}.log")))?;
        cmd.stdout(log.try_clone()?).stderr(log);
        command_hidden(&mut cmd);
        let child = cmd.spawn()?;
        if name == "plamo-server" {
            self.backend_pid = Some(child.id());
        } else {
            self.asr_pid = Some(child.id());
            self.asr_started_at = Some(Instant::now());
        }
        self.children.push(child);
        Ok(())
    }
    pub(super) fn audio_environment_ready(&self) -> bool {
        if !Path::new(&self.settings.python).is_file()
            || !app_root().join("scripts/asr_server.py").is_file()
        {
            return false;
        }
        if Path::new(&self.settings.asr_model).is_dir() {
            return true;
        }
        let repo =
            if self.settings.asr_backend == "whisper" && !self.settings.asr_model.contains('/') {
                format!("Systran/faster-whisper-{}", self.settings.asr_model)
            } else {
                self.settings.asr_model.clone()
            };
        let cache = std::env::var_os("HF_HUB_CACHE")
            .map(PathBuf::from)
            .or_else(|| {
                directories::BaseDirs::new().map(|d| d.home_dir().join(".cache/huggingface/hub"))
            });
        cache.is_some_and(|p| {
            p.join(format!("models--{}", repo.replace('/', "--")))
                .join("snapshots")
                .is_dir()
        })
    }
    pub(super) fn load_audio_installation(&mut self) -> Result<()> {
        let path = data_root().join("audio-installation.json");
        if !path.exists() {
            return Ok(());
        }
        let data: serde_json::Value = serde_json::from_slice(&fs::read(path)?)?;
        anyhow::ensure!(data["version"] == 1, "音声認識の保存情報が対応外です");
        let python = data["python_path"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("Pythonの保存情報がありません"))?;
        let model = data["model_path"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("音声モデルの保存情報がありません"))?;
        anyhow::ensure!(
            Path::new(python).is_file() && Path::new(model).is_dir(),
            "音声認識のファイルが見つかりません"
        );
        self.settings.python = python.into();
        self.settings.asr_model = model.into();
        self.settings.asr_backend = data["backend"].as_str().unwrap_or("whisper").into();
        self.settings.asr_device = data["device"].as_str().unwrap_or("cpu").into();
        Ok(())
    }
    pub(super) fn prepare_audio(&mut self) {
        if !self.backend_ready {
            self.open_setup();
            self.status = "先に翻訳モデルを準備してください。".into();
            return;
        }
        if self.audio_environment_ready() {
            self.start_asr();
        } else {
            self.audio_confirmation = true;
        }
    }
    fn begin_audio_install(&mut self) {
        if self.audio_installing {
            return;
        }
        let script = app_root().join("scripts/setup-audio.ps1");
        if !script.is_file() {
            self.service_error =
                "音声認識の準備ファイルがありません。配布パッケージを確認してください。".into();
            return;
        }
        self.audio_installing = true;
        self.audio_cancel = Arc::new(AtomicBool::new(false));
        self.service_error.clear();
        let tx = self.tx.clone();
        let root = data_root();
        let backend = self.settings.asr_backend.clone();
        let device = if self.settings.asr_device == "cuda" {
            "cuda"
        } else {
            "cpu"
        };
        let cancel = self.audio_cancel.clone();
        thread::spawn(move || {
            let result=(||->Result<()>{fs::create_dir_all(root.join("logs"))?;let log=fs::OpenOptions::new().create(true).append(true).open(root.join("logs/audio-setup.log"))?;
                let mut cmd=Command::new("powershell.exe");cmd.args(["-NoProfile","-ExecutionPolicy","Bypass","-File"]).arg(script).arg("-InstallRoot").arg(root).arg("-Backend").arg(backend).arg("-Device").arg(device).stdout(log.try_clone()?).stderr(log);command_hidden(&mut cmd);let mut child=cmd.spawn()?;
                loop {
                    if cancel.load(Ordering::Relaxed) {
                        kill_owned_child(&mut child);
                        anyhow::bail!("音声認識の準備を中止しました。取得済みのデータは再利用できます。");
                    }
                    if let Some(status) = child.try_wait()? {
                        anyhow::ensure!(status.success(), "音声認識の準備に失敗しました。詳細は音声セットアップのログで確認できます。");
                        return Ok(());
                    }
                    thread::sleep(Duration::from_millis(200));
                }
            })().map_err(|e|format!("{e:#}"));
            let _ = tx.send(Event::AudioPrepared(result));
        });
    }
    pub(super) fn audio_setup_ui(&mut self, ctx: &egui::Context) {
        if !self.audio_confirmation && !self.audio_installing {
            return;
        }
        let width = (ctx.content_rect().width() - 48.0).clamp(260.0, 510.0);
        egui::Window::new("音声認識の追加準備").collapsible(false).resizable(false).default_width(width).anchor(egui::Align2::CENTER_CENTER,[0.0,0.0]).show(ctx,|ui|{
            ui.label("再生中の音声を、このPCで文字起こしする機能です。YouTube字幕だけを使う場合は不要です。");
            ui.add_space(12.0);ui.label(if self.settings.asr_backend=="qwen"{"Qwen3-ASR · 多言語 / GPU対応"}else{"Whisper · 多言語 / CPU"});
            ui.label(if self.settings.asr_backend=="qwen"{"音声モデルと実行環境の取得に数GBの通信と空き容量が必要です。"}else{"小型の音声モデルと、音声用の実行環境を追加します。"});
            ui.label("準備中も音声の取得は始まりません。インストールはアプリ専用フォルダーだけで行います。");
            ui.add_space(12.0);
            if self.audio_installing {
                ui.horizontal(|ui| { ui.spinner(); ui.label("音声認識を準備しています…"); });
                if ui.button("中止").clicked() { self.audio_cancel.store(true,Ordering::Relaxed); }
            } else {
                ui.horizontal_wrapped(|ui| {
                    if ui.add(ui::primary("追加データを取得").min_size(Vec2::new(170.0,36.0))).clicked() {
                        self.begin_audio_install();
                    }
                    if ui.button("今はしない").clicked() { self.audio_confirmation=false; self.audio_start_requested=false; }
                });
            }
            if !self.service_error.is_empty(){ui.colored_label(Color32::from_rgb(240,122,122),&self.service_error);}
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn service_identity_must_match_a_newly_selected_model() {
        let mut app = App::inert();
        app.settings.model = "models/plamo-3-translate-8b-Q4_K_M.gguf".into();
        app.expect_selected_model = true;
        app.startup_decided = true;
        app.accept_service_status(Ok("接続済み: plamo-3-translate-2b-Q4_K_M".into()), None);
        assert!(!app.backend_ready);
        app.accept_service_status(Ok("接続済み: plamo-3-translate-8b-Q4_K_M".into()), None);
        assert!(app.backend_ready);
    }
    #[test]
    fn audio_health_checks_the_selected_backend_model_and_device() {
        let mut app = App::inert();
        app.startup_decided = true;
        app.settings.asr_backend = "whisper".into();
        app.settings.asr_model = "base".into();
        app.settings.asr_device = "cpu".into();
        let status = || AsrStatus {
            backend: "qwen".into(),
            model: "base".into(),
            device: "cpu".into(),
        };
        app.accept_service_status(Ok("接続済み".into()), Some(status()));
        assert!(!app.asr_ready);
        let mut wrong_model = status();
        wrong_model.backend = "whisper".into();
        wrong_model.model = "small".into();
        assert!(!wrong_model.matches(&app.settings));
        let right = AsrStatus {
            backend: "whisper".into(),
            model: "base".into(),
            device: "cpu".into(),
        };
        app.accept_service_status(Ok("接続済み".into()), Some(right));
        assert!(app.asr_ready);
        app.settings.asr_device = "cuda".into();
        let wrong_device = AsrStatus {
            backend: "whisper".into(),
            model: "base".into(),
            device: "cpu".into(),
        };
        assert!(!wrong_device.matches(&app.settings));
    }
    #[test]
    fn preparing_audio_requires_download_consent_and_does_not_record() {
        let mut app = App::inert();
        app.backend_ready = true;
        app.settings.python = std::env::temp_dir()
            .join(format!("missing-audio-{}", uuid::Uuid::new_v4()))
            .to_string_lossy()
            .into();
        app.prepare_audio();
        assert!(app.audio_confirmation);
        assert!(!app.audio_installing && !app.audio_enabled && !app.audio_start_requested);
        assert!(app.audio.is_none());
    }
}
