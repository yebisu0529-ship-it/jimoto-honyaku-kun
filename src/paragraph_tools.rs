//! Local sentence support, independent of source scopes and translation jobs.
use super::setup_chain::Step;
use super::*;
use plamo_translate::{
    installer::Progress,
    punctuation::PunctuationEngine,
    punctuation_setup::{self, PunctuationInstallation},
};

pub(super) struct ToolsState {
    pub engine: Option<Arc<PunctuationEngine>>,
    pub preparing: bool,
    pub loading: bool,
    pub progress: Option<Progress>,
    pub error: String,
    pub cancel: Arc<AtomicBool>,
    pub worker: Option<thread::JoinHandle<()>>,
    checked: bool,
    download_requested: bool,
}
impl Default for ToolsState {
    fn default() -> Self {
        Self {
            engine: None,
            preparing: false,
            loading: false,
            progress: None,
            error: String::new(),
            cancel: Arc::new(AtomicBool::new(false)),
            worker: None,
            checked: false,
            download_requested: false,
        }
    }
}
pub(super) struct ReadyTools {
    installation: PunctuationInstallation,
    engine: Arc<PunctuationEngine>,
}
pub(super) enum ToolsEvent {
    Progress(Progress),
    Prepared {
        result: std::result::Result<Option<ReadyTools>, String>,
        cancelled: bool,
    },
}

/// Automatic language selection is deliberately conservative. An explicit
/// non-English source always bypasses the English tagger.
pub(super) fn use_english_support(
    options: &TranslationOptions,
    kind: SourceKindForTools,
    text: &str,
) -> bool {
    if options.source_lang == "en" {
        return true;
    }
    if options.source_lang != "auto" {
        return false;
    }
    if kind == SourceKindForTools::EnglishAsr {
        return true;
    }
    let letters = text.chars().filter(|c| c.is_alphabetic()).count();
    let ascii = text.chars().filter(char::is_ascii_alphabetic).count();
    if letters < 20 || ascii * 100 < letters * 95 {
        return false;
    }
    text.split_whitespace()
        .filter(|word| {
            matches!(
                word.trim_matches(|c: char| !c.is_ascii_alphabetic())
                    .to_ascii_lowercase()
                    .as_str(),
                "the"
                    | "and"
                    | "is"
                    | "that"
                    | "of"
                    | "to"
                    | "we"
                    | "this"
                    | "will"
                    | "with"
                    | "from"
                    | "for"
                    | "can"
                    | "should"
            )
        })
        .take(3)
        .count()
        >= 3
}
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum SourceKindForTools {
    EnglishAsr,
    Other,
}

