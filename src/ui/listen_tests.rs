//! 「聞く」 turns on the input that was chosen, 「止める」 turns off every input,
//! and Ctrl+Alt+L is the same button.
use super::home_tests::Window;
use super::*;
use crate::chat::testing::filled;
use global_hotkey::hotkey::{Code, HotKey, Modifiers};

/// The chat with the translation model ready and speech recognition not on
/// this PC: Whisper, with no Python to run it. Nothing is installed, no file of
/// this PC is read and no audio is captured.
fn chat_without_speech() -> App {
    let mut app = App::inert();
    app.backend_ready = true;
    app.settings.asr_backend = "whisper".into();
    app.settings.python = std::env::temp_dir()
        .join(format!("missing-python-{}", uuid::Uuid::new_v4()))
        .to_string_lossy()
        .into();
    app
}

fn ctrl_alt(code: Code) -> u32 {
    HotKey::new(Some(Modifiers::CONTROL | Modifiers::ALT), code).id()
}

/// The four inputs as the menu names them.
const INPUTS: [(&str, LiveSourceMode); 4] = [
    ("自動（字幕を優先）", LiveSourceMode::Auto),
    ("YouTube字幕だけ", LiveSourceMode::Captions),
    ("PC音声だけ", LiveSourceMode::Audio),
    ("画面の文字", LiveSourceMode::Screen),
];

/// 「聞く」 pressed on the keyboard.
fn listen(app: &mut App) {
    app.press_hotkey(&egui::Context::default(), ctrl_alt(Code::KeyL));
}

/// Chooses `label` under 入力の切り替え in the menu, the way a pointer does.
fn choose(window: &mut Window, app: &mut App, label: &str) {
    window.open_menu(app);
    window.click(app, "入力の切り替え");
    window.click(app, label);
}

/// The captions and the PC audio as the app holds them.
fn armed(app: &App) -> (bool, bool) {
    (app.caption_enabled, app.audio_start_requested)
}

#[test]
fn ctrl_alt_l_listens_in_the_automatic_mode_and_the_next_press_stops_every_input() {
    let ctx = egui::Context::default();
    let mut app = chat_without_speech();
    assert!(app.listen_input == LiveSourceMode::Auto);

    app.press_hotkey(&ctx, ctrl_alt(Code::KeyL));
    assert!(
        app.caption_enabled && app.audio_start_requested,
        "captions and PC audio are both armed, as 両方 did"
    );
    assert!(
        app.audio_confirmation,
        "speech recognition is not prepared, so the usual question is asked"
    );
    assert!(
        !app.audio_enabled && app.audio.is_none(),
        "no audio is taken"
    );

    app.press_hotkey(&ctx, ctrl_alt(Code::KeyL));
    assert!(!app.caption_enabled && !app.audio_enabled && !app.audio_start_requested);
}

#[test]
fn the_press_that_stops_ends_pc_audio_and_the_screen_feed_too() {
    let ctx = egui::Context::default();

    let mut app = chat_without_speech();
    app.audio_enabled = true;
    app.press_hotkey(&ctx, ctrl_alt(Code::KeyL));
    assert!(
        !app.audio_enabled,
        "PC audio was listening, so the press stops it"
    );
    assert!(!app.caption_enabled, "and it does not start anything else");

    let mut app = chat_without_speech();
    app.start_screen_feed(1, (0, 0, 120, 40));
    assert!(
        app.screen_feed.is_some(),
        "Windows OCR needs no preparation"
    );
    app.press_hotkey(&ctx, ctrl_alt(Code::KeyL));
    assert!(app.screen_feed.is_none() && !app.caption_enabled);
}

#[test]
fn without_a_translation_model_the_shortcut_starts_nothing_and_says_why() {
    let ctx = egui::Context::default();
    let mut app = chat_without_speech();
    app.backend_ready = false;
    app.press_hotkey(&ctx, ctrl_alt(Code::KeyL));
    assert!(!app.caption_enabled && !app.audio_start_requested && !app.audio_confirmation);
    assert!(
        app.tab == Tab::Live,
        "the model screen is not opened for it"
    );
    assert_eq!(app.status, "モデルの準備ができると、聞けます。");
}

#[test]
fn the_other_shortcut_still_reads_a_range() {
    let ctx = egui::Context::default();
    let mut app = chat_without_speech();
    app.press_hotkey(&ctx, ctrl_alt(Code::KeyT));
    assert_eq!(
        app.status, "取得できる画面がありません。",
        "this PC has no screen to read, and says so"
    );
    assert!(!app.caption_enabled && !app.audio_start_requested);
}

