//! The chat is the home screen and every other screen is behind one menu.
use super::*;
use crate::chat::testing::{find_text, press, texts};
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
}

fn hardware() -> Hardware {
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

const SCREENS: [(Tab, &str); 4] = [
    (Tab::Text, "文章を翻訳"),
    (Tab::Files, "履歴"),
    (Tab::Settings, "設定"),
    (Tab::Setup, "モデル"),
];

/// Every screen that has the page header: the chat and the four behind the menu.
const HEADERED: [(Tab, &str); 5] = [
    (Tab::Text, "文章"),
    (Tab::Live, "チャット"),
    (Tab::Files, "履歴"),
    (Tab::Settings, "設定"),
    (Tab::Setup, "モデル"),
];

#[test]
fn the_app_opens_on_the_chat_with_its_name_and_one_menu_and_nothing_else() {
    assert!(Tab::default() == Tab::Live);
    for (width, height) in [(860.0, 620.0), (2560.0, 1369.0)] {
        let mut app = chat();
        let mut window = Window::new(width, height);
        window.settle(&mut app);
        for shown in ["地元翻訳くん", "PC内で翻訳", "⋯ その他", credit::LABEL] {
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
            "新しく始める",
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
fn a_missing_model_opens_the_model_screen_with_a_way_back() {
    let mut app = chat();
    app.backend_ready = false;
    app.open_setup();
    let mut window = Window::new(860.0, 620.0);
    window.settle(&mut app);
    assert!(app.tab == Tab::Setup);
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

#[test]
fn the_menu_lists_the_screens_in_order_with_the_shortcut_each_has() {
    let mut app = chat();
    let mut window = Window::new(860.0, 620.0);
    window.open_menu(&mut app);
    let items = [
        "字幕ウィンドウ",
        "ミニ表示",
        "文章を翻訳",
        "範囲翻訳",
        "画面の文字を翻訳",
        "ファイルを翻訳",
        "履歴",
        "モデル",
        "設定",
        "状態の詳細",
    ];
    let tops: Vec<f32> = items.iter().map(|item| window.at(item).top()).collect();
    assert!(
        tops.windows(2).all(|pair| pair[0] < pair[1]),
        "the items are out of order: {tops:?}"
    );
    for (item, shortcut) in [
        ("ミニ表示", "Ctrl + Shift + M"),
        ("文章を翻訳", "Ctrl + 1"),
        ("範囲翻訳", "Ctrl + Alt + T"),
        ("履歴", "Ctrl + 3"),
        ("設定", "Ctrl + 4"),
    ] {
        let (row, hint) = (window.at(item), window.at(shortcut));
        assert!(
            hint.left() > row.right() && (hint.center().y - row.center().y).abs() < 2.0,
            "{shortcut} is not at the end of the {item} row"
        );
    }
    for (item, shortcut) in [
        ("字幕ウィンドウ", "Ctrl"),
        ("画面の文字を翻訳", "Ctrl"),
        ("ファイルを翻訳", "Ctrl"),
        ("モデル", "Ctrl"),
        ("状態の詳細", "Ctrl"),
    ] {
        let row = window.at(item);
        assert!(
            !texts(&window.output).iter().any(|(text, rect, _)| {
                text.starts_with(shortcut) && (rect.center().y - row.center().y).abs() < 2.0
            }),
            "{item} has no shortcut to show"
        );
    }
    let last = window.at("状態の詳細").bottom();
    for item in ["新しく始める", "書き出し", "入力の切り替え", "表示"] {
        assert!(
            window.at(item).top() > last,
            "{item} is not below the screens"
        );
    }
    assert!(
        window.at("表示").bottom() < 620.0 - theme::CONTROL_H,
        "the menu runs into the status row at the smallest window"
    );
}

#[test]
fn the_menu_on_another_screen_has_the_screens_and_not_what_belongs_to_the_chat() {
    let mut app = chat();
    app.tab = Tab::Files;
    let mut window = Window::new(860.0, 620.0);
    window.open_menu(&mut app);
    assert!(window.has("設定") && window.has("字幕ウィンドウ"));
    for chat_only in ["新しく始める", "入力の切り替え", "表示"] {
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
        window.open_menu(&mut app);
        window.click(&mut app, item);
        assert!(app.tab == screen, "{item} did not open its screen");
        assert!(
            !window.has("地元翻訳くん"),
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
        assert!(window.has("地元翻訳くん"));
        assert!(!window.has("← チャットに戻る"));
    }
}

#[test]
fn the_other_items_do_what_their_buttons_did() {
    let mut app = chat();
    let mut window = Window::new(860.0, 620.0);
    window.open_menu(&mut app);
    window.click(&mut app, "字幕ウィンドウ");
    assert!(app.subtitle_window.open);

    let mut app = chat();
    let mut window = Window::new(860.0, 620.0);
    window.open_menu(&mut app);
    window.click(&mut app, "ミニ表示");
    assert!(app.window_mode.is_mini());

    // The reading of a range is shown on the 文章 screen, so it opens there.
    // The test machine has no screen to read, and says so.
    let mut app = chat();
    app.status = "before".into();
    let mut window = Window::new(860.0, 620.0);
    window.open_menu(&mut app);
    window.click(&mut app, "範囲翻訳");
    assert!(app.tab == Tab::Text);
    assert_eq!(app.status, "取得できる画面がありません。");

    // The 画面 input feeds the chat, so it stays there.
    let mut app = chat();
    app.status = "before".into();
    let mut window = Window::new(860.0, 620.0);
    window.open_menu(&mut app);
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
        window.open_menu(&mut app);
        [
            "字幕ウィンドウ",
            "ミニ表示",
            "文章を翻訳",
            "範囲翻訳",
            "画面の文字を翻訳",
            "ファイルを翻訳",
            "履歴",
            "モデル",
            "設定",
            "状態の詳細",
            "新しく始める",
            "入力の切り替え",
            "表示",
        ]
        .map(|item| (item, window.ink(item)))
    };
    let needs_the_model = ["ミニ表示", "範囲翻訳", "画面の文字を翻訳", "ファイルを翻訳"];
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
    let screen = |id: u32, name: &str| platform::Screen {
        id,
        name: name.into(),
        x: 0,
        y: 0,
        width: 1920,
        height: 1080,
        scale: 1.0,
    };
    let mut app = chat();
    app.screens = vec![screen(1, "DISPLAY1")];
    let mut window = Window::new(860.0, 620.0);
    window.open_menu(&mut app);
    assert!(
        !window.has("画面選択"),
        "one screen leaves nothing to choose"
    );

    app.screens.push(screen(2, "DISPLAY2"));
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

#[test]
fn the_readout_switch_stays_beside_the_menu() {
    let mut app = chat();
    app.settings.readout_enabled = true;
    let mut window = Window::new(860.0, 620.0);
    window.settle(&mut app);
    let (switch, menu) = (window.at("自動読み上げ"), window.at("⋯ その他"));
    assert!(switch.right() < menu.left());
    assert!((switch.center().y - menu.center().y).abs() < 2.0);
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
        (Tab::Text, "文章翻訳"),
        (Tab::Files, "履歴"),
        (Tab::Settings, "設定"),
        (Tab::Setup, "このPCに合うモデルを準備"),
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
            (Tab::Text, "文章"),
            (Tab::Files, "履歴"),
            (Tab::Settings, "設定"),
            (Tab::Setup, "モデル"),
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
        for (screen, _, key) in PAGE_KEYS {
            let mut app = chat();
            app.tab = from;
            let mut window = Window::new(860.0, 620.0);
            window.draw(
                &mut app,
                vec![egui::Event::Key {
                    key,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: egui::Modifiers::CTRL,
                }],
            );
            assert!(
                app.tab == screen,
                "Ctrl+{} did not open its screen",
                key.name()
            );
        }
    }
}

/// What the status row painted before the chat took one state in its place:
/// the name of a model and, unless it is ready, its state as one text.
const CHIPS: [&str; 4] = ["翻訳", "文字認識", "音声認識未準備", "読み上げ未準備"];

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

    // An error stands in the state's place, in red and in its own words.
    let window = draw(|app| app.service_error = LOAD_FAILED.into());
    only(&window, "");
    assert_eq!(find_text(&window.output, LOAD_FAILED).1, theme::ERROR);
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
        vec![egui::Event::Key {
            key: egui::Key::Escape,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        }],
    );
    assert!(!app.details_open);

    app.details_open = true;
    window.open_menu(&mut app);
    window.click(&mut app, "設定");
    assert!(!app.details_open);
}
