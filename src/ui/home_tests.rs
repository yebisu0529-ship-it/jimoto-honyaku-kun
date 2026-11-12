//! The chat is the home screen and every other screen is behind one menu.
use super::*;
use crate::chat::testing::{
    assert_drawn_dark, background, fill_behind, find_text, light_colours, press, texts, wheel,
};
use plamo_translate::hardware::Hardware;

/// One window whose egui state lasts from frame to frame, so a menu that has
/// been opened stays open.
pub(super) struct Window {
    context: egui::Context,
    size: egui::Vec2,
    frame: u32,
    pub(super) output: egui::FullOutput,
}

impl Window {
    pub(super) fn new(width: f32, height: f32) -> Self {
        let context = egui::Context::default();
        configure_theme(&context);
        Self {
            context,
            size: egui::vec2(width, height),
            frame: 0,
            output: egui::FullOutput::default(),
        }
    }

    pub(super) fn draw(&mut self, app: &mut App, events: Vec<egui::Event>) {
        self.frame += 1;
        self.output = self.context.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, self.size)),
                time: Some(f64::from(self.frame) / 60.0),
                events,
                ..Default::default()
            },
            |ctx| app.render_ui(ctx),
        );
    }

    pub(super) fn settle(&mut self, app: &mut App) {
        for _ in 0..4 {
            self.draw(app, vec![]);
        }
    }

    /// The same window at another size, as when the app makes it small for
    /// the mini view. What egui remembers of it stays.
    pub(super) fn resize(&mut self, width: f32, height: f32) {
        self.size = egui::vec2(width, height);
    }

    /// The same window on a screen of `pixels_per_point` pixels to a point,
    /// as 1.25 at the 125% scale of Windows.
    pub(super) fn scale(&mut self, pixels_per_point: f32) {
        self.context.set_pixels_per_point(pixels_per_point);
    }

    pub(super) fn has(&self, label: &str) -> bool {
        texts(&self.output).iter().any(|(text, _, _)| text == label)
    }

    /// Where the one text drawn exactly as `label` is.
    pub(super) fn at(&self, label: &str) -> egui::Rect {
        let found: Vec<_> = texts(&self.output)
            .into_iter()
            .filter(|(text, _, _)| text == label)
            .collect();
        assert_eq!(
            found.len(),
            1,
            "expected one {label}, found {}",
            found.len()
        );
        found[0].1
    }

    /// The colour the first letter of the text is painted in. A disabled
    /// control paints its text paler than the colour it was given.
    pub(super) fn ink(&self, label: &str) -> Color32 {
        fn visit(shape: &egui::epaint::Shape, label: &str, out: &mut Vec<Color32>) {
            match shape {
                egui::epaint::Shape::Text(text) if text.galley.text() == label => {
                    out.push(text.galley.rows[0].visuals.mesh.vertices[0].color)
                }
                egui::epaint::Shape::Vec(shapes) => {
                    shapes.iter().for_each(|shape| visit(shape, label, out))
                }
                _ => {}
            }
        }
        let mut found = Vec::new();
        for clipped in &self.output.shapes {
            visit(&clipped.shape, label, &mut found);
        }
        assert_eq!(
            found.len(),
            1,
            "expected one {label}, found {}",
            found.len()
        );
        found[0]
    }

    /// Presses and releases over the centre of the text, the way a pointer does.
    pub(super) fn click(&mut self, app: &mut App, label: &str) {
        let at = self.at(label).center();
        self.draw(app, vec![egui::Event::PointerMoved(at)]);
        self.draw(app, vec![press(at, egui::PointerButton::Primary, true)]);
        self.draw(app, vec![press(at, egui::PointerButton::Primary, false)]);
        self.settle(app);
    }

    pub(super) fn open_menu(&mut self, app: &mut App) {
        self.settle(app);
        self.click(app, "⋯ その他");
    }

    pub(super) fn open_other_features(&mut self, app: &mut App) {
        self.open_menu(app);
        self.click(app, OTHER_FEATURES);
    }

    /// Rests the pointer over the text, long enough for its hover text.
    pub(super) fn hover(&mut self, app: &mut App, label: &str) {
        self.context
            .style_mut(|style| style.interaction.tooltip_delay = 0.0);
        let at = self.at(label).center();
        self.draw(app, vec![egui::Event::PointerMoved(at)]);
        self.settle(app);
    }
}

pub(super) fn hardware() -> Hardware {
    Hardware {
        os_arch: "windows-x86_64".into(),
        cpu_logical: 16,
        ram_total_bytes: Some(32 * 1_073_741_824),
        ram_available_bytes: Some(20 * 1_073_741_824),
        disk_free_bytes: Some(120 * 1_073_741_824),
        disk_target: "fixture".into(),
        gpus: vec![],
        warnings: vec![],
    }
}

/// The chat with the translation model ready. Opening the model screen asks
/// for the PC's capacity, so that answer is already in.
pub(super) fn chat() -> App {
    let mut app = App::inert();
    app.backend_ready = true;
    app.setup.hardware = Some(hardware());
    app
}

/// A fixed message of the talk.
pub(super) fn row(id: u64, kind: &str, translation: &str) -> Row {
    Row {
        record: Record {
            id,
            start_ms: 0,
            end_ms: 1000,
            source: format!("Original {id}."),
            translation: translation.into(),
            standard_translation: None,
            kind: kind.into(),
            video_id: String::new(),
            created_at: String::new(),
            paragraph: None,
        },
        pending: false,
        error: None,
        generation: 0,
    }
}

pub(super) fn key(key: egui::Key, modifiers: egui::Modifiers) -> egui::Event {
    egui::Event::Key {
        key,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers,
    }
}

const OTHER: [&str; 6] = [
    "範囲翻訳",
    "画面選択",
    "画面の文字を翻訳",
    "画像から翻訳",
    FILE,
    "字幕ウィンドウ",
];

const FILE: &str = "ファイルを翻訳";
const FILE_HINT: &str = "テキスト・Markdown・SRT・WebVTTを翻訳します。字幕の時刻は維持されます。";

const SCREENS: [(Tab, &str); 5] = [
    (Tab::Text, "シンプル翻訳"),
    (Tab::Files, "履歴"),
    (Tab::Settings, "設定"),
    (Tab::Setup, "モデル"),
    (Tab::YouTube, "YouTube字幕の接続"),
];

/// Every screen that has the page header: the chat and the five behind the menu.
const HEADERED: [(Tab, &str); 6] = [
    (Tab::Text, "シンプル翻訳"),
    (Tab::Live, "チャット"),
    (Tab::Files, "履歴"),
    (Tab::Settings, "設定"),
    (Tab::Setup, "モデル"),
    (Tab::YouTube, "YouTube字幕の接続"),
];

