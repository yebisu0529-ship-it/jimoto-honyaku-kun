//! The chat is the home screen and every other screen is behind one menu.
use super::*;
use crate::chat::testing::{press, texts};
use plamo_translate::hardware::Hardware;

/// One window whose egui state lasts from frame to frame, so a menu that has
/// been opened stays open.
struct Window {
    context: egui::Context,
    size: egui::Vec2,
    frame: u32,
    output: egui::FullOutput,
}

impl Window {
    fn new(width: f32, height: f32) -> Self {
        let context = egui::Context::default();
        configure_theme(&context);
        Self {
            context,
            size: egui::vec2(width, height),
            frame: 0,
            output: egui::FullOutput::default(),
        }
    }

    fn draw(&mut self, app: &mut App, events: Vec<egui::Event>) {
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

    fn settle(&mut self, app: &mut App) {
        for _ in 0..4 {
            self.draw(app, vec![]);
        }
    }

    fn has(&self, label: &str) -> bool {
        texts(&self.output).iter().any(|(text, _, _)| text == label)
    }

    /// Where the one text drawn exactly as `label` is.
    fn at(&self, label: &str) -> egui::Rect {
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
    fn ink(&self, label: &str) -> Color32 {
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
    fn click(&mut self, app: &mut App, label: &str) {
        let at = self.at(label).center();
        self.draw(app, vec![egui::Event::PointerMoved(at)]);
        self.draw(app, vec![press(at, egui::PointerButton::Primary, true)]);
        self.draw(app, vec![press(at, egui::PointerButton::Primary, false)]);
        self.settle(app);
    }

    fn open_menu(&mut self, app: &mut App) {
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
fn chat() -> App {
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
    ] {
        let row = window.at(item);
        assert!(
            !texts(&window.output).iter().any(|(text, rect, _)| {
                text.starts_with(shortcut) && (rect.center().y - row.center().y).abs() < 2.0
            }),
            "{item} has no shortcut to show"
        );
    }
    let last = window.at("設定").bottom();
    for item in ["新しく始める", "書き出し", "表示"] {
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
    for chat_only in ["新しく始める", "表示"] {
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
            "新しく始める",
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
    for (tab, page, _) in PAGE_KEYS {
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
    for (tab, page, _) in PAGE_KEYS {
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
