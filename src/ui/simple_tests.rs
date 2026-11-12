//! シンプル翻訳: what is put on the left is translated on the right, by itself
//! once the input has rested for a second, and at once on 翻訳する.
use super::home_tests::{Window, chat, key, row};
use super::*;
use crate::chat::testing::{has_text_containing, press, texts};
use crate::document::tests::{Completion, translation_stub};

const TITLE: &str = "シンプル翻訳";
const SAYS: &str = "左に入れた文を、右に訳します。";

/// The screen with the translation model ready. Its translations go to the
/// translator at `endpoint`.
fn screen(endpoint: String) -> App {
    let mut app = chat();
    app.tab = Tab::Text;
    app.settings.translation.endpoint = endpoint;
    app
}

/// Frames for `seconds` of the app's own time, with nothing else happening.
fn wait(window: &mut Window, app: &mut App, seconds: f64) {
    for _ in 0..(seconds * 60.0).round() as u32 {
        window.draw(app, vec![]);
    }
}

/// The next prompt the translator was sent, once its translation is done and
/// the app has taken in what came back, as its event loop does.
fn translated(app: &mut App, prompts: &mpsc::Receiver<String>) -> String {
    let prompt = prompts
        .recv_timeout(Duration::from_secs(10))
        .expect("the translator was not asked");
    let mut events = Vec::new();
    loop {
        let event = app
            .rx
            .recv_timeout(Duration::from_secs(10))
            .expect("the translation did not end");
        let done = matches!(event, Event::Done { .. });
        events.push(event);
        if done {
            break;
        }
    }
    for event in events {
        app.tx.send(event).unwrap();
    }
    app.process(&egui::Context::default());
    prompt
}

/// The words a prompt asks to have translated.
fn words(prompt: &str) -> &str {
    let input = &prompt[prompt
        .find("<|plamo:tag|>input")
        .expect("a translation prompt")..];
    let start = input.find("<|plamo:msg|>").unwrap() + "<|plamo:msg|>".len();
    &input[start..input.find("<|plamo:tag|>output").unwrap()]
}

/// The language a prompt asks for, with its form when it has one: `ja/polite`.
fn into(prompt: &str) -> String {
    let output = &prompt[prompt.find("<|plamo:tag|>output").unwrap()..];
    output
        .split("<|plamo:val|>")
        .skip(1)
        .map(|value| value.split("<|plamo:").next().unwrap())
        .collect::<Vec<_>>()
        .join("/")
}

fn history(app: &App) -> String {
    fs::read_to_string(&app.journal.as_ref().unwrap().path).unwrap_or_default()
}

/// Ctrl+Enter in a frame of its own: egui takes Ctrl from the frame, not
/// from the key.
fn ctrl_enter(app: &mut App) {
    let ctx = egui::Context::default();
    configure_theme(&ctx);
    let _ = ctx.run(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(860.0, 620.0),
            )),
            modifiers: egui::Modifiers::CTRL,
            events: vec![key(egui::Key::Enter, egui::Modifiers::CTRL)],
            ..Default::default()
        },
        |ctx| app.render_ui(ctx),
    );
}

