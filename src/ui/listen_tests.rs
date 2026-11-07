//! 「聞く」 turns on the input that was chosen, 「止める」 turns off every input,
//! and Ctrl+Alt+L is the same button.
use super::home_tests::{Window, hardware};
use super::*;
use crate::chat::testing::{filled, find_text, texts};
use global_hotkey::hotkey::{Code, HotKey, Modifiers};

/// The chat with the translation model ready and speech recognition not on
/// this PC: Whisper, with no Python to run it. Nothing is installed, no audio
/// is captured, and the PC's capacity is already known, so opening the model
/// screen measures nothing.
fn chat_without_speech() -> App {
    let mut app = App::inert();
    app.backend_ready = true;
    app.setup.hardware = Some(hardware());
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

fn drawn(app: &mut App, width: f32, height: f32) -> Window {
    let mut window = Window::new(width, height);
    window.settle(app);
    window
}

#[test]
fn the_button_listens_in_the_automatic_mode_and_stops_every_input() {
    let mut app = chat_without_speech();
    let mut window = drawn(&mut app, 860.0, 620.0);
    assert!(window.has("● 聞く") && window.has("止まっています"));
    assert!(!window.has("■ 止める"));

    window.click(&mut app, "● 聞く");
    assert_eq!(
        armed(&app),
        (true, true),
        "captions and PC audio, as 両方 did"
    );
    assert!(
        app.audio_confirmation,
        "speech recognition is not prepared, so the usual question is asked"
    );
    assert!(
        !app.audio_enabled && app.audio.is_none(),
        "no audio is taken"
    );
    assert!(window.has("■ 止める") && !window.has("● 聞く"));
    assert!(!window.has("止まっています"));

    window.click(&mut app, "■ 止める");
    assert_eq!(armed(&app), (false, false));
    assert!(!app.audio_enabled && app.screen_feed.is_none());
    assert!(window.has("● 聞く") && window.has("止まっています"));
}

#[test]
fn the_button_stops_pc_audio_and_the_screen_feed_that_are_on() {
    let mut app = chat_without_speech();
    app.audio_enabled = true;
    app.asr_ready = true;
    let mut window = drawn(&mut app, 860.0, 620.0);
    window.click(&mut app, "■ 止める");
    assert!(!app.audio_enabled && !app.caption_enabled);
    assert!(window.has("● 聞く"));

    let mut app = chat_without_speech();
    app.start_screen_feed(1, (0, 0, 120, 40));
    let mut window = drawn(&mut app, 860.0, 620.0);
    window.click(&mut app, "■ 止める");
    assert!(app.screen_feed.is_none() && !app.caption_enabled);
}

/// Everything the line beside the button can say about the inputs and the model.
const STATE_WORDS: [&str; 9] = [
    "止まっています",
    "YouTube字幕を受け取っています",
    "PC音声を聞いています",
    "YouTube字幕を待っています",
    "音声認識を準備しています",
    "画面の文字を読み取っています",
    "入力を待っています",
    "翻訳モデルを準備しています",
    "翻訳モデルの準備が必要です",
];

#[test]
fn the_line_beside_the_button_says_what_is_going_on() {
    type Change = fn(&mut App);
    let states: [(&str, Change, &str, Color32); 7] = [
        ("止まっています", |_| {}, "● 聞く", theme::TEXT_SECONDARY),
        (
            "YouTube字幕を受け取っています",
            |app| {
                app.caption_enabled = true;
                app.caption_paused = false;
                app.last_caption = Some(Instant::now());
            },
            "■ 止める",
            theme::ACCENT,
        ),
        (
            "PC音声を聞いています",
            |app| {
                app.audio_enabled = true;
                app.asr_ready = true;
            },
            "■ 止める",
            theme::ACCENT,
        ),
        (
            "YouTube字幕を受け取っています",
            |app| {
                app.caption_enabled = true;
                app.caption_paused = false;
                app.last_caption = Some(Instant::now());
                app.audio_enabled = true;
                app.asr_ready = true;
            },
            "■ 止める",
            theme::ACCENT,
        ),
        (
            "YouTube字幕を待っています",
            |app| app.caption_enabled = true,
            "■ 止める",
            theme::TEXT_SECONDARY,
        ),
        (
            "音声認識を準備しています",
            |app| app.audio_start_requested = true,
            "■ 止める",
            theme::TEXT_SECONDARY,
        ),
        (
            "画面の文字を読み取っています",
            |app| app.start_screen_feed(1, (0, 0, 120, 40)),
            "■ 止める",
            theme::ACCENT,
        ),
    ];
    for (words, change, button, color) in states {
        let mut app = chat_without_speech();
        change(&mut app);
        let window = drawn(&mut app, 860.0, 620.0);
        assert_eq!(find_text(&window.output, words).1, color, "{words}");
        assert!(window.has(button), "{words}: the button is not {button}");
        let drawn_states: Vec<_> = STATE_WORDS
            .into_iter()
            .filter(|state| window.has(state))
            .collect();
        assert_eq!(
            drawn_states,
            [words],
            "{words}: more than one state is drawn"
        );
    }
}

#[test]
fn without_a_translation_model_the_button_cannot_start_and_the_line_says_what_is_needed() {
    let ink = |ready: bool| {
        let mut app = chat_without_speech();
        app.backend_ready = ready;
        drawn(&mut app, 860.0, 620.0).ink("● 聞く")
    };
    assert_ne!(ink(true), ink(false), "the button is not faded");

    let mut app = chat_without_speech();
    app.backend_ready = false;
    let mut window = drawn(&mut app, 860.0, 620.0);
    assert!(window.has("翻訳モデルの準備が必要です"));
    assert!(window.has("モデルの準備"), "the way to the model screen");
    window.click(&mut app, "● 聞く");
    assert_eq!(armed(&app), (false, false));
    assert!(!app.audio_confirmation && app.tab == Tab::Live);

    app.backend_loading = true;
    window.settle(&mut app);
    assert!(window.has("翻訳モデルを準備しています"));
    assert!(!window.has("翻訳モデルの準備が必要です"));

    app.backend_loading = false;
    window.settle(&mut app);
    window.click(&mut app, "モデルの準備");
    assert!(app.tab == Tab::Setup, "the way to the model screen works");
}

#[test]
fn a_model_that_goes_away_while_listening_does_not_keep_the_button_from_stopping() {
    let mut app = chat_without_speech();
    app.caption_enabled = true;
    app.backend_ready = false;
    let mut window = drawn(&mut app, 860.0, 620.0);
    let mut ready = chat_without_speech();
    ready.caption_enabled = true;
    assert_eq!(
        window.ink("■ 止める"),
        drawn(&mut ready, 860.0, 620.0).ink("■ 止める"),
        "the button is faded"
    );
    window.click(&mut app, "■ 止める");
    assert!(!app.caption_enabled);
}

#[test]
fn an_error_is_the_line_beside_the_button_in_red() {
    let mut app = chat_without_speech();
    app.service_error = "音声認識を読み込めませんでした。".into();
    app.caption_enabled = true;
    let window = drawn(&mut app, 860.0, 620.0);
    let (_, color) = find_text(&window.output, "音声認識を読み込めませんでした。");
    assert_eq!(color, theme::ERROR);
    assert!(
        window.has("■ 止める"),
        "listening goes on and can be stopped"
    );
    assert!(!window.has("YouTube字幕を待っています"));
}

#[test]
fn speech_that_is_not_prepared_follows_the_same_path_as_before() {
    let mut app = chat_without_speech();
    let mut window = drawn(&mut app, 860.0, 620.0);
    window.click(&mut app, "● 聞く");
    assert!(app.audio_confirmation && app.audio_start_requested);
    assert!(app.tab == Tab::Live, "the model screen is not opened");

    // "今はしない" in the question: the captions stay on, as 両方 left them.
    app.audio_confirmation = false;
    app.audio_start_requested = false;
    window.settle(&mut app);
    assert!(app.caption_enabled);
    assert!(window.has("■ 止める") && window.has("YouTube字幕を待っています"));
}

#[test]
fn the_hover_text_names_the_shortcut() {
    let mut app = chat_without_speech();
    let mut window = drawn(&mut app, 860.0, 620.0);
    window.hover(&mut app, "● 聞く");
    assert!(
        texts(&window.output)
            .iter()
            .any(|(text, _, _)| text.contains("Ctrl + Alt + L")),
        "{:?}",
        texts(&window.output)
    );

    app.caption_enabled = true;
    let mut window = drawn(&mut app, 860.0, 620.0);
    window.hover(&mut app, "■ 止める");
    assert!(
        texts(&window.output)
            .iter()
            .any(|(text, _, _)| text.contains("Ctrl + Alt + L")),
        "stopping has the key in its hover text as well"
    );
}

#[test]
fn the_mini_window_and_the_subtitle_window_see_the_same_state() {
    use crate::subtitle_window::Playback;
    let caption = bridge::Caption {
        text: "The library opens at nine.".into(),
        video_id: "video-a".into(),
        position: 1.0,
        paused: false,
        epoch: 2,
    };

    let mut app = chat_without_speech();
    app.document_update_subtitle_window();
    assert_eq!(app.subtitle_window.playback(), Playback::Stopped);
    let mut window = drawn(&mut app, 860.0, 620.0);
    window.click(&mut app, "● 聞く");
    app.accept_caption(caption);
    app.document_update_subtitle_window();
    assert_eq!(app.subtitle_window.playback(), Playback::Running);

    app.window_mode.enter(&egui::ViewportInfo::default());
    window.settle(&mut app);
    assert!(
        window.has("一時停止") && !window.has("入力を選ぶ"),
        "the mini window offers to pause what is listening"
    );

    let mut app = chat_without_speech();
    let mut window = drawn(&mut app, 860.0, 620.0);
    window.click(&mut app, "● 聞く");
    window.click(&mut app, "■ 止める");
    app.document_update_subtitle_window();
    assert_eq!(app.subtitle_window.playback(), Playback::Stopped);
    app.window_mode.enter(&egui::ViewportInfo::default());
    window.settle(&mut app);
    assert!(
        window.has("入力を選ぶ") && !window.has("一時停止"),
        "the mini window offers the inputs again"
    );
}

#[test]
fn the_chat_has_no_five_way_switch_and_no_explanation_line() {
    for (width, height) in [(860.0, 620.0), (2560.0, 1369.0)] {
        let mut app = chat_without_speech();
        app.caption_enabled = true;
        app.audio_enabled = true;
        app.asr_ready = true;
        let window = drawn(&mut app, width, height);
        for gone in ["停止", "字幕", "PC音声", "両方", "画面"] {
            assert!(
                !window.has(gone),
                "{gone} is still drawn at {width}x{height}"
            );
        }
        for gone in [
            "字幕を優先し",
            "停止中",
            "字幕を受信中",
            "音声を認識中",
            "入力を待っています",
            "入力を選ぶと翻訳が始まります",
        ] {
            assert!(
                !texts(&window.output)
                    .iter()
                    .any(|(text, _, _)| text.contains(gone)),
                "{gone} is still drawn at {width}x{height}"
            );
        }
    }
}

#[test]
fn the_bar_is_under_the_chat_above_the_status_row_and_inside_the_gutters() {
    for (width, height) in [(860.0, 620.0), (2560.0, 1369.0)] {
        let mut app = chat_without_speech();
        app.settings.readout_enabled = true;
        app.service_error = "Long service error. ".repeat(60);
        let window = drawn(&mut app, width, height);
        let bar = app.ui_probe.listen_bar.expect("the bar is drawn");
        let button = app.ui_probe.listen_button.expect("the button is drawn");
        let chat = app.live_chat.panel().expect("the chat is drawn");
        assert!(
            chat.bottom() <= bar.top(),
            "{width}x{height}: {chat:?} {bar:?}"
        );
        assert!(
            bar.bottom() <= height - theme::CONTROL_H,
            "the bar runs into the status row at {width}x{height}"
        );
        assert!(
            (bar.left() - theme::GUTTER).abs() < 1.0
                && (bar.right() - (width - theme::GUTTER)).abs() < 1.0,
            "{bar:?}"
        );
        assert!(
            button.height() >= theme::CONTROL_H_LARGE - 0.5,
            "{button:?}"
        );
        assert!(bar.contains_rect(button));

        let inside: Vec<_> = texts(&window.output)
            .into_iter()
            .filter(|(_, rect, _)| rect.center().y > bar.top() && rect.center().y < bar.bottom())
            .collect();
        assert!(inside.len() >= 3, "{inside:?}");
        for (index, (first, a, _)) in inside.iter().enumerate() {
            assert!(
                bar.contains_rect(*a),
                "{first} leaves the bar at {width}x{height}"
            );
            for (second, b, _) in &inside[index + 1..] {
                assert!(
                    !a.intersects(*b),
                    "{first} overlaps {second} at {width}x{height}"
                );
            }
        }
    }
}