#[test]
fn the_four_inputs_are_one_menu_item_on_the_chat_and_on_no_other_screen() {
    let mut app = chat_without_speech();
    let mut window = Window::new(860.0, 620.0);
    window.open_menu(&mut app);
    assert!(
        !window.has("自動（字幕を優先）"),
        "they are behind the item"
    );
    window.click(&mut app, "入力の切り替え");
    let tops: Vec<f32> = INPUTS
        .iter()
        .map(|(label, _)| window.at(label).top())
        .collect();
    assert!(
        tops.windows(2).all(|pair| pair[0] < pair[1]),
        "the inputs are out of order: {tops:?}"
    );

    for tab in [Tab::Text, Tab::Files, Tab::Settings, Tab::Setup] {
        let mut app = chat_without_speech();
        app.tab = tab;
        let mut window = Window::new(860.0, 620.0);
        window.open_menu(&mut app);
        assert!(!window.has("入力の切り替え"));
    }
}

#[test]
fn the_menu_marks_the_input_that_is_on_or_else_the_one_the_next_listen_turns_on() {
    let marked = |window: &Window| {
        INPUTS
            .iter()
            .filter(|(label, _)| {
                filled(&window.output, theme::ACCENT_SOFT)
                    .iter()
                    .any(|rect| rect.contains_rect(window.at(label)))
            })
            .map(|(label, _)| *label)
            .collect::<Vec<_>>()
    };
    let mut app = chat_without_speech();
    let mut window = Window::new(860.0, 620.0);
    window.open_menu(&mut app);
    window.click(&mut app, "入力の切り替え");
    assert_eq!(marked(&window), ["自動（字幕を優先）"], "the default");

    window.click(&mut app, "PC音声だけ");
    window.open_menu(&mut app);
    window.click(&mut app, "入力の切り替え");
    assert_eq!(
        marked(&window),
        ["PC音声だけ"],
        "what the next 聞く turns on"
    );

    // The captions run on their own, as the mini window can leave them.
    app.caption_enabled = true;
    window.settle(&mut app);
    assert_eq!(marked(&window), ["YouTube字幕だけ"], "what is on now");
}

#[test]
fn a_choice_made_while_stopped_waits_for_the_next_listen() {
    for (label, mode) in INPUTS {
        let mut app = chat_without_speech();
        let settings = serde_json::to_value(&app.settings).unwrap();
        let mut window = Window::new(860.0, 620.0);
        choose(&mut window, &mut app, label);
        assert!(app.listen_input == mode, "{label} is not remembered");
        assert_eq!(armed(&app), (false, false), "{label} started by itself");
        assert!(app.screen_feed.is_none() && !app.audio_confirmation);
        assert!(app.status.is_empty(), "{label}: {}", app.status);
        assert_eq!(
            serde_json::to_value(&app.settings).unwrap(),
            settings,
            "the choice is not a setting"
        );

        listen(&mut app);
        match mode {
            LiveSourceMode::Auto => assert_eq!(armed(&app), (true, true), "{label}"),
            LiveSourceMode::Captions => assert_eq!(armed(&app), (true, false), "{label}"),
            LiveSourceMode::Audio => assert_eq!(armed(&app), (false, true), "{label}"),
            LiveSourceMode::Screen => {
                assert_eq!(armed(&app), (false, false), "{label}");
                assert_eq!(
                    app.status, "取得できる画面がありません。",
                    "this PC has no screen to read, and says so"
                );
            }
        }
        listen(&mut app);
        if mode != LiveSourceMode::Screen {
            assert_eq!(armed(&app), (false, false), "{label}: 止める");
        }
    }
}

#[test]
fn a_choice_made_while_listening_switches_at_once() {
    for (label, mode) in INPUTS {
        let mut app = chat_without_speech();
        listen(&mut app);
        assert_eq!(armed(&app), (true, true));
        let generation = app.caption_generation;
        let mut window = Window::new(860.0, 620.0);
        choose(&mut window, &mut app, label);
        assert!(app.listen_input == mode);
        match mode {
            LiveSourceMode::Auto => {
                assert_eq!(armed(&app), (true, true), "{label}");
                assert_eq!(
                    app.caption_generation, generation,
                    "{label} did not restart"
                );
            }
            LiveSourceMode::Captions => assert_eq!(armed(&app), (true, false), "{label}"),
            LiveSourceMode::Audio => assert_eq!(armed(&app), (false, true), "{label}"),
            LiveSourceMode::Screen => {
                assert_eq!(
                    app.status, "取得できる画面がありません。",
                    "the range to read is asked for first"
                );
                assert_eq!(
                    armed(&app),
                    (true, true),
                    "and what runs goes on until then"
                );
            }
        }
    }

    // PC audio that is really on stops when captions are chosen.
    let mut app = chat_without_speech();
    app.audio_enabled = true;
    let mut window = Window::new(860.0, 620.0);
    choose(&mut window, &mut app, "YouTube字幕だけ");
    assert!(!app.audio_enabled && app.caption_enabled);
}

#[test]
fn the_menu_entry_for_the_screen_still_starts_reading_at_once_and_is_the_same_choice() {
    let mut app = chat_without_speech();
    let mut window = Window::new(860.0, 620.0);
    window.open_menu(&mut app);
    window.click(&mut app, "画面の文字を翻訳");
    assert!(app.listen_input == LiveSourceMode::Screen);
    assert_eq!(
        app.status, "取得できる画面がありません。",
        "it asks for a range now, without a press of 聞く"
    );
}
