use super::*;
use plamo_translate::hardware::{Gpu, Hardware};

const GIB: u64 = 1_073_741_824;
const SMALL: (f32, f32) = (860.0, 620.0);
const LARGE: (f32, f32) = (2560.0, 1369.0);

fn pc(gpu: Option<(u64, u64)>) -> Hardware {
    Hardware {
        os_arch: "windows-x86_64".into(),
        cpu_logical: 16,
        ram_total_bytes: Some(32 * GIB),
        ram_available_bytes: Some(20 * GIB),
        disk_free_bytes: Some(120 * GIB),
        disk_target: "fixture".into(),
        gpus: gpu
            .into_iter()
            .map(|(total, used)| Gpu {
                name: "NVIDIA GeForce RTX 4090".into(),
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
    }
}
/// A setup screen after measuring a PC with `(total, used)` GPU memory, or none.
fn app_on(gpu: Option<(u64, u64)>) -> App {
    let mut app = App::inert();
    app.tab = Tab::Setup;
    app.setup.hardware = Some(pc(gpu));
    app.recalculate_recommendation();
    app
}
fn model(id: &str) -> Model {
    model_catalog::bundled()
        .unwrap()
        .models
        .into_iter()
        .find(|m| m.id == id)
        .unwrap()
}
/// An app whose server has `loaded` on a GPU that other programs also use.
fn app_with_loaded(loaded: &str, total: u64, others: u64) -> App {
    let loaded = model(loaded);
    let mut app = App::inert();
    app.tab = Tab::Setup;
    app.setup.hardware = Some(pc(Some((
        total,
        others + model_catalog::inference_bytes(&loaded),
    ))));
    app.setup.in_use = Some(loaded);
    app.settings.gpu_layers = 99;
    app.backend_ready = true;
    app.recalculate_recommendation();
    app
}
fn picker(app: &App) -> Picker {
    Picker::new(
        app.setup.recommendation.as_ref().unwrap(),
        app.setup.in_use.as_ref(),
        &app.setup.saved,
    )
}
/// Each row as "size: status", marked when it cannot be chosen.
fn rows(picker: &Picker) -> Vec<String> {
    picker
        .rows
        .iter()
        .map(|r| {
            format!(
                "{}{}: {}",
                r.size,
                if r.status.selectable() {
                    ""
                } else {
                    " (disabled)"
                },
                r.status.text().unwrap_or_default()
            )
        })
        .collect()
}

#[test]
fn the_list_has_exactly_four_rows_one_per_size_from_the_q4_k_m_files() {
    let picker = picker(&app_on(Some((24 * GIB, 5 * GIB))));
    assert_eq!(
        picker
            .rows
            .iter()
            .map(|r| r.id.as_str())
            .collect::<Vec<_>>(),
        [
            "plamo3-0.8b-q4_k_m",
            "plamo3-2b-q4_k_m",
            "plamo3-8b-q4_k_m",
            "plamo3-31b-q4_k_m"
        ]
    );
    assert_eq!(
        picker
            .rows
            .iter()
            .map(|r| r.size.as_str())
            .collect::<Vec<_>>(),
        ["0.8B", "2B", "8B", "31B"]
    );
    assert_eq!(
        picker
            .rows
            .iter()
            .map(|r| bytes(r.bytes))
            .collect::<Vec<_>>(),
        ["546 MB", "1.7 GB", "4.9 GB", "18.2 GB"]
    );
}

#[test]
fn a_cpu_pc_can_choose_only_the_smallest_row() {
    let app = app_on(None);
    assert_eq!(
        rows(&picker(&app)),
        [
            "0.8B: おすすめ",
            "2B (disabled): GPUが必要です",
            "8B (disabled): GPUが必要です",
            "31B (disabled): GPUが必要です"
        ]
    );
    assert_eq!(app.setup.selected_id, "plamo3-0.8b-q4_k_m");
    assert!(app.meter_view().is_none(), "no GPU, no meter");
}

#[test]
fn turning_on_cpu_mode_disables_the_larger_rows_even_with_a_big_gpu() {
    let mut app = app_on(Some((24 * GIB, 5 * GIB)));
    assert_eq!(app.setup.selected_id, "plamo3-8b-q4_k_m");
    app.setup.force_cpu = true;
    app.recalculate_recommendation();
    assert_eq!(
        rows(&picker(&app)),
        [
            "0.8B: おすすめ",
            "2B (disabled): GPUが必要です",
            "8B (disabled): GPUが必要です",
            "31B (disabled): GPUが必要です"
        ]
    );
    assert_eq!(app.setup.selected_id, "plamo3-0.8b-q4_k_m");
    assert!(app.meter_view().is_none());
    app.setup.force_cpu = false;
    app.recalculate_recommendation();
    assert_eq!(app.setup.selected_id, "plamo3-8b-q4_k_m");
    assert!(app.meter_view().is_some());
}

#[test]
fn a_row_that_does_not_fit_is_disabled_and_says_how_much_gpu_memory_is_missing() {
    let app = app_on(Some((8 * GIB, 3 * GIB)));
    assert_eq!(
        rows(&picker(&app)),
        [
            "0.8B: ",
            "2B: おすすめ",
            "8B (disabled): GPUメモリが足りません（あと3.4 GB）",
            "31B (disabled): GPUメモリが足りません（あと20.0 GB）"
        ]
    );
    let busy = app_on(Some((8 * GIB, 13 * GIB / 2)));
    assert_eq!(
        rows(&picker(&busy)),
        [
            "0.8B: おすすめ",
            "2B (disabled): GPUメモリが足りません（あと2.5 GB）",
            "8B (disabled): GPUメモリが足りません（あと6.9 GB）",
            "31B (disabled): GPUメモリが足りません（あと23.5 GB）"
        ],
        "a full GPU is not a missing GPU"
    );
}

#[test]
fn a_shortfall_never_reads_as_zero_and_other_reasons_have_words() {
    assert_eq!(
        blocker_text(Blocker::GpuMemory(1)),
        "GPUメモリが足りません（あと1 MB）"
    );
    assert_eq!(
        blocker_text(Blocker::Memory(3 * GIB)),
        "メモリが足りません（あと3.0 GB）"
    );
    assert_eq!(
        blocker_text(Blocker::Disk(GIB / 2)),
        "空き容量が足りません（あと512 MB）"
    );
    assert_eq!(blocker_text(Blocker::Unavailable), "このPCでは使えません");
}

#[test]
fn status_words_follow_in_use_then_saved_then_recommended() {
    let mut app = app_on(Some((24 * GIB, 5 * GIB)));
    assert_eq!(rows(&picker(&app))[..3], ["0.8B: ", "2B: ", "8B: おすすめ"]);
    app.setup.saved = ["plamo3-2b-q4_k_m", "plamo3-8b-q4_k_m"]
        .map(String::from)
        .into();
    app.setup.in_use = Some(model("plamo3-2b-q4_k_m"));
    assert_eq!(
        rows(&picker(&app))[..3],
        ["0.8B: ", "2B: 使用中", "8B: 保存済み"],
        "saved takes the place of the recommendation, in use the place of saved"
    );
}

#[test]
fn another_quantization_in_use_is_named_on_its_size_row_and_stays_selected() {
    let mut app = app_on(Some((24 * GIB, 5 * GIB)));
    app.setup.in_use = Some(model("plamo3-8b-q3_k_m"));
    app.recalculate_recommendation();
    assert_eq!(rows(&picker(&app))[2], "8B: 使用中（Q3_K_M）");
    assert_eq!(app.setup.selected_id, "plamo3-8b-q3_k_m");
    assert_eq!(
        picker(&app).row_of(&app.setup_model().unwrap()),
        Some(2),
        "the 8B row is the selected one"
    );
    Session::new().settle(&mut app, SMALL);
    assert_eq!(
        app.setup.selected_id, "plamo3-8b-q3_k_m",
        "drawing the screen does not change the choice"
    );
}

#[test]
fn the_recommendation_is_preselected_unless_a_model_is_in_use() {
    let mut app = app_on(Some((24 * GIB, 5 * GIB)));
    assert_eq!(app.setup.selected_id, "plamo3-8b-q4_k_m");
    app.setup.in_use = Some(model("plamo3-2b-q4_k_m"));
    app.recalculate_recommendation();
    assert_eq!(app.setup.selected_id, "plamo3-2b-q4_k_m");
    app.setup.force_cpu = true;
    app.recalculate_recommendation();
    assert_eq!(
        app.setup.selected_id, "plamo3-0.8b-q4_k_m",
        "an in-use size that can no longer be chosen is not preselected"
    );
}

#[test]
fn a_recommendation_in_another_quantization_starts_from_a_row_that_can_be_chosen() {
    // The 2B Q4_K_M needs 2.98 GiB and the Q3_K_M 2.66, with 2.8 free after the reserve.
    let app = app_on(Some((8 * GIB, 4 * GIB + 2 * GIB / 10)));
    let recommendation = app.setup.recommendation.as_ref().unwrap();
    assert_eq!(recommendation.selected.id, "plamo3-2b-q3_k_m");
    assert_eq!(app.setup.selected_id, "plamo3-0.8b-q4_k_m");
    assert_eq!(rows(&picker(&app))[0], "0.8B: おすすめ");
}

#[test]
fn the_meter_reports_use_and_the_selected_models_estimate() {
    let view = app_on(Some((24 * GIB, 5 * GIB))).meter_view().unwrap();
    assert_eq!(view.name, "NVIDIA GeForce RTX 4090");
    assert_eq!(view.used_line(), "使用中 5.0 GB / 24.0 GB");
    assert_eq!(view.model_line(), "このモデルで +7.4 GB");
    assert!(view.draws_estimate() && !view.exceeds() && view.shortfall_line().is_none());
    assert!(!view.estimated, "nvidia-smi measures the whole card");
}

#[test]
fn the_meter_turns_to_the_warning_and_says_how_much_is_missing() {
    let mut app = app_on(Some((8 * GIB, 3 * GIB)));
    app.setup.selected_id = "plamo3-8b-q4_k_m".into();
    let view = app.meter_view().unwrap();
    assert_eq!(view.model_line(), "このモデルで +7.4 GB");
    assert!(view.exceeds());
    assert_eq!(view.shortfall_line().unwrap(), "あと3.4 GB足りません");
    assert_eq!(view.items().last().unwrap().color, WARNING);
    assert_eq!(view.items()[1].key, Some(WARNING));
}

#[test]
fn a_dxgi_reading_is_labelled_an_estimate() {
    let mut app = app_on(Some((24 * GIB, 5 * GIB)));
    app.setup.hardware.as_mut().unwrap().gpus[0].memory_telemetry = "dxgi-local-budget".into();
    app.refresh_recommendation();
    assert!(app.meter_view().unwrap().estimated);
}

#[test]
fn a_full_gpu_says_the_model_runs_on_the_cpu() {
    let view = app_on(Some((8 * GIB, 13 * GIB / 2))).meter_view().unwrap();
    assert_eq!(view.used_line(), "使用中 6.5 GB / 8.0 GB");
    assert_eq!(view.model_line(), "GPUの空きが足りず、CPUで実行します");
    assert!(!view.exceeds() && !view.draws_estimate());
    assert!(view.items().iter().all(|item| item.key != Some(ACCENT)));
}

#[test]
fn the_loaded_model_adds_no_segment_and_reads_loaded_then_loading() {
    let mut app = app_with_loaded("plamo3-8b-q4_k_m", 24 * GIB, 5 * GIB);
    assert_eq!(app.setup.selected_id, "plamo3-8b-q4_k_m");
    let view = app.meter_view().unwrap();
    assert_eq!(view.used_line(), "使用中 12.4 GB / 24.0 GB");
    assert_eq!(view.model_line(), "読み込み済み");
    assert!(!view.draws_estimate() && view.items()[1].key.is_none());
    app.backend_ready = false;
    app.backend_loading = true;
    assert_eq!(app.meter_view().unwrap().model_line(), "読み込み中");
}

#[test]
fn a_loaded_model_is_replaced_not_stacked_when_another_size_is_chosen() {
    // A 12 GiB card with the 2B loaded: 2 GiB used by others plus the model's 2.98 GiB.
    let mut app = app_with_loaded("plamo3-2b-q4_k_m", 12 * GIB, 2 * GIB);
    assert_eq!(
        rows(&picker(&app))[2],
        "8B: ",
        "8B fits once the 2B is released"
    );
    app.setup.selected_id = "plamo3-8b-q4_k_m".into();
    let view = app.meter_view().unwrap();
    assert_eq!(view.model_line(), "このモデルで +4.4 GB");
    assert!(!view.exceeds());
    // With nothing loaded the same memory use is another program's, and 8B does not fit.
    app.backend_ready = false;
    app.refresh_recommendation();
    assert_eq!(
        rows(&picker(&app))[2],
        "8B (disabled): GPUメモリが足りません（あと1.4 GB）"
    );
}

#[test]
fn choosing_a_smaller_model_than_the_loaded_one_shows_memory_freed() {
    let mut app = app_with_loaded("plamo3-8b-q4_k_m", 24 * GIB, 5 * GIB);
    app.setup.selected_id = "plamo3-2b-q4_k_m".into();
    assert_eq!(
        app.meter_view().unwrap().model_line(),
        "このモデルで -4.4 GB"
    );
}

/// One egui context and its clock, which only moves forward.
struct Session {
    context: egui::Context,
    time: f64,
}
impl Session {
    fn new() -> Self {
        let context = egui::Context::default();
        ui::configure_theme(&context);
        Self { context, time: 0.0 }
    }
    fn frame(
        &mut self,
        app: &mut App,
        size: (f32, f32),
        events: Vec<egui::Event>,
    ) -> egui::FullOutput {
        self.time += 1.0 / 60.0;
        self.context.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(size.0, size.1),
                )),
                time: Some(self.time),
                events,
                ..Default::default()
            },
            |ctx| app.render_ui(ctx),
        )
    }
    /// Three frames, so fonts and layout have settled.
    fn settle(&mut self, app: &mut App, size: (f32, f32)) -> egui::FullOutput {
        self.frame(app, size, vec![]);
        self.frame(app, size, vec![]);
        self.frame(app, size, vec![])
    }
    fn press(&mut self, app: &mut App, size: (f32, f32), key: egui::Key) {
        let event = |pressed| egui::Event::Key {
            key,
            physical_key: Some(key),
            pressed,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        };
        self.frame(app, size, vec![event(true)]);
        self.frame(app, size, vec![event(false)]);
    }
    /// The rectangle of whatever has keyboard focus.
    fn focused(&self) -> Option<egui::Rect> {
        let id = self.context.memory(|memory| memory.focused())?;
        self.context.read_response(id).map(|response| response.rect)
    }
    fn click(&mut self, app: &mut App, size: (f32, f32), at: egui::Pos2) {
        let button = |pressed| egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        self.frame(app, size, vec![egui::Event::PointerMoved(at)]);
        self.frame(app, size, vec![button(true)]);
        self.frame(app, size, vec![button(false)]);
    }
}
type Drawn = Vec<(egui::Rect, egui::Rect, String)>;
/// Every text drawn: the clip rectangle it was drawn under, its own rectangle and its words.
fn texts(output: &egui::FullOutput) -> Drawn {
    fn visit(shape: &egui::epaint::Shape, clip: egui::Rect, out: &mut Drawn) {
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
    let mut out = Vec::new();
    for shape in &output.shapes {
        visit(&shape.shape, shape.clip_rect, &mut out);
    }
    out
}
/// A point on a row's empty middle, away from any of its text.
fn row_point(drawn: &Drawn, size_label: &str) -> egui::Pos2 {
    let label = drawn
        .iter()
        .find(|(_, _, text)| text == size_label)
        .unwrap_or_else(|| panic!("row {size_label} not drawn"))
        .1;
    egui::pos2(label.center().x + 200.0, label.center().y)
}

#[test]
fn clicking_anywhere_on_a_row_chooses_it() {
    let mut app = app_on(Some((24 * GIB, 5 * GIB)));
    let mut session = Session::new();
    let drawn = texts(&session.settle(&mut app, SMALL));
    session.click(&mut app, SMALL, row_point(&drawn, "2B"));
    assert_eq!(app.setup.selected_id, "plamo3-2b-q4_k_m");
    let drawn = texts(&session.settle(&mut app, SMALL));
    session.click(&mut app, SMALL, row_point(&drawn, "0.8B"));
    assert_eq!(app.setup.selected_id, "plamo3-0.8b-q4_k_m");
}

#[test]
fn a_row_is_chosen_from_the_keyboard_with_tab_and_space() {
    let mut app = app_on(Some((24 * GIB, 5 * GIB)));
    let mut session = Session::new();
    let drawn = texts(&session.settle(&mut app, SMALL));
    let wanted = row_point(&drawn, "2B");
    let mut reached = false;
    for _ in 0..40 {
        session.press(&mut app, SMALL, egui::Key::Tab);
        // The scroll area around the rows can hold focus too, so ask for a row-sized target.
        if session
            .focused()
            .is_some_and(|rect| rect.height() <= theme::CONTROL_H_LARGE && rect.contains(wanted))
        {
            reached = true;
            break;
        }
    }
    assert!(reached, "Tab never reached the 2B row");
    assert_eq!(
        app.setup.selected_id, "plamo3-8b-q4_k_m",
        "focus alone chooses nothing"
    );
    session.press(&mut app, SMALL, egui::Key::Space);
    assert_eq!(app.setup.selected_id, "plamo3-2b-q4_k_m");
}

#[test]
fn a_disabled_row_cannot_be_clicked() {
    let mut app = app_on(None);
    let mut session = Session::new();
    let drawn = texts(&session.settle(&mut app, SMALL));
    for size in ["2B", "8B", "31B"] {
        session.click(&mut app, SMALL, row_point(&drawn, size));
        assert_eq!(app.setup.selected_id, "plamo3-0.8b-q4_k_m", "{size}");
    }
}

#[test]
fn rows_are_locked_while_a_download_runs() {
    let mut app = app_on(Some((24 * GIB, 5 * GIB)));
    let mut chain = setup_chain::Chain::new([setup_chain::Step::Translation]);
    chain.take_next();
    app.setup.chain = Some(chain);
    let mut session = Session::new();
    let drawn = texts(&session.settle(&mut app, SMALL));
    session.click(&mut app, SMALL, row_point(&drawn, "2B"));
    assert_eq!(app.setup.selected_id, "plamo3-8b-q4_k_m");
}

#[test]
fn rows_and_meter_fit_without_clipping_or_overlap_at_both_window_sizes() {
    for (name, gpu) in [
        ("roomy", Some((24 * GIB, 5 * GIB))),
        ("busy", Some((8 * GIB, 13 * GIB / 2))),
        ("small", Some((8 * GIB, 3 * GIB))),
        ("cpu", None),
    ] {
        for size in [SMALL, LARGE] {
            let mut app = app_on(gpu);
            if name == "small" {
                app.setup.selected_id = "plamo3-8b-q4_k_m".into();
            }
            let mut pool = texts(&Session::new().settle(&mut app, size));
            let picker = picker(&app);
            let mut wanted: Vec<String> = Vec::new();
            for row in &picker.rows {
                wanted.extend([row.size.clone(), row.blurb.to_owned(), bytes(row.bytes)]);
                wanted.extend(row.status.text());
            }
            match app.meter_view() {
                Some(view) => {
                    wanted.push("GPUメモリ".into());
                    wanted.extend(view.items().into_iter().map(|item| item.text));
                }
                None => wanted.extend(cpu_note_lines(false, size == LARGE)),
            }
            let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(size.0, size.1));
            let all = pool.clone();
            // Each wanted text uses up one drawn text, so equal words in two rows are two shapes.
            let mut found = Vec::new();
            for want in &wanted {
                let at = pool
                    .iter()
                    .position(|(_, _, text)| text == want || text.ends_with(&format!(" {want}")))
                    .unwrap_or_else(|| panic!("{name} {size:?}: {want:?} is not drawn whole"));
                let (clip, rect, _) = pool.remove(at);
                assert!(
                    clip.expand(0.5).contains_rect(rect) && screen.contains_rect(rect),
                    "{name} {size:?}: {want:?} is cut off: {rect:?} in {clip:?}"
                );
                found.push((want, rect));
            }
            for (index, (first, a)) in found.iter().enumerate() {
                for (second, b) in &found[index + 1..] {
                    assert!(
                        !a.shrink(0.5).intersects(b.shrink(0.5)),
                        "{name} {size:?}: {first:?} overlaps {second:?}"
                    );
                }
            }
            let action = all
                .iter()
                .find(|(_, _, text)| text.starts_with("はじめる"))
                .unwrap_or_else(|| panic!("{name} {size:?}: no action"));
            assert!(action.0.contains_rect(action.1) && action.1.bottom() <= size.1);
        }
    }
}