#[test]
fn the_app_opens_on_the_chat_with_its_name_one_menu_and_the_listen_button() {
    assert!(Tab::default() == Tab::Live);
    for (width, height) in [(860.0, 620.0), (2560.0, 1369.0)] {
        let mut app = chat();
        let mut window = Window::new(width, height);
        window.settle(&mut app);
        for shown in [
            "Talk風Live翻訳",
            "PC内で翻訳",
            "クリア",
            "⋯ その他",
            "● 聞く",
            "止まっています",
            credit::LABEL,
        ] {
            assert!(
                window.has(shown),
                "{shown} is not drawn at {width}x{height}"
            );
        }
        // The rail, the buttons of the old header and the two menus of the
        // input row are all behind the one menu now.
        for gone in [
            "文章",
            "ライブ",
            "履歴",
            "設定",
            "範囲翻訳",
            "音声翻訳",
            "音声停止",
            "モデル",
            "字幕ウィンドウ",
            "ミニ表示",
            "状態の詳細",
            "表示",
            "その他",
            "書き出し",
            "ライブ翻訳",
        ] {
            assert!(
                !window.has(gone),
                "{gone} is still drawn at {width}x{height}"
            );
        }
    }
}

#[test]
fn the_way_back_to_the_chat_works_without_a_model() {
    let mut app = chat();
    app.backend_ready = false;
    app.open_setup();
    let mut window = Window::new(860.0, 620.0);
    window.settle(&mut app);
    for shown in [
        "このPCに合うモデルを準備",
        "← チャットに戻る",
        credit::LABEL,
    ] {
        assert!(window.has(shown), "{shown} is not drawn");
    }
    window.click(&mut app, "← チャットに戻る");
    assert!(app.tab == Tab::Live);
}

fn assert_shortcuts(window: &Window, rows: &[(&str, &str)]) {
    for (item, shortcut) in rows {
        let (row, hint) = (window.at(item), window.at(shortcut));
        assert!(
            hint.left() > row.right() && (hint.center().y - row.center().y).abs() < 2.0,
            "{shortcut} is not at the end of the {item} row"
        );
    }
}

fn assert_no_shortcut(window: &Window, rows: &[&str]) {
    for item in rows {
        let row = window.at(item);
        assert!(
            !texts(&window.output).iter().any(|(text, rect, _)| {
                text.starts_with("Ctrl") && (rect.center().y - row.center().y).abs() < 2.0
            }),
            "{item} has no shortcut to show"
        );
    }
}

#[test]
fn the_menu_lists_the_screens_in_order_with_the_shortcut_each_has() {
    let mut app = chat();
    let mut window = Window::new(860.0, 620.0);
    window.open_menu(&mut app);
    let items = [
        "ミニ表示",
        "履歴",
        "モデル",
        "設定",
        "状態の詳細",
        "入力の切り替え",
        "書き出し",
        "表示",
        OTHER_FEATURES,
    ];
    let tops: Vec<f32> = items.iter().map(|item| window.at(item).top()).collect();
    assert!(
        tops.windows(2).all(|pair| pair[0] < pair[1]),
        "the items are out of order: {tops:?}"
    );
    assert_shortcuts(
        &window,
        &[
            ("ミニ表示", "Ctrl + Shift + M"),
            ("履歴", "Ctrl + 3"),
            ("設定", "Ctrl + 4"),
        ],
    );
    assert_no_shortcut(
        &window,
        &["モデル", "状態の詳細", "書き出し", OTHER_FEATURES],
    );
    assert!(
        !window.has("新しく始める"),
        "新しく始める is still in the menu"
    );
    let step = |from: &str, to: &str| window.at(to).top() - window.at(from).top();
    assert!(
        step("表示", OTHER_FEATURES) > step("書き出し", "表示") + 4.0,
        "no line above {OTHER_FEATURES}"
    );
    assert!(
        window.at(OTHER_FEATURES).bottom() < 620.0 - theme::CONTROL_H,
        "the menu runs into the status row at the smallest window"
    );

    window.hover(&mut app, "ミニ表示");
    window.settle(&mut app);
    assert!(
        window.has("同じチャットを、小さな窓で読みます。"),
        "the hover text of ミニ表示"
    );
}

#[test]
fn the_menu_has_none_of_the_other_features_until_their_row_is_opened() {
    for tab in [Tab::Live, Tab::Files] {
        let mut app = chat();
        app.tab = tab;
        app.screens = two_screens();
        let mut window = Window::new(860.0, 620.0);
        window.open_menu(&mut app);
        for other in OTHER {
            assert!(!window.has(other), "{other} is at the top of the menu");
        }
        window.click(&mut app, OTHER_FEATURES);
        for other in OTHER {
            assert!(window.has(other), "{other} is not behind {OTHER_FEATURES}");
        }
    }
}

#[test]
fn the_youtube_connection_is_the_last_of_the_other_features() {
    for tab in [Tab::Live, Tab::Files] {
        let mut app = chat();
        app.tab = tab;
        let mut window = Window::new(860.0, 620.0);
        window.open_menu(&mut app);
        assert!(
            !window.has("YouTube字幕の接続"),
            "it is at the top of the menu"
        );
        window.click(&mut app, OTHER_FEATURES);
        assert!(
            window.at("字幕ウィンドウ").top() < window.at("YouTube字幕の接続").top(),
            "it is not last"
        );
        assert!(
            window.at("YouTube字幕の接続").bottom() < 620.0 - theme::CONTROL_H,
            "it runs into the status row at the smallest window"
        );
        window.click(&mut app, "YouTube字幕の接続");
        assert!(app.tab == Tab::YouTube, "it does not open its screen");
    }
}

#[test]
fn the_other_features_are_behind_one_row_in_order_with_their_shortcuts() {
    let mut app = chat();
    app.screens = two_screens();
    let mut window = Window::new(860.0, 620.0);
    window.open_other_features(&mut app);
    let tops: Vec<f32> = OTHER.iter().map(|item| window.at(item).top()).collect();
    assert!(
        tops.windows(2).all(|pair| pair[0] < pair[1]),
        "the other features are out of order: {tops:?}"
    );
    assert_shortcuts(&window, &[("範囲翻訳", "Ctrl + Alt + T")]);
    assert_no_shortcut(
        &window,
        &[
            "画面選択",
            "画面の文字を翻訳",
            "画像から翻訳",
            FILE,
            "字幕ウィンドウ",
        ],
    );
    for item in OTHER {
        let row = window.at(item);
        assert!(
            row.left() >= 0.0 && row.right() <= 860.0,
            "{item} leaves the window"
        );
    }
    assert!(
        window.at("字幕ウィンドウ").bottom() < 620.0 - theme::CONTROL_H,
        "the rows run into the status row at the smallest window"
    );

    // A click on ファイルを翻訳 asks the system for a file, which a test cannot
    // answer. Its row still says what the button on the 履歴 screen said.
    window.hover(&mut app, FILE);
    window.settle(&mut app);
    assert!(window.has(FILE_HINT), "the hover text of {FILE}");
}

#[test]
fn the_menu_on_another_screen_has_the_screens_and_not_what_belongs_to_the_chat() {
    let mut app = chat();
    app.tab = Tab::Files;
    let mut window = Window::new(860.0, 620.0);
    window.open_menu(&mut app);
    assert!(window.has("設定") && window.has(OTHER_FEATURES));
    for chat_only in ["入力の切り替え", "表示"] {
        assert!(
            !window.has(chat_only),
            "{chat_only} is in the menu of another screen"
        );
    }
}

