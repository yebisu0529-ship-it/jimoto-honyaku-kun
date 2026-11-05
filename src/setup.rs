//! First-run preparation uses local capacity measurements and a pinned catalog.
use super::*;
use plamo_translate::{
    hardware::{self, Hardware},
    installer::{self, InstallRequest, InstallResult, Progress},
    model_catalog::{self, Model, Policy, Recommendation},
};

pub(super) struct SetupState {
    pub hardware: Option<Hardware>,
    pub recommendation: Option<Recommendation>,
    pub policy: Policy,
    pub selected_id: String,
    pub detecting: bool,
    pub installing: bool,
    pub progress: Option<Progress>,
    pub error: String,
    pub completed: bool,
    pub cancelled: bool,
    pub cancel: Arc<AtomicBool>,
    pub force_cpu: bool,
}
impl Default for SetupState {
    fn default() -> Self {
        Self {
            hardware: None,
            recommendation: None,
            policy: Policy::Realtime,
            selected_id: String::new(),
            detecting: false,
            installing: false,
            progress: None,
            error: String::new(),
            completed: false,
            cancelled: false,
            cancel: Arc::new(AtomicBool::new(false)),
            force_cpu: false,
        }
    }
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
        if self.setup.hardware.is_none() && !self.setup.detecting {
            self.detect_capacity();
        }
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
    pub(super) fn recalculate_recommendation(&mut self) {
        if let Some(hardware) = &self.setup.hardware {
            let mut measured = hardware.clone();
            if self.setup.force_cpu {
                measured.gpus.clear();
            }
            let rec = model_catalog::recommend(&measured, self.setup.policy);
            self.setup.selected_id = rec.selected.id.clone();
            self.setup.cancelled = false;
            self.setup.recommendation = Some(rec);
        }
    }
    fn setup_model(&self) -> Option<Model> {
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
        if let Some(rec) = self.setup.recommendation.clone() {
            let model = self.setup_model().unwrap_or_else(|| rec.selected.clone());
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
                ui.label(
                    RichText::new("PCの空き容量を確認し、使い方に合う翻訳モデルを選びます。")
                        .size(theme::SIZE_CAPTION)
                        .color(theme::TEXT_MUTED),
                );
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
                if let Some(hardware) = self.setup.hardware.clone() {
                    theme::card().show(ui, |ui| {
                        ui.set_min_width(ui.available_width());
                        let gpu = hardware.gpus.iter().find(|g| !g.is_software);
                        ui.horizontal_wrapped(|ui| {
                            ui.label(
                                RichText::new(gpu.map(|g| g.name.as_str()).unwrap_or("CPUで実行"))
                                    .strong(),
                            );
                            if ui
                                .add_enabled(!self.setup.installing, egui::Button::new("再確認"))
                                .clicked()
                            {
                                self.detect_capacity();
                            }
                        });
                        ui.horizontal_wrapped(|ui| {
                            ui.label(format!(
                                "空きVRAM {}",
                                measured(gpu.and_then(|g| g.free_vram_bytes))
                            ));
                            ui.label(format!("空きメモリ {}", measured(hardware.ram_available_bytes)));
                            ui.label(format!("空き容量 {}", measured(hardware.disk_free_bytes)));
                        });
                        egui::CollapsingHeader::new("PC情報と保存先").show(ui, |ui| {
                            ui.label(format!(
                                "CPU {}スレッド / メモリ合計 {}",
                                hardware.cpu_logical,
                                measured(hardware.ram_total_bytes)
                            ));
                            ui.label(format!("モデルの保存先: {}", self.model_store_root().display()));
                            if ui
                                .add_enabled(!self.setup.installing, egui::Button::new("保存先を変更"))
                                .clicked()
                                && let Some(folder) = rfd::FileDialog::new().pick_folder()
                            {
                                self.settings.model_store =
                                    folder.join("地元翻訳くん/engine").to_string_lossy().into();
                                self.detect_capacity();
                            }
                            let old = self.setup.force_cpu;
                            ui.add_enabled_ui(!self.setup.installing, |ui| {
                                ui::toggle(ui, &mut self.setup.force_cpu, "GPUを使わずCPUで実行する");
                            });
                            if old != self.setup.force_cpu {
                                self.recalculate_recommendation();
                            }
                        });
                        ui.label(theme::caption("PC情報は外部へ送信しません。"));
                    });
                }
                ui.add_space(theme::SPACE_3);
                let mut policy = self.setup.policy;
                ui.add_enabled_ui(!self.setup.installing, |ui| {
                    ui.horizontal_wrapped(|ui| {
                        ui.label("使い方");
                        for (value, label) in [
                            (Policy::Realtime, "速度を優先"),
                            (Policy::Balanced, "バランス"),
                            (Policy::Quality, "精度を優先"),
                        ] {
                            ui.selectable_value(&mut policy, value, label);
                        }
                    });
                });
                if policy != self.setup.policy {
                    self.setup.policy = policy;
                    self.recalculate_recommendation();
                }
                if let Some(rec) = self.setup.recommendation.clone() {
                    let model = self.setup_model().unwrap_or_else(|| rec.selected.clone());
                    ui.add_space(theme::SPACE_3);
                    theme::card()
                        .stroke(egui::Stroke::new(1.0_f32, ACCENT))
                        .show(ui, |ui| {
                            ui.set_min_width(ui.available_width());
                            ui.label(
                                RichText::new(if model.id == rec.selected.id {
                                    "このPCへのおすすめ"
                                } else {
                                    "選択中のモデル"
                                })
                                .color(ACCENT)
                                .size(theme::SIZE_LABEL)
                                .strong(),
                            );
                            ui.label(RichText::new(&model.label).size(theme::SIZE_TITLE).strong());
                            ui.horizontal_wrapped(|ui| {
                                ui.label(format!("ダウンロード {}", bytes(model.bytes)));
                                ui.label(format!(
                                    "実行: {}",
                                    match rec.backend.as_str() {
                                        "cuda" => "NVIDIA GPU",
                                        "vulkan" => "GPU",
                                        "cpu" => "CPU",
                                        _ => "CPU",
                                    }
                                ));
                                ui.label(format!(
                                    "必要な空き容量 約{}",
                                    bytes(2 * model.bytes + 6 * 1_073_741_824)
                                ));
                            });
                            ui.label(theme::label(
                                "空きメモリと同時利用の余裕から選んだ目安です。速度を実測した結果ではありません。",
                            ));
                            egui::CollapsingHeader::new("選んだ理由を見る").show(ui, |ui| {
                                for reason in &rec.reasons {
                                    ui.label(reason);
                                }
                                for warning in &rec.warnings {
                                    ui.colored_label(theme::WARNING, warning);
                                }
                            });
                        });
                    ui.add_space(theme::SPACE_3);
                    egui::CollapsingHeader::new("ほかのモデルから選ぶ（14種類）").show(ui, |ui| {
                        ui.add_enabled_ui(!self.setup.installing, |ui| {
                            for alternative in &rec.alternatives {
                                ui.horizontal_wrapped(|ui| {
                                    if ui
                                        .add_enabled(
                                            alternative.fits,
                                            egui::RadioButton::new(
                                                self.setup.selected_id == alternative.model.id,
                                                format!(
                                                    "{}  ·  {}",
                                                    alternative.model.label,
                                                    bytes(alternative.model.bytes)
                                                ),
                                            ),
                                        )
                                        .clicked()
                                    {
                                        self.setup.selected_id = alternative.model.id.clone();
                                        self.setup.cancelled = false;
                                    }
                                    if !alternative.fits {
                                        ui.label(
                                            RichText::new("容量などの条件を満たしません")
                                                .size(theme::SIZE_CAPTION)
                                                .color(theme::WARNING),
                                        )
                                        .on_hover_text(alternative.reasons.join("\n"));
                                    }
                                });
                            }
                        });
                    });
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
            let label = if self.setup.error.is_empty() && !self.setup.cancelled {
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