/// Whether the one text drawn as `label` is cut short.
fn cut(window: &Window, label: &str) -> bool {
    fn visit(shape: &egui::epaint::Shape, label: &str, out: &mut Vec<bool>) {
        match shape {
            egui::epaint::Shape::Text(text) if text.galley.text() == label => {
                out.push(text.galley.elided)
            }
            egui::epaint::Shape::Vec(shapes) => {
                shapes.iter().for_each(|shape| visit(shape, label, out))
            }
            _ => {}
        }
    }
    let mut found = Vec::new();
    for clipped in &window.output.shapes {
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

/// Where the row `label` of the open menu is: the one text below the menu
/// button, apart from the button of the same name in the header.
fn in_menu(window: &Window, label: &str) -> egui::Rect {
    let below = window.at("⋯ その他").bottom();
    let found: Vec<_> = texts(&window.output)
        .into_iter()
        .filter(|(text, rect, _)| text == label && rect.top() >= below)
        .collect();
    assert_eq!(
        found.len(),
        1,
        "expected one {label} in the menu, found {}",
        found.len()
    );
    found[0].1
}

#[test]
fn the_text_screen_is_simple_translation_and_says_that_it_translates_the_left_on_the_right() {
    let mut app = chat();
    app.tab = Tab::Text;
    let mut window = Window::new(860.0, 620.0);
    window.settle(&mut app);
    assert!(window.has(TITLE), "the title is not {TITLE}");
    assert!(window.has(SAYS), "the subtitle is not {SAYS}");
    assert!(!window.has("文章翻訳"), "the old title is still drawn");
}

#[test]
fn the_chat_header_opens_simple_translation_from_a_button_left_of_its_talk_buttons() {
    for readout in [false, true] {
        let mut app = chat();
        app.settings.readout_enabled = readout;
        let mut window = Window::new(860.0, 620.0);
        window.settle(&mut app);
        let (button, copy, clear) = (
            window.at(TITLE),
            window.at("全文コピー"),
            window.at("クリア"),
        );
        assert!(
            button.right() < copy.left() && copy.right() < clear.left(),
            "{TITLE} is not left of 全文コピー and クリア: {button:?} {copy:?} {clear:?}"
        );
        assert!(
            (button.center().y - clear.center().y).abs() < 2.0,
            "{TITLE} is not on the row of クリア"
        );
        let header: Vec<_> = texts(&window.output)
            .into_iter()
            .filter(|(_, rect, _)| rect.top() < 16.0 + theme::CONTROL_H)
            .collect();
        for (index, (first, a, _)) in header.iter().enumerate() {
            assert!(
                a.left() >= 0.0 && a.right() <= 860.0,
                "readout {readout}: {first} leaves the window"
            );
            for (second, b, _) in &header[index + 1..] {
                assert!(
                    !a.intersects(*b),
                    "readout {readout}: {first} overlaps {second}"
                );
            }
        }
        for whole in ["Talk風Live翻訳", "PC内で翻訳", TITLE] {
            assert!(!cut(&window, whole), "readout {readout}: {whole} is cut");
        }
        window.hover(&mut app, TITLE);
        assert!(window.has(SAYS), "the hover text of {TITLE}");
        window.click(&mut app, TITLE);
        assert!(app.tab == Tab::Text, "{TITLE} does not open its screen");
    }

    // It can always be pressed: with an empty chat and no model as with a
    // talk and the model, while クリア cannot be pressed in an empty chat.
    let inks = |ready: bool, talk: bool| {
        let mut app = chat();
        app.backend_ready = ready;
        if talk {
            app.rows.push(row(1, "音声", "こんにちは。"));
        }
        let mut window = Window::new(860.0, 620.0);
        window.settle(&mut app);
        (window.ink(TITLE), window.ink("クリア"))
    };
    let (empty, faded_clear) = inks(false, false);
    let (full, clear) = inks(true, true);
    assert_ne!(faded_clear, clear, "クリア is not faded in an empty chat");
    assert_eq!(
        empty, full,
        "{TITLE} is faded in an empty chat without the model"
    );
}

#[test]
fn the_menu_starts_with_simple_translation_and_ctrl_1_and_the_other_features_no_longer_have_it() {
    let mut app = chat();
    let mut window = Window::new(860.0, 620.0);
    window.open_menu(&mut app);
    let first = in_menu(&window, TITLE);
    for item in [
        "ミニ表示",
        "履歴",
        "モデル",
        "設定",
        "状態の詳細",
        "入力の切り替え",
        "書き出し",
        "表示",
        OTHER_FEATURES,
    ] {
        assert!(
            first.bottom() <= window.at(item).top(),
            "{item} is above {TITLE}"
        );
    }
    let shortcut = in_menu(&window, "Ctrl + 1");
    assert!(
        shortcut.left() > first.right() && (shortcut.center().y - first.center().y).abs() < 2.0,
        "Ctrl + 1 is not at the end of the {TITLE} row"
    );
    let at = first.center();
    window.draw(&mut app, vec![egui::Event::PointerMoved(at)]);
    window.draw(
        &mut app,
        vec![press(at, egui::PointerButton::Primary, true)],
    );
    window.draw(
        &mut app,
        vec![press(at, egui::PointerButton::Primary, false)],
    );
    window.settle(&mut app);
    assert!(app.tab == Tab::Text, "the row does not open the screen");

    let mut app = chat();
    let mut window = Window::new(860.0, 620.0);
    window.open_other_features(&mut app);
    assert!(
        !window.has("文章を翻訳"),
        "文章を翻訳 is still behind {OTHER_FEATURES}"
    );

    // The keys of what is in front start with Ctrl + 1 now.
    let mut app = chat();
    app.tab = Tab::Settings;
    let mut window = Window::new(860.0, 3200.0);
    window.settle(&mut app);
    window.click(&mut app, "ショートカットと保存先");
    assert!(
        window.has("シンプル翻訳・チャット・履歴・設定: Ctrl + 1〜4"),
        "the settings do not list Ctrl + 1 with the keys of what is in front"
    );
}

#[test]
fn a_picture_is_read_from_the_other_features_and_the_screen_keeps_only_its_cards() {
    let rows = |ready: bool| {
        let mut app = chat();
        app.backend_ready = ready;
        let mut window = Window::new(860.0, 620.0);
        window.open_other_features(&mut app);
        (
            window.at("画面の文字を翻訳"),
            window.at("画像から翻訳"),
            window.at("ファイルを翻訳"),
            window.ink("画像から翻訳"),
        )
    };
    let (screen_text, picture, file, ready) = rows(true);
    assert!(
        screen_text.bottom() <= picture.top() && picture.bottom() <= file.top(),
        "画像から翻訳 is not the row after 画面の文字を翻訳"
    );
    assert_ne!(
        rows(false).3,
        ready,
        "画像から翻訳 is not faded without the model"
    );

    let mut app = chat();
    app.tab = Tab::Text;
    app.file_busy = true;
    let mut window = Window::new(860.0, 620.0);
    window.settle(&mut app);
    for gone in ["画像から翻訳", "ファイルを翻訳", "読み上げ"] {
        assert!(!window.has(gone), "{gone} is still on the screen");
    }
    for shown in [
        TITLE,
        SAYS,
        "← チャットに戻る",
        "⋯ その他",
        "翻訳する",
        "ファイルを翻訳しています",
        "中止",
        "貼り付け",
        "クリア",
        "訳文",
        "コピー",
        "保存",
    ] {
        assert!(window.has(shown), "{shown} is not on the screen");
    }
    app.settings.readout_enabled = true;
    window.settle(&mut app);
    assert!(
        window.has("読み上げ"),
        "読み上げ is not on the screen once read-aloud is on"
    );
}

#[test]
fn the_input_is_translated_by_itself_a_second_after_it_stops_changing_and_not_before() {
    let (endpoint, prompts) = translation_stub(vec![Completion::Text("こんにちは。")]);
    let mut app = screen(endpoint);
    let mut window = Window::new(860.0, 620.0);
    window.settle(&mut app);
    app.input = "Hel".into();
    wait(&mut window, &mut app, 0.5);
    app.input = "Hello.".into();
    window.draw(&mut app, vec![]);
    let asked_for = window.output.viewport_output[&egui::ViewportId::ROOT].repaint_delay;
    assert!(
        Duration::from_millis(900) <= asked_for && asked_for <= Duration::from_secs(1),
        "egui is not asked for a frame at the end of the pause: {asked_for:?}"
    );
    wait(&mut window, &mut app, 0.85);
    assert!(
        app.manual_busy.is_none(),
        "the input was sent before it rested for a second"
    );
    wait(&mut window, &mut app, 0.25);
    assert!(
        app.manual_busy.is_some(),
        "the input was not sent a second after it stopped changing"
    );
    assert_eq!(words(&translated(&mut app, &prompts)), "Hello.");
    window.settle(&mut app);
    assert!(
        window.has("こんにちは。"),
        "the translation is not on the right"
    );
    wait(&mut window, &mut app, 1.2);
    assert!(
        app.manual_busy.is_none() && prompts.try_recv().is_err(),
        "the input was sent again"
    );
}

#[test]
fn only_the_newest_input_is_sent_once_the_translation_in_flight_is_done() {
    let (endpoint, prompts) = translation_stub(vec![
        Completion::Text("一つ目。"),
        Completion::Text("三つ目。"),
    ]);
    let mut app = screen(endpoint);
    let mut window = Window::new(860.0, 620.0);
    app.input = "One.".into();
    wait(&mut window, &mut app, 1.2);
    let first = app.manual_busy.expect("One. was not sent by itself");

    // The reader goes on typing and stops twice while One. is translated.
    app.input = "Two.".into();
    wait(&mut window, &mut app, 1.2);
    app.input = "Three.".into();
    wait(&mut window, &mut app, 1.2);
    assert_eq!(
        app.manual_busy,
        Some(first),
        "a second translation was sent while the first was in flight"
    );
    assert_eq!(words(&translated(&mut app, &prompts)), "One.");
    assert_eq!(
        app.output, "一つ目。",
        "the result for the older input is not shown when it comes"
    );

    window.draw(&mut app, vec![]);
    assert!(
        app.manual_busy.is_some_and(|id| id != first),
        "the newest input was not sent once the lane was free"
    );
    assert_eq!(words(&translated(&mut app, &prompts)), "Three.");
    assert_eq!(app.output, "三つ目。");
    wait(&mut window, &mut app, 1.2);
    assert!(
        app.manual_busy.is_none() && prompts.try_recv().is_err(),
        "an older input was sent as well"
    );
}

#[test]
fn emptying_the_input_clears_the_translation_and_a_result_that_comes_after_is_not_shown() {
    let (endpoint, prompts) = translation_stub(vec![
        Completion::Text("こんにちは。"),
        Completion::Text("おやすみなさい。"),
    ]);
    let mut app = screen(endpoint);
    let mut window = Window::new(860.0, 620.0);
    app.input = "Hello.".into();
    wait(&mut window, &mut app, 1.2);
    assert!(
        app.manual_busy.is_some(),
        "the input was not sent by itself"
    );
    translated(&mut app, &prompts);
    window.settle(&mut app);
    assert!(window.has("こんにちは。"));
    window.click(&mut app, "クリア");
    assert!(app.input.is_empty());
    assert!(
        app.output.is_empty() && !window.has("こんにちは。"),
        "the translation stays after the input is emptied"
    );

    app.input = "Good night.".into();
    wait(&mut window, &mut app, 1.2);
    assert!(app.manual_busy.is_some(), "Good night. was not sent");
    window.click(&mut app, "クリア");
    assert_eq!(words(&translated(&mut app, &prompts)), "Good night.");
    window.settle(&mut app);
    assert!(
        app.output.is_empty() && !window.has("おやすみなさい。"),
        "a result that came after the input was emptied is shown"
    );
    wait(&mut window, &mut app, 1.2);
    assert!(
        app.manual_busy.is_none() && prompts.try_recv().is_err(),
        "something was sent for an empty input"
    );
}

#[test]
fn a_change_of_languages_or_style_translates_the_input_again() {
    let (endpoint, prompts) = translation_stub(vec![
        Completion::Text("こんにちは。"),
        Completion::Text("Hello."),
        Completion::Text("こんにちは。"),
        Completion::Text("こんにちは。"),
    ]);
    let mut app = screen(endpoint);
    app.settings.translation.source_lang = "en".into();
    let mut window = Window::new(860.0, 620.0);
    app.input = "Hello.".into();
    wait(&mut window, &mut app, 1.2);
    assert!(
        app.manual_busy.is_some(),
        "the input was not sent by itself"
    );
    assert_eq!(into(&translated(&mut app, &prompts)), "ja/plain");

    for (change, wanted) in [("交換", "en"), ("交換", "ja/plain")] {
        window.click(&mut app, change);
        wait(&mut window, &mut app, 1.2);
        assert!(
            app.manual_busy.is_some(),
            "{change} did not translate again"
        );
        assert_eq!(into(&translated(&mut app, &prompts)), wanted);
    }
    window.click(&mut app, "常体");
    window.click(&mut app, "です・ます");
    wait(&mut window, &mut app, 1.2);
    assert!(
        app.manual_busy.is_some(),
        "です・ます did not translate again"
    );
    assert_eq!(into(&translated(&mut app, &prompts)), "ja/polite");
    wait(&mut window, &mut app, 1.2);
    assert!(prompts.try_recv().is_err(), "a change was translated twice");
}

#[test]
fn an_automatic_translation_stays_out_of_the_history_and_a_press_is_written_to_it_once() {
    let folder = tempfile::tempdir().unwrap();
    let (endpoint, prompts) = translation_stub(vec![
        Completion::Text("こんにちは。"),
        Completion::Text("こんにちは。"),
    ]);
    let mut app = screen(endpoint);
    app.journal = Some(Journal::new(folder.path()).unwrap());
    let mut window = Window::new(860.0, 620.0);
    app.input = "Hello.".into();
    wait(&mut window, &mut app, 1.2);
    assert!(
        app.manual_busy.is_some(),
        "the input was not sent by itself"
    );
    translated(&mut app, &prompts);
    assert_eq!(app.output, "こんにちは。");
    assert!(
        !history(&app).contains("Hello."),
        "the automatic translation was written to 履歴"
    );
    app.tab = Tab::Files;
    window.settle(&mut app);
    assert!(
        !has_text_containing(&window.output, "Hello."),
        "the 履歴 screen lists the automatic translation"
    );

    app.tab = Tab::Text;
    window.settle(&mut app);
    window.click(&mut app, "翻訳する");
    assert!(
        app.manual_busy.is_some(),
        "翻訳する does not translate at once"
    );
    assert_eq!(words(&translated(&mut app, &prompts)), "Hello.");
    assert_eq!(
        history(&app).matches("Hello.").count(),
        1,
        "the pressed translation is not in 履歴 once"
    );
    app.tab = Tab::Files;
    window.settle(&mut app);
    assert!(
        has_text_containing(&window.output, "Hello."),
        "the 履歴 screen does not list the pressed translation"
    );
}

#[test]
fn a_press_while_a_translation_is_in_flight_waits_for_it_and_writes_the_newest_input_once() {
    let folder = tempfile::tempdir().unwrap();
    let (endpoint, prompts) = translation_stub(vec![
        Completion::Text("一つ目。"),
        Completion::Text("二つ目。"),
        Completion::Text("三つ目。"),
    ]);
    let mut app = screen(endpoint);
    app.journal = Some(Journal::new(folder.path()).unwrap());
    let mut window = Window::new(860.0, 620.0);
    app.input = "One.".into();
    wait(&mut window, &mut app, 1.2);
    let first = app.manual_busy.expect("One. was not sent by itself");

    app.input = "Two.".into();
    window.settle(&mut app);
    window.click(&mut app, "翻訳する");
    assert_eq!(
        app.manual_busy,
        Some(first),
        "the press did not wait for the translation in flight"
    );
    assert_eq!(words(&translated(&mut app, &prompts)), "One.");
    window.draw(&mut app, vec![]);
    assert!(
        app.manual_busy.is_some_and(|id| id != first),
        "the press was not carried out once the translation in flight was done"
    );
    assert_eq!(words(&translated(&mut app, &prompts)), "Two.");
    assert_eq!(app.output, "二つ目。");
    assert_eq!(history(&app).matches("Two.").count(), 1);
    assert!(
        !history(&app).contains("One."),
        "the automatic translation was written to 履歴"
    );

    // Ctrl+Enter while the newest input itself is being translated: that
    // translation is the one written to 履歴, and nothing is sent twice.
    app.input = "Three.".into();
    wait(&mut window, &mut app, 1.2);
    let third = app.manual_busy.expect("Three. was not sent by itself");
    ctrl_enter(&mut app);
    assert_eq!(
        app.manual_busy,
        Some(third),
        "a second translation of the same input was sent"
    );
    assert_eq!(words(&translated(&mut app, &prompts)), "Three.");
    wait(&mut window, &mut app, 1.2);
    assert!(
        app.manual_busy.is_none() && prompts.try_recv().is_err(),
        "Three. was translated twice"
    );
    assert_eq!(
        history(&app).matches("Three.").count(),
        1,
        "Ctrl+Enter did not write the translation in flight to 履歴 once"
    );
}

#[test]
fn what_the_screen_already_shows_is_not_sent_again_whichever_way_it_was_translated() {
    let (endpoint, prompts) = translation_stub(vec![
        Completion::Text("押して訳した文です。"),
        Completion::Text("読み取って訳した文です。"),
        Completion::Text("書き足して訳した文です。"),
    ]);
    let mut app = screen(endpoint);
    let mut window = Window::new(860.0, 620.0);
    window.settle(&mut app);
    app.input = "Pressed.".into();
    window.click(&mut app, "翻訳する");
    assert_eq!(words(&translated(&mut app, &prompts)), "Pressed.");
    wait(&mut window, &mut app, 1.2);
    assert!(
        app.manual_busy.is_none(),
        "the pressed translation was sent again"
    );

    // Text read from a range or a picture.
    app.accept_ocr("Read from a picture.".into());
    app.process(&egui::Context::default());
    assert_eq!(
        words(&translated(&mut app, &prompts)),
        "Read from a picture."
    );
    assert_eq!(app.rows.last().unwrap().record.kind, "画面OCR");
    wait(&mut window, &mut app, 1.2);
    assert!(app.manual_busy.is_none(), "the reading was sent again");

    app.input.push_str(" Again.");
    wait(&mut window, &mut app, 1.2);
    assert!(
        app.manual_busy.is_some(),
        "the changed input was not sent by itself"
    );
    assert_eq!(
        words(&translated(&mut app, &prompts)),
        "Read from a picture. Again."
    );
    wait(&mut window, &mut app, 1.2);
    assert!(
        app.manual_busy.is_none() && prompts.try_recv().is_err(),
        "the automatic translation was sent again"
    );
}

#[test]
fn a_dropped_text_file_is_translated_like_typing_and_stays_out_of_the_history() {
    let folder = tempfile::tempdir().unwrap();
    let note = folder.path().join("note.txt");
    fs::write(&note, "A dropped note.").unwrap();
    let (endpoint, prompts) = translation_stub(vec![Completion::Text("落とした文です。")]);
    let mut app = chat();
    app.settings.translation.endpoint = endpoint;
    let input = egui::RawInput {
        dropped_files: vec![egui::DroppedFile {
            path: Some(note),
            ..Default::default()
        }],
        ..Default::default()
    };
    let _ = egui::Context::default().run(input, |ctx| app.open_dropped_file(ctx));
    assert!(app.tab == Tab::Text && app.input == "A dropped note.");
    let mut window = Window::new(860.0, 620.0);
    wait(&mut window, &mut app, 1.2);
    assert!(
        app.manual_busy.is_some(),
        "the dropped text was not translated by itself"
    );
    assert_eq!(words(&translated(&mut app, &prompts)), "A dropped note.");
    assert_eq!(app.output, "落とした文です。");
    assert!(app.rows.is_empty(), "the dropped text was written to 履歴");
}

#[test]
fn nothing_is_sent_while_the_model_is_not_ready_or_the_screen_is_out_of_view() {
    let (endpoint, prompts) = translation_stub(vec![
        Completion::Text("こんにちは。"),
        Completion::Text("Hello."),
    ]);
    let mut app = screen(endpoint);
    app.backend_ready = false;
    let mut window = Window::new(860.0, 620.0);
    app.input = "Hello.".into();
    wait(&mut window, &mut app, 1.5);
    assert!(
        app.manual_busy.is_none(),
        "the input was sent without the model"
    );
    app.backend_ready = true;
    window.draw(&mut app, vec![]);
    assert!(
        app.manual_busy.is_some(),
        "the newest input was not sent once the model was ready"
    );
    assert_eq!(words(&translated(&mut app, &prompts)), "Hello.");

    // The language is changed on the settings screen. シンプル翻訳 translates
    // again when it is shown, not behind the other screen.
    app.tab = Tab::Settings;
    app.settings.translation.target_lang = "en".into();
    wait(&mut window, &mut app, 1.5);
    assert!(
        app.manual_busy.is_none(),
        "the input was translated out of view"
    );
    app.tab = Tab::Text;
    window.draw(&mut app, vec![]);
    assert!(
        app.manual_busy.is_some(),
        "the screen did not translate again once it was shown"
    );
    assert_eq!(into(&translated(&mut app, &prompts)), "en");
}

#[test]
fn the_translation_stays_on_the_card_until_the_first_part_of_the_next_one_arrives() {
    let (endpoint, prompts) = translation_stub(vec![
        Completion::Text("こんにちは。"),
        Completion::Text("こんにちは。またね。"),
        Completion::Text("こんにちは。また明日。"),
    ]);
    let mut app = screen(endpoint);
    let mut window = Window::new(860.0, 620.0);
    app.input = "Hello.".into();
    wait(&mut window, &mut app, 1.2);
    translated(&mut app, &prompts);
    window.settle(&mut app);
    assert!(window.has("こんにちは。"));
    let done = (window.ink("コピー"), window.ink("保存"));

    app.input = "Hello. See you.".into();
    wait(&mut window, &mut app, 1.2);
    assert!(
        app.manual_busy.is_some(),
        "the changed input was not sent by itself"
    );
    assert!(
        window.has("こんにちは。"),
        "the card was emptied while the next translation was coming"
    );
    assert_ne!(
        (window.ink("コピー"), window.ink("保存")),
        done,
        "コピー and 保存 can be pressed while the translation is being replaced"
    );

    // The translator's answer is held back after its first part.
    prompts
        .recv_timeout(Duration::from_secs(10))
        .expect("the translator was not asked");
    let part = app.rx.recv_timeout(Duration::from_secs(10)).unwrap();
    let end = app.rx.recv_timeout(Duration::from_secs(10)).unwrap();
    assert!(matches!(part, Event::Partial { .. }) && matches!(end, Event::Done { .. }));
    app.tx.send(part).unwrap();
    app.process(&egui::Context::default());
    window.settle(&mut app);
    assert!(
        window.has("こんにちは。またね。") && !window.has("こんにちは。"),
        "the first part of the new translation did not replace the one before"
    );
    app.tx.send(end).unwrap();
    app.process(&egui::Context::default());
    window.settle(&mut app);
    assert_eq!((window.ink("コピー"), window.ink("保存")), done);

    window.click(&mut app, "翻訳する");
    assert!(
        app.manual_busy.is_some(),
        "翻訳する does not translate at once"
    );
    assert!(
        window.has("こんにちは。またね。"),
        "翻訳する emptied the card it was pressed on"
    );
    assert_eq!(words(&translated(&mut app, &prompts)), "Hello. See you.");
    window.settle(&mut app);
    assert!(window.has("こんにちは。また明日。"));
}

#[test]
fn a_reading_that_takes_the_lane_ends_a_waiting_press_and_is_written_to_the_history_once() {
    const READ: &str = "Read from a picture.";
    let folder = tempfile::tempdir().unwrap();
    let (endpoint, prompts) = translation_stub(vec![
        Completion::Text("一つ目。"),
        Completion::Text("読み取った文です。"),
        Completion::Text("もう一度読み取った文です。"),
    ]);
    let mut app = screen(endpoint);
    app.journal = Some(Journal::new(folder.path()).unwrap());
    let mut window = Window::new(860.0, 620.0);
    app.input = "One.".into();
    wait(&mut window, &mut app, 1.2);
    let first = app.manual_busy.expect("One. was not sent by itself");

    // 翻訳する on another input waits for One., and a picture is read
    // before One. is done.
    app.input = "Two.".into();
    window.settle(&mut app);
    window.click(&mut app, "翻訳する");
    assert_eq!(app.manual_busy, Some(first), "the press did not wait");
    app.accept_ocr(READ.into());
    assert_eq!(words(&translated(&mut app, &prompts)), "One.");
    assert_eq!(app.input, READ, "the reading did not take the lane");
    assert_eq!(words(&translated(&mut app, &prompts)), READ);
    wait(&mut window, &mut app, 1.2);
    app.tab = Tab::Files;
    window.settle(&mut app);
    let listed = texts(&window.output)
        .into_iter()
        .filter(|(text, _, _)| text == READ)
        .count();
    assert_eq!(listed, 1, "履歴 does not list the reading once");
    assert!(
        app.manual_busy.is_none() && prompts.try_recv().is_err(),
        "the waiting press sent the reading's text again"
    );
    assert_eq!(history(&app).matches(READ).count(), 1);

    // The press is over: the next change is translated by itself.
    app.tab = Tab::Text;
    app.input.push_str(" Again.");
    wait(&mut window, &mut app, 1.2);
    assert_eq!(
        words(&translated(&mut app, &prompts)),
        "Read from a picture. Again."
    );
    assert!(
        !history(&app).contains("Again."),
        "the press that waited wrote a later input to 履歴"
    );
}

#[test]
fn emptying_the_input_ends_its_failure_and_the_mini_view_opens_on_the_chat() {
    let (endpoint, prompts) = translation_stub(vec![Completion::Failure]);
    let mut app = screen(endpoint);
    let mut window = Window::new(860.0, 620.0);
    app.input = "Hello.".into();
    wait(&mut window, &mut app, 1.2);
    translated(&mut app, &prompts);
    let failure = app.status.clone();
    assert!(!failure.is_empty(), "the request did not fail");
    window.settle(&mut app);
    window.click(&mut app, "クリア");
    assert!(app.input.is_empty());

    window.draw(
        &mut app,
        vec![key(
            egui::Key::M,
            egui::Modifiers::CTRL | egui::Modifiers::SHIFT,
        )],
    );
    window.resize(520.0, 360.0);
    window.settle(&mut app);
    assert!(app.window_mode.is_mini());
    assert!(
        !window.has(BACK_TO_CHAT) && !has_text_containing(&window.output, &failure),
        "the mini view opens on the failure of an input that was emptied"
    );
    assert!(
        has_text_containing(&window.output, "上の「● 聞く」を押すと始まります。"),
        "the mini view does not open on the chat"
    );
}

#[test]
fn the_mini_view_opens_on_the_chat_after_the_input_was_emptied_while_it_was_being_translated() {
    let (endpoint, prompts) = translation_stub(vec![Completion::Text("こんにちは。")]);
    let mut app = screen(endpoint);
    let mut window = Window::new(860.0, 620.0);
    app.input = "Hello.".into();
    wait(&mut window, &mut app, 1.2);
    assert!(
        app.manual_busy.is_some(),
        "the input was not sent by itself"
    );
    // The answer is not taken in: the translation is still in flight.
    prompts
        .recv_timeout(Duration::from_secs(10))
        .expect("the translator was not asked");
    window.click(&mut app, "クリア");
    assert!(app.input.is_empty() && app.manual_busy.is_some());

    window.draw(
        &mut app,
        vec![key(
            egui::Key::M,
            egui::Modifiers::CTRL | egui::Modifiers::SHIFT,
        )],
    );
    window.resize(520.0, 360.0);
    window.settle(&mut app);
    assert!(app.window_mode.is_mini());
    assert!(
        !window.has(BACK_TO_CHAT) && !window.has("翻訳中"),
        "the mini view opens on a translation that will not be shown"
    );
    assert!(
        has_text_containing(&window.output, "上の「● 聞く」を押すと始まります。"),
        "the mini view does not open on the chat"
    );
}

#[test]
fn a_reading_does_not_show_the_translation_of_the_text_before_it_while_its_own_is_coming() {
    let (endpoint, prompts) = translation_stub(vec![
        Completion::Text("こんにちは。"),
        Completion::Text("読み取った文です。"),
    ]);
    let mut app = screen(endpoint);
    let mut window = Window::new(860.0, 620.0);
    app.input = "Hello.".into();
    wait(&mut window, &mut app, 1.2);
    translated(&mut app, &prompts);
    window.settle(&mut app);
    assert!(window.has("こんにちは。"));

    app.accept_ocr("Read from a picture.".into());
    app.process(&egui::Context::default());
    assert!(app.manual_busy.is_some(), "the reading was not sent");
    window.settle(&mut app);
    assert!(
        !window.has("こんにちは。"),
        "the translation of another text is shown beside the reading"
    );
    assert_eq!(
        words(&translated(&mut app, &prompts)),
        "Read from a picture."
    );
    window.settle(&mut app);
    assert!(window.has("読み取った文です。"));
}

#[test]
fn a_failed_request_leaves_the_card_empty_and_is_sent_again_only_once_the_input_changes() {
    let (endpoint, prompts) = translation_stub(vec![
        Completion::Text("こんにちは。"),
        Completion::Failure,
        Completion::Text("また会いましょう。"),
    ]);
    let mut app = screen(endpoint);
    let mut window = Window::new(860.0, 620.0);
    app.input = "Hello.".into();
    wait(&mut window, &mut app, 1.2);
    translated(&mut app, &prompts);
    window.settle(&mut app);
    assert!(window.has("こんにちは。"));

    app.input = "See you.".into();
    wait(&mut window, &mut app, 1.2);
    assert_eq!(words(&translated(&mut app, &prompts)), "See you.");
    window.settle(&mut app);
    assert!(
        window.has(TRANSLATION_HINT) && !window.has("こんにちは。"),
        "the card is not empty after the request failed"
    );
    wait(&mut window, &mut app, 5.0);
    assert!(
        app.manual_busy.is_none() && prompts.try_recv().is_err(),
        "the failed request was sent again by itself"
    );

    app.input = "See you later.".into();
    wait(&mut window, &mut app, 1.2);
    assert_eq!(words(&translated(&mut app, &prompts)), "See you later.");
    window.settle(&mut app);
    assert!(window.has("また会いましょう。"));
}

#[test]
fn a_press_whose_input_is_emptied_before_its_result_is_not_shown_but_is_written_to_the_history() {
    let folder = tempfile::tempdir().unwrap();
    let (endpoint, prompts) = translation_stub(vec![
        Completion::Text("こんにちは。"),
        Completion::Text("また会いましょう。"),
    ]);
    let mut app = screen(endpoint);
    app.journal = Some(Journal::new(folder.path()).unwrap());
    let mut window = Window::new(860.0, 620.0);
    app.input = "Hello.".into();
    wait(&mut window, &mut app, 1.2);
    translated(&mut app, &prompts);
    window.settle(&mut app);
    assert!(window.has("こんにちは。"));

    app.input = "See you.".into();
    window.settle(&mut app);
    window.click(&mut app, "翻訳する");
    assert!(
        app.manual_busy.is_some(),
        "翻訳する does not translate at once"
    );
    window.click(&mut app, "クリア");
    assert!(
        window.has(TRANSLATION_HINT) && !window.has("こんにちは。"),
        "the card is not empty once the input is"
    );
    assert_eq!(words(&translated(&mut app, &prompts)), "See you.");
    window.settle(&mut app);
    assert!(
        window.has(TRANSLATION_HINT) && !window.has("また会いましょう。"),
        "the result of the emptied input is shown"
    );
    assert_eq!(
        history(&app).matches("See you.").count(),
        1,
        "the press is not in 履歴 once"
    );
}

#[test]
fn a_waiting_press_is_sent_while_another_screen_is_shown() {
    let folder = tempfile::tempdir().unwrap();
    let (endpoint, prompts) = translation_stub(vec![
        Completion::Text("一つ目。"),
        Completion::Text("二つ目。"),
    ]);
    let mut app = screen(endpoint);
    app.journal = Some(Journal::new(folder.path()).unwrap());
    let mut window = Window::new(860.0, 620.0);
    app.input = "One.".into();
    wait(&mut window, &mut app, 1.2);
    let first = app.manual_busy.expect("One. was not sent by itself");
    app.input = "Two.".into();
    window.settle(&mut app);
    window.click(&mut app, "翻訳する");
    window.click(&mut app, BACK_TO_CHAT);
    assert!(app.tab == Tab::Live);

    assert_eq!(words(&translated(&mut app, &prompts)), "One.");
    window.draw(&mut app, vec![]);
    assert!(
        app.manual_busy.is_some_and(|id| id != first),
        "the waiting press was not sent while the chat was shown"
    );
    assert_eq!(words(&translated(&mut app, &prompts)), "Two.");
    assert_eq!(history(&app).matches("Two.").count(), 1);
}

#[test]
fn emptying_the_input_drops_a_waiting_press() {
    let folder = tempfile::tempdir().unwrap();
    let (endpoint, prompts) = translation_stub(vec![
        Completion::Text("一つ目。"),
        Completion::Text("三つ目。"),
    ]);
    let mut app = screen(endpoint);
    app.journal = Some(Journal::new(folder.path()).unwrap());
    let mut window = Window::new(860.0, 620.0);
    app.input = "One.".into();
    wait(&mut window, &mut app, 1.2);
    assert!(app.manual_busy.is_some(), "One. was not sent by itself");
    app.input = "Two.".into();
    window.settle(&mut app);
    window.click(&mut app, "翻訳する");
    window.click(&mut app, "クリア");

    // Typed again before One. is done: that input waits for the pause.
    app.input = "Three.".into();
    window.draw(&mut app, vec![]);
    translated(&mut app, &prompts);
    window.draw(&mut app, vec![]);
    assert!(
        app.manual_busy.is_none(),
        "the press was carried out after the input was emptied"
    );
    wait(&mut window, &mut app, 1.2);
    assert_eq!(words(&translated(&mut app, &prompts)), "Three.");
    assert!(
        !history(&app).contains("Two.") && !history(&app).contains("Three."),
        "a press that was dropped wrote to 履歴"
    );
}

#[test]
fn nothing_is_sent_by_itself_while_the_mini_view_is_shown() {
    let (endpoint, prompts) = translation_stub(vec![Completion::Text("こんにちは。")]);
    let mut app = screen(endpoint);
    let mut window = Window::new(860.0, 620.0);
    window.settle(&mut app);
    app.input = "Hello.".into();
    window.draw(
        &mut app,
        vec![key(
            egui::Key::M,
            egui::Modifiers::CTRL | egui::Modifiers::SHIFT,
        )],
    );
    window.resize(520.0, 360.0);
    wait(&mut window, &mut app, 1.5);
    assert!(app.window_mode.is_mini());
    assert!(
        app.manual_busy.is_none() && prompts.try_recv().is_err(),
        "the input was sent while the mini view was shown"
    );

    window.draw(
        &mut app,
        vec![key(egui::Key::Escape, egui::Modifiers::NONE)],
    );
    window.resize(860.0, 620.0);
    window.draw(&mut app, vec![]);
    assert!(!app.window_mode.is_mini() && app.tab == Tab::Text);
    assert!(
        app.manual_busy.is_some(),
        "the input was not sent once the screen was shown again"
    );
    assert_eq!(words(&translated(&mut app, &prompts)), "Hello.");
}

#[test]
fn an_automatic_translation_is_not_read_aloud_and_a_pressed_one_is() {
    const NO_VOICE: &str = "選択した固定声がありません";
    let (endpoint, prompts) = translation_stub(vec![
        Completion::Text("こんにちは。"),
        Completion::Text("こんにちは。"),
    ]);
    let mut app = screen(endpoint);
    app.settings.readout_enabled = true;
    app.settings.auto_read = true;
    // With a voice that is not there, an attempt to read is an error message
    // instead of a voice, so the attempt itself can be seen.
    app.readout.ready = Some(plamo_translate::speech_setup::SpeechInstallation {
        executable: Default::default(),
        model: Default::default(),
        backend: String::new(),
        voice: String::new(),
    });
    app.settings.tts_voice_profile = "missing".into();
    let mut window = Window::new(860.0, 620.0);
    app.input = "Hello.".into();
    wait(&mut window, &mut app, 1.2);
    translated(&mut app, &prompts);
    window.settle(&mut app);
    assert!(window.has("こんにちは。"));
    assert!(
        !has_text_containing(&window.output, NO_VOICE),
        "the automatic translation was read aloud"
    );

    window.click(&mut app, "翻訳する");
    translated(&mut app, &prompts);
    window.settle(&mut app);
    assert!(
        has_text_containing(&window.output, NO_VOICE),
        "the pressed translation was not read aloud"
    );
}
