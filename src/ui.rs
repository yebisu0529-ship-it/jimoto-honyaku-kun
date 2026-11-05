use super::*;

const BG: Color32 = Color32::from_rgb(20, 24, 31);
const SIDEBAR: Color32 = Color32::from_rgb(16, 20, 25);
const SURFACE: Color32 = Color32::from_rgb(27, 33, 42);
const INPUT: Color32 = Color32::from_rgb(17, 22, 28);
const BORDER: Color32 = Color32::from_rgb(42, 50, 61);
const TEXT: Color32 = Color32::from_rgb(230, 234, 240);
const SECONDARY: Color32 = Color32::from_rgb(163, 173, 186);
const MINT: Color32 = Color32::from_rgb(60, 191, 164);
const ERROR: Color32 = Color32::from_rgb(240, 122, 122);

pub(super) fn configure_theme(ctx: &egui::Context) {
    install_fonts(ctx);
    let mut style = (*ctx.style()).clone();
    style.visuals = egui::Visuals::dark();
    style
        .text_styles
        .insert(egui::TextStyle::Heading, egui::FontId::proportional(20.0));
    style
        .text_styles
        .insert(egui::TextStyle::Body, egui::FontId::proportional(16.0));
    style
        .text_styles
        .insert(egui::TextStyle::Button, egui::FontId::proportional(14.0));
    style
        .text_styles
        .insert(egui::TextStyle::Small, egui::FontId::proportional(12.0));
    style
        .text_styles
        .insert(egui::TextStyle::Monospace, egui::FontId::monospace(14.0));
    style.spacing.item_spacing = Vec2::new(8.0, 8.0);
    style.spacing.button_padding = Vec2::new(12.0, 6.0);
    style.spacing.interact_size.y = 32.0;
    style.visuals.override_text_color = Some(TEXT);
    style.visuals.panel_fill = BG;
    style.visuals.window_fill = SURFACE;
    style.visuals.extreme_bg_color = INPUT;
    style.visuals.faint_bg_color = SURFACE;
    style.visuals.selection.bg_fill = Color32::from_rgb(31, 74, 67);
    style.visuals.selection.stroke = egui::Stroke::new(2.0_f32, MINT);
    for widget in [
        &mut style.visuals.widgets.inactive,
        &mut style.visuals.widgets.hovered,
        &mut style.visuals.widgets.active,
        &mut style.visuals.widgets.open,
    ] {
        widget.bg_fill = SURFACE;
        widget.weak_bg_fill = SURFACE;
        widget.bg_stroke = egui::Stroke::new(1.0_f32, BORDER);
        widget.fg_stroke = egui::Stroke::new(1.0_f32, TEXT);
        widget.corner_radius = egui::CornerRadius::same(6);
    }
    style.visuals.widgets.hovered.bg_stroke =
        egui::Stroke::new(1.0_f32, Color32::from_rgb(58, 68, 82));
    style.visuals.widgets.active.bg_stroke = egui::Stroke::new(2.0_f32, MINT);
    style.visuals.widgets.noninteractive.bg_stroke = egui::Stroke::new(1.0_f32, BORDER);
    style.visuals.widgets.noninteractive.fg_stroke = egui::Stroke::new(1.0_f32, SECONDARY);
    ctx.set_style(style);
}

