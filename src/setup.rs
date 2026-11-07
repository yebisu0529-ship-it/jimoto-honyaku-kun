//! First-run preparation uses local capacity measurements and a pinned catalog.
use super::*;
use plamo_translate::{
    hardware::{self, Gpu, Hardware},
    installer::{self, InstallRequest, InstallResult, Progress},
    model_catalog::{self, Model, Policy, Recommendation},
};
use std::collections::HashSet;

/// How often the GPU memory meter is measured again while the screen is open.
const GPU_POLL_INTERVAL: Duration = Duration::from_secs(3);
/// A measurement that has not reported back by now is treated as lost.
const GPU_POLL_TIMEOUT: Duration = Duration::from_secs(10);

pub(super) struct SetupState {
    pub hardware: Option<Hardware>,
    pub recommendation: Option<Recommendation>,
    pub selected_id: String,
    pub detecting: bool,
    pub installing: bool,
    pub progress: Option<Progress>,
    pub error: String,
    pub completed: bool,
    pub cancelled: bool,
    pub cancel: Arc<AtomicBool>,
    pub force_cpu: bool,
    /// Catalog models whose file is already in the model store.
    pub saved: HashSet<String>,
    /// The catalog model `settings.model` points at, while that file exists.
    pub in_use: Option<Model>,
    pub gpu_polling: bool,
    pub gpu_polled_at: Instant,
}
impl Default for SetupState {
    fn default() -> Self {
        Self {
            hardware: None,
            recommendation: None,
            selected_id: String::new(),
            detecting: false,
            installing: false,
            progress: None,
            error: String::new(),
            completed: false,
            cancelled: false,
            cancel: Arc::new(AtomicBool::new(false)),
            force_cpu: false,
            saved: HashSet::new(),
            in_use: None,
            gpu_polling: false,
            gpu_polled_at: Instant::now(),
        }
    }
}

/// The models whose complete file is in the store. The installer renames a download
/// into place only after it is verified, so a file of the right size is saved.
fn saved_models(store: &Path, models: &[Model]) -> HashSet<String> {
    let folder = store.join("models");
    models
        .iter()
        .filter(|m| {
            fs::metadata(folder.join(&m.revision).join(&m.filename))
                .is_ok_and(|file| file.is_file() && file.len() == m.bytes)
        })
        .map(|m| m.id.clone())
        .collect()
}

/// The catalog model a settings path names, while the file is there. A fresh
/// install's default path names a model it does not have yet.
fn in_use_model(path: &Path, models: Vec<Model>) -> Option<Model> {
    let name = path.file_name().and_then(|s| s.to_str())?;
    models
        .into_iter()
        .find(|m| m.filename == name)
        .filter(|_| path.is_file())
}

/// Whether a GPU memory reading should start now. The app asks every frame, so
/// nothing runs while another tab is open or the window is minimized.
fn gpu_poll_due(visible: bool, has_gpu: bool, running: bool, since_last: Duration) -> bool {
    visible
        && has_gpu
        && since_last >= GPU_POLL_INTERVAL
        && (!running || since_last >= GPU_POLL_TIMEOUT)
}
pub(super) fn install_root() -> PathBuf {
    data_root().join("engine")
}
pub(super) fn bytes(value: u64) -> String {
    if value >= 1_073_741_824 {
        format!("{:.1} GB", value as f64 / 1_073_741_824.0)
    } else {
        format!("{} MB", value / 1_048_576)
    }
}
fn measured(value: Option<u64>) -> String {
    value.map(bytes).unwrap_or_else(|| "確認できません".into())
}