#[test]
fn the_meter_the_rows_and_the_action_stay_whole_in_each_state_of_the_chain() {
    use plamo_translate::installer::Progress;
    use setup_chain::{Chain, Step};
    // The chain after `done` of its three steps, with the next one started.
    fn chain_after(done: usize) -> Chain {
        let mut chain = Chain::new([Step::Translation, Step::Punctuation, Step::Speech]);
        for _ in 0..done {
            let step = chain.take_next().unwrap();
            chain.complete(step);
        }
        chain.take_next();
        chain
    }
    // Each state puts the chain where it is and answers with the words its action area shows.
    type State = (&'static str, fn(&mut App) -> Vec<String>);
    let states: [State; 3] = [
        ("before starting", |app| {
            vec![setup::start_label("はじめる", app.download_plan())]
        }),
        ("step 2 of 3", |app| {
            let message = "sherpa-onnx-punct-ct-transformer-zh-en-vocab272727-2024-04-12-int8.tar.bz2を取得中";
            app.setup.chain = Some(chain_after(1));
            app.paragraph_tools.progress = Some(Progress {
                stage: "punctuation_download".into(),
                downloaded_bytes: 41_000_000,
                total_bytes: Some(64_717_756),
                message: message.into(),
            });
            [
                "2/3 英語の文章区切り補助を準備中",
                message,
                "39 MB / 61 MB",
                "中止",
            ]
            .map(String::from)
            .to_vec()
        }),
        ("a failure at step 3", |app| {
            let mut chain = chain_after(2);
            chain.fail(Step::Speech, "HTTP 503".into());
            app.setup.chain = Some(chain);
            [
                "3/3 音声認識を準備できませんでした。ここまでの準備は保持しています。",
                "もう一度",
            ]
            .map(String::from)
            .to_vec()
        }),
    ];
    for size in [SMALL, LARGE] {
        for (name, build) in states {
            let nothing = tempfile::tempdir().unwrap();
            let mut app = app_on(Some((24 * GIB, 5 * GIB)));
            app.seed_plan(&setup_chain::Roots::under(nothing.path()));
            let mut wanted = build(&mut app);
            for row in &picker(&app).rows {
                wanted.extend([row.size.clone(), row.blurb.to_owned(), bytes(row.bytes)]);
                wanted.extend(row.status.text());
            }
            wanted.push("GPUメモリ".into());
            let view = app.meter_view().expect("this PC has a GPU");
            wanted.extend(view.items().into_iter().map(|item| item.text));
            let mut pool = texts(&Session::new().settle(&mut app, size));
            let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(size.0, size.1));
            let mut found = Vec::new();
            for want in &wanted {
                let at = pool
                    .iter()
                    .position(|(_, _, text)| text == want || text.ends_with(&format!(" {want}")))
                    .unwrap_or_else(|| panic!("{name} {size:?}: {want:?} is not drawn whole"));
                let (clip, rect, _) = pool.remove(at);
                assert!(
                    clip.expand(0.5).contains_rect(rect) && screen.contains_rect(rect),
                    "{name} {size:?}: {want:?} is cut off: {rect:?} in {clip:?}"
                );
                found.push((want, rect));
            }
            for (index, (first, a)) in found.iter().enumerate() {
                for (second, b) in &found[index + 1..] {
                    assert!(
                        !a.shrink(0.5).intersects(b.shrink(0.5)),
                        "{name} {size:?}: {first:?} overlaps {second:?}"
                    );
                }
            }
        }
    }
}