#[test]
fn each_screen_in_the_menu_opens_and_the_back_button_returns_to_the_chat() {
    for (screen, item) in SCREENS {
        let mut app = chat();
        let mut window = Window::new(860.0, 620.0);
        match screen {
            Tab::YouTube => window.open_other_features(&mut app),
            // The menu's row has the name of a button in the chat's header,
            // so the click here is on the button.
            Tab::Text => window.settle(&mut app),
            _ => window.open_menu(&mut app),
        }
        window.click(&mut app, item);
        assert!(app.tab == screen, "{item} did not open its screen");
        assert!(
            !window.has("Talk風Live翻訳"),
            "{item}: the chat is still shown"
        );
        assert!(
            window.has("← チャットに戻る"),
            "{item}: there is no way back"
        );
        assert!(window.has(credit::LABEL), "{item}: the credit is missing");
        window.click(&mut app, "← チャットに戻る");
        assert!(
            app.tab == Tab::Live,
            "{item}: the back button did not return"
        );
        assert!(window.has("Talk風Live翻訳"));
        assert!(!window.has("← チャットに戻る"));
    }
}

#[test]
fn the_other_items_do_what_their_buttons_did() {
    let mut app = chat();
    let mut window = Window::new(860.0, 620.0);
    window.open_other_features(&mut app);
    window.click(&mut app, "字幕ウィンドウ");
    assert!(app.subtitle_window.open);

    let mut app = chat();
    let mut window = Window::new(860.0, 620.0);
    window.open_menu(&mut app);
    window.click(&mut app, "ミニ表示");
    assert!(app.window_mode.is_mini());

    // The reading of a range is shown on the シンプル翻訳 screen, so it opens
    // there.
    // The test machine has no screen to read, and says so.
    let mut app = chat();
    app.status = "before".into();
    let mut window = Window::new(860.0, 620.0);
    window.open_other_features(&mut app);
    window.click(&mut app, "範囲翻訳");
    assert!(app.tab == Tab::Text);
    assert_eq!(app.status, "取得できる画面がありません。");

    // The 画面 input feeds the chat, so it stays there.
    let mut app = chat();
    app.status = "before".into();
    let mut window = Window::new(860.0, 620.0);
    window.open_other_features(&mut app);
    window.click(&mut app, "画面の文字を翻訳");
    assert!(app.tab == Tab::Live);
    assert_eq!(app.status, "取得できる画面がありません。");
    assert!(!app.screen_area_requested, "no area is waited for");
}

#[test]
fn items_that_need_the_model_are_faded_until_it_is_ready_and_the_others_never_are() {
    let inks = |ready: bool| {
        let mut app = chat();
        app.backend_ready = ready;
        let mut window = Window::new(860.0, 620.0);
        window.open_other_features(&mut app);
        [
            "ミニ表示",
            "履歴",
            "YouTube字幕の接続",
            "モデル",
            "設定",
            "状態の詳細",
            "入力の切り替え",
            "表示",
            OTHER_FEATURES,
            "範囲翻訳",
            "画面の文字を翻訳",
            "画像から翻訳",
            "ファイルを翻訳",
            "字幕ウィンドウ",
        ]
        .map(|item| (item, window.ink(item)))
    };
    let needs_the_model = [
        "ミニ表示",
        "範囲翻訳",
        "画面の文字を翻訳",
        "画像から翻訳",
        "ファイルを翻訳",
    ];
    for ((item, ready), (_, waiting)) in inks(true).into_iter().zip(inks(false)) {
        assert_eq!(
            ready != waiting,
            needs_the_model.contains(&item),
            "{item} while the model is not ready"
        );
    }
}

#[test]
fn the_screen_to_read_is_chosen_in_the_menu_when_there_are_several() {
    let mut app = chat();
    app.screens = two_screens().into_iter().take(1).collect();
    let mut window = Window::new(860.0, 620.0);
    window.open_other_features(&mut app);
    assert!(
        !window.has("画面選択"),
        "one screen leaves nothing to choose"
    );

    app.screens = two_screens();
    window.settle(&mut app);
    window.click(&mut app, "画面選択");
    window.click(&mut app, "2: DISPLAY2");
    assert_eq!(app.screen_index, 1);
}

#[test]
fn the_display_options_moved_into_the_menu_and_still_work() {
    let mut app = chat();
    assert!(!app.settings.live_show_original);
    let mut window = Window::new(860.0, 620.0);
    window.open_menu(&mut app);
    window.click(&mut app, "表示");
    for option in [
        "原文をすべて表示",
        "末尾へ自動スクロール",
        "訳文の文字サイズ",
    ] {
        assert!(window.has(option), "{option} is not in the 表示 menu");
    }
    window.click(&mut app, "原文をすべて表示");
    assert!(app.settings.live_show_original);
}

/// What 「ダークモード」 is saved as in the settings file.
pub(super) fn dark_chat_saved(app: &App) -> serde_json::Value {
    serde_json::to_value(&app.settings).unwrap()["dark_chat"].clone()
}

/// `app` with 「ダークモード」 on, as a settings file that has it on gives it.
pub(super) fn with_dark_chat(mut app: App) -> App {
    let mut saved = serde_json::to_value(&app.settings).unwrap();
    saved["dark_chat"] = true.into();
    app.settings = serde_json::from_value(saved).unwrap();
    app
}

/// Turns 「ダークモード」 over in the 表示 menu, the way a pointer does.
fn flip_dark_mode(window: &mut Window, app: &mut App) {
    window.open_menu(app);
    window.click(app, "表示");
    window.click(app, "ダークモード");
}

/// Drags across the one text drawn as `words`, the way a pointer selects it.
fn select(window: &mut Window, app: &mut App, words: &str) {
    let text = window.at(words);
    let from = text.left_center() + egui::vec2(2.0, 0.0);
    let to = text.right_center() - egui::vec2(2.0, 0.0);
    window.draw(app, vec![egui::Event::PointerMoved(from)]);
    window.draw(app, vec![press(from, egui::PointerButton::Primary, true)]);
    for step in 1..=4 {
        let at = from + (to - from) * (step as f32 / 4.0);
        window.draw(app, vec![egui::Event::PointerMoved(at)]);
    }
    window.draw(app, vec![press(to, egui::PointerButton::Primary, false)]);
    window.settle(app);
}

/// The words of the first message of the previews' talk, in a bubble of
/// their own.
const FIRST_WORDS: &str = "おかえりなさい。今日は小さなアプリを作ります。";

#[test]
fn the_display_menu_has_dark_mode_right_after_auto_scroll_and_it_starts_off() {
    let mut app = chat();
    let mut window = Window::new(860.0, 620.0);
    window.open_menu(&mut app);
    window.click(&mut app, "表示");
    assert!(
        window.has("ダークモード"),
        "ダークモード is not in the 表示 menu"
    );
    let tops: Vec<f32> = [
        "原文をすべて表示",
        "末尾へ自動スクロール",
        "ダークモード",
        "訳文の文字サイズ",
    ]
    .iter()
    .map(|option| window.at(option).top())
    .collect();
    assert!(
        tops.windows(2).all(|pair| pair[0] < pair[1]),
        "ダークモード is not right after 末尾へ自動スクロール: {tops:?}"
    );
    assert_eq!(
        dark_chat_saved(&app),
        serde_json::json!(false),
        "it is not a saved setting that starts off"
    );
    window.click(&mut app, "ダークモード");
    assert_eq!(dark_chat_saved(&app), serde_json::json!(true));
    flip_dark_mode(&mut window, &mut app);
    assert_eq!(dark_chat_saved(&app), serde_json::json!(false));
}

