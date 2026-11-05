//! Optional screen/image OCR. Setup and inference never block the GUI thread.
use super::*;
use plamo_translate::{
    indra_ocr::IndraEngine,
    indra_setup::{self, IndraInstallation},
    installer::Progress,
    ocr_runtime::ProcessControl,
    paddle_ocr::PaddleEngine,
    paddle_setup::{self, PaddleInstallation},
};

enum Runtime {
    Indra(IndraEngine),
    Paddle(PaddleEngine),
}
pub(super) struct OcrState {
    pub generation: u64,
    pub busy: Option<u64>,
    pending: Option<(u64, image::RgbaImage)>,
    runtime: Arc<Mutex<Option<Runtime>>>,
    pub control: Option<ProcessControl>,
    indra: Option<IndraInstallation>,
    paddle: Option<PaddleInstallation>,
    confirm: bool,
    installing: bool,
    setup_epoch: u64,
    worker_epoch: u64,
    pub setup_worker: Option<thread::JoinHandle<()>>,
    pub cancel: Arc<AtomicBool>,
    progress: Option<Progress>,
    error: String,
}
impl Default for OcrState {
    fn default() -> Self {
        Self {
            generation: 0,
            busy: None,
            pending: None,
            runtime: Arc::new(Mutex::new(None)),
            control: None,
            indra: None,
            paddle: None,
            confirm: false,
            installing: false,
            setup_epoch: 0,
            worker_epoch: 0,
            setup_worker: None,
            cancel: Arc::new(AtomicBool::new(false)),
            progress: None,
            error: String::new(),
        }
    }
}
pub(super) enum OcrSetupEvent {
    Progress {
        epoch: u64,
        progress: Progress,
    },
    Indra {
        epoch: u64,
        result: std::result::Result<IndraInstallation, String>,
    },
    Paddle {
        epoch: u64,
        result: std::result::Result<PaddleInstallation, String>,
    },
}
impl App {
    pub(super) fn screen_ocr_needs_normal_view(&self) -> bool {
        self.ocr.confirm || self.ocr.installing
    }
    pub(super) fn initialize_screen_ocr(&mut self) {
        match indra_setup::load(&data_root().join("ocr/indra")) {
            Ok(value) => self.ocr.indra = value,
            Err(error) => self.ocr.error = format!("Indra OCRの準備状況を確認できません: {error}"),
        }
        match paddle_setup::load(&data_root().join("ocr/paddle")) {
            Ok(value) => self.ocr.paddle = value,
            Err(error) => self.ocr.error = format!("PaddleOCRの準備状況を確認できません: {error}"),
        }
    }
    pub(super) fn do_ocr(&mut self, image: image::RgbaImage) {
        self.mini.clear_manual_failure();
        self.pending_ocr = None;
        self.ocr.generation = self.ocr.generation.wrapping_add(1);
        self.ocr.pending = Some((self.ocr.generation, image));
        self.kick_screen_ocr();
    }
    pub(super) fn reset_screen_ocr(&mut self) {
        self.pending_ocr = None;
        self.ocr.setup_epoch = self.ocr.setup_epoch.wrapping_add(1);
        self.ocr.generation = self.ocr.generation.wrapping_add(1);
        self.ocr.pending = None;
        self.ocr.busy = None;
        self.ocr.cancel.store(true, Ordering::Release);
        self.discard_screen_ocr_runtime();
    }
    pub(super) fn discard_screen_ocr_runtime(&mut self) {
        if let Some(control) = self.ocr.control.take() {
            control.stop();
        }
        self.ocr.runtime = Arc::new(Mutex::new(None));
    }
    pub(super) fn kick_screen_ocr(&mut self) {
        if self.ocr.busy.is_some() || self.ocr.pending.is_none() {
            return;
        }
        if !self.screen_ocr_ready() {
            self.ocr.confirm = true;
            return;
        }
        let (generation, image) = self.ocr.pending.take().unwrap();
        self.ocr.busy = Some(generation);
        self.status = "画像から文字を読み取っています…".into();
        let backend = self.settings.ocr_backend.clone();
        let language = self.settings.translation.source_lang.clone();
        let indra = self.ocr.indra.clone();
        let paddle = self.ocr.paddle.clone();
        let runtime = self.ocr.runtime.clone();
        let control = self
            .ocr
            .control
            .get_or_insert_with(ProcessControl::new)
            .clone();
        let runtime_root = self.model_store_root();
        let device = if self.settings.ocr_device != "cpu"
            && self.settings.backend != "cpu"
            && self.setup.hardware.as_ref().is_some_and(|h| {
                h.gpus
                    .iter()
                    .any(|g| g.free_vram_bytes.is_some_and(|n| n >= 4 * 1_073_741_824))
            }) {
            self.settings.backend.clone()
        } else {
            "cpu".into()
        };
        let tx = self.tx.clone();
        thread::spawn(move || {
            let result = (|| -> Result<String> {
                if backend == "windows" {
                    return platform::ocr(&image, &language);
                }
                let mut owned = runtime.lock().unwrap_or_else(|e| e.into_inner());
                if owned.is_none() {
                    *owned = Some(match backend.as_str() {
                        "indra" => Runtime::Indra(IndraEngine::start(
                            indra.ok_or_else(|| {
                                anyhow::anyhow!("Indra OCRの追加データがありません。")
                            })?,
                            control.clone(),
                        )?),
                        "paddle" => {
                            let installation =
                                plamo_translate::installer::load_installation(&runtime_root)?
                                    .ok_or_else(|| {
                                        anyhow::anyhow!(
                                            "先に翻訳モデルと実行環境を準備してください。"
                                        )
                                    })?;
                            Runtime::Paddle(PaddleEngine::start(
                                paddle.ok_or_else(|| {
                                    anyhow::anyhow!("PaddleOCRの追加データがありません。")
                                })?,
                                installation.llama_server,
                                device,
                                control.clone(),
                            )?)
                        }
                        _ => anyhow::bail!("文字認識の方式を設定で選び直してください。"),
                    });
                }
                match owned.as_ref().unwrap() {
                    Runtime::Indra(engine) => engine.recognize(&image),
                    Runtime::Paddle(engine) => engine.recognize(&image),
                }
            })()
            .map_err(|e| format!("{e:#}"));
            let _ = tx.send(Event::Ocr { generation, result });
        });
    }
    pub(super) fn screen_ocr_preparing(&self) -> bool {
        self.ocr.installing
    }
    pub(super) fn screen_ocr_ready(&self) -> bool {
        match self.settings.ocr_backend.as_str() {
            "windows" => true,
            "indra" => self.ocr.indra.is_some(),
            "paddle" => self.ocr.paddle.is_some(),
            _ => false,
        }
    }
    fn begin_ocr_install(&mut self) {
        if self.ocr.installing {
            return;
        }
        self.ocr.installing = true;
        self.ocr.error.clear();
        self.ocr.progress = None;
        self.ocr.cancel = Arc::new(AtomicBool::new(false));
        self.ocr.setup_epoch = self.ocr.setup_epoch.wrapping_add(1);
        let epoch = self.ocr.setup_epoch;
        self.ocr.worker_epoch = epoch;
        let cancel = self.ocr.cancel.clone();
        let backend = self.settings.ocr_backend.clone();
        let root = data_root().join("ocr").join(&backend);
        let tx = self.tx.clone();
        let shutdown = self.stop.clone();
        self.ocr.setup_worker = Some(thread::spawn(move || {
            let mut on_progress = |progress| {
                let _ = tx.try_send(Event::OcrSetup(OcrSetupEvent::Progress { epoch, progress }));
            };
            let event = if backend == "indra" {
                OcrSetupEvent::Indra {
                    epoch,
                    result: indra_setup::install(&root, cancel, on_progress)
                        .map_err(|e| format!("{e:#}")),
                }
            } else {
                OcrSetupEvent::Paddle {
                    epoch,
                    result: paddle_setup::install(&root, &cancel, &mut on_progress)
                        .map_err(|e| format!("{e:#}")),
                }
            };
            let mut event = Event::OcrSetup(event);
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
    pub(super) fn accept_ocr_setup(&mut self, event: OcrSetupEvent) {
        let epoch = match &event {
            OcrSetupEvent::Progress { epoch, .. }
            | OcrSetupEvent::Indra { epoch, .. }
            | OcrSetupEvent::Paddle { epoch, .. } => *epoch,
        };
        if !matches!(&event, OcrSetupEvent::Progress { .. }) && epoch == self.ocr.worker_epoch {
            if let Some(worker) = self.ocr.setup_worker.take() {
                let _ = worker.join();
            }
            self.ocr.installing = false;
            self.ocr.progress = None;
        }
        match event {
            OcrSetupEvent::Progress { epoch, progress } => {
                if epoch == self.ocr.setup_epoch {
                    self.ocr.progress = Some(progress);
                }
            }
            OcrSetupEvent::Indra { epoch, result } => {
                let current = epoch == self.ocr.setup_epoch && self.settings.ocr_backend == "indra";
                match result {
                    Ok(installation) => {
                        self.ocr.indra = Some(installation);
                        if current {
                            self.ocr.confirm = false;
                            self.ocr.error.clear();
                        }
                    }
                    Err(error) => {
                        if current {
                            self.ocr.error = error;
                            self.ocr.pending = None;
                        }
                    }
                }
            }
            OcrSetupEvent::Paddle { epoch, result } => {
                let current =
                    epoch == self.ocr.setup_epoch && self.settings.ocr_backend == "paddle";
                match result {
                    Ok(installation) => {
                        self.ocr.paddle = Some(installation);
                        if current {
                            self.ocr.confirm = false;
                            self.ocr.error.clear();
                        }
                    }
                    Err(error) => {
                        if current {
                            self.ocr.error = error;
                            self.ocr.pending = None;
                        }
                    }
                }
            }
        }
    }
    pub(super) fn screen_ocr_settings_ui(&mut self, ui: &mut egui::Ui) {
        ui.label(RichText::new("画面・画像の文字認識").strong());
        let previous = (
            self.settings.ocr_backend.clone(),
            self.settings.ocr_device.clone(),
        );
        egui::ComboBox::from_id_salt("screen-ocr-backend")
            .selected_text(ocr_name(&self.settings.ocr_backend))
            .show_ui(ui, |ui| {
                for (key, label) in [
                    ("indra", "Indra OCR · 軽量 / CPU"),
                    ("paddle", "PaddleOCR-VL-1.6 · 精度重視"),
                    ("windows", "Windows OCR · 追加データ不要"),
                ] {
                    ui.selectable_value(&mut self.settings.ocr_backend, key.into(), label);
                }
            });
        if self.settings.ocr_backend == "indra" {
            ui.label(theme::label(
                "日英の画面文字をCPUで読み取ります。認識言語は行ごとに自動判定します。",
            ));
        } else if self.settings.ocr_backend == "paddle" {
            ui.label(theme::label("画像の文字を読み取る生成モデルです。複雑な文章や小さな文字は、原文との照合が必要です。"));
            ui.horizontal_wrapped(|ui| {
                ui.label("実行方式");
                ui.selectable_value(&mut self.settings.ocr_device, "auto".into(), "自動");
                ui.selectable_value(&mut self.settings.ocr_device, "cpu".into(), "CPU");
            });
        } else {
            ui.label(theme::label("Windowsに導入済みの文字認識を使います。"));
        }
        ui.label(theme::label(if self.screen_ocr_ready() {
            "文字認識の準備完了"
        } else {
            "追加データの準備が必要です"
        }));
        if self.settings.ocr_backend != "windows" && ui.button("文字認識を準備").clicked() {
            self.ocr.confirm = true;
        }
        if previous
            != (
                self.settings.ocr_backend.clone(),
                self.settings.ocr_device.clone(),
            )
        {
            self.reset_screen_ocr();
        }
        if !self.ocr.error.is_empty() {
            ui.colored_label(theme::ERROR, &self.ocr.error);
        }
    }
    pub(super) fn screen_ocr_setup_ui(&mut self, ctx: &egui::Context) {
        if !self.ocr.confirm && !self.ocr.installing {
            return;
        }
        egui::Window::new("文字認識の追加準備").collapsible(false).resizable(false).default_width(520.0).anchor(egui::Align2::CENTER_CENTER,[0.0,0.0]).show(ctx,|ui|{
            ui.label(RichText::new(ocr_name(&self.settings.ocr_backend)).strong());
            if self.settings.ocr_backend=="indra" {
                ui.label("日英モデルは約54MB、実行環境を含む初回取得は約184MBです。専用のPython環境をアプリの保存先に用意します。");
                ui.label("重みはModified OpenRAIL-Mです。出力のShare-a-Like、競合製品・サービス、売上・調達額などに関する条件があります。利用条件を確認してから追加してください。");
                ui.hyperlink_to("Indraモデルの利用条件", "https://huggingface.co/datalab-to/indra-ocr/blob/bd02167fdf2005a19962cc15188f8496a2c87d3b/LICENSE");
            } else {
                ui.label("公式モデル約1.82GBを取得し、翻訳用の実行環境を再利用します。展開用を含め約4GBの空き容量を目安にしてください。Pythonは不要です。");
                ui.label("PaddleOCRのモデルにはApache-2.0が適用されます。");
                ui.hyperlink_to("PaddleOCRモデルの利用条件", "https://huggingface.co/PaddlePaddle/PaddleOCR-VL-1.6/blob/c5630abae1d940eafe0697512a0325494b02ab42/LICENSE");
            }
            ui.label("準備中も画面取得や音声取得は始まりません。画像はPC内で処理します。");
            if self.ocr.installing {
                ui.spinner();
                if let Some(progress)=&self.ocr.progress {ui.label(&progress.message);if let Some(total)=progress.total_bytes.filter(|n|*n>0){ui.add(egui::ProgressBar::new((progress.downloaded_bytes as f64/total as f64).clamp(0.0,1.0) as f32).show_percentage());}}
                if ui.button("中止").clicked(){self.ocr.cancel.store(true,Ordering::Release);}
            } else {
                ui.horizontal_wrapped(|ui|{
                    if ui.button("利用条件を確認して追加").clicked(){self.begin_ocr_install();}
                    if ui.button("今はしない").clicked(){self.ocr.confirm=false;self.ocr.pending=None;}
                });
            }
            if !self.ocr.error.is_empty(){ui.colored_label(theme::ERROR,&self.ocr.error);}
        });
    }
}
fn ocr_name(backend: &str) -> &'static str {
    match backend {
        "indra" => "Indra OCR · 軽量 / CPU",
        "paddle" => "PaddleOCR-VL-1.6 · 精度重視",
        _ => "Windows OCR · 追加データ不要",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn new_image_invalidates_recognized_text_waiting_for_manual_translation() {
        let mut app = App::inert();
        app.settings.ocr_backend = "indra".into();
        app.pending_ocr = Some("old result".into());
        app.manual_busy = Some(99);
        app.do_ocr(image::RgbaImage::new(2, 2));
        assert!(app.pending_ocr.is_none());
        assert!(app.ocr.pending.is_some());
        assert!(app.ocr.confirm);
        assert!(app.ocr.control.is_none());
        assert!(!app.audio_enabled);
    }
    #[test]
    fn switched_backend_discards_an_old_preparation_failure_without_losing_new_image() {
        let mut app = App::inert();
        app.settings.ocr_backend = "paddle".into();
        app.ocr.setup_epoch = 8;
        app.ocr.worker_epoch = 7;
        app.ocr.pending = Some((10, image::RgbaImage::new(2, 2)));
        app.ocr.confirm = true;
        app.accept_ocr_setup(OcrSetupEvent::Indra {
            epoch: 7,
            result: Err("cancelled".into()),
        });
        assert!(app.ocr.pending.is_some());
        assert!(app.ocr.confirm);
        assert!(app.ocr.error.is_empty());
    }
    #[test]
    fn late_recognition_cannot_clear_a_newer_request() {
        let mut app = App::inert();
        app.manual_busy = Some(99);
        app.ocr.generation = 10;
        app.ocr.busy = Some(10);
        app.tx
            .send(Event::Ocr {
                generation: 9,
                result: Ok("stale".into()),
            })
            .unwrap();
        app.process(&egui::Context::default());
        assert_eq!(app.ocr.busy, Some(10));
        assert!(app.pending_ocr.is_none());
        app.tx
            .send(Event::Ocr {
                generation: 10,
                result: Ok("current".into()),
            })
            .unwrap();
        app.process(&egui::Context::default());
        assert!(app.ocr.busy.is_none());
        assert_eq!(app.pending_ocr.as_deref(), Some("current"));
    }
}