fn card() -> egui::Frame {
    egui::Frame::new()
        .fill(SURFACE)
        .stroke(egui::Stroke::new(1.0_f32, BORDER))
        .corner_radius(8)
        .inner_margin(16.0)
}
fn secondary(ui: &mut egui::Ui, text: impl Into<String>) {
    ui.label(RichText::new(text).size(14.0).color(SECONDARY));
}
pub(super) fn primary(label: &str) -> egui::Button<'_> {
    egui::Button::new(RichText::new(label).color(Color32::from_rgb(11, 21, 18)))
        .fill(MINT)
        .min_size(Vec2::new(132.0, 36.0))
}
fn standard(label: &str) -> egui::Button<'_> {
    egui::Button::new(label).min_size(Vec2::new(0.0, 32.0))
}
pub(super) fn toggle(ui: &mut egui::Ui, value: &mut bool, label: &str) -> egui::Response {
    ui.scope(|ui| {
        let checked = *value;
        let visuals = &mut ui.style_mut().visuals;
        for widget in [
            &mut visuals.widgets.inactive,
            &mut visuals.widgets.hovered,
            &mut visuals.widgets.active,
            &mut visuals.widgets.open,
        ] {
            widget.corner_radius = egui::CornerRadius::same(3);
            widget.bg_fill = if checked { MINT } else { INPUT };
            widget.weak_bg_fill = widget.bg_fill;
            widget.bg_stroke = egui::Stroke::new(
                1.5_f32,
                if checked {
                    MINT
                } else {
                    Color32::from_rgb(127, 138, 152)
                },
            );
            widget.fg_stroke = egui::Stroke::new(
                2.0_f32,
                if checked {
                    Color32::from_rgb(11, 21, 18)
                } else {
                    TEXT
                },
            );
        }
        visuals.widgets.active.bg_stroke = egui::Stroke::new(2.0_f32, MINT);
        ui.checkbox(value, label)
    })
    .inner
}
fn parse_glossary(input: &str) -> std::result::Result<Vec<(String, String)>, String> {
    let mut entries = Vec::new();
    for (index, line) in input.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let Some((source, target)) = line.split_once('\t').or_else(|| line.split_once('=')) else {
            return Err(format!(
                "{}行目: 原語と訳語を = またはタブで区切ってください。",
                index + 1
            ));
        };
        if source.trim().is_empty() || target.trim().is_empty() {
            return Err(format!(
                "{}行目: 原語と訳語の両方を入力してください。",
                index + 1
            ));
        }
        entries.push((source.trim().to_owned(), target.trim().to_owned()));
    }
    Ok(entries)
}