#[test]
fn an_old_settings_file_reads_dark_mode_as_off_and_a_saved_one_keeps_it() {
    let old: Settings = serde_json::from_slice(br#"{"font_size": 24.0}"#).unwrap();
    assert_eq!(old.font_size, 24.0, "the file's own settings are lost");
    assert_eq!(
        serde_json::to_value(&old).unwrap()["dark_chat"],
        serde_json::json!(false),
        "a file from before the switch does not read as off"
    );

    let mut app = chat();
    let mut window = Window::new(860.0, 620.0);
    flip_dark_mode(&mut window, &mut app);
    // What save_settings writes, read back as the next start reads it.
    let file = serde_json::to_vec_pretty(&app.settings).unwrap();
    let read: Settings = serde_json::from_slice(&file).unwrap();
    assert_eq!(
        serde_json::to_value(&read).unwrap()["dark_chat"],
        serde_json::json!(true),
        "the switch is not kept in the file"
    );
}

#[test]
fn dark_mode_draws_the_chat_dark_and_turning_it_off_brings_today_s_colours_back_at_once() {
    // The previews' talk: names, times, a date, an opened original with its
    // video time, and a message still forming with its dots.
    let mut app = with_dark_chat(chat());
    app.rows = crate::demo_talk(5);
    app.current_live_id = Some(5);
    app.live_chat.toggle_original(3, 0);
    let mut window = Window::new(1280.0, 880.0);
    window.settle(&mut app);
    let panel = app.live_chat.panel().expect("the chat is drawn");
    assert_drawn_dark(&window.output, panel, FIRST_WORDS);
    select(&mut window, &mut app, FIRST_WORDS);
    assert!(app.live_chat.has_selection(), "the drag selected nothing");
    assert_eq!(
        light_colours(&window.output, panel),
        Vec::<&str>::new(),
        "parts of the chat are still light"
    );

    flip_dark_mode(&mut window, &mut app);
    assert_eq!(background(&window.output, panel), theme::CHAT_BACKGROUND);
    assert_eq!(fill_behind(&window.output, FIRST_WORDS), theme::CHAT_BUBBLE);
    assert_eq!(find_text(&window.output, FIRST_WORDS).1, theme::CHAT_TEXT);

    flip_dark_mode(&mut window, &mut app);
    assert_drawn_dark(&window.output, panel, FIRST_WORDS);
}

#[test]
fn the_dark_chat_draws_its_pills_its_scroll_bar_and_its_empty_words_dark_too() {
    // Read from the start of a longer talk, with the pointer on the pill.
    let mut app = with_dark_chat(chat());
    app.rows = crate::demo_talk(10);
    app.current_live_id = Some(10);
    app.live_chat.reading_from_start(7);
    let mut window = Window::new(860.0, 620.0);
    window.settle(&mut app);
    window.hover(&mut app, "↓ 新しいメッセージ 3件");
    let panel = app.live_chat.panel().expect("the chat is drawn");
    assert_drawn_dark(&window.output, panel, FIRST_WORDS);
    for pill in ["11月7日（土）", "↓ 新しいメッセージ 3件"] {
        let (fill, (_, ink)) = (
            fill_behind(&window.output, pill),
            find_text(&window.output, pill),
        );
        assert!(
            theme::contrast_ratio(ink, fill) >= 4.5,
            "{pill} in {ink:?} cannot be read on {fill:?}"
        );
    }
    assert_eq!(
        light_colours(&window.output, panel),
        Vec::<&str>::new(),
        "parts of the chat are still light"
    );

    let mut app = with_dark_chat(chat());
    let mut window = Window::new(860.0, 620.0);
    window.settle(&mut app);
    let panel = app.live_chat.panel().expect("the chat is drawn");
    let background = background(&window.output, panel);
    assert!(
        theme::relative_luminance(background) < 0.01,
        "the empty chat is on {background:?}"
    );
    let (_, _, ink) = texts(&window.output)
        .into_iter()
        .find(|(text, _, _)| text.starts_with(chat::EMPTY_HINT))
        .expect("the empty chat says what it is for");
    assert!(
        theme::contrast_ratio(ink, background) >= 4.5,
        "the words of the empty chat in {ink:?} cannot be read on {background:?}"
    );
    assert_eq!(light_colours(&window.output, panel), Vec::<&str>::new());
}

/// The subtitle window, drawn as `App::update` draws it on each frame.
fn subtitle_window(ctx: &egui::Context, app: &mut App) -> egui::FullOutput {
    let mut output = egui::FullOutput::default();
    for _ in 0..4 {
        output = ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(720.0, 360.0),
                )),
                ..Default::default()
            },
            |ctx| {
                app.subtitle_window.show(ctx, app.chat_palette());
            },
        );
    }
    output
}

#[test]
fn the_subtitle_window_follows_dark_mode_at_once() {
    let words = "共通の用語集で地域の地名を統一します。";
    let mut app = with_dark_chat(chat());
    app.subtitle_window = crate::demo_subtitle(false, 3, crate::FORMING_LONG);
    app.subtitle_window.open = true;
    let ctx = crate::chat::testing::context();
    let output = subtitle_window(&ctx, &mut app);
    let panel = app.subtitle_window.chat.panel().expect("the chat is drawn");
    assert_drawn_dark(&output, panel, words);
    assert_eq!(
        light_colours(&output, panel),
        Vec::<&str>::new(),
        "parts of the subtitle window's chat are still light"
    );

    let mut window = Window::new(860.0, 620.0);
    flip_dark_mode(&mut window, &mut app);
    let output = subtitle_window(&ctx, &mut app);
    assert_eq!(background(&output, panel), theme::CHAT_BACKGROUND);
    assert_eq!(fill_behind(&output, words), theme::CHAT_BUBBLE);
    assert_eq!(find_text(&output, words).1, theme::CHAT_TEXT);
}

#[test]
fn the_readout_switch_is_in_the_bar_on_the_chat_and_beside_the_menu_on_the_other_screens() {
    let mut app = chat();
    app.settings.readout_enabled = true;
    let mut window = Window::new(860.0, 620.0);
    window.settle(&mut app);
    let (switch, menu, listen) = (
        window.at("自動読み上げ"),
        window.at("⋯ その他"),
        window.at("● 聞く"),
    );
    let bar = app.ui_probe.listen_bar.expect("the bar is drawn");
    assert!(bar.contains_rect(switch), "the switch is not in the bar");
    assert!(switch.top() > menu.bottom(), "the header still has it");
    assert!(listen.right() < switch.left());
    assert!((switch.center().y - listen.center().y).abs() < 2.0);

    for tab in [
        Tab::Text,
        Tab::Files,
        Tab::Settings,
        Tab::Setup,
        Tab::YouTube,
    ] {
        let mut app = chat();
        app.tab = tab;
        app.settings.readout_enabled = true;
        let mut window = Window::new(860.0, 620.0);
        window.settle(&mut app);
        let (switch, menu) = (window.at("自動読み上げ"), window.at("⋯ その他"));
        assert!(switch.right() < menu.left());
        assert!((switch.center().y - menu.center().y).abs() < 2.0);
    }
}