impl App {
    pub(super) fn initialize_paragraph_tools(&mut self) {
        if !self.settings.english_punctuation
            || self.paragraph_tools.checked
            || self.paragraph_tools.loading
            || self.paragraph_tools.preparing
        {
            return;
        }
        self.begin_paragraph_tools(false);
    }
    pub(super) fn prepare_paragraph_tools(&mut self) {
        if self.paragraph_tools.engine.is_some() || self.paragraph_tools.preparing {
            return;
        }
        if self.paragraph_tools.loading {
            self.paragraph_tools.download_requested = true;
            return;
        }
        self.begin_paragraph_tools(true);
    }
    fn begin_paragraph_tools(&mut self, download: bool) {
        if let Some(previous) = self.paragraph_tools.worker.take() {
            let _ = previous.join();
        }
        self.paragraph_tools.checked = true;
        self.paragraph_tools.loading = !download;
        self.paragraph_tools.preparing = download;
        self.paragraph_tools.progress = None;
        self.paragraph_tools.error.clear();
        self.paragraph_tools.cancel = Arc::new(AtomicBool::new(false));
        let cancel = self.paragraph_tools.cancel.clone();
        let tx = self.tx.clone();
        let root = data_root().join("text-punctuation");
        self.paragraph_tools.worker = Some(thread::spawn(move || {
            let result = (|| -> Result<Option<ReadyTools>> {
                let installation = if download {
                    Some(punctuation_setup::install(&root, cancel.clone(), |p| {
                        let _ = tx.try_send(Event::ParagraphTools(ToolsEvent::Progress(p)));
                    })?)
                } else {
                    punctuation_setup::load(&root)?
                };
                if cancel.load(Ordering::Acquire) {
                    anyhow::bail!("cancelled");
                }
                installation
                    .map(|installation| {
                        let engine = Arc::new(PunctuationEngine::start(installation.clone())?);
                        Ok(ReadyTools {
                            installation,
                            engine,
                        })
                    })
                    .transpose()
            })()
            .map_err(|error| format!("{error:#}"));
            let mut event = Event::ParagraphTools(ToolsEvent::Prepared {
                result,
                cancelled: cancel.load(Ordering::Acquire),
            });
            loop {
                match tx.try_send(event) {
                    Ok(()) | Err(mpsc::TrySendError::Disconnected(_)) => break,
                    Err(mpsc::TrySendError::Full(returned)) => {
                        if cancel.load(Ordering::Acquire) {
                            break;
                        }
                        event = returned;
                        thread::sleep(Duration::from_millis(20));
                    }
                }
            }
        }));
    }
    pub(super) fn accept_paragraph_tools(&mut self, event: ToolsEvent) {
        match event {
            ToolsEvent::Progress(progress) => self.paragraph_tools.progress = Some(progress),
            ToolsEvent::Prepared { result, cancelled } => {
                self.paragraph_tools.loading = false;
                self.paragraph_tools.preparing = false;
                self.paragraph_tools.progress = None;
                if let Some(worker) = self.paragraph_tools.worker.take() {
                    let _ = worker.join();
                }
                if cancelled || self.paragraph_tools.cancel.load(Ordering::Acquire) {
                    self.paragraph_tools.error.clear();
                    self.paragraph_tools.download_requested = false;
                    self.stop_step(Step::Punctuation, true, String::new());
                    self.initialize_paragraph_tools();
                    return;
                }
                match result {
                    Ok(Some(ready)) => {
                        debug_assert!(ready.installation.model.is_absolute());
                        if self.settings.english_punctuation {
                            self.paragraph_tools.engine = Some(ready.engine);
                        }
                        self.paragraph_tools.error.clear();
                    }
                    Ok(None) => {}
                    Err(error) => self.paragraph_tools.error = error,
                }
                if std::mem::take(&mut self.paragraph_tools.download_requested)
                    && self.paragraph_tools.engine.is_none()
                    && self.paragraph_tools.error.is_empty()
                {
                    self.prepare_paragraph_tools();
                }
                self.punctuation_settled();
            }
        }
    }
    /// The helper is ready, is no longer wanted, or the attempt failed: each settles
    /// the chain's step. A run still going, or started again just now, leaves it waiting.
    fn punctuation_settled(&mut self) {
        if self.paragraph_tools.engine.is_some() || !self.wants(Step::Punctuation) {
            self.finish_step(Step::Punctuation);
        } else if !self.paragraph_tools.error.is_empty() {
            let error = self.paragraph_tools.error.clone();
            self.stop_step(Step::Punctuation, false, error);
        }
    }
    pub(super) fn paragraph_tools_settings_ui(&mut self, ui: &mut egui::Ui) {
        ui.label(RichText::new("文章をつなぐ翻訳").strong());
        ui.label(theme::label(if self.settings.readout_enabled {
            "字幕と音声の原文を段落にまとめ、前の文章も参照して訳文を更新します。読み上げは文章の確定後に行います。"
        } else {
            "字幕と音声の原文を段落にまとめ、前の文章も参照して訳文を更新します。"
        }));
        ui.add_space(theme::SPACE_2);
        ui.label(theme::label("区切りの長さ"));
        ui.horizontal_wrapped(|ui| {
            for (length, name) in document::ChunkLength::CHOICES {
                ui.radio_value(&mut self.settings.live_chunk, length, name);
            }
        });
        ui.label(theme::caption(
            "短いほど早く確定して読みやすく、長いほど広い文脈で訳します。短くしても、直前の文章を参考にして訳します。",
        ));
        ui.add_space(theme::SPACE_2);
        if ui
            .checkbox(
                &mut self.settings.english_punctuation,
                "英語の文章区切りを補助する",
            )
            .changed()
        {
            if self.settings.english_punctuation {
                self.paragraph_tools.checked = false;
                self.initialize_paragraph_tools();
            } else {
                self.paragraph_tools.cancel.store(true, Ordering::Release);
                self.paragraph_tools.engine.take();
            }
        }
        ui.label(theme::caption(
            "英語の句読点補助モデルをPC内で使います。原文の単語や数字を変える出力は採用しません。",
        ));
        if self.paragraph_tools.loading || self.paragraph_tools.preparing {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(if self.paragraph_tools.loading {
                    "文章区切りの補助を読み込み中…"
                } else {
                    "文章区切りの補助を準備中…"
                });
            });
            if let Some(p) = &self.paragraph_tools.progress {
                ui.label(&p.message);
                if let Some(total) = p.total_bytes.filter(|t| *t > 0) {
                    ui.add(
                        egui::ProgressBar::new(
                            (p.downloaded_bytes as f64 / total as f64).clamp(0.0, 1.0) as f32,
                        )
                        .text(format!(
                            "{} / {}",
                            setup::bytes(p.downloaded_bytes),
                            setup::bytes(total)
                        )),
                    );
                }
            }
            if ui.button("準備を中止").clicked() {
                self.paragraph_tools.cancel.store(true, Ordering::Release);
            }
        } else if self.paragraph_tools.engine.is_some() {
            ui.colored_label(ACCENT, "英語の文章区切り補助 · 準備完了");
        } else {
            ui.label(theme::label(
                "初回は約86MBを取得します。未準備でも、文章をつなぐ翻訳は利用できます。",
            ));
            ui.hyperlink_to("補助モデルの利用条件（Apache 2.0）", "https://github.com/k2-fsa/sherpa-onnx/blob/af2d8d81dbfa61c4cea4c568da78f3982edca005/LICENSE");
            if ui
                .add_enabled(
                    self.settings.english_punctuation,
                    ui::standard("文章区切りの補助を準備"),
                )
                .clicked()
            {
                self.prepare_paragraph_tools();
            }
        }
        if !self.paragraph_tools.error.is_empty() {
            ui.colored_label(
                theme::ERROR,
                "区切り補助を準備できませんでした。文章の翻訳は続けられます。",
            );
            egui::CollapsingHeader::new("区切り補助のエラー詳細").show(ui, |ui| {
                ui.label(&self.paragraph_tools.error);
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn explicit_languages_and_short_or_non_english_auto_input_are_respected() {
        let mut options = TranslationOptions::default();
        let english = "The public exhibition will show how we can compare the original source with a translation";
        assert!(use_english_support(
            &options,
            SourceKindForTools::Other,
            english
        ));
        assert!(!use_english_support(
            &options,
            SourceKindForTools::Other,
            "La biblioteca presenta una nueva exposición de fotografías antiguas"
        ));
        assert!(!use_english_support(
            &options,
            SourceKindForTools::Other,
            "図書館の展示をみんなで読みます。"
        ));
        assert!(!use_english_support(
            &options,
            SourceKindForTools::Other,
            "New York"
        ));
        options.source_lang = "ja".into();
        assert!(!use_english_support(
            &options,
            SourceKindForTools::EnglishAsr,
            english
        ));
        options.source_lang = "en".into();
        assert!(use_english_support(
            &options,
            SourceKindForTools::Other,
            "New York"
        ));
    }
}
