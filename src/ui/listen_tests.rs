//! 「聞く」 turns on the input that was chosen, 「止める」 turns off every input,
//! and Ctrl+Alt+L is the same button.
use super::home_tests::{Window, chat, hardware, key};
use super::*;
use crate::chat::testing::{filled, find_text, texts};
use global_hotkey::hotkey::{Code, HotKey, Modifiers};

/// The chat with the translation model ready and speech recognition not on
/// this PC: Whisper, with no Python to run it. Nothing is installed, no audio
/// is captured, and the PC's capacity is already known, so opening the model
/// screen measures nothing.
pub(super) fn chat_without_speech() -> App {
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

pub(super) const INPUTS: [(&str, LiveSourceMode, (bool, bool)); 3] = [
    ("PC音声だけ", LiveSourceMode::Audio, (false, true)),
    ("YouTube字幕だけ", LiveSourceMode::Captions, (true, false)),
    ("自動（字幕を優先）", LiveSourceMode::Auto, (true, true)),
];
pub(super) const SCREEN: &str = "画面の文字";

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
pub(super) fn armed(app: &App) -> (bool, bool) {
    (app.caption_enabled, app.audio_start_requested)
}

/// `app` with 「YouTube字幕も訳す」 on, as a settings file that has it on
/// gives it.
pub(super) fn with_youtube_captions(mut app: App) -> App {
    let mut saved = serde_json::to_value(&app.settings).unwrap();
    saved["translate_youtube_captions"] = true.into();
    app.settings = serde_json::from_value(saved).unwrap();
    app
}

const SWITCH: &str = "YouTube字幕も訳す";

/// What the line beside the button and the 音声認識 card say while Nemotron
/// cannot hear the language to translate from.
const ENGLISH_ONLY: &str = "Nemotronは英語だけを聞き取ります。中国語などほかの言語は、設定の「音声認識」でQwen3-ASR（GPU・数GB）かWhisper（CPU・約4GB）を選んでください。";

#[test]
fn listening_starts_with_pc_audio_alone() {
    let mut app = chat_without_speech();
    listen(&mut app);
    assert_eq!(
        armed(&app),
        (false, true),
        "聞く turns on the PC audio and no captions"
    );
    assert!(
        app.audio_confirmation,
        "speech recognition is not prepared, so the usual question is asked"
    );

    let mut app = with_youtube_captions(chat_without_speech());
    listen(&mut app);
    assert_eq!(
        armed(&app),
        (false, true),
        "the PC audio comes first while the captions are offered too"
    );
}

#[test]
fn ctrl_alt_l_listens_in_the_automatic_mode_and_the_next_press_stops_every_input() {
    let ctx = egui::Context::default();
    let mut app = with_youtube_captions(chat_without_speech());
    app.listen_input = LiveSourceMode::Auto;

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
fn without_a_translation_model_the_range_shortcut_reads_nothing_and_says_why() {
    let ctx = egui::Context::default();
    let mut app = chat_without_speech();
    app.backend_ready = false;
    app.press_hotkey(&ctx, ctrl_alt(Code::KeyT));
    assert_eq!(app.status, "モデルの準備ができると、範囲翻訳を使えます。");
    assert!(!app.capture_busy && app.tab == Tab::Live);

    let mut app = chat_without_speech();
    app.capture_busy = true;
    app.status = "before".into();
    app.press_hotkey(&ctx, ctrl_alt(Code::KeyT));
    assert_eq!(
        app.status, "before",
        "a reading already running is left alone"
    );
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

fn listed(window: &Window) -> Vec<&'static str> {
    let mut drawn: Vec<_> = INPUTS
        .iter()
        .map(|(label, _, _)| *label)
        .chain([SCREEN])
        .filter(|label| window.has(label))
        .collect();
    drawn.sort_by(|a, b| window.at(a).top().total_cmp(&window.at(b).top()));
    drawn
}

#[test]
fn the_inputs_are_one_menu_item_on_the_chat_and_on_no_other_screen() {
    let mut app = chat_without_speech();
    let mut window = Window::new(860.0, 620.0);
    window.open_menu(&mut app);
    assert!(!window.has("PC音声だけ"), "they are behind the item");
    window.click(&mut app, "入力の切り替え");
    assert_eq!(listed(&window), ["PC音声だけ"]);

    for tab in [Tab::Text, Tab::Files, Tab::Settings, Tab::Setup] {
        let mut app = chat_without_speech();
        app.tab = tab;
        let mut window = Window::new(860.0, 620.0);
        window.open_menu(&mut app);
        assert!(!window.has("入力の切り替え"));
    }
}

#[test]
fn the_inputs_are_pc_audio_alone_until_youtube_captions_are_turned_on() {
    let open_list = |app: &mut App| {
        let mut window = Window::new(860.0, 620.0);
        window.open_menu(app);
        window.click(app, "入力の切り替え");
        window
    };
    let mut app = chat_without_speech();
    let window = open_list(&mut app);
    assert_eq!(
        listed(&window),
        ["PC音声だけ"],
        "the captions are an option"
    );
    assert_eq!(marked(&window), ["PC音声だけ"]);

    let mut app = with_youtube_captions(chat_without_speech());
    let window = open_list(&mut app);
    assert_eq!(
        listed(&window),
        ["PC音声だけ", "YouTube字幕だけ", "自動（字幕を優先）"]
    );
    assert_eq!(
        marked(&window),
        ["PC音声だけ"],
        "the first input is still the PC audio"
    );
}

#[test]
fn turning_youtube_captions_off_stops_them_and_leaves_pc_audio_for_the_next_listen() {
    for (input, left) in [
        ("自動（字幕を優先）", (false, true)),
        ("YouTube字幕だけ", (false, false)),
    ] {
        let mut app = with_youtube_captions(chat_without_speech());
        let mut window = Window::new(860.0, 620.0);
        choose(&mut window, &mut app, input);
        listen(&mut app);
        assert!(app.caption_enabled, "{input}: the captions are on");

        app.tab = Tab::YouTube;
        let mut window = Window::new(1280.0, 1369.0);
        window.settle(&mut app);
        assert!(window.has(SWITCH), "{input}: the switch is not drawn");
        window.click(&mut app, SWITCH);
        assert_eq!(
            armed(&app),
            left,
            "{input}: the captions stop and the PC audio is left as it was"
        );
        assert!(
            app.listen_input == LiveSourceMode::Audio,
            "{input}: the next 聞く turns on the PC audio"
        );
    }
}

#[test]
fn captions_that_arrive_while_they_are_not_translated_are_told_once_on_the_status_line() {
    let ctx = egui::Context::default();
    let caption = |text: &str, paused: bool| bridge::Caption {
        text: text.into(),
        video_id: "video-a".into(),
        position: 1.0,
        paused,
        epoch: 2,
    };
    let mut app = chat_without_speech();
    let (extension, from_extension) = mpsc::sync_channel(8);
    app.bridge_rx = from_extension;
    listen(&mut app);

    extension.send(caption("", true)).unwrap();
    app.process(&ctx);
    assert!(
        app.status.is_empty(),
        "a cleared caption is no caption: {}",
        app.status
    );

    extension
        .send(caption("The library opens at nine.", false))
        .unwrap();
    app.process(&ctx);
    assert_eq!(
        app.status,
        "YouTube字幕が届いています。設定の「その他の機能」で「YouTube字幕も訳す」をオンにすると訳せます。"
    );
    assert!(
        app.pending_caption.is_none() && app.rows.is_empty(),
        "the caption is not translated"
    );

    app.status = "設定を保存しました。".into();
    extension
        .send(caption("It closes at five.", false))
        .unwrap();
    app.process(&ctx);
    assert_eq!(app.status, "設定を保存しました。", "it is told once");
    assert!(app.pending_caption.is_none() && app.rows.is_empty());
}

/// The inputs behind 入力の切り替え that are drawn as the chosen one.
pub(super) fn marked(window: &Window) -> Vec<&'static str> {
    INPUTS
        .iter()
        .map(|(label, _, _)| *label)
        .chain([SCREEN])
        .filter(|label| {
            window.has(label)
                && filled(&window.output, theme::ACCENT_SOFT)
                    .iter()
                    .any(|rect| rect.contains_rect(window.at(label)))
        })
        .collect()
}

#[test]
fn the_list_has_the_screen_only_while_it_is_the_input_shown() {
    let three = ["PC音声だけ", "YouTube字幕だけ", "自動（字幕を優先）"];
    let four = [
        "PC音声だけ",
        "YouTube字幕だけ",
        "自動（字幕を優先）",
        SCREEN,
    ];
    let open_list = |window: &mut Window, app: &mut App| {
        window.open_menu(app);
        window.click(app, "入力の切り替え");
    };
    let mut app = with_youtube_captions(chat_without_speech());
    let mut window = Window::new(860.0, 620.0);
    open_list(&mut window, &mut app);
    assert_eq!(
        listed(&window),
        three,
        "{SCREEN} is listed while it is not the input"
    );
    window.draw(
        &mut app,
        vec![key(egui::Key::Escape, egui::Modifiers::NONE)],
    );

    window.open_other_features(&mut app);
    window.click(&mut app, "画面の文字を翻訳");
    open_list(&mut window, &mut app);
    assert_eq!(listed(&window), four);
    assert_eq!(marked(&window), [SCREEN]);

    window.click(&mut app, "PC音声だけ");
    assert!(app.listen_input == LiveSourceMode::Audio);
    open_list(&mut window, &mut app);
    assert_eq!(listed(&window), three);
    assert_eq!(marked(&window), ["PC音声だけ"]);

    let mut app = with_youtube_captions(chat_without_speech());
    app.start_screen_feed(1, (0, 0, 120, 40));
    let mut window = Window::new(860.0, 620.0);
    open_list(&mut window, &mut app);
    assert_eq!(listed(&window), four);
    assert_eq!(marked(&window), [SCREEN]);
    window.click(&mut app, "YouTube字幕だけ");
    assert!(app.screen_feed.is_none() && app.caption_enabled);
}

#[test]
fn the_menu_marks_the_input_that_is_on_or_else_the_one_the_next_listen_turns_on() {
    let mut app = with_youtube_captions(chat_without_speech());
    let mut window = Window::new(860.0, 620.0);
    window.open_menu(&mut app);
    window.click(&mut app, "入力の切り替え");
    assert_eq!(marked(&window), ["PC音声だけ"], "the default");

    window.click(&mut app, "自動（字幕を優先）");
    window.open_menu(&mut app);
    window.click(&mut app, "入力の切り替え");
    assert_eq!(
        marked(&window),
        ["自動（字幕を優先）"],
        "what the next 聞く turns on"
    );

    // The captions run on their own, as the mini window can leave them.
    app.caption_enabled = true;
    window.settle(&mut app);
    assert_eq!(marked(&window), ["YouTube字幕だけ"], "what is on now");
}

#[test]
fn a_choice_made_while_stopped_waits_for_the_next_listen() {
    for (label, mode, on) in INPUTS {
        let mut app = with_youtube_captions(chat_without_speech());
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
        assert_eq!(armed(&app), on, "{label}");
        listen(&mut app);
        assert_eq!(armed(&app), (false, false), "{label}: 止める");
    }
}

#[test]
fn a_choice_made_while_listening_switches_at_once() {
    for (label, mode, on) in INPUTS {
        let mut app = with_youtube_captions(chat_without_speech());
        app.listen_input = LiveSourceMode::Auto;
        listen(&mut app);
        assert_eq!(armed(&app), (true, true));
        let generation = app.caption_generation;
        let mut window = Window::new(860.0, 620.0);
        choose(&mut window, &mut app, label);
        assert!(app.listen_input == mode);
        assert_eq!(armed(&app), on, "{label}");
        if mode == LiveSourceMode::Auto {
            assert_eq!(
                app.caption_generation, generation,
                "{label} did not restart"
            );
        }
    }

    // PC audio that is really on stops when captions are chosen.
    let mut app = with_youtube_captions(chat_without_speech());
    app.audio_enabled = true;
    let mut window = Window::new(860.0, 620.0);
    choose(&mut window, &mut app, "YouTube字幕だけ");
    assert!(!app.audio_enabled && app.caption_enabled);
}

#[test]
fn the_menu_entry_for_the_screen_still_starts_reading_at_once_and_is_the_same_choice() {
    let mut app = chat_without_speech();
    let mut window = Window::new(860.0, 620.0);
    window.open_other_features(&mut app);
    window.click(&mut app, "画面の文字を翻訳");
    assert!(app.listen_input == LiveSourceMode::Screen);
    assert_eq!(
        app.status, "取得できる画面がありません。",
        "it asks for a range now, without a press of 聞く"
    );
    app.status.clear();
    listen(&mut app);
    assert_eq!(armed(&app), (false, false));
    assert_eq!(
        app.status, "取得できる画面がありません。",
        "the next 聞く asks for the range again"
    );

    let mut app = chat_without_speech();
    listen(&mut app);
    let mut window = Window::new(860.0, 620.0);
    window.open_other_features(&mut app);
    window.click(&mut app, "画面の文字を翻訳");
    assert!(app.listen_input == LiveSourceMode::Screen);
    assert_eq!(app.status, "取得できる画面がありません。");
    assert_eq!(armed(&app), (false, true));
}

pub(super) fn drawn(app: &mut App, (width, height): (f32, f32)) -> Window {
    let mut window = Window::new(width, height);
    window.settle(app);
    window
}

#[test]
fn the_button_listens_in_the_automatic_mode_and_stops_every_input() {
    let mut app = with_youtube_captions(chat_without_speech());
    app.listen_input = LiveSourceMode::Auto;
    let mut window = drawn(&mut app, (860.0, 620.0));
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
    let mut window = drawn(&mut app, (860.0, 620.0));
    window.click(&mut app, "■ 止める");
    assert!(!app.audio_enabled && !app.caption_enabled);
    assert!(window.has("● 聞く"));

    let mut app = chat_without_speech();
    app.start_screen_feed(1, (0, 0, 120, 40));
    let mut window = drawn(&mut app, (860.0, 620.0));
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
        let window = drawn(&mut app, (860.0, 620.0));
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
fn the_line_names_pc_audio_once_the_captions_are_no_longer_translated() {
    let mut app = chat_without_speech();
    app.caption_enabled = true;
    app.caption_paused = false;
    app.last_caption = Instant::now().checked_sub(Duration::from_secs(6));
    app.audio_enabled = true;
    app.asr_ready = true;
    let window = drawn(&mut app, (860.0, 620.0));
    let drawn_states: Vec<_> = STATE_WORDS
        .into_iter()
        .filter(|state| window.has(state))
        .collect();
    assert_eq!(
        drawn_states,
        ["PC音声を聞いています"],
        "the last caption came six seconds ago"
    );
}

#[test]
fn without_a_translation_model_the_button_cannot_start_and_the_line_says_what_is_needed() {
    let ink = |ready: bool| {
        let mut app = chat_without_speech();
        app.backend_ready = ready;
        drawn(&mut app, (860.0, 620.0)).ink("● 聞く")
    };
    assert_ne!(ink(true), ink(false), "the button is not faded");

    let mut app = chat_without_speech();
    app.backend_ready = false;
    let mut window = drawn(&mut app, (860.0, 620.0));
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
    let mut window = drawn(&mut app, (860.0, 620.0));
    let mut ready = chat_without_speech();
    ready.caption_enabled = true;
    assert_eq!(
        window.ink("■ 止める"),
        drawn(&mut ready, (860.0, 620.0)).ink("■ 止める"),
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
    let window = drawn(&mut app, (860.0, 620.0));
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
    let mut app = with_youtube_captions(chat_without_speech());
    app.listen_input = LiveSourceMode::Auto;
    let mut window = drawn(&mut app, (860.0, 620.0));
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
fn the_mini_window_and_the_subtitle_window_see_the_same_state() {
    use crate::subtitle_window::Playback;
    let caption = bridge::Caption {
        text: "The library opens at nine.".into(),
        video_id: "video-a".into(),
        position: 1.0,
        paused: false,
        epoch: 2,
    };

    let mut app = with_youtube_captions(chat_without_speech());
    app.listen_input = LiveSourceMode::Auto;
    app.document_update_subtitle_window();
    assert_eq!(app.subtitle_window.playback(), Playback::Stopped);
    let mut window = drawn(&mut app, (860.0, 620.0));
    window.click(&mut app, "● 聞く");
    app.accept_caption(caption);
    app.document_update_subtitle_window();
    assert_eq!(app.subtitle_window.playback(), Playback::Running);

    app.window_mode.enter(&egui::ViewportInfo::default());
    window.settle(&mut app);
    assert!(
        window.has("■ 止める") && !window.has("● 聞く"),
        "the mini window has the button that stops what is listening"
    );

    let mut app = chat_without_speech();
    let mut window = drawn(&mut app, (860.0, 620.0));
    window.click(&mut app, "● 聞く");
    window.click(&mut app, "■ 止める");
    app.document_update_subtitle_window();
    assert_eq!(app.subtitle_window.playback(), Playback::Stopped);
    app.window_mode.enter(&egui::ViewportInfo::default());
    window.settle(&mut app);
    assert!(
        window.has("● 聞く") && !window.has("■ 止める"),
        "the mini window has the button that listens again"
    );
}

#[test]
fn the_chat_has_no_five_way_switch_and_no_explanation_line() {
    for (width, height) in [(860.0, 620.0), (2560.0, 1369.0)] {
        let mut app = chat_without_speech();
        app.caption_enabled = true;
        app.audio_enabled = true;
        app.asr_ready = true;
        let window = drawn(&mut app, (width, height));
        for gone in ["停止", "字幕", "両方"] {
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
        let window = drawn(&mut app, (width, height));
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

pub(super) fn hint(app: &mut App, size: (f32, f32)) -> String {
    let window = drawn(app, size);
    let panel = app.live_chat.panel().expect("the chat is drawn");
    let inside: Vec<_> = texts(&window.output)
        .into_iter()
        .filter(|(_, rect, _)| panel.contains_rect(*rect))
        .map(|(text, _, _)| text)
        .collect();
    assert_eq!(inside.len(), 1, "{inside:?}");
    inside.into_iter().next().unwrap()
}

#[test]
fn the_empty_chat_says_what_to_do_next_in_each_state() {
    let mut app = chat_without_speech();
    let stopped = hint(&mut app, (860.0, 620.0));
    assert!(stopped.starts_with(chat::EMPTY_HINT), "{stopped}");
    assert!(
        stopped.ends_with("\n下の「● 聞く」を押すと始まります。"),
        "{stopped}"
    );

    let mut app = chat_without_speech();
    app.audio_enabled = true;
    app.asr_ready = true;
    assert_eq!(
        hint(&mut app, (860.0, 620.0)),
        "英語の動画や音声を再生してください。\n訳が、ここに出ます。"
    );

    // Audio played now would not be heard yet.
    let mut app = chat_without_speech();
    app.audio_start_requested = true;
    assert_eq!(
        hint(&mut app, (860.0, 620.0)),
        "音声認識を準備しています。\n終わると、英語の動画や音声の訳がここに出ます。"
    );

    let mut app = chat_without_speech();
    app.caption_enabled = true;
    assert_eq!(
        hint(&mut app, (860.0, 620.0)),
        "YouTubeで動画を開き、字幕（CC）をオンにして再生してください。\n字幕の訳が、ここに出ます。"
    );

    let mut app = chat_without_speech();
    app.start_screen_feed(1, (0, 0, 120, 40));
    assert_eq!(
        hint(&mut app, (860.0, 620.0)),
        "選んだ範囲の文字が変わると、ここに訳が出ます。"
    );
}

#[test]
fn the_empty_chat_says_that_listen_translates_the_english_playing_on_the_pc() {
    let mut app = chat_without_speech();
    assert_eq!(
        hint(&mut app, (860.0, 620.0)),
        "PCで再生中の英語の音声を、ここにやさしい日本語で表示します。\n下の「● 聞く」を押すと始まります。"
    );
    assert!(
        !chat::EMPTY_HINT.contains("YouTube"),
        "the subtitle window names YouTube: {}",
        chat::EMPTY_HINT
    );

    let mut app = with_youtube_captions(chat_without_speech());
    assert_eq!(
        hint(&mut app, (860.0, 620.0)),
        "PCで再生中の英語の音声やYouTube字幕を、ここにやさしい日本語で表示します。\n下の「● 聞く」を押すと始まります。"
    );
}

/// The words of the question before speech recognition is prepared, as it
/// is drawn.
fn audio_question(app: &mut App) -> Vec<String> {
    let ctx = egui::Context::default();
    configure_theme(&ctx);
    let mut said = Vec::new();
    for frame in 0..4 {
        let output = ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(860.0, 620.0),
                )),
                time: Some(f64::from(frame) / 60.0),
                ..Default::default()
            },
            |ctx| app.audio_setup_ui(ctx),
        );
        said = texts(&output)
            .into_iter()
            .map(|(text, _, _)| text)
            .collect();
    }
    said
}

#[test]
fn the_question_before_preparing_speech_names_youtube_only_while_its_captions_are_translated() {
    const WHAT: &str = "再生中の音声を、このPCで文字起こしする機能です。";
    let mut app = chat_without_speech();
    listen(&mut app);
    assert!(app.audio_confirmation);
    let said = audio_question(&mut app);
    assert!(said.iter().any(|text| text == WHAT), "{said:?}");
    assert!(
        !said.iter().any(|text| text.contains("YouTube")),
        "{said:?}"
    );

    let mut app = with_youtube_captions(chat_without_speech());
    listen(&mut app);
    let said = audio_question(&mut app);
    assert!(
        said.iter()
            .any(|text| text == &format!("{WHAT}YouTube字幕だけを使う場合は不要です。")),
        "{said:?}"
    );
}

#[test]
fn nemotron_with_a_language_it_cannot_hear_starts_nothing_and_the_line_says_what_to_choose() {
    let mut app = chat();
    assert_eq!(app.settings.asr_backend, "nemotron", "the default");
    app.settings.translation.source_lang = "zh-hans".into();
    let mut window = drawn(&mut app, (860.0, 620.0));
    assert!(
        window.has(ENGLISH_ONLY),
        "the line beside 聞く does not say it"
    );
    assert_eq!(find_text(&window.output, ENGLISH_ONLY).1, theme::WARNING);

    window.click(&mut app, "● 聞く");
    assert_eq!(armed(&app), (false, false), "nothing pretends to listen");
    assert!(
        !app.audio_confirmation,
        "the English model is not offered for download"
    );
    assert!(app.nemotron.is_none(), "nor loaded");
    assert!(window.has("● 聞く") && window.has(ENGLISH_ONLY));

    // 英語, 自動判定, or a recognizer that hears the language takes it away.
    for (language, recognizer) in [
        ("en", "nemotron"),
        ("auto", "nemotron"),
        ("zh-hans", "whisper"),
    ] {
        app.settings.translation.source_lang = language.into();
        app.settings.asr_backend = recognizer.into();
        window.settle(&mut app);
        assert!(!window.has(ENGLISH_ONLY), "{language} with {recognizer}");
        assert!(window.has("止まっています"), "{language} with {recognizer}");
    }
}

#[test]
fn pc_audio_that_nemotron_hears_stops_when_the_language_changes_and_the_line_says_why() {
    let mut app = chat();
    app.audio_enabled = true;
    app.asr_ready = true;
    app.settings.translation.source_lang = "zh-hans".into();
    app.process(&egui::Context::default());
    assert!(!app.audio_enabled, "the PC audio goes on");
    let mut window = drawn(&mut app, (860.0, 620.0));
    assert!(window.has(ENGLISH_ONLY), "the line does not say why");

    app.settings.translation.source_lang = "en".into();
    window.settle(&mut app);
    assert!(
        !window.has(ENGLISH_ONLY) && window.has("止まっています"),
        "the words stay once English is chosen again"
    );
}

#[test]
fn the_speech_card_says_so_too_and_names_the_multilingual_choices_with_their_size() {
    let mut app = chat();
    app.tab = Tab::Settings;
    app.settings.translation.source_lang = "zh-hans".into();
    let mut window = Window::new(1280.0, 2400.0);
    window.settle(&mut app);
    assert!(
        window.has(ENGLISH_ONLY),
        "the 音声認識 card does not say it"
    );
    let (heading, words, prepare) = (
        window.at("音声認識"),
        window.at(ENGLISH_ONLY),
        window.at("音声認識を準備"),
    );
    assert!(
        heading.bottom() <= words.top() && words.bottom() <= prepare.top(),
        "it is not in the card"
    );

    window.click(&mut app, "Nemotron Speech · 英語 / 連続認識");
    for choice in [
        "Qwen3-ASR · 多言語 / GPU · 数GB",
        "Whisper · 多言語 / CPU · 約4GB",
    ] {
        assert!(window.has(choice), "{choice} is not a choice");
    }

    app.settings.asr_backend = "whisper".into();
    window.settle(&mut app);
    assert!(!window.has(ENGLISH_ONLY), "Whisper hears the language");
}

#[test]
fn without_a_translation_model_the_empty_chat_points_at_the_model_and_not_at_the_button() {
    let mut app = chat_without_speech();
    app.backend_ready = false;
    let missing = hint(&mut app, (860.0, 620.0));
    assert!(
        missing.contains("「モデルの準備」") && !missing.contains("聞く"),
        "{missing}"
    );
    assert!(
        drawn(&mut app, (860.0, 620.0)).has("モデルの準備"),
        "the button the words name is on the screen"
    );

    app.backend_loading = true;
    assert_eq!(
        hint(&mut app, (860.0, 620.0)),
        "翻訳モデルを準備しています。\n終わると、下の「● 聞く」を押せます。"
    );
}