#[test]
fn the_header_of_every_screen_fits_at_the_smallest_window_with_the_readout_switch_on() {
    for (tab, page) in HEADERED {
        let mut app = chat();
        app.tab = tab;
        app.settings.readout_enabled = true;
        let mut window = Window::new(860.0, 620.0);
        window.settle(&mut app);
        let header: Vec<_> = texts(&window.output)
            .into_iter()
            .filter(|(_, rect, _)| rect.top() < 16.0 + theme::CONTROL_H)
            .collect();
        for (index, (first, a, _)) in header.iter().enumerate() {
            assert!(
                a.left() >= 0.0 && a.right() <= 860.0,
                "{page}: {first} leaves the window"
            );
            for (second, b, _) in &header[index + 1..] {
                assert!(!a.intersects(*b), "{page}: {first} overlaps {second}");
            }
        }
    }
}

#[test]
fn the_buttons_of_the_header_are_32_points_tall_on_every_screen() {
    for (tab, page) in HEADERED {
        let mut app = chat();
        app.tab = tab;
        let mut window = Window::new(860.0, 620.0);
        window.settle(&mut app);
        let menu = app.ui_probe.menu_button.expect("the menu button is drawn");
        assert!(
            menu.height() >= 32.0,
            "{page}: the menu button is {} tall",
            menu.height()
        );
        if tab != Tab::Live {
            let back = app.ui_probe.back_button.expect("the back button is drawn");
            assert!(
                back.height() >= 32.0,
                "{page}: the back button is {} tall",
                back.height()
            );
        }
    }
}

#[test]
fn each_screen_behind_the_menu_has_its_title_on_the_row_of_the_back_button_and_the_menu() {
    for (tab, title) in [
        (Tab::Text, "シンプル翻訳"),
        (Tab::Files, "履歴"),
        (Tab::Settings, "設定"),
        (Tab::Setup, "このPCに合うモデルを準備"),
        (Tab::YouTube, "YouTube字幕の接続"),
    ] {
        let mut app = chat();
        app.tab = tab;
        let mut window = Window::new(860.0, 620.0);
        window.settle(&mut app);
        let back = app.ui_probe.back_button.expect("the back button is drawn");
        let menu = app.ui_probe.menu_button.expect("the menu button is drawn");
        let heading = window.at(title);
        assert!(
            back.right() <= heading.left() && heading.right() <= menu.left(),
            "{title}: it is not between the back button and the menu"
        );
        for button in [back, menu] {
            assert!(
                (button.top()..=button.bottom()).contains(&heading.center().y),
                "{title}: it is not on the row of the buttons: {heading:?} and {button:?}"
            );
        }
    }
}

#[test]
fn the_credit_stays_at_the_bottom_right_of_every_screen() {
    for (width, height) in [(860.0, 620.0), (2560.0, 1369.0)] {
        for (tab, page) in [
            (Tab::Live, "チャット"),
            (Tab::Text, "シンプル翻訳"),
            (Tab::Files, "履歴"),
            (Tab::Settings, "設定"),
            (Tab::Setup, "モデル"),
            (Tab::YouTube, "YouTube字幕の接続"),
        ] {
            let mut app = chat();
            app.tab = tab;
            let mut window = Window::new(width, height);
            window.settle(&mut app);
            let credit = window.at(credit::LABEL);
            assert!(
                credit.right() <= width - theme::GUTTER + 1.0
                    && credit.right() > width - 2.0 * theme::GUTTER
                    && credit.bottom() <= height
                    && credit.top() >= height - theme::CONTROL_H,
                "{page} at {width}x{height}: the credit is at {credit:?}"
            );
            for (text, rect, _) in texts(&window.output) {
                assert!(
                    text == credit::LABEL || !rect.intersects(credit),
                    "{page} at {width}x{height}: {text} covers the credit"
                );
            }
        }
    }
}

#[test]
fn ctrl_number_still_opens_its_screen_from_every_screen() {
    for (from, _, _) in PAGE_KEYS {
        for (screen, _, pressed) in PAGE_KEYS {
            let mut app = chat();
            app.tab = from;
            let mut window = Window::new(860.0, 620.0);
            window.draw(&mut app, vec![key(pressed, egui::Modifiers::CTRL)]);
            assert!(
                app.tab == screen,
                "Ctrl+{} did not open its screen",
                pressed.name()
            );
        }
    }
}

/// What the status row painted before the chat took one state in its place:
/// the name of a model and, unless it is ready, its state as one text.
const CHIPS: [&str; 4] = ["翻訳", "音声認識未準備", "文字認識", "読み上げ未準備"];

const LOAD_FAILED: &str =
    "モデルを読み込めませんでした。「モデルの準備」で小さいモデルやCPU実行を試してください。";

#[test]
fn the_chat_shows_one_state_where_the_four_models_were() {
    let draw = |setup: fn(&mut App)| {
        let mut app = chat();
        app.settings.readout_enabled = true;
        setup(&mut app);
        let mut window = Window::new(860.0, 620.0);
        window.settle(&mut app);
        window
    };
    let only = |window: &Window, state: &str| {
        for other in ["準備完了", "準備中…", "準備が必要です"] {
            assert_eq!(
                window.has(other),
                other == state,
                "{other}, expecting {state}"
            );
        }
        for chip in CHIPS {
            assert!(!window.has(chip), "{chip} is still in the status row");
        }
    };

    // Speech recognition and readout have not been started, and that is fine.
    let window = draw(|_| {});
    only(&window, "準備完了");
    assert!(window.at("準備完了").top() > 620.0 - theme::CONTROL_H);

    let window = draw(|app| {
        app.backend_ready = false;
        app.backend_loading = true;
    });
    only(&window, "準備中…");
    assert!(!window.has("モデルの準備"));

    let window = draw(|app| app.backend_ready = false);
    only(&window, "準備が必要です");
    assert!(window.has("モデルの準備"));

    // An error is beside the button, in red and in its own words, and only
    // there. The row keeps the state of the models.
    let window = draw(|app| app.service_error = LOAD_FAILED.into());
    only(&window, "準備完了");
    let (rect, color) = find_text(&window.output, LOAD_FAILED);
    assert_eq!(color, theme::ERROR);
    assert!(
        rect.bottom() < 620.0 - theme::CONTROL_H,
        "the error is in the status row, not beside the button"
    );
    assert_eq!(
        texts(&window.output)
            .iter()
            .filter(|(text, _, _)| text.contains(LOAD_FAILED))
            .count(),
        1,
        "the error is drawn twice"
    );
}

#[test]
fn the_button_beside_the_missing_state_opens_the_model_screen() {
    let mut app = chat();
    app.backend_ready = false;
    let mut window = Window::new(860.0, 620.0);
    window.settle(&mut app);
    window.click(&mut app, "モデルの準備");
    assert!(app.tab == Tab::Setup);
    window.settle(&mut app);
    assert!(
        !window.has("モデルの準備"),
        "the model screen has no use for a button to itself"
    );
}

#[test]
fn the_state_belongs_to_the_chat_and_the_other_screens_leave_the_row_to_the_messages() {
    for (tab, _, _) in PAGE_KEYS {
        let mut app = chat();
        app.tab = tab;
        app.status = "設定を保存しました。".into();
        let mut window = Window::new(860.0, 620.0);
        window.settle(&mut app);
        let in_the_row = |label: &str| {
            texts(&window.output)
                .iter()
                .any(|(text, rect, _)| text == label && rect.top() > 620.0 - theme::CONTROL_H)
        };
        assert_eq!(in_the_row("準備完了"), tab == Tab::Live);
        assert!(in_the_row("設定を保存しました。"));
    }
}