impl App {
    pub(super) fn languages(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            egui::ComboBox::from_id_salt("source-language")
                .width(126.0)
                .selected_text(language_label(&self.settings.translation.source_lang))
                .show_ui(ui, |ui| {
                    ui.selectable_value(
                        &mut self.settings.translation.source_lang,
                        "auto".into(),
                        "自動判定",
                    );
                    for (code, label) in engine::LANGUAGES {
                        ui.selectable_value(
                            &mut self.settings.translation.source_lang,
                            (*code).into(),
                            *label,
                        );
                    }
                });
            if ui
                .add_enabled(
                    self.settings.translation.source_lang != "auto",
                    standard("交換"),
                )
                .clicked()
            {
                std::mem::swap(
                    &mut self.settings.translation.source_lang,
                    &mut self.settings.translation.target_lang,
                );
            }
            egui::ComboBox::from_id_salt("target-language")
                .width(126.0)
                .selected_text(language_label(&self.settings.translation.target_lang))
                .show_ui(ui, |ui| {
                    for (code, label) in engine::LANGUAGES {
                        ui.selectable_value(
                            &mut self.settings.translation.target_lang,
                            (*code).into(),
                            *label,
                        );
                    }
                });
            if self.settings.translation.target_lang == "ja" {
                egui::ComboBox::from_id_salt("form")
                    .width(108.0)
                    .selected_text(if self.settings.translation.style == "polite" {
                        "です・ます"
                    } else {
                        "常体"
                    })
                    .show_ui(ui, |ui| {
                        ui.selectable_value(
                            &mut self.settings.translation.style,
                            "plain".into(),
                            "常体",
                        );
                        ui.selectable_value(
                            &mut self.settings.translation.style,
                            "polite".into(),
                            "です・ます",
                        );
                    });
            }
        });
    }
    fn backend_notice(&mut self, ui: &mut egui::Ui) {
        if !self.backend_ready {
            ui.horizontal_wrapped(|ui| {
                secondary(
                    ui,
                    if self.backend_loading {
                        "翻訳モデルを読み込んでいます。"
                    } else {
                        "翻訳するにはモデルの準備が必要です。"
                    },
                );
                if ui.add(standard("モデルの準備")).clicked() {
                    self.open_setup();
                }
            });
        }
    }
    fn input_card(&mut self, ui: &mut egui::Ui, height: f32) {
        card().show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.label(
                    RichText::new(format!("原文 · {}字", self.input.chars().count()))
                        .size(14.0)
                        .strong(),
                );
                if ui.add(standard("貼り付け")).clicked() {
                    match arboard::Clipboard::new().and_then(|mut c| c.get_text()) {
                        Ok(text) => self.input = text,
                        Err(error) => self.status = error.to_string(),
                    }
                }
                if ui
                    .add_enabled(!self.input.is_empty(), standard("クリア"))
                    .clicked()
                {
                    self.input.clear();
                }
            });
            egui::ScrollArea::vertical()
                .id_salt("source-scroll")
                .max_height(height)
                .min_scrolled_height(height)
                .show(ui, |ui| {
                    ui.add_sized(
                        [ui.available_width(), height],
                        egui::TextEdit::multiline(&mut self.input)
                            .hint_text("翻訳したい文章を入力してください")
                            .font(egui::FontId::proportional(16.0))
                            .margin(12.0)
                            .desired_width(f32::INFINITY),
                    );
                });
        });
    }
    fn output_card(&mut self, ui: &mut egui::Ui, height: f32) {
        card().show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new("訳文").size(14.0).strong().color(MINT));
                if ui
                    .add_enabled(!self.output.is_empty(), standard("コピー"))
                    .clicked()
                    && let Ok(mut clipboard) = arboard::Clipboard::new()
                {
                    let _ = clipboard.set_text(self.output.clone());
                }
                if ui
                    .add_enabled(!self.output.is_empty(), standard("保存"))
                    .clicked()
                {
                    self.save_text();
                }
                if ui
                    .add_enabled(!self.output.is_empty(), standard("読み上げ"))
                    .clicked()
                {
                    self.read_translation(self.output.clone());
                }
                if self.readout_busy() && ui.add(standard("停止")).clicked() {
                    self.stop_readout();
                }
            });
            egui::ScrollArea::vertical()
                .id_salt("output-scroll")
                .max_height(height)
                .min_scrolled_height(height)
                .show(ui, |ui| {
                    // An immutable text buffer remains selectable, but never changes the translation.
                    let mut output = self.output.as_str();
                    ui.add_sized(
                        [ui.available_width(), height],
                        egui::TextEdit::multiline(&mut output)
                            .hint_text("訳文はここに表示されます")
                            .font(egui::FontId::proportional(17.0))
                            .margin(12.0)
                            .desired_width(f32::INFINITY),
                    );
                });
        });
    }
    pub(super) fn text_ui(&mut self, ui: &mut egui::Ui) {
        ui.heading("文章翻訳");
        secondary(ui, "文章を入力するか、画面や画像から読み取って翻訳します。");
        ui.add_space(8.0);
        self.languages(ui);
        self.backend_notice(ui);
        ui.add_space(4.0);
        let stacked = ui.available_width() < 650.0;
        let available = (ui.available_height() - 56.0).max(220.0);
        let height = if stacked {
            (available / 2.0 - 82.0).max(64.0)
        } else {
            (available - 82.0).max(120.0)
        };
        if stacked {
            self.input_card(ui, height);
            self.output_card(ui, height);
        } else {
            ui.columns(2, |columns| {
                self.input_card(&mut columns[0], height);
                self.output_card(&mut columns[1], height);
            });
        }
        ui.add_space(4.0);
        let ready =
            self.backend_ready && self.manual_busy.is_none() && !self.input.trim().is_empty();
        ui.horizontal_wrapped(|ui| {
            if ui.add_enabled(ready, primary("翻訳する")).clicked() {
                self.manual_translate();
            }
            ui.label(RichText::new("Ctrl + Enter").size(12.0).color(SECONDARY));
            if self.manual_busy.is_some() {
                ui.spinner();
            }
            if ui
                .add_enabled(self.backend_ready, standard("画像から翻訳"))
                .clicked()
            {
                self.open_image();
            }
        });
        if ready && ui.input(|i| i.modifiers.ctrl && i.key_pressed(egui::Key::Enter)) {
            self.manual_translate();
        }
    }
    pub(super) fn live_ui(&mut self, ui: &mut egui::Ui) {
        ui.heading("ライブ翻訳");
        ui.add_space(8.0);
        let caption_receiving = self.caption_enabled
            && !self.caption_paused
            && self
                .last_caption
                .is_some_and(|time| time.elapsed() < Duration::from_secs(8));
        ui.columns(2, |columns| {
            card().show(&mut columns[0], |ui| {
                let mut enabled = self.caption_enabled;
                if toggle(ui, &mut enabled, "YouTube字幕").changed() {
                    self.set_caption_enabled(enabled);
                }
                secondary(
                    ui,
                    if caption_receiving {
                        "字幕を受信中"
                    } else if self.caption_enabled {
                        "字幕の受信待ち"
                    } else {
                        "停止中"
                    },
                );
            });
            card().show(&mut columns[1], |ui| {
                let mut enabled = self.audio_enabled || self.audio_start_requested;
                let can_toggle =
                    self.backend_ready || self.audio_enabled || self.audio_start_requested;
                if ui
                    .add_enabled_ui(can_toggle, |ui| toggle(ui, &mut enabled, "PC音声"))
                    .inner
                    .changed()
                {
                    if !enabled && self.audio_start_requested && !self.audio_enabled {
                        self.audio_start_requested = false;
                    } else if self.audio_enabled
                        || (self.audio_environment_ready() && self.asr_ready && self.backend_ready)
                    {
                        self.toggle_audio();
                    } else {
                        self.audio_start_requested = true;
                        self.prepare_audio();
                    }
                }
                secondary(
                    ui,
                    if self.audio_enabled && self.asr_ready {
                        if self.asr_busy {
                            "音声を認識中"
                        } else {
                            "聞き取り中"
                        }
                    } else if self.audio_start_requested {
                        "音声認識の準備中"
                    } else {
                        "停止中"
                    },
                );
            });
        });
        ui.add_space(8.0);
        ui.horizontal_wrapped(|ui| {
            toggle(ui, &mut self.overlay, "字幕ウィンドウ");
            ui.menu_button("表示", |ui| {
                toggle(ui, &mut self.settings.show_original, "原文を表示");
                toggle(ui, &mut self.settings.auto_scroll, "最新へ自動スクロール");
                ui.add(
                    egui::Slider::new(&mut self.settings.font_size, 18.0..=24.0)
                        .integer()
                        .text("訳文の文字サイズ"),
                );
            });
            if ui.add(standard("新しく始める")).clicked() {
                match Journal::new(&data_root().join("sessions")) {
                    Ok(journal) => {
                        self.journal = Some(journal);
                        self.journal_error.clear();
                        self.rows.clear();
                        self.invalidate_timeline();
                    }
                    Err(error) => self.status = error.to_string(),
                }
            }
            if ui
                .add_enabled(!self.rows.is_empty(), standard("書き出し"))
                .clicked()
            {
                self.export_history();
            }
        });
        ui.horizontal_wrapped(|ui| {
            if self.live_busy.is_some() {
                ui.spinner();
                secondary(ui, "翻訳中");
            }
            let saved = if !self.journal_error.is_empty() {
                "保存できていません"
            } else if self.journal.is_some() {
                "タイムラインへ自動保存"
            } else {
                "プレビュー · 保存されません"
            };
            ui.label(
                RichText::new(saved)
                    .size(12.0)
                    .color(if self.journal_error.is_empty() {
                        SECONDARY
                    } else {
                        ERROR
                    }),
            );
            if self.caption_enabled && self.audio_enabled {
                ui.label(
                    RichText::new("字幕を優先し、字幕がない間は音声を使います")
                        .size(12.0)
                        .color(SECONDARY),
                );
            }
        });
        if !self.service_error.is_empty() {
            ui.colored_label(ERROR, &self.service_error);
        }
        if !self.journal_error.is_empty() {
            ui.colored_label(ERROR, &self.journal_error);
        }
        ui.add_space(4.0);
        let mut read_requested = None;
        egui::ScrollArea::vertical()
            .id_salt("timeline")
            .stick_to_bottom(self.settings.auto_scroll)
            .auto_shrink([false, false])
            .show(ui, |ui| {
                if self.rows.is_empty() {
                    card().show(ui, |ui| {
                        ui.set_min_width(ui.available_width());
                        ui.label("開始すると、ここに対訳が時刻付きで並びます。");
                        secondary(ui, "YouTube字幕またはPC音声をオンにしてください。");
                    });
                }
                for row in &self.rows {
                    card().show(ui, |ui| {
                        ui.set_min_width(ui.available_width());
                        ui.horizontal_wrapped(|ui| {
                            ui.label(
                                RichText::new(clock(row.record.start_ms))
                                    .size(12.0)
                                    .color(SECONDARY),
                            );
                            let color = match row.record.kind.as_str() {
                                "YouTube" => Color32::from_rgb(217, 144, 106),
                                "音声" => Color32::from_rgb(127, 167, 224),
                                _ => Color32::from_rgb(179, 157, 219),
                            };
                            egui::Frame::new()
                                .stroke(egui::Stroke::new(1.0_f32, color))
                                .corner_radius(4)
                                .inner_margin(4.0)
                                .show(ui, |ui| {
                                    ui.label(
                                        RichText::new(&row.record.kind).size(12.0).color(color),
                                    );
                                });
                            if row.pending {
                                ui.spinner();
                            }
                            if ui
                                .add_enabled(!row.record.translation.is_empty(), standard("コピー"))
                                .clicked()
                                && let Ok(mut clipboard) = arboard::Clipboard::new()
                            {
                                let _ = clipboard.set_text(row.record.translation.clone());
                            }
                            if ui
                                .add_enabled(
                                    !row.record.translation.is_empty(),
                                    standard("読み上げ"),
                                )
                                .clicked()
                            {
                                read_requested = Some(row.record.translation.clone());
                            }
                        });
                        if let Some(error) = &row.error {
                            ui.colored_label(ERROR, error);
                        } else if !row.record.translation.is_empty() {
                            ui.add(
                                egui::Label::new(
                                    RichText::new(&row.record.translation)
                                        .size(self.settings.font_size.clamp(18.0, 24.0))
                                        .color(TEXT),
                                )
                                .wrap()
                                .selectable(true),
                            );
                        } else {
                            secondary(ui, "翻訳中…");
                        }
                        if self.settings.show_original {
                            ui.add(
                                egui::Label::new(
                                    RichText::new(&row.record.source)
                                        .size(15.0)
                                        .color(SECONDARY),
                                )
                                .wrap()
                                .selectable(true),
                            );
                        }
                    });
                    ui.add_space(4.0);
                }
            });
        if let Some(text) = read_requested {
            self.read_translation(text);
        }
    }
    pub(super) fn files_ui(&mut self, ui: &mut egui::Ui) {
        ui.heading("ファイル・履歴");
        secondary(
            ui,
            "テキスト・Markdown・SRT・WebVTTを翻訳します。字幕の時刻は維持されます。",
        );
        ui.add_space(8.0);
        self.languages(ui);
        self.backend_notice(ui);
        ui.horizontal_wrapped(|ui| {
            if ui
                .add_enabled(
                    self.backend_ready && !self.file_busy,
                    primary("ファイルを選んで翻訳"),
                )
                .clicked()
            {
                self.translate_file();
            }
            if self.file_busy {
                ui.spinner();
                if ui.add(standard("中止")).clicked() {
                    self.file_cancel.store(true, Ordering::Relaxed);
                }
            }
            if ui
                .add_enabled(!self.rows.is_empty(), standard("タイムラインを書き出す"))
                .clicked()
            {
                self.export_history();
            }
        });
        ui.add_space(8.0);
        ui.add_sized(
            [ui.available_width(), 36.0],
            egui::TextEdit::singleline(&mut self.search)
                .hint_text("原文・訳文を検索")
                .margin(8.0),
        );
        egui::ScrollArea::vertical().show(ui, |ui| {
            let search = self.search.to_lowercase();
            for row in self.rows.iter().rev() {
                if !search.is_empty()
                    && !row.record.source.to_lowercase().contains(&search)
                    && !row.record.translation.to_lowercase().contains(&search)
                {
                    continue;
                }
                card().show(ui, |ui| {
                    ui.set_min_width(ui.available_width());
                    ui.label(
                        RichText::new(format!("{} · {}", row.record.created_at, row.record.kind))
                            .size(12.0)
                            .color(SECONDARY),
                    );
                    ui.add(
                        egui::Label::new(RichText::new(&row.record.translation).size(17.0))
                            .wrap()
                            .selectable(true),
                    );
                    ui.add(
                        egui::Label::new(
                            RichText::new(&row.record.source)
                                .size(15.0)
                                .color(SECONDARY),
                        )
                        .wrap()
                        .selectable(true),
                    );
                });
            }
            if self.rows.is_empty() {
                secondary(ui, "翻訳履歴はここに表示されます。");
            }
        });
    }
    pub(super) fn settings_ui(&mut self, ui: &mut egui::Ui) {
        let previous_asr = (
            self.settings.asr_backend.clone(),
            self.settings.asr_model.clone(),
            self.settings.asr_endpoint.clone(),
        );
        egui::ScrollArea::vertical().show(ui, |ui| {
            ui.heading("設定");
            secondary(ui, "翻訳と音声認識はこのPC内で処理します。");
            ui.add_space(8.0);
            card().show(ui, |ui| {
                ui.set_min_width(ui.available_width());
                ui.label(RichText::new("翻訳モデル").strong());
                ui.label(self.setup_summary());
                secondary(ui, self.connection_label());
                if ui.add(standard("モデルを変更")).clicked() {
                    self.open_setup();
                }
            });
            card().show(ui, |ui| {
                ui.set_min_width(ui.available_width());
                ui.label(RichText::new("音声認識").strong());
                egui::ComboBox::from_id_salt("asr-backend")
                    .selected_text(if self.settings.asr_backend == "qwen" {
                        "Qwen3-ASR · 多言語"
                    } else {
                        "Whisper · 多言語 / CPU"
                    })
                    .show_ui(ui, |ui| {
                        if ui
                            .selectable_value(
                                &mut self.settings.asr_backend,
                                "qwen".into(),
                                "Qwen3-ASR · 多言語",
                            )
                            .changed()
                        {
                            self.settings.asr_model = "Qwen/Qwen3-ASR-0.6B-hf".into();
                            self.settings.asr_device = self
                                .setup
                                .recommendation
                                .as_ref()
                                .filter(|rec| rec.asr_backend == "qwen")
                                .map(|rec| rec.asr_device.clone())
                                .unwrap_or_else(|| "cpu".into());
                        }
                        if ui
                            .selectable_value(
                                &mut self.settings.asr_backend,
                                "whisper".into(),
                                "Whisper · 多言語 / CPU",
                            )
                            .changed()
                        {
                            self.settings.asr_model = "base".into();
                            self.settings.asr_device = "cpu".into();
                        }
                    });
                ui.horizontal_wrapped(|ui| {
                    secondary(ui, "音声の区切り");
                    for (value, label) in [(2.0, "短め"), (3.0, "標準"), (4.0, "長め")] {
                        ui.selectable_value(&mut self.settings.audio_seconds, value, label);
                    }
                });
                secondary(
                    ui,
                    if self.asr_ready {
                        "音声認識の準備完了"
                    } else {
                        "初回は必要なモデルを準備します"
                    },
                );
                if ui.add(standard("音声認識を準備")).clicked() {
                    self.prepare_audio();
                }
            });
            card().show(ui, |ui| {
                ui.set_min_width(ui.available_width());
                self.readout_settings_ui(ui);
            });
            card().show(ui, |ui| {
                ui.set_min_width(ui.available_width());
                ui.label(RichText::new("YouTube字幕の接続").strong());
                secondary(
                    ui,
                    "ブラウザーの字幕連携に、下の接続キーを設定してください。",
                );
                ui.label(RichText::new("接続キー").size(14.0));
                ui.horizontal_wrapped(|ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut self.settings.bridge_token)
                            .password(true)
                            .interactive(false)
                            .desired_width(220.0),
                    );
                    if ui.add(standard("接続キーをコピー")).clicked()
                        && let Ok(mut clipboard) = arboard::Clipboard::new()
                    {
                        let _ = clipboard.set_text(self.settings.bridge_token.clone());
                    }
                });
                if ui.add(standard("字幕連携のフォルダーを開く")).clicked() {
                    let mut command = Command::new("explorer.exe");
                    command.arg(app_root().join("extension"));
                    command_hidden(&mut command);
                    let _ = command.spawn();
                }
                secondary(ui, "拡張機能の追加はブラウザーで行ってください。");
            });
            card().show(ui, |ui| {
                ui.set_min_width(ui.available_width());
                ui.label(RichText::new("用語集と文脈").strong());
                secondary(ui, "1行に「原語 = 訳語」を入力します。例: inference = 推論");
                ui.add_sized(
                    [ui.available_width(), 160.0],
                    egui::TextEdit::multiline(&mut self.glossary_input)
                        .desired_width(f32::INFINITY)
                        .margin(12.0),
                );
                if let Err(error) = parse_glossary(&self.glossary_input) {
                    ui.colored_label(ERROR, error);
                }
                ui.label(RichText::new("文脈（翻訳する文章の背景）").size(14.0));
                ui.add_sized(
                    [ui.available_width(), 88.0],
                    egui::TextEdit::multiline(&mut self.settings.translation.context)
                        .desired_width(f32::INFINITY)
                        .margin(12.0),
                );
            });
            card().show(ui, |ui| {
                ui.set_min_width(ui.available_width());
                ui.label(RichText::new("ショートカットと保存先").strong());
                secondary(ui, "範囲翻訳: Ctrl + Alt + T　 音声翻訳: Ctrl + Alt + L");
                if ui.add(standard("保存フォルダーを開く")).clicked() {
                    self.open_data_folder();
                }
            });
            egui::CollapsingHeader::new("詳細（トラブル時）")
                .default_open(false)
                .show(ui, |ui| {
                    for (label, value) in [
                        ("翻訳モデルのファイル", &mut self.settings.model),
                        ("翻訳エンジン", &mut self.settings.llama_server),
                        ("翻訳サーバー", &mut self.settings.translation.endpoint),
                        ("音声認識モデル", &mut self.settings.asr_model),
                        ("音声認識の実行環境", &mut self.settings.python),
                        ("音声認識サーバー", &mut self.settings.asr_endpoint),
                    ] {
                        ui.label(RichText::new(label).size(14.0));
                        ui.add(egui::TextEdit::singleline(value).desired_width(f32::INFINITY));
                    }
                    ui.label(
                        RichText::new(format!("保存先: {}", data_root().display())).size(12.0),
                    );
                    if ui.add(standard("翻訳エンジンを起動")).clicked() {
                        self.start_translation_server();
                    }
                });
            if ui
                .add_enabled(
                    parse_glossary(&self.glossary_input).is_ok(),
                    primary("設定を保存"),
                )
                .clicked()
            {
                match parse_glossary(&self.glossary_input) {
                    Ok(glossary) => {
                        self.settings.translation.glossary = glossary;
                        match self.persist_preferences() {
                            Ok(()) => {
                                self.settings_load_failed = false;
                                self.status = "設定を保存しました。".into();
                            }
                            Err(error) => self.status = error.to_string(),
                        }
                    }
                    Err(error) => self.status = error,
                }
            }
            if ui.add(standard("アプリを終了")).clicked() {
                ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
            }
        });
        if previous_asr
            != (
                self.settings.asr_backend.clone(),
                self.settings.asr_model.clone(),
                self.settings.asr_endpoint.clone(),
            )
        {
            if self.audio_enabled {
                self.stop_audio();
            }
            self.asr_ready = false;
            self.asr_started_at = None;
            self.audio_start_requested = false;
            self.request_service_status();
        }
    }
    pub(super) fn render_ui(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::bottom("status")
            .min_height(28.0)
            .frame(
                egui::Frame::new()
                    .fill(SIDEBAR)
                    .inner_margin(egui::Margin::symmetric(12, 6)),
            )
            .show(ctx, |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.label(
                        RichText::new(format!(
                            "{} · {}",
                            self.current_model_label(),
                            self.connection_label()
                        ))
                        .size(12.0)
                        .color(if self.backend_ready {
                            MINT
                        } else {
                            SECONDARY
                        }),
                    );
                    if !self.status.is_empty() {
                        ui.label(RichText::new(&self.status).size(12.0).color(SECONDARY));
                    }
                    if !self.readout.error.is_empty() {
                        ui.label(RichText::new(&self.readout.error).size(12.0).color(ERROR));
                    } else if !self.readout.status.is_empty() {
                        ui.label(
                            RichText::new(&self.readout.status)
                                .size(12.0)
                                .color(SECONDARY),
                        );
                    }
                });
            });
        egui::SidePanel::left("sidebar")
            .exact_width(200.0)
            .resizable(false)
            .frame(egui::Frame::new().fill(SIDEBAR).inner_margin(12.0))
            .show(ctx, |ui| {
                ui.add_space(12.0);
                ui.label(
                    RichText::new("地元翻訳くん")
                        .size(20.0)
                        .strong()
                        .color(TEXT),
                );
                ui.label(
                    RichText::new("translated by PLaMo")
                        .size(12.0)
                        .color(SECONDARY),
                );
                ui.add_space(24.0);
                for (tab, label) in [
                    (Tab::Text, "文章翻訳"),
                    (Tab::Live, "ライブ翻訳"),
                    (Tab::Files, "ファイル・履歴"),
                    (Tab::Settings, "設定"),
                ] {
                    let selected = self.tab == tab;
                    let response = ui.add_sized(
                        [ui.available_width(), 36.0],
                        egui::Button::new(RichText::new(label).size(14.0).color(TEXT))
                            .fill(if selected {
                                Color32::from_rgb(31, 38, 48)
                            } else {
                                SIDEBAR
                            })
                            .stroke(egui::Stroke::NONE),
                    );
                    if selected {
                        ui.painter().rect_filled(
                            egui::Rect::from_min_size(
                                response.rect.min,
                                Vec2::new(3.0, response.rect.height()),
                            ),
                            0.0,
                            MINT,
                        );
                    }
                    if response.clicked() {
                        self.tab = tab;
                    }
                }
                ui.add_space(16.0);
                ui.separator();
                ui.add_space(8.0);
                if ui
                    .add_sized([ui.available_width(), 36.0], standard("モデルの準備"))
                    .clicked()
                {
                    self.open_setup();
                }
                ui.add_space(16.0);
                if ui
                    .add_enabled(
                        self.backend_ready && !self.capture_busy,
                        egui::Button::new("範囲を翻訳")
                            .min_size(Vec2::new(ui.available_width(), 40.0)),
                    )
                    .clicked()
                {
                    self.capture(ctx);
                }
                ui.label(RichText::new("Ctrl + Alt + T").size(12.0).color(SECONDARY));
                if !self.backend_ready {
                    ui.label(
                        RichText::new("モデルの準備が必要です")
                            .size(12.0)
                            .color(SECONDARY),
                    );
                }
                if self.screens.len() > 1 {
                    egui::ComboBox::from_id_salt("screen")
                        .width(ui.available_width())
                        .selected_text(format!("画面 {}", self.screen_index + 1))
                        .show_ui(ui, |ui| {
                            for (index, screen) in self.screens.iter().enumerate() {
                                ui.selectable_value(
                                    &mut self.screen_index,
                                    index,
                                    format!("{}: {}", index + 1, screen.name),
                                );
                            }
                        });
                }
                ui.add_space(8.0);
                let audio_label = if self.audio_enabled {
                    "音声翻訳を停止"
                } else if self.audio_start_requested {
                    "開始を取り消す"
                } else {
                    "音声翻訳を開始"
                };
                let can_toggle =
                    self.backend_ready || self.audio_enabled || self.audio_start_requested;
                if ui
                    .add_enabled_ui(can_toggle, |ui| {
                        ui.add_sized([ui.available_width(), 40.0], standard(audio_label))
                    })
                    .inner
                    .clicked()
                {
                    if self.audio_start_requested && !self.audio_enabled {
                        self.audio_start_requested = false;
                    } else if self.audio_enabled
                        || (self.audio_environment_ready() && self.asr_ready && self.backend_ready)
                    {
                        self.toggle_audio();
                    } else {
                        self.audio_start_requested = true;
                        self.prepare_audio();
                    }
                }
                ui.label(RichText::new("Ctrl + Alt + L").size(12.0).color(SECONDARY));
                ui.add_space(16.0);
                ui.label(
                    RichText::new("翻訳はこのPC内で処理します")
                        .size(12.0)
                        .color(SECONDARY),
                );
            });
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(BG).inner_margin(24.0))
            .show(ctx, |ui| match self.tab {
                Tab::Text => self.text_ui(ui),
                Tab::Live => self.live_ui(ui),
                Tab::Files => self.files_ui(ui),
                Tab::Settings => self.settings_ui(ui),
                Tab::Setup => self.setup_ui(ui),
            });
    }
}