#[test]
fn the_meter_sits_beside_the_list_when_wide_and_above_it_when_narrow() {
    let mut app = app_on(Some((24 * GIB, 5 * GIB)));
    let mut session = Session::new();
    let find = |drawn: &Drawn, words: &str| {
        drawn
            .iter()
            .find(|(_, _, text)| text == words)
            .unwrap_or_else(|| panic!("{words} not drawn"))
            .1
    };
    let narrow = texts(&session.settle(&mut app, SMALL));
    assert!(
        find(&narrow, "GPUメモリ").bottom() < find(&narrow, "0.8B").top(),
        "above the list"
    );
    let wide = texts(&session.settle(&mut app, LARGE));
    let meter = find(&wide, "GPUメモリ");
    let list_end = find(&wide, "GPUメモリが足りません（あと6.0 GB）");
    assert!(meter.left() > list_end.right(), "beside the list");
    assert!(
        (meter.center().y - find(&wide, "0.8B").center().y).abs() < 40.0,
        "level with the top row"
    );
}

#[test]
fn the_screen_lists_each_download_with_its_size_and_the_folder_it_goes_to() {
    use setup_chain::Step;
    // Tall enough that the list under the model rows is in view without scrolling.
    for size in [(SMALL.0, 1600.0), LARGE] {
        let nothing = tempfile::tempdir().unwrap();
        let mut app = app_on(Some((24 * GIB, 5 * GIB)));
        app.seed_plan(&setup_chain::Roots::under(nothing.path()));
        let plan = app.download_plan().expect("a plan for the chosen model");
        let model = app.setup_model().expect("a chosen model");
        let drawn = texts(&Session::new().settle(&mut app, size));
        let at = |want: &str| {
            drawn
                .iter()
                .find(|(_, _, text)| text == want)
                .map(|(_, rect, _)| *rect)
                .unwrap_or_else(|| panic!("{size:?}: {want:?} is not drawn"))
        };
        for (step, name) in [
            (
                Step::Translation,
                format!("翻訳モデル（{}）と実行環境", model.label),
            ),
            (Step::Speech, "英語の音声認識　Nemotron Speech".to_owned()),
            (Step::Punctuation, "英語の文章区切り補助".to_owned()),
        ] {
            let bytes_of = plan
                .steps
                .iter()
                .find(|(planned, _)| *planned == step)
                .unwrap()
                .1;
            let name = at(&name);
            let amount = bytes(bytes_of.next_multiple_of(1_048_576));
            assert!(
                drawn.iter().any(|(_, rect, text)| *text == amount
                    && rect.left() > name.right()
                    && (rect.center().y - name.center().y).abs() < 4.0),
                "{size:?}: {amount} is on the row of {step:?}"
            );
        }
        at(&format!("保存先　{}", data_root().display()));
        assert!(
            !drawn
                .iter()
                .any(|(_, _, text)| text.starts_with("続けて、")),
            "{size:?}: the list replaces the sentence about what comes next"
        );
    }
}