impl App {
    pub(super) fn model_store_root(&self) -> PathBuf {
        if self.settings.model_store.is_empty() {
            install_root()
        } else {
            PathBuf::from(&self.settings.model_store)
        }
    }
    pub(super) fn open_setup(&mut self) {
        self.tab = Tab::Setup;
        self.refresh_store_state();
        if self.setup.hardware.is_none() && !self.setup.detecting {
            self.detect_capacity();
        }
    }
    /// Which models are saved and which one is in use. A few file lookups, so
    /// only when the answer can have changed, never once per frame.
    pub(super) fn refresh_store_state(&mut self) {
        let Ok(catalog) = model_catalog::bundled() else {
            return;
        };
        self.setup.saved = saved_models(&self.model_store_root(), &catalog.models);
        self.setup.in_use = in_use_model(Path::new(&self.settings.model), catalog.models);
    }
    /// The in-use model is on the GPU, or loading there.
    pub(super) fn gpu_resident(&self) -> bool {
        (self.backend_ready || self.backend_loading) && self.settings.gpu_layers > 0
    }
    /// GPU memory the loaded model holds. Choosing another model releases it.
    pub(super) fn held_gpu_bytes(&self) -> u64 {
        if !self.gpu_resident() {
            return 0;
        }
        self.setup
            .in_use
            .as_ref()
            .map_or(0, model_catalog::inference_bytes)
    }
    /// Start a GPU memory reading on a background thread when one is due.
    pub(super) fn poll_gpu_memory(&mut self, ctx: &egui::Context) {
        let visible = self.tab == Tab::Setup
            && !self.window_mode.is_mini()
            && ctx.input(|input| input.viewport().minimized != Some(true));
        let has_gpu = self
            .setup
            .recommendation
            .as_ref()
            .is_some_and(|r| r.gpu.is_some());
        if gpu_poll_due(
            visible,
            has_gpu,
            self.setup.gpu_polling,
            self.setup.gpu_polled_at.elapsed(),
        ) {
            self.start_gpu_poll();
        }
    }
    fn start_gpu_poll(&mut self) {
        let Some(hardware) = &self.setup.hardware else {
            return;
        };
        let mut gpus = hardware.gpus.clone();
        self.setup.gpu_polling = true;
        self.setup.gpu_polled_at = Instant::now();
        let tx = self.tx.clone();
        thread::spawn(move || {
            hardware::refresh_gpu_memory(&mut gpus);
            let _ = tx.send(Event::GpuMemory(gpus));
        });
    }
    pub(super) fn accept_gpu_memory(&mut self, fresh: Vec<Gpu>) {
        self.setup.gpu_polling = false;
        let Some(hardware) = &mut self.setup.hardware else {
            return;
        };
        // A re-check may have replaced the list while this reading ran.
        if hardware.gpus.len() != fresh.len() {
            return;
        }
        for (gpu, fresh) in hardware.gpus.iter_mut().zip(fresh) {
            if gpu.name == fresh.name {
                gpu.free_vram_bytes = fresh.free_vram_bytes;
                gpu.memory_telemetry = fresh.memory_telemetry;
            }
        }
        self.refresh_recommendation();
    }
    pub(super) fn detect_capacity(&mut self) {
        if self.setup.installing || self.setup.detecting {
            return;
        }
        self.setup.detecting = true;
        self.setup.error.clear();
        let tx = self.tx.clone();
        let target = self.model_store_root();
        thread::spawn(move || {
            let result = hardware::detect(&target).map_err(|e| format!("{e:#}"));
            let _ = tx.send(Event::Hardware(result));
        });
    }
    /// Plan again from the current measurements. The user's choice stays.
    pub(super) fn refresh_recommendation(&mut self) {
        let Some(hardware) = &self.setup.hardware else {
            return;
        };
        let mut measured = hardware.clone();
        if self.setup.force_cpu {
            measured.gpus.clear();
        }
        self.setup.recommendation = Some(model_catalog::recommend_replacing(
            &measured,
            Policy::Realtime,
            self.held_gpu_bytes(),
        ));
    }
    /// A new measurement or a changed CPU setting: start over from the preselected row.
    pub(super) fn recalculate_recommendation(&mut self) {
        self.refresh_recommendation();
        if let Some(recommendation) = &self.setup.recommendation {
            let picker = picker::Picker::new(
                recommendation,
                self.setup.in_use.as_ref(),
                &self.setup.saved,
            );
            self.setup.selected_id = picker.default_choice(self.setup.in_use.as_ref()).to_owned();
            self.setup.cancelled = false;
        }
    }
    pub(super) fn setup_model(&self) -> Option<Model> {
        self.setup
            .recommendation
            .as_ref()
            .and_then(|r| {
                r.alternatives
                    .iter()
                    .find(|a| a.model.id == self.setup.selected_id)
            })
            .map(|a| a.model.clone())
    }
    fn setup_model_fits(&self) -> bool {
        self.setup
            .recommendation
            .as_ref()
            .and_then(|r| {
                r.alternatives
                    .iter()
                    .find(|a| a.model.id == self.setup.selected_id)
            })
            .is_some_and(|a| a.fits)
    }
    fn begin_install(&mut self) {
        if self.setup.installing || !self.setup_model_fits() {
            return;
        }
        let Some(model) = self.setup_model() else {
            return;
        };
        let Some(recommendation) = &self.setup.recommendation else {
            return;
        };
        self.setup.installing = true;
        self.setup.completed = false;
        self.setup.cancelled = false;
        self.setup.error.clear();
        self.setup.progress = None;
        self.setup.cancel = Arc::new(AtomicBool::new(false));
        let request = InstallRequest {
            repo: model.repo,
            revision: model.revision,
            filename: model.filename,
            bytes: model.bytes,
            sha256: model.sha256,
            backend: recommendation.backend.as_str().into(),
            install_root: self.model_store_root(),
        };
        let tx = self.tx.clone();
        let cancel = self.setup.cancel.clone();
        thread::spawn(move || {
            let result = installer::install(request, cancel, |p| {
                let _ = tx.try_send(Event::InstallProgress(p));
            })
            .map_err(|e| format!("{e:#}"));
            let _ = tx.send(Event::Installed(result));
        });
    }
    pub(super) fn adopt_installation(&mut self, result: InstallResult) {
        self.settings.model = result.model_path.to_string_lossy().into();
        self.settings.llama_server = result.llama_server.to_string_lossy().into();
        self.settings.template = result.template_path.to_string_lossy().into();
        self.settings.backend = result.backend.clone();
        self.settings.gpu_layers = if result.backend == "cpu" { 0 } else { 99 };
        if let Some(rec) = &self.setup.recommendation
            && self.settings.asr_backend != "nemotron"
        {
            self.settings.asr_backend = rec.asr_backend.clone();
            self.settings.asr_device = rec.asr_device.clone();
            self.settings.asr_model = if rec.asr_backend == "whisper" {
                "base".into()
            } else {
                "Qwen/Qwen3-ASR-0.6B-hf".into()
            };
        }
        self.setup.installing = false;
        self.setup.completed = true;
        self.refresh_store_state();
        if self.settings.english_punctuation {
            self.prepare_paragraph_tools();
        }
        self.setup.progress = None;
        self.asr_ready = false;
        self.asr_started_at = None;
        self.audio_start_requested = false;
        let pointer = serde_json::json!({"root":self.model_store_root()});
        let _ = fs::create_dir_all(data_root());
        let _ = fs::write(
            data_root().join("selected-engine.json"),
            pointer.to_string(),
        );
        match self.persist_preferences() {
            Ok(()) => self.status = "翻訳モデルの準備ができました。".into(),
            Err(e) => {
                self.service_error =
                    format!("モデルは保存できましたが、設定の保存に失敗しました: {e}")
            }
        }
        let external_running = self.backend_ready && self.backend_pid.is_none();
        self.expect_selected_model = true;
        if self.audio_enabled {
            self.stop_audio();
        }
        self.backend_ready = false;
        if external_running {
            self.service_error="別の翻訳環境が動いています。終了してから設定の「モデルを読み込む」を押してください。".into();
        } else {
            self.start_translation_server();
        }
    }
    pub(super) fn current_model_label(&self) -> String {
        let filename = Path::new(&self.settings.model)
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("");
        model_catalog::bundled()
            .ok()
            .and_then(|c| c.models.into_iter().find(|m| m.filename == filename))
            .map(|m| m.label)
            .unwrap_or_else(|| "PLaMo 3 Translate".into())
    }
    pub(super) fn setup_summary(&self) -> String {
        self.current_model_label()
    }
    pub(super) fn connection_label(&self) -> &'static str {
        if self.backend_ready {
            "準備完了"
        } else if self.backend_loading {
            "モデルを読み込み中"
        } else if self.setup.installing {
            "ダウンロード中"
        } else {
            "準備が必要"
        }
    }
    pub(super) fn open_data_folder(&mut self) {
        let _ = Command::new("explorer.exe").arg(data_root()).spawn();
    }
    pub(super) fn setup_ui(&mut self, ui: &mut egui::Ui) {
        // The step that moves the user forward stays in view under the form,
        // whatever the window height.
        let chosen = self.setup_model().or_else(|| {
            self.setup
                .recommendation
                .as_ref()
                .map(|r| r.selected.clone())
        });
        if let Some(model) = chosen {
            egui::TopBottomPanel::bottom("setup-actions")
                .frame(egui::Frame::new().inner_margin(egui::Margin {
                    top: theme::SPACE_3 as i8,
                    ..Default::default()
                }))
                .show_inside(ui, |ui| self.setup_actions(ui, &model));
        }
        egui::ScrollArea::vertical()
            .id_salt("model-setup-scroll")
            .show(ui, |ui| {
                ui.label(theme::title(if self.setup.completed {
                    "翻訳の準備ができました"
                } else {
                    "このPCに合うモデルを準備"
                }));
                ui.label(theme::caption(
                    "大きいモデルほど自然に訳せますが、GPUのメモリを多く使います。",
                ));
                ui.add_space(theme::SPACE_4);
                self.setup_steps(ui);
                ui.add_space(theme::SPACE_4);
                if self.setup.detecting {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label("GPU・メモリ・空き容量を確認しています…");
                    });
                }
                if self.setup.hardware.is_none()
                    && !self.setup.detecting
                    && ui
                        .add(ui::primary("PCの情報を確認").min_size(Vec2::new(190.0, theme::CONTROL_H_LARGE)))
                        .clicked()
                {
                    self.detect_capacity();
                }
                if self.setup.recommendation.is_some() {
                    self.picker_ui(ui);
                    ui.add_space(theme::SPACE_3);
                    self.setup_pc_details(ui);
                    ui.add_space(theme::SPACE_4);
                    ui.add_enabled_ui(!self.setup.installing, |ui| {
                        ui.checkbox(
                            &mut self.settings.english_punctuation,
                            "英語の文章区切り補助も準備する（追加で約86MB）",
                        );
                    });
                    ui.label(theme::caption(
                        "原文を段落につなぎ、意味の区切りで訳文をまとめます。補助モデルはApache 2.0です。",
                    ));
                    if self.paragraph_tools.preparing || self.paragraph_tools.loading {
                        ui.horizontal(|ui| {
                            ui.spinner();
                            ui.label("英語の文章区切り補助を準備しています。翻訳は先に始められます。");
                        });
                        if let Some(p) = &self.paragraph_tools.progress {
                            ui.label(&p.message);
                        }
                    }
                    ui.add_space(theme::SPACE_3);
                    ui.hyperlink_to(
                        "PLaMoのモデル利用条件",
                        "https://plamo.preferredai.jp/info/plamo-community-license-ja",
                    );
                    ui.label(theme::caption(
                        "モデルはPFN公式、実行環境はllama.cpp公式から取得します。音声認識の追加準備はライブ画面から行えます。",
                    ));
                    self.setup_error_details(ui);
                } else {
                    self.setup_error_summary(ui);
                    self.setup_error_details(ui);
                }
            });
    }

    /// What was measured, the model store and the CPU switch, folded away until wanted.
    fn setup_pc_details(&mut self, ui: &mut egui::Ui) {
        let Some(hardware) = self.setup.hardware.clone() else {
            return;
        };
        let warnings = self
            .setup
            .recommendation
            .as_ref()
            .map(|r| r.warnings.clone())
            .unwrap_or_default();
        egui::CollapsingHeader::new("PC情報と保存先").show(ui, |ui| {
            let gpus: Vec<_> = hardware
                .gpus
                .iter()
                .filter(|g| !g.is_software)
                .map(|g| g.name.as_str())
                .collect();
            ui.label(format!(
                "GPU {}",
                if gpus.is_empty() {
                    "なし".into()
                } else {
                    gpus.join(" / ")
                }
            ));
            ui.label(format!(
                "CPU {}スレッド / メモリ合計 {} / 空きメモリ {}",
                hardware.cpu_logical,
                measured(hardware.ram_total_bytes),
                measured(hardware.ram_available_bytes)
            ));
            ui.label(format!("空き容量 {}", measured(hardware.disk_free_bytes)));
            ui.label(format!(
                "モデルの保存先: {}",
                self.model_store_root().display()
            ));
            ui.horizontal_wrapped(|ui| {
                if ui
                    .add_enabled(!self.setup.installing, egui::Button::new("保存先を変更"))
                    .clicked()
                    && let Some(folder) = rfd::FileDialog::new().pick_folder()
                {
                    self.settings.model_store =
                        folder.join("地元翻訳くん/engine").to_string_lossy().into();
                    self.detect_capacity();
                }
                if ui
                    .add_enabled(!self.setup.installing, egui::Button::new("再確認"))
                    .clicked()
                {
                    self.detect_capacity();
                }
            });
            let old = self.setup.force_cpu;
            ui.add_enabled_ui(!self.setup.installing, |ui| {
                ui::toggle(ui, &mut self.setup.force_cpu, "GPUを使わずCPUで実行する");
            });
            if old != self.setup.force_cpu {
                self.recalculate_recommendation();
            }
            for warning in &warnings {
                ui.colored_label(theme::WARNING, warning);
            }
        });
        ui.label(theme::caption("PC情報は外部へ送信しません。"));
    }

    /// Current step in the accent colour, finished steps readable, later ones muted.
    fn setup_steps(&self, ui: &mut egui::Ui) {
        let checked = self.setup.hardware.is_some();
        let downloading = self.setup.installing || self.setup.completed;
        let steps = [
            ("1", "PCを確認", !checked, checked),
            ("2", "モデルを選択", checked && !downloading, downloading),
            ("3", "ダウンロード", downloading, false),
        ];
        ui.horizontal_wrapped(|ui| {
            for (index, (number, label, current, done)) in steps.into_iter().enumerate() {
                if index > 0 {
                    ui.label(RichText::new("→").color(theme::TEXT_MUTED));
                }
                let color = if current {
                    ACCENT
                } else if done {
                    theme::TEXT_SECONDARY
                } else {
                    theme::TEXT_MUTED
                };
                ui.label(
                    RichText::new(format!("{number}  {label}"))
                        .color(color)
                        .strong(),
                );
            }
        });
    }

    /// Download, progress or the way into the app, then a one-line failure.
    fn setup_actions(&mut self, ui: &mut egui::Ui, model: &Model) {
        if self.setup.installing {
            if let Some(p) = &self.setup.progress {
                ui.label(&p.message);
                if let Some(total) = p.total_bytes.filter(|t| *t > 0) {
                    ui.add(
                        egui::ProgressBar::new(
                            (p.downloaded_bytes as f64 / total as f64).clamp(0.0, 1.0) as f32,
                        )
                        .text(format!(
                            "{} / {}",
                            bytes(p.downloaded_bytes),
                            bytes(total)
                        )),
                    );
                } else {
                    ui.spinner();
                }
            } else {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label("準備を始めています…");
                });
            }
            if self.setup.cancel.load(Ordering::Relaxed) {
                ui.label("中止しています。取得途中のデータは残ります。");
            } else if ui.button("中止").clicked() {
                self.setup.cancel.store(true, Ordering::Relaxed);
            }
        } else if self.setup.completed {
            ui.colored_label(ACCENT, "整合性の確認と保存が完了しました。");
            ui.horizontal(|ui| {
                if ui
                    .add_enabled(
                        self.backend_ready,
                        ui::primary("翻訳を始める")
                            .min_size(Vec2::new(200.0, theme::CONTROL_H_LARGE)),
                    )
                    .clicked()
                {
                    self.tab = Tab::Text;
                }
                if self.backend_loading {
                    ui.spinner();
                    ui.label("モデルを読み込み中…");
                }
            });
        } else {
            if self.setup.cancelled {
                ui.label("中止しました。続きから再開するか、別のモデルを選べます。");
            }
            let label = if self.setup.saved.contains(&model.id) {
                "このモデルに切り替える".to_owned()
            } else if self.setup.error.is_empty() && !self.setup.cancelled {
                format!("ダウンロードして準備（{}）", bytes(model.bytes))
            } else {
                format!("続きから再開（{}）", bytes(model.bytes))
            };
            if ui
                .add_enabled(
                    self.setup_model_fits(),
                    ui::primary(&label).min_size(Vec2::new(260.0, theme::CONTROL_H_LARGE)),
                )
                .clicked()
            {
                self.begin_install();
            }
            if !self.setup_model_fits() {
                ui.colored_label(
                    theme::WARNING,
                    "容量の条件を確認できません。空き容量を増やすか、別のモデルを選んでください。",
                );
            }
        }
        self.setup_error_summary(ui);
    }

    /// One line per failure, so the fixed bar never pushes its button away.
    fn setup_error_summary(&self, ui: &mut egui::Ui) {
        if !self.setup.error.is_empty() {
            ui.colored_label(
                theme::ERROR,
                "準備を完了できませんでした。取得済みのデータは保持しています。",
            );
        }
        if !self.service_error.is_empty() {
            ui.add(
                egui::Label::new(RichText::new(&self.service_error).color(theme::ERROR)).truncate(),
            )
            .on_hover_text(&self.service_error);
        }
    }

    /// The full failure text can be long, so it scrolls with the form.
    fn setup_error_details(&self, ui: &mut egui::Ui) {
        if !self.setup.error.is_empty() {
            egui::CollapsingHeader::new("エラーの詳細").show(ui, |ui| {
                ui.label(&self.setup.error);
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const GIB: u64 = 1_073_741_824;
    /// A PC whose one GPU no real adapter shares a name with, so a real reading leaves it alone.
    fn app_with_fixture_gpu(total: u64, used: u64) -> App {
        let mut app = App::inert();
        app.tab = Tab::Setup;
        app.setup.hardware = Some(Hardware {
            os_arch: "windows-x86_64".into(),
            cpu_logical: 16,
            ram_total_bytes: Some(32 * GIB),
            ram_available_bytes: Some(20 * GIB),
            disk_free_bytes: Some(120 * GIB),
            disk_target: "fixture".into(),
            gpus: vec![Gpu {
                name: "Fixture GPU 9000".into(),
                vendor: "nvidia".into(),
                dedicated_vram_bytes: Some(total),
                free_vram_bytes: Some(total - used),
                driver_version: Some("570.00".into()),
                cuda_eligible: true,
                vulkan_eligible: true,
                is_software: false,
                memory_telemetry: "nvidia-smi".into(),
            }],
            warnings: vec![],
        });
        app.recalculate_recommendation();
        app
    }
    fn fits(app: &App, id: &str) -> bool {
        app.setup
            .recommendation
            .as_ref()
            .unwrap()
            .alternatives
            .iter()
            .find(|a| a.model.id == id)
            .unwrap()
            .fits
    }
    #[test]
    fn a_reading_is_due_every_three_seconds_while_the_screen_is_open_one_at_a_time() {
        let seconds = Duration::from_secs;
        assert!(gpu_poll_due(true, true, false, seconds(3)));
        assert!(!gpu_poll_due(
            true,
            true,
            false,
            Duration::from_millis(2900)
        ));
        assert!(
            !gpu_poll_due(false, true, false, seconds(60)),
            "another tab is open"
        );
        assert!(
            !gpu_poll_due(true, false, false, seconds(60)),
            "no GPU, no meter"
        );
        assert!(!gpu_poll_due(true, true, true, seconds(5)), "one at a time");
        assert!(
            gpu_poll_due(true, true, true, seconds(10)),
            "a reading that never came back does not stop the meter for good"
        );
    }
    #[test]
    fn nothing_is_measured_from_another_tab_or_a_minimized_window() {
        let mut app = app_with_fixture_gpu(24 * GIB, 5 * GIB);
        app.setup.gpu_polled_at = Instant::now() - Duration::from_secs(60);
        let context = egui::Context::default();
        for tab in [Tab::Text, Tab::Live, Tab::Files, Tab::Settings] {
            app.tab = tab;
            app.poll_gpu_memory(&context);
            assert!(!app.setup.gpu_polling, "started from another tab");
        }
        app.tab = Tab::Setup;
        let mut input = egui::RawInput::default();
        input
            .viewports
            .get_mut(&egui::ViewportId::ROOT)
            .unwrap()
            .minimized = Some(true);
        let _ = context.run(input, |ctx| app.poll_gpu_memory(ctx));
        assert!(!app.setup.gpu_polling, "started while minimized");
        assert!(app.rx.try_recv().is_err());
    }
    #[test]
    fn drawing_the_screen_never_starts_a_measurement() {
        let mut app = app_with_fixture_gpu(24 * GIB, 5 * GIB);
        app.setup.gpu_polled_at = Instant::now() - Duration::from_secs(60);
        let context = egui::Context::default();
        ui::configure_theme(&context);
        for frame in 0..3 {
            let _ = context.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(860.0, 620.0),
                    )),
                    time: Some(f64::from(frame) / 60.0),
                    ..Default::default()
                },
                |ctx| app.render_ui(ctx),
            );
        }
        assert!(!app.setup.gpu_polling && app.rx.try_recv().is_err());
    }
    #[test]
    fn a_reading_runs_off_the_ui_thread_and_arrives_as_an_event() {
        let mut app = app_with_fixture_gpu(24 * GIB, 5 * GIB);
        app.start_gpu_poll();
        assert!(app.setup.gpu_polling);
        let event = app
            .rx
            .recv_timeout(Duration::from_secs(10))
            .expect("the reading is sent over the app's event channel");
        assert!(matches!(event, Event::GpuMemory(ref gpus) if gpus.len() == 1));
        app.tx.send(event).unwrap();
        app.process(&egui::Context::default());
        assert!(!app.setup.gpu_polling);
        assert_eq!(
            app.setup.hardware.as_ref().unwrap().gpus[0].free_vram_bytes,
            Some(19 * GIB),
            "a card the probes do not know keeps its last value"
        );
    }
    #[test]
    fn a_new_reading_updates_what_fits_and_keeps_the_users_choice() {
        let mut app = app_with_fixture_gpu(24 * GIB, 5 * GIB);
        assert!(fits(&app, "plamo3-8b-q4_k_m"));
        app.setup.selected_id = "plamo3-2b-q4_k_m".into();
        let mut fresh = app.setup.hardware.as_ref().unwrap().gpus.clone();
        fresh[0].free_vram_bytes = Some(5 * GIB);
        app.setup.gpu_polling = true;
        app.tx.send(Event::GpuMemory(fresh)).unwrap();
        app.process(&egui::Context::default());
        assert!(!app.setup.gpu_polling);
        assert_eq!(
            app.setup.hardware.as_ref().unwrap().gpus[0].free_vram_bytes,
            Some(5 * GIB)
        );
        assert!(
            !fits(&app, "plamo3-8b-q4_k_m"),
            "another program took the memory"
        );
        assert!(fits(&app, "plamo3-2b-q4_k_m"));
        assert_eq!(app.setup.selected_id, "plamo3-2b-q4_k_m");
    }
    #[test]
    fn a_reading_for_a_different_list_of_gpus_is_ignored() {
        let mut app = app_with_fixture_gpu(24 * GIB, 5 * GIB);
        let mut fresh = app.setup.hardware.as_ref().unwrap().gpus.clone();
        fresh[0].free_vram_bytes = Some(1);
        fresh.push(fresh[0].clone());
        app.tx.send(Event::GpuMemory(fresh)).unwrap();
        app.process(&egui::Context::default());
        assert_eq!(
            app.setup.hardware.as_ref().unwrap().gpus[0].free_vram_bytes,
            Some(19 * GIB)
        );
    }
    fn small_model(id: &str, filename: &str, bytes: u64) -> Model {
        Model {
            id: id.into(),
            filename: filename.into(),
            bytes,
            ..model_catalog::bundled().unwrap().models.remove(0)
        }
    }
    #[test]
    fn a_model_is_saved_only_when_its_complete_file_is_in_the_store() {
        let store = tempfile::tempdir().unwrap();
        let models = [
            small_model("whole", "whole.gguf", 5),
            small_model("partial", "partial.gguf", 5),
            small_model("missing", "missing.gguf", 5),
        ];
        let folder = store.path().join("models").join(&models[0].revision);
        fs::create_dir_all(&folder).unwrap();
        fs::write(folder.join("whole.gguf"), b"12345").unwrap();
        fs::write(folder.join("partial.gguf"), b"123").unwrap();
        assert_eq!(
            saved_models(store.path(), &models),
            HashSet::from(["whole".to_string()])
        );
        assert!(saved_models(&store.path().join("nowhere"), &models).is_empty());
    }
    #[test]
    fn a_settings_path_names_the_model_in_use_only_while_that_file_exists() {
        let folder = tempfile::tempdir().unwrap();
        let models = vec![small_model("two", "two.gguf", 5)];
        let present = folder.path().join("two.gguf");
        fs::write(&present, b"12345").unwrap();
        fs::write(folder.path().join("other.gguf"), b"12345").unwrap();
        assert_eq!(
            in_use_model(&present, models.clone()).map(|m| m.id),
            Some("two".into())
        );
        assert!(
            in_use_model(&folder.path().join("absent/two.gguf"), models.clone()).is_none(),
            "a fresh install's default path names a model it does not have yet"
        );
        assert!(
            in_use_model(&folder.path().join("other.gguf"), models).is_none(),
            "a file the catalog does not know is not a catalog model in use"
        );
    }
    #[test]
    fn long_failures_never_push_the_resume_button_out_of_the_window() {
        fn visit(
            shape: &egui::epaint::Shape,
            clip: egui::Rect,
            out: &mut Vec<(egui::Rect, egui::Rect, String)>,
        ) {
            match shape {
                egui::epaint::Shape::Text(text) => out.push((
                    clip,
                    egui::Rect::from_min_size(text.pos, text.galley.size()),
                    text.galley.job.text.clone(),
                )),
                egui::epaint::Shape::Vec(shapes) => {
                    shapes.iter().for_each(|s| visit(s, clip, out));
                }
                _ => {}
            }
        }
        let mut app = App::inert();
        app.tab = Tab::Setup;
        app.setup.hardware = Some(Hardware {
            os_arch: "windows-x86_64".into(),
            cpu_logical: 16,
            ram_total_bytes: Some(32 * 1_073_741_824),
            ram_available_bytes: Some(20 * 1_073_741_824),
            disk_free_bytes: Some(120 * 1_073_741_824),
            disk_target: "fixture".into(),
            gpus: vec![],
            warnings: vec![],
        });
        app.recalculate_recommendation();
        app.setup.error = "checksum mismatch in a long log line. ".repeat(200);
        app.service_error = "翻訳サービスの長いエラーです。".repeat(80);
        let context = egui::Context::default();
        ui::configure_theme(&context);
        let mut output = None;
        for frame in 0..3 {
            output = Some(context.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(860.0, 620.0),
                    )),
                    time: Some(frame as f64 / 60.0),
                    ..Default::default()
                },
                |ctx| app.render_ui(ctx),
            ));
        }
        let mut texts = Vec::new();
        for shape in &output.unwrap().shapes {
            visit(&shape.shape, shape.clip_rect, &mut texts);
        }
        let (clip, rect, _) = texts
            .iter()
            .find(|(_, _, text)| text.starts_with("続きから再開"))
            .expect("resume button drawn");
        assert!(
            clip.contains_rect(*rect) && rect.bottom() <= 620.0,
            "resume button cut off: {rect:?} in {clip:?}"
        );
    }
    #[test]
    fn low_capacity_cannot_start_a_download_or_replace_existing_preferences() {
        let mut app = App::inert();
        let original = app.settings.model.clone();
        app.setup.hardware = Some(Hardware {
            os_arch: "windows-x86_64".into(),
            cpu_logical: 2,
            ram_total_bytes: Some(1_073_741_824),
            ram_available_bytes: Some(256 * 1_048_576),
            disk_free_bytes: Some(1_073_741_824),
            disk_target: "fixture".into(),
            gpus: vec![],
            warnings: vec![],
        });
        app.recalculate_recommendation();
        app.begin_install();
        assert!(!app.setup.installing);
        assert!(!app.setup.recommendation.as_ref().unwrap().feasible);
        assert_eq!(app.settings.model, original);
        assert!(app.rx.try_recv().is_err());
    }
    #[test]
    fn failed_preparation_preserves_the_previous_working_selection() {
        let mut app = App::inert();
        let original = app.settings.model.clone();
        app.setup.installing = true;
        app.tx
            .send(Event::Installed(Err("checksum mismatch".into())))
            .unwrap();
        app.process(&egui::Context::default());
        assert_eq!(app.settings.model, original);
        assert!(!app.setup.installing);
        assert!(app.setup.error.contains("checksum"));
    }
    #[test]
    fn user_cancellation_is_recoverable_without_an_error_or_model_switch() {
        let mut app = App::inert();
        let original = app.settings.model.clone();
        app.setup.installing = true;
        app.setup.cancel.store(true, Ordering::Relaxed);
        app.tx
            .send(Event::Installed(Err("cancelled".into())))
            .unwrap();
        app.process(&egui::Context::default());
        assert!(app.setup.cancelled);
        assert!(!app.setup.installing && !app.setup.completed);
        assert!(app.setup.error.is_empty());
        assert_eq!(app.settings.model, original);
    }
}
