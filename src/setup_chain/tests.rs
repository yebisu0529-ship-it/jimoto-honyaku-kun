use super::*;
use crate::setup::{bytes, start_label, tests::app_with_fixture_gpu};
use plamo_translate::{
    hardware::Hardware, installer::InstallResult, nemotron_setup, punctuation_setup,
};

const GIB: u64 = 1_073_741_824;

fn failed<T>() -> Result<T, String> {
    Err("checksum mismatch".into())
}
fn stopped<T>() -> Result<T, String> {
    Err("準備を中止しました。".into())
}

type Drawn = Vec<(egui::Rect, egui::Rect, String)>;
/// Every text of a shape tree: the clip it is drawn under, its own rectangle and its words.
fn visit(shape: &egui::epaint::Shape, clip: egui::Rect, out: &mut Drawn) {
    match shape {
        egui::epaint::Shape::Text(text) => out.push((
            clip,
            egui::Rect::from_min_size(text.pos, text.galley.size()),
            text.galley.job.text.clone(),
        )),
        egui::epaint::Shape::Vec(shapes) => shapes.iter().for_each(|s| visit(s, clip, out)),
        _ => {}
    }
}
fn texts_of(output: &egui::FullOutput) -> Drawn {
    let mut out = Vec::new();
    for shape in &output.shapes {
        visit(&shape.shape, shape.clip_rect, &mut out);
    }
    out
}
/// The whole window at this size, after three frames so layout has settled.
fn drawn(app: &mut App, size: (f32, f32)) -> Drawn {
    let context = egui::Context::default();
    ui::configure_theme(&context);
    let mut output = None;
    for frame in 0..3 {
        output = Some(context.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(size.0, size.1),
                )),
                time: Some(f64::from(frame) / 60.0),
                ..Default::default()
            },
            |ctx| app.render_ui(ctx),
        ));
    }
    texts_of(&output.unwrap())
}
fn installed() -> InstallResult {
    InstallResult {
        model_path: "C:/store/models/rev/plamo.gguf".into(),
        llama_server: "C:/store/runtime/llama/llama-server.exe".into(),
        template_path: "C:/store/templates/plamo.jinja".into(),
        backend: "cuda".into(),
    }
}
fn settings_of(app: &App) -> serde_json::Value {
    serde_json::to_value(&app.settings).unwrap()
}
/// Keeps the model store in a folder of the test's own.
fn stored_in(mut app: App, store: &tempfile::TempDir) -> App {
    app.settings.model_store = store.path().to_string_lossy().into();
    app
}
/// The chain with these steps, the first one started, as the frame loop leaves it.
fn started(app: &mut App, steps: &[Step]) {
    let mut chain = Chain::new(steps.iter().copied());
    chain.take_next();
    app.setup.chain = Some(chain);
}
fn next_step(app: &mut App) -> Option<Step> {
    app.setup.chain.as_mut().and_then(Chain::take_next)
}
fn process(app: &mut App, event: Event) {
    app.tx.send(event).unwrap();
    app.process(&egui::Context::default());
}
#[test]
fn the_button_prepares_the_helper_speech_recognition_and_voice_only_as_chosen() {
    let nothing = tempfile::tempdir().unwrap();
    let roots = Roots::under(nothing.path());
    let mut app = app_with_fixture_gpu(24 * GIB, 5 * GIB);
    let steps = |app: &App| app.plan_under(&roots).unwrap().steps();
    assert_eq!(
        steps(&app),
        [Step::Translation, Step::Punctuation, Step::Speech]
    );
    app.settings.english_punctuation = false;
    assert_eq!(
        steps(&app),
        [Step::Translation, Step::Speech],
        "the box is off"
    );
    app.settings.readout_enabled = true;
    assert_eq!(
        steps(&app),
        [Step::Translation, Step::Speech, Step::Voice],
        "the voice comes with 読み上げを使う"
    );
    app.settings.asr_backend = "whisper".into();
    assert_eq!(
        steps(&app),
        [Step::Translation, Step::Voice],
        "Qwen and Whisper are prepared from the settings"
    );
    app.settings.readout_enabled = false;
    assert_eq!(steps(&app), [Step::Translation]);
}
#[test]
fn finishing_the_last_step_opens_the_live_screen_without_starting_to_listen() {
    let store = tempfile::tempdir().unwrap();
    let mut app = stored_in(app_with_fixture_gpu(24 * GIB, 5 * GIB), &store);
    app.tab = Tab::Setup;
    let order = [Step::Translation, Step::Punctuation, Step::Speech];
    app.setup.chain = Some(Chain::new(order));
    let mut ran = Vec::new();
    while let Some(step) = next_step(&mut app) {
        assert!(app.tab == Tab::Setup, "{step:?} runs on the setup screen");
        ran.push(step);
        app.finish_step(step);
    }
    assert_eq!(ran, order);
    assert!(app.setup.chain.is_none());
    assert!(app.tab == Tab::Live);
    assert_eq!(app.status, "準備ができました。");
    assert!(
        !app.audio_enabled && !app.audio_start_requested && app.audio.is_none(),
        "listening starts only when the user starts it"
    );
    assert!(!app.caption_enabled);
}
#[test]
fn a_user_who_went_to_another_screen_is_not_pulled_back_when_it_finishes() {
    let store = tempfile::tempdir().unwrap();
    let mut app = stored_in(app_with_fixture_gpu(24 * GIB, 5 * GIB), &store);
    app.tab = Tab::Text;
    started(&mut app, &[Step::Translation]);
    app.finish_step(Step::Translation);
    assert!(app.setup.chain.is_none());
    assert!(app.tab == Tab::Text);
    assert_eq!(app.status, "準備ができました。");
}
#[test]
fn a_failure_in_step_three_keeps_the_first_two_and_again_resumes_at_step_three() {
    let mut app = app_with_fixture_gpu(24 * GIB, 5 * GIB);
    app.apply_installation(&installed());
    app.setup.chain = Some(Chain::new([
        Step::Translation,
        Step::Punctuation,
        Step::Speech,
    ]));
    for step in [Step::Translation, Step::Punctuation] {
        assert_eq!(next_step(&mut app), Some(step));
        app.finish_step(step);
    }
    assert_eq!(next_step(&mut app), Some(Step::Speech));
    let adopted = settings_of(&app);
    process(
        &mut app,
        Event::AudioPrepared {
            backend: "nemotron".into(),
            result: Err("HTTP 503".into()),
        },
    );
    let chain = app.setup.chain.as_ref().unwrap();
    assert_eq!(chain.outcome(), Outcome::Failed);
    assert_eq!(chain.failure(), Some("HTTP 503"));
    assert_eq!(chain.done(), [Step::Translation, Step::Punctuation]);
    assert_eq!(chain.position(), (3, 3));
    assert!(!app.setup.installing());
    assert_eq!(
        settings_of(&app),
        adopted,
        "the model adopted in step 1 stays in use"
    );
    assert!(
        app.service_error.is_empty() && !app.audio_confirmation,
        "the chain shows the failure itself"
    );
    app.retry_chain();
    assert!(app.setup.installing());
    assert_eq!(
        next_step(&mut app),
        Some(Step::Speech),
        "step 3 again, not step 1"
    );
}
#[test]
fn each_step_reports_its_own_failure_and_changes_no_setting() {
    for (step, event) in [
        (Step::Translation, Event::Installed(failed())),
        (
            Step::Punctuation,
            Event::ParagraphTools(paragraph_tools::ToolsEvent::Prepared {
                result: failed(),
                cancelled: false,
            }),
        ),
        (
            Step::Speech,
            Event::AudioPrepared {
                backend: "nemotron".into(),
                result: failed(),
            },
        ),
        (
            Step::Voice,
            Event::Readout(readout::ReadoutEvent::Installed(failed())),
        ),
    ] {
        let mut app = app_with_fixture_gpu(24 * GIB, 5 * GIB);
        started(&mut app, &[step]);
        let before = settings_of(&app);
        process(&mut app, event);
        let chain = app.setup.chain.as_ref().unwrap();
        assert_eq!(chain.outcome(), Outcome::Failed, "{step:?}");
        assert_eq!(chain.failure(), Some("checksum mismatch"), "{step:?}");
        assert_eq!(settings_of(&app), before, "{step:?}");
    }
}
#[test]
fn cancelling_stops_the_step_that_runs_and_adopts_nothing_from_it() {
    for (step, event) in [
        (Step::Translation, Event::Installed(stopped())),
        (
            Step::Punctuation,
            Event::ParagraphTools(paragraph_tools::ToolsEvent::Prepared {
                result: stopped(),
                cancelled: true,
            }),
        ),
        (
            Step::Speech,
            Event::AudioPrepared {
                backend: "nemotron".into(),
                result: stopped(),
            },
        ),
        (
            Step::Voice,
            Event::Readout(readout::ReadoutEvent::Installed(stopped())),
        ),
    ] {
        let mut app = app_with_fixture_gpu(24 * GIB, 5 * GIB);
        app.settings.english_punctuation = false;
        let earlier = if step == Step::Translation {
            vec![]
        } else {
            vec![Step::Translation]
        };
        app.setup.chain = Some(Chain::new(earlier.iter().copied().chain([step])));
        for done in &earlier {
            next_step(&mut app);
            app.finish_step(*done);
        }
        next_step(&mut app);
        let before = settings_of(&app);
        app.cancel_chain();
        assert!(app.step_cancel(step).load(Ordering::Relaxed), "{step:?}");
        process(&mut app, event);
        let chain = app.setup.chain.as_ref().unwrap();
        assert_eq!(chain.outcome(), Outcome::Cancelled, "{step:?}");
        assert_eq!(chain.failure(), None, "{step:?}");
        assert_eq!(chain.done(), earlier, "{step:?}");
        assert_eq!(settings_of(&app), before, "{step:?}");
        assert!(app.setup.progress.is_none() && app.service_error.is_empty());
        app.retry_chain();
        assert_eq!(next_step(&mut app), Some(step), "{step:?} again");
    }
}
#[test]
fn a_new_installation_gets_nemotron_and_an_existing_choice_is_kept() {
    assert_eq!(Settings::default().asr_backend, "nemotron");
    let fresh: Settings = serde_json::from_str("{}").unwrap();
    assert_eq!(fresh.asr_backend, "nemotron");
    for (backend, model, device) in [
        (
            "nemotron",
            "nvidia/nemotron-speech-streaming-en-0.6b",
            "auto",
        ),
        ("whisper", "base", "cpu"),
        ("qwen", "Qwen/Qwen3-ASR-0.6B-hf", "cpu"),
    ] {
        let mut app = app_with_fixture_gpu(24 * GIB, 5 * GIB);
        app.setup.hardware.as_mut().unwrap().gpus[0].driver_version = Some("572.00".into());
        app.refresh_recommendation();
        assert_eq!(
            app.setup.recommendation.as_ref().unwrap().asr_backend,
            "qwen",
            "this PC would be given Qwen3-ASR"
        );
        app.settings.asr_backend = backend.into();
        app.settings.asr_model = model.into();
        app.settings.asr_device = device.into();
        app.apply_installation(&installed());
        assert_eq!(
            (
                app.settings.asr_backend.as_str(),
                app.settings.asr_model.as_str(),
                app.settings.asr_device.as_str()
            ),
            (backend, model, device)
        );
        assert_eq!(app.settings.model, "C:/store/models/rev/plamo.gguf");
    }
}
#[test]
fn the_total_is_what_the_steps_still_have_to_download() {
    let nothing = tempfile::tempdir().unwrap();
    let roots = Roots::under(nothing.path());
    let app = app_with_fixture_gpu(24 * GIB, 5 * GIB);
    let mut request = app.install_request().unwrap();
    request.install_root = roots.engine.clone();
    let steps = [
        installer::pending_download_bytes(&request),
        punctuation_setup::pending_download_bytes(&roots.punctuation),
        nemotron_setup::pending_download_bytes(&roots.nemotron, "vulkan"),
    ];
    assert_eq!(steps[1], 86_018_001);
    assert_eq!(steps[2], 22_417_032 + 699_872_960);
    let fresh = app.plan_under(&roots).unwrap();
    assert_eq!(fresh.total(), steps.iter().sum::<u64>());
    let model = app.setup_model().unwrap();
    let saved = roots
        .engine
        .join("models")
        .join(&model.revision)
        .join(&model.filename);
    fs::create_dir_all(saved.parent().unwrap()).unwrap();
    fs::File::create(&saved)
        .unwrap()
        .set_len(model.bytes)
        .unwrap();
    assert_eq!(
        app.plan_under(&roots).unwrap().total(),
        fresh.total() - model.bytes,
        "a model that is saved is not counted again"
    );
}
#[test]
fn the_label_gives_the_total_rounded_up_and_nothing_when_all_is_saved() {
    let plan = |bytes| Plan {
        steps: vec![(Step::Translation, bytes)],
    };
    assert_eq!(
        start_label("はじめる", Some(plan(5_734 * 1_048_576))),
        "はじめる（合計 5.6 GB）"
    );
    assert_eq!(
        start_label("はじめる", Some(plan(1))),
        "はじめる（合計 1 MB）",
        "a small remainder never reads as 0 MB"
    );
    assert_eq!(start_label("はじめる", Some(plan(0))), "はじめる");
    assert_eq!(start_label("はじめる", None), "はじめる");
    assert_eq!(
        start_label("続きから再開", Some(plan(300 * 1_048_576))),
        "続きから再開（合計 300 MB）"
    );
}
#[test]
fn the_button_names_the_total_and_goes_plain_when_there_is_nothing_to_download() {
    let nothing = tempfile::tempdir().unwrap();
    let mut app = app_with_fixture_gpu(24 * GIB, 5 * GIB);
    app.seed_plan(&Roots::under(nothing.path()));
    let total = app.download_plan().unwrap().total();
    let label = format!(
        "はじめる（合計 {}）",
        bytes(total.next_multiple_of(1_048_576))
    );
    for size in [(860.0, 620.0), (2560.0, 1369.0)] {
        assert!(
            drawn(&mut app, size)
                .iter()
                .any(|(_, _, text)| *text == label),
            "{label} at {size:?}"
        );
    }
    app.setup.plan = Some((app.plan_key().unwrap(), Plan::default()));
    assert!(
        drawn(&mut app, (860.0, 620.0))
            .iter()
            .any(|(_, _, text)| text == "はじめる"),
        "everything is saved"
    );
}
#[test]
fn a_stopped_chain_is_forgotten_once_the_choice_changes_but_a_running_one_is_not() {
    let mut app = app_with_fixture_gpu(24 * GIB, 5 * GIB);
    started(&mut app, &[Step::Translation, Step::Speech]);
    app.forget_stopped_chain();
    assert!(app.setup.installing(), "a running chain stays");
    app.stop_step(Step::Translation, false, "HTTP 503".into());
    app.forget_stopped_chain();
    assert!(app.setup.chain.is_none());
    started(&mut app, &[Step::Translation]);
    app.stop_step(Step::Translation, true, String::new());
    app.recalculate_recommendation();
    assert!(app.setup.chain.is_none(), "a new measurement starts over");
}
#[test]
fn a_model_in_use_and_loaded_on_the_chosen_runtime_is_not_installed_again() {
    let store = tempfile::tempdir().unwrap();
    let server = store.path().join("llama-server.exe");
    fs::write(&server, b"").unwrap();
    let loaded = || {
        let mut app = stored_in(app_with_fixture_gpu(24 * GIB, 5 * GIB), &store);
        app.setup.in_use = app.setup_model();
        app.settings.llama_server = server.to_string_lossy().into();
        app.settings.backend = app
            .setup
            .recommendation
            .as_ref()
            .unwrap()
            .backend
            .as_str()
            .into();
        app.backend_ready = true;
        app
    };
    let mut app = loaded();
    assert!(app.translation_is_current());
    let before = settings_of(&app);
    app.setup.chain = Some(Chain::new([Step::Translation]));
    app.run_chain();
    assert!(
        app.setup.chain.is_none(),
        "nothing to install, so it finished"
    );
    assert_eq!(settings_of(&app), before);
    assert!(app.rx.try_recv().is_err(), "no installer was started");
    for (name, change) in [
        (
            "the server is not running",
            (|app: &mut App| app.backend_ready = false) as fn(&mut App),
        ),
        ("another runtime", |app| app.settings.backend = "cpu".into()),
        ("no model in use", |app| app.setup.in_use = None),
        ("another model chosen", |app| {
            app.setup.selected_id = "plamo3-2b-q4_k_m".into()
        }),
        ("the runtime is gone", |app| {
            app.settings.llama_server = "missing/llama-server.exe".into()
        }),
    ] {
        let mut other = loaded();
        change(&mut other);
        assert!(!other.translation_is_current(), "{name}");
    }
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
    app.start_chain();
    assert!(app.setup.chain.is_none());
    assert!(!app.setup.recommendation.as_ref().unwrap().feasible);
    assert_eq!(app.settings.model, original);
    assert!(app.rx.try_recv().is_err());
}
#[test]
fn the_steps_own_dialogs_stay_closed_while_the_chain_runs_them() {
    let titles = |app: &mut App| {
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
                    time: Some(f64::from(frame) / 60.0),
                    ..Default::default()
                },
                |ctx| {
                    app.audio_setup_ui(ctx);
                    app.readout_setup_ui(ctx);
                },
            ));
        }
        texts_of(&output.unwrap())
            .into_iter()
            .map(|(_, _, text)| text)
            .collect::<Vec<_>>()
    };
    for (step, title) in [
        (Step::Speech, "音声認識の追加準備"),
        (Step::Voice, "読み上げの準備"),
    ] {
        let mut app = app_with_fixture_gpu(24 * GIB, 5 * GIB);
        app.audio_installing = step == Step::Speech;
        app.readout.preparing = step == Step::Voice;
        assert!(
            titles(&mut app).iter().any(|text| text == title),
            "{title} shows its own progress when the chain is not the one preparing"
        );
        started(&mut app, &[step]);
        assert!(
            !titles(&mut app).iter().any(|text| text == title),
            "{title} would repeat the chain's progress"
        );
    }
}
#[test]
fn every_state_of_the_bar_is_drawn_whole_inside_the_window_at_both_sizes() {
    fn progress(message: &str) -> Progress {
        Progress {
            stage: "download".into(),
            downloaded_bytes: 41_000_000,
            total_bytes: Some(64_717_756),
            message: message.into(),
        }
    }
    type State = (&'static str, fn(&mut App), &'static [&'static str]);
    let states: [State; 8] = [
        ("before starting", |_| {}, &["はじめる（合計"]),
        (
            "the translation model",
            |app| {
                started(app, &[Step::Translation, Step::Punctuation, Step::Speech]);
                app.setup.progress = Some(progress("plamo-3-translate-8b-Q4_K_M.ggufを取得中"));
            },
            &["1/3 翻訳モデルを準備中", "plamo-3-translate-8b", "中止"],
        ),
        (
            "the helper with a long file name",
            |app| {
                started(app, &[Step::Punctuation, Step::Speech]);
                app.paragraph_tools.progress = Some(progress(
                    "sherpa-onnx-punct-ct-transformer-zh-en-vocab272727-2024-04-12-int8.tar.bz2を取得中",
                ));
            },
            &[
                "1/2 英語の文章区切り補助を準備中",
                "sherpa-onnx-punct",
                "中止",
            ],
        ),
        (
            "speech recognition before any progress",
            |app| started(app, &[Step::Speech]),
            &["1/1 音声認識を準備中", "準備を始めています", "中止"],
        ),
        (
            "a cancel that was asked for",
            |app| {
                started(app, &[Step::Voice]);
                app.cancel_chain();
            },
            &["1/1 読み上げの声を準備中", "中止しています"],
        ),
        (
            "a failure in the first step",
            |app| {
                started(app, &[Step::Translation, Step::Punctuation, Step::Speech]);
                app.stop_step(Step::Translation, false, "HTTP 503".into());
            },
            &[
                "1/3 翻訳モデルを準備できませんでした。取得済みのデータは保持しています。",
                "もう一度",
            ],
        ),
        (
            "a failure with long texts",
            |app| {
                started(app, &[Step::Translation, Step::Punctuation, Step::Speech]);
                for step in [Step::Translation, Step::Punctuation] {
                    app.finish_step(step);
                    next_step(app);
                }
                app.stop_step(
                    Step::Speech,
                    false,
                    "checksum mismatch in a long log line. ".repeat(200),
                );
                app.service_error = "翻訳サービスの長いエラーです。".repeat(80);
            },
            &[
                "3/3 音声認識を準備できませんでした。ここまでの準備は保持しています。",
                "もう一度",
            ],
        ),
        (
            "a cancelled run",
            |app| {
                started(app, &[Step::Translation, Step::Speech]);
                app.stop_step(Step::Translation, true, String::new());
            },
            &["中止しました。", "続きから再開"],
        ),
    ];
    for size in [(860.0, 620.0), (2560.0, 1369.0)] {
        for (name, build, wanted) in states {
            let nothing = tempfile::tempdir().unwrap();
            let mut app = app_with_fixture_gpu(24 * GIB, 5 * GIB);
            app.seed_plan(&Roots::under(nothing.path()));
            build(&mut app);
            let pool = drawn(&mut app, size);
            let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(size.0, size.1));
            let mut found = Vec::new();
            for want in wanted {
                let (clip, rect, _) = pool
                    .iter()
                    .find(|(_, _, text)| text.starts_with(want))
                    .unwrap_or_else(|| panic!("{name} {size:?}: {want:?} is not drawn"));
                assert!(
                    clip.expand(0.5).contains_rect(*rect) && screen.contains_rect(*rect),
                    "{name} {size:?}: {want:?} is cut off: {rect:?} in {clip:?}"
                );
                found.push((*want, *rect));
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

const ALL: [Step; 4] = [
    Step::Translation,
    Step::Punctuation,
    Step::Speech,
    Step::Voice,
];

#[test]
fn steps_start_one_after_another_in_the_order_given() {
    let mut chain = Chain::new(ALL);
    let mut started = Vec::new();
    while let Some(step) = chain.take_next() {
        assert_eq!(chain.take_next(), None, "the next waits for this one");
        started.push(step);
        assert!(chain.complete(step));
    }
    assert_eq!(started, ALL);
    assert_eq!(chain.done(), ALL);
    assert!(chain.is_finished());
}

#[test]
fn the_number_follows_the_step_that_runs() {
    let mut chain = Chain::new([Step::Translation, Step::Punctuation, Step::Speech]);
    assert_eq!(chain.position(), (1, 3));
    chain.take_next();
    assert_eq!(chain.position(), (1, 3));
    chain.complete(Step::Translation);
    assert_eq!(chain.position(), (2, 3));
    chain.take_next();
    chain.complete(Step::Punctuation);
    chain.take_next();
    assert_eq!(chain.position(), (3, 3));
    chain.complete(Step::Speech);
    assert_eq!(chain.position(), (3, 3), "never past the last step");
}

#[test]
fn only_the_step_that_runs_can_finish_or_stop() {
    let mut chain = Chain::new([Step::Translation, Step::Speech]);
    assert!(!chain.complete(Step::Translation), "not started");
    chain.take_next();
    assert!(!chain.complete(Step::Speech), "a later step");
    assert!(!chain.fail(Step::Speech, "late".into()));
    assert!(!chain.cancel(Step::Speech));
    assert_eq!(chain.outcome(), Outcome::Running);
    assert_eq!(chain.current(), Some(Step::Translation));
}

#[test]
fn a_failure_keeps_the_finished_steps_and_retry_starts_at_the_failed_one() {
    let mut chain = Chain::new([Step::Translation, Step::Punctuation, Step::Speech]);
    for step in [Step::Translation, Step::Punctuation] {
        assert_eq!(chain.take_next(), Some(step));
        chain.complete(step);
    }
    assert_eq!(chain.take_next(), Some(Step::Speech));
    assert!(chain.fail(Step::Speech, "HTTP 503".into()));
    assert_eq!(chain.outcome(), Outcome::Failed);
    assert_eq!(chain.failure(), Some("HTTP 503"));
    assert_eq!(chain.done(), [Step::Translation, Step::Punctuation]);
    assert_eq!(chain.position(), (3, 3));
    assert_eq!(
        chain.take_next(),
        None,
        "nothing starts while it is stopped"
    );
    assert!(chain.retry());
    assert_eq!(chain.failure(), None);
    assert_eq!(chain.take_next(), Some(Step::Speech));
    assert!(chain.complete(Step::Speech));
    assert!(chain.is_finished());
    assert_eq!(chain.done().len(), 3);
}

#[test]
fn a_cancelled_step_is_started_again_by_retry_and_the_others_stay_done() {
    let mut chain = Chain::new([Step::Translation, Step::Punctuation]);
    chain.take_next();
    chain.complete(Step::Translation);
    chain.take_next();
    assert!(chain.cancel(Step::Punctuation));
    assert_eq!(chain.outcome(), Outcome::Cancelled);
    assert_eq!(chain.failure(), None);
    assert!(!chain.is_finished());
    assert!(chain.retry());
    assert!(!chain.retry(), "nothing to retry while it runs");
    assert_eq!(chain.take_next(), Some(Step::Punctuation));
    assert_eq!(chain.done(), [Step::Translation]);
}