#[test]
fn the_history_screen_finds_and_exports_the_talk_and_files_are_translated_on_the_text_screen() {
    let mut app = chat();
    app.tab = Tab::Files;
    app.file_busy = true;
    let mut window = Window::new(860.0, 620.0);
    window.settle(&mut app);
    for shown in ["これまでの訳文を探せます。", "原文・訳文を検索", "書き出し"]
    {
        assert!(window.has(shown), "{shown} is not on the 履歴 screen");
    }
    for gone in [
        "ファイルを選んで翻訳",
        FILE,
        "ファイル翻訳の言語",
        "自動判定",
        "ファイルを翻訳しています",
        "中止",
    ] {
        assert!(!window.has(gone), "{gone} is still on the 履歴 screen");
    }

    app.tab = Tab::Text;
    window.settle(&mut app);
    assert!(window.has("自動判定") && window.has("翻訳する"));
    assert!(window.has("ファイルを翻訳しています"));
    window.click(&mut app, "中止");
    assert!(
        app.file_cancel.load(Ordering::Relaxed),
        "中止 does not stop the file"
    );

    // As the button on the 履歴 screen was, the row behind その他の機能 is
    // faded without the model and while a file is translated. A click asks
    // the system for a file, which a test cannot answer.
    let ink = |ready: bool, busy: bool| {
        let mut app = chat();
        app.tab = Tab::Text;
        app.backend_ready = ready;
        app.file_busy = busy;
        let mut window = Window::new(860.0, 620.0);
        window.open_other_features(&mut app);
        window.ink(FILE)
    };
    let faded = ink(false, false);
    assert_ne!(
        ink(true, false),
        faded,
        "{FILE} is faded with the model ready"
    );
    assert_eq!(
        ink(true, true),
        faded,
        "{FILE} is not faded while a file is translated"
    );
}

#[test]
fn the_chat_calls_what_it_saves_the_history_as_the_menu_does() {
    let mut app = chat();
    let session_dir = std::env::temp_dir().join(format!("jimoto-test-{}", uuid::Uuid::new_v4()));
    app.journal = Some(Journal::new(&session_dir).expect("creates a test journal"));
    let mut window = Window::new(860.0, 620.0);
    window.settle(&mut app);
    assert!(window.has("履歴に自動保存"));
    assert!(
        !texts(&window.output)
            .iter()
            .any(|(text, _, _)| text.contains("タイムライン"))
    );
    let _ = fs::remove_dir_all(&session_dir);
}

#[test]
fn only_the_translation_model_can_make_the_chat_need_preparation() {
    let mut app = chat();
    // Speech recognition starts with the first audio, and readout is off.
    assert!(!app.asr_ready);
    assert_eq!(overall(&app.model_statuses()), Readiness::Ready);

    app.audio_installing = true;
    assert_eq!(overall(&app.model_statuses()), Readiness::Preparing);
    app.audio_installing = false;

    app.settings.readout_enabled = true;
    app.readout.preparing = true;
    assert_eq!(overall(&app.model_statuses()), Readiness::Preparing);
    app.readout.preparing = false;
    assert_eq!(overall(&app.model_statuses()), Readiness::Ready);

    app.backend_ready = false;
    assert_eq!(overall(&app.model_statuses()), Readiness::Missing);
    app.audio_installing = true;
    assert_eq!(
        overall(&app.model_statuses()),
        Readiness::Missing,
        "a model that is being prepared does not hide the one that is missing"
    );
    app.backend_loading = true;
    assert_eq!(overall(&app.model_statuses()), Readiness::Preparing);
}

#[test]
fn the_details_list_every_model_with_the_words_the_status_row_used() {
    let mut app = chat();
    app.settings.readout_enabled = true;
    let mut window = Window::new(860.0, 620.0);
    window.open_menu(&mut app);
    window.hover(&mut app, "状態の詳細");
    window.settle(&mut app);
    assert!(
        window.has("翻訳・音声認識・文字認識の準備の状態を見ます"),
        "the hover text of the row"
    );
    window.click(&mut app, "状態の詳細");
    assert!(app.details_open);
    assert!(window.has("状態の詳細"));
    let tops: Vec<f32> = CHIPS.iter().map(|chip| window.at(chip).top()).collect();
    assert!(
        tops.windows(2).all(|pair| pair[0] < pair[1]),
        "the models are out of order: {tops:?}"
    );
    for detail in ["Windows OCR", "Nemotron Speech", "Irodori-TTS"] {
        assert!(window.has(detail), "{detail} is not drawn");
    }
    assert!(
        texts(&window.output)
            .iter()
            .any(|(text, _, _)| text.ends_with("· 準備完了")),
        "the translation model's own state is not drawn"
    );
    assert!(window.has(credit::LABEL), "the credit is covered");

    window.click(&mut app, "閉じる");
    assert!(!app.details_open);
    assert!(!window.has("状態の詳細"));

    app.settings.readout_enabled = false;
    app.details_open = true;
    window.settle(&mut app);
    assert!(window.has("音声認識未準備") && !window.has("読み上げ未準備"));
}

#[test]
fn the_details_show_a_long_error_in_full_and_a_model_that_is_not_ready_leads_to_its_screen() {
    let mut app = chat();
    app.service_error = LOAD_FAILED.repeat(4);
    app.details_open = true;
    let mut window = Window::new(860.0, 620.0);
    window.settle(&mut app);
    assert!(
        texts(&window.output)
            .iter()
            .any(|(text, _, _)| text == &LOAD_FAILED.repeat(4)),
        "the error is cut short"
    );
    window.click(&mut app, "音声認識未準備");
    assert!(app.tab == Tab::Settings);
    assert!(
        !app.details_open,
        "the details stay over the screen they led to"
    );
}

#[test]
fn the_details_close_on_escape_and_when_another_item_is_chosen() {
    let mut app = chat();
    app.details_open = true;
    let mut window = Window::new(860.0, 620.0);
    window.settle(&mut app);
    window.draw(
        &mut app,
        vec![key(egui::Key::Escape, egui::Modifiers::NONE)],
    );
    assert!(!app.details_open);

    app.details_open = true;
    window.open_menu(&mut app);
    window.click(&mut app, "設定");
    assert!(!app.details_open);
}

fn assert_in_view(window: &Window, heading: &str, size: (f32, f32)) {
    assert!(window.has(heading), "{size:?}: {heading} is not drawn");
    let (rect, save) = (window.at(heading), window.at("設定を保存"));
    assert!(
        rect.top() >= 0.0 && rect.bottom() <= save.top(),
        "{size:?}: {heading} is out of view at {rect:?}"
    );
}

fn follow_details_row(window: &mut Window, app: &mut App, row: &str) {
    app.details_open = true;
    window.settle(app);
    window.click(app, row);
    window.settle(app);
    window.settle(app);
}

fn assert_row_opens_its_card(row: &str, heading: &str, readout: bool) {
    for size in [(860.0, 620.0), (1280.0, 880.0)] {
        let mut app = chat();
        app.settings.ocr_backend = "indra".into();
        app.settings.readout_enabled = readout;
        let mut window = Window::new(size.0, size.1);
        follow_details_row(&mut window, &mut app, row);
        assert!(app.tab == Tab::Settings, "{size:?}: {row} leads elsewhere");
        assert_in_view(&window, heading, size);
    }
}

#[test]
fn the_character_recognition_row_of_the_details_opens_its_card_in_the_settings() {
    assert_row_opens_its_card("文字認識未準備", "画面・画像の文字認識", false);
}

#[test]
fn the_readout_row_of_the_details_opens_its_card_in_the_settings() {
    assert_row_opens_its_card("読み上げ未準備", "訳文の読み上げ", true);
}

#[test]
fn turning_on_automatic_readout_without_a_voice_opens_the_settings_at_the_readout_card() {
    for size in [(860.0, 620.0), (1280.0, 880.0)] {
        let mut app = chat();
        app.settings.readout_enabled = true;
        let mut window = Window::new(size.0, size.1);
        window.settle(&mut app);
        window.click(&mut app, "自動読み上げ");
        window.settle(&mut app);
        window.settle(&mut app);
        assert!(
            app.tab == Tab::Settings && app.readout.confirm && !app.settings.auto_read,
            "{size:?}: the voice is not asked for"
        );
        assert_in_view(&window, "訳文の読み上げ", size);
    }
}

#[test]
fn the_rows_of_speech_and_translation_open_what_they_did_and_leave_the_other_features_closed() {
    let mut app = chat();
    app.backend_ready = false;
    let mut window = Window::new(860.0, 3200.0);
    follow_details_row(&mut window, &mut app, "音声認識未準備");
    assert!(app.tab == Tab::Settings);
    assert!(window.has(OTHER_FEATURES));
    for other in OTHER_SETTINGS {
        assert!(!window.has(other), "{other} is shown: the section opened");
    }
    follow_details_row(&mut window, &mut app, "翻訳未準備");
    assert!(
        app.tab == Tab::Setup,
        "翻訳 no longer opens the model screen"
    );
}

#[test]
fn after_an_arrival_the_settings_from_the_menu_neither_reopen_the_section_nor_scroll() {
    let mut app = chat();
    app.settings.ocr_backend = "indra".into();
    let mut window = Window::new(860.0, 620.0);
    follow_details_row(&mut window, &mut app, "文字認識未準備");
    assert_in_view(&window, "画面・画像の文字認識", (860.0, 620.0));

    window.draw(
        &mut app,
        vec![egui::Event::PointerMoved(egui::pos2(430.0, 300.0))],
    );
    window.draw(&mut app, vec![wheel(20_000.0)]);
    window.settle(&mut app);
    window.settle(&mut app);
    assert!(window.has("翻訳モデル"), "the page is not back at its top");
    window.click(&mut app, BACK_TO_CHAT);
    window.open_menu(&mut app);
    window.click(&mut app, "設定");
    window.settle(&mut app);
    assert!(
        window.has("翻訳モデル") && !window.has("画面・画像の文字認識"),
        "the page scrolled to the card again"
    );

    window.resize(860.0, 3200.0);
    window.settle(&mut app);
    assert!(
        window.has("画面・画像の文字認識"),
        "the section closed by itself"
    );
    window.click(&mut app, OTHER_FEATURES);
    window.click(&mut app, BACK_TO_CHAT);
    window.open_menu(&mut app);
    window.click(&mut app, "設定");
    window.settle(&mut app);
    assert!(
        !window.has("画面・画像の文字認識"),
        "the section opened again"
    );
    window.click(&mut app, BACK_TO_CHAT);
    window.draw(&mut app, vec![key(egui::Key::Num4, egui::Modifiers::CTRL)]);
    window.settle(&mut app);
    assert!(app.tab == Tab::Settings);
    assert!(
        !window.has("画面・画像の文字認識"),
        "Ctrl + 4 opened the section again"
    );
}

#[test]
fn the_chat_header_shows_clear_to_the_left_of_the_menu_and_enables_with_content() {
    let mut app = chat();
    let mut window = Window::new(860.0, 620.0);
    window.settle(&mut app);

    let (clear, menu) = (window.at("クリア"), window.at("⋯ その他"));
    assert!(
        clear.right() < menu.left(),
        "クリア must sit to the left of ⋯ その他"
    );
    assert!(
        (clear.center().y - menu.center().y).abs() < 2.0,
        "クリア must be on the same row as ⋯ その他"
    );

    // Disabled while the chat is empty and no speech is being recognised.
    let empty_ink = window.ink("クリア");

    // Enabled when the chat has a row.
    app.rows.push(row(1, "音声", "こんにちは"));
    window.settle(&mut app);
    let enabled_ink = window.ink("クリア");
    assert_ne!(
        empty_ink, enabled_ink,
        "クリア should be enabled when the chat has a row"
    );

    window.hover(&mut app, "クリア");
    assert!(
        texts(&window.output).iter().any(|(text, _, _)| text
            .contains("表示中の会話を消して、新しく始めます。消した会話は「履歴」に残ります。")),
        "hover text is missing or does not match"
    );

    // Enabled when speech is being recognised even if rows are empty.
    app.rows.clear();
    app.asr_partial = " recognizing... ".into();
    window.settle(&mut app);
    assert_eq!(
        window.ink("クリア"),
        enabled_ink,
        "クリア should be enabled while speech is recognised"
    );

    // Disabled when asr_partial is empty after trim.
    app.asr_partial = "   ".into();
    window.settle(&mut app);
    assert_eq!(
        window.ink("クリア"),
        empty_ink,
        "クリア should be disabled when asr_partial is empty after trim"
    );
}

#[test]
fn the_chat_header_copies_the_whole_talk_from_beside_clear() {
    let mut app = chat();
    let mut window = Window::new(860.0, 620.0);
    window.settle(&mut app);

    let (copy, clear) = (window.at("全文コピー"), window.at("クリア"));
    assert!(
        copy.right() < clear.left(),
        "全文コピー must sit to the left of クリア"
    );
    assert!(
        (copy.center().y - clear.center().y).abs() < 2.0,
        "全文コピー must be on the same row as クリア"
    );
    let empty_ink = window.ink("全文コピー");

    for (id, translation) in [(1, "こんにちは。"), (2, "ありがとう。")] {
        app.rows.push(row(id, "音声", translation));
    }
    window.settle(&mut app);
    assert_ne!(
        window.ink("全文コピー"),
        empty_ink,
        "全文コピー should be enabled once a row is translated"
    );
    window.click(&mut app, "全文コピー");
    assert_eq!(
        app.live_chat.copied(),
        Some(credit::published("こんにちは。\n\nありがとう。").as_str())
    );
}

#[test]
fn clicking_clear_empties_the_chat_and_starts_a_new_session_preserving_earlier_records() {
    let mut app = chat();
    let session_dir = std::env::temp_dir().join(format!("jimoto-test-{}", uuid::Uuid::new_v4()));
    let initial_journal = Journal::new(&session_dir).expect("creates initial test journal");
    let initial_path = initial_journal.path.clone();

    let record = Record {
        id: 1,
        start_ms: 0,
        end_ms: 1000,
        source: "Hello world.".into(),
        translation: "こんにちは世界。".into(),
        standard_translation: None,
        kind: "音声".into(),
        video_id: String::new(),
        created_at: "2026-11-08T10:00:00+09:00".into(),
        paragraph: None,
    };
    initial_journal
        .append(&record)
        .expect("appends record to initial journal");

    app.journal = Some(initial_journal);
    app.rows = vec![Row {
        record,
        pending: false,
        error: None,
        generation: 0,
    }];

    let mut window = Window::new(860.0, 620.0);
    window.settle(&mut app);
    assert!(!app.rows.is_empty());

    window.click(&mut app, "クリア");

    assert!(app.rows.is_empty(), "chat rows must be emptied");
    let new_journal = app.journal.as_ref().expect("a new session is started");
    assert_ne!(
        new_journal.path, initial_path,
        "the journal must be a different session"
    );
    assert_eq!(
        new_journal.path.parent(),
        Some(session_dir.as_path()),
        "the new session file must be in the same folder"
    );

    let prior_contents = fs::read_to_string(&initial_path).expect("earlier session file remains");
    assert!(
        prior_contents.contains("Hello world."),
        "earlier session file still holds its records"
    );

    let _ = fs::remove_dir_all(&session_dir);
}

#[test]
fn copy_all_and_clear_follow_what_the_chat_shows() {
    let inks = |rows: Vec<Row>| {
        let mut app = chat();
        app.rows = rows;
        let mut window = Window::new(860.0, 620.0);
        window.settle(&mut app);
        (window.ink("全文コピー"), window.ink("クリア"))
    };
    let faded = inks(Vec::new());
    assert_eq!(
        inks(vec![
            row(1, "文章", "文章の訳文です。"),
            row(2, "画面OCR", "範囲の訳文です。")
        ]),
        faded,
        "text and range translations are not in the chat"
    );
    let shown = inks(vec![row(1, "音声", "こんにちは。")]);
    assert!(
        shown.0 != faded.0 && shown.1 != faded.1,
        "a PC-audio message is in the chat"
    );
}

#[test]
fn clear_keeps_a_text_translation_that_is_still_coming_and_its_end_arrives() {
    for result in [
        Ok("完成した訳文です。"),
        Err("翻訳サーバーに接続できませんでした。"),
    ] {
        let history = tempfile::tempdir().unwrap();
        let mut app = chat();
        app.journal = Some(Journal::new(history.path()).unwrap());
        app.rows.push(row(100, "音声", "こんにちは。"));
        app.settings.translation.endpoint = "no translator".into();
        app.input = "Hello.".into();
        app.manual_translate();
        let id = app.manual_busy.expect("the translation is in flight");
        assert!(matches!(
            app.rx.recv_timeout(Duration::from_secs(10)),
            Ok(Event::Done { .. })
        ));
        let ctx = egui::Context::default();
        app.tx
            .send(Event::Partial {
                id,
                text: "途中の訳".into(),
            })
            .unwrap();
        app.process(&ctx);

        let mut window = Window::new(860.0, 620.0);
        window.settle(&mut app);
        window.click(&mut app, "クリア");
        app.tx
            .send(Event::Done {
                id,
                result: result.map(Into::into).map_err(Into::into),
                elapsed: 0.1,
            })
            .unwrap();
        app.process(&ctx);
        match result {
            Ok(text) => {
                assert_eq!(app.output, text, "the 訳文 card keeps the unfinished text");
                let saved = fs::read_to_string(&app.journal.as_ref().unwrap().path).unwrap();
                assert!(saved.contains(text), "the history misses the result");
            }
            Err(error) => assert_eq!(app.status, error, "the failure is shown nowhere"),
        }
    }
}

const ONE_COLUMN: [&str; 8] = [
    "翻訳モデル",
    "音声認識",
    "文章をつなぐ翻訳",
    "YouTube字幕の接続",
    "用語集と文脈",
    "ショートカットと保存先",
    "詳細（トラブル時）",
    OTHER_FEATURES,
];
const LEFT: [&str; 3] = ["翻訳モデル", "文章をつなぐ翻訳", "用語集と文脈"];
const RIGHT: [&str; 5] = [
    "音声認識",
    "YouTube字幕の接続",
    "ショートカットと保存先",
    "詳細（トラブル時）",
    OTHER_FEATURES,
];
const OTHER_SETTINGS: [&str; 2] = ["訳文の読み上げ", "画面・画像の文字認識"];

fn assert_in_order(window: &Window, width: f32, headings: &[&str]) {
    let tops: Vec<f32> = headings
        .iter()
        .map(|heading| window.at(heading).top())
        .collect();
    assert!(
        tops.windows(2).all(|pair| pair[0] < pair[1]),
        "{width}: {headings:?} are out of order: {tops:?}"
    );
}

#[test]
fn the_settings_start_with_the_language_pair_and_open_the_other_features_on_request() {
    for width in [860.0, 1400.0] {
        let mut app = chat();
        let mut window = Window::new(width, 3200.0);
        window.open_menu(&mut app);
        window.click(&mut app, "設定");
        window.settle(&mut app);
        if width < 1100.0 {
            assert_in_order(&window, width, &ONE_COLUMN);
        } else {
            assert_in_order(&window, width, &LEFT);
            assert_in_order(&window, width, &RIGHT);
            let middle = width / 2.0;
            for heading in LEFT {
                assert!(
                    window.at(heading).right() < middle,
                    "{heading} is not on the left"
                );
            }
            for heading in RIGHT {
                assert!(
                    window.at(heading).left() > middle,
                    "{heading} is not on the right"
                );
            }
        }

        let (model, pair) = (window.at("翻訳モデル"), window.at("翻訳する言語"));
        let (source, target) = (window.at("自動判定"), window.at("日本語"));
        let next = window.at(if width < 1100.0 {
            "音声認識"
        } else {
            "文章をつなぐ翻訳"
        });
        assert!(
            model.top() < pair.top()
                && pair.bottom() <= source.top()
                && source.bottom() < next.top(),
            "{width}: the pair is not in the card of the model"
        );
        assert!(source.right() < target.left() && window.has("交換") && window.has("常体"));

        for other in OTHER_SETTINGS {
            assert!(
                !window.has(other),
                "{width}: {other} is shown before {OTHER_FEATURES} is opened"
            );
        }
        window.click(&mut app, OTHER_FEATURES);
        let mut opened = vec![OTHER_FEATURES];
        opened.extend(OTHER_SETTINGS);
        assert_in_order(&window, width, &opened);
        window.click(&mut app, "読み上げを使う");
        assert!(
            app.settings.readout_enabled,
            "{width}: the switch of readout"
        );
    }
}

#[test]
fn the_settings_list_the_shortcuts_of_what_is_in_front() {
    let mut app = chat();
    app.tab = Tab::Settings;
    let mut window = Window::new(860.0, 3200.0);
    window.settle(&mut app);
    window.click(&mut app, "ショートカットと保存先");
    assert_in_order(
        &window,
        860.0,
        &[
            "聞く・止める: Ctrl + Alt + L",
            "ミニ表示: Ctrl + Shift + M",
            "シンプル翻訳・チャット・履歴・設定: Ctrl + 1〜4",
        ],
    );
    assert!(
        !texts(&window.output)
            .iter()
            .any(|(text, _, _)| text.contains("範囲翻訳")),
        "a key of the other features is listed"
    );
}
