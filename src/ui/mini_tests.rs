//! The mini view is the chat in a small window, with the main window's words
//! and its one listen button. A reading or a text translation asked for there
//! shows its result on a card in the chat's place.
use super::home_tests::Window;
use super::listen_tests::{INPUTS, armed, chat_without_speech};
use super::*;
use crate::chat::testing::{
    box_of, cut_at_the_top, dots, filled, find_text, has_text_containing, press, shapes, texts,
    wheel,
};
use plamo_translate::journal::{ParagraphMeta, ParagraphState};

/// The smallest size of the mini view, and the size it opens at.
const SMALLEST: (f32, f32) = (400.0, 260.0);
const USUAL: (f32, f32) = (520.0, 360.0);
const SIZES: [(f32, f32); 2] = [SMALLEST, USUAL];
/// The main window at its smallest.
const MAIN: (f32, f32) = (860.0, 620.0);

const NO_SCREEN: &str = "取得できる画面がありません。";
const READING: &str = "画像から文字を読み取っています…";
const EMPTY_CARD: &str = "訳文はここに表示されます";

/// Words of the mini view before it became the chat. None may be drawn.
const REMOVED: [&str; 23] = [
    "いま ·",
    "最後の確定訳",
    "停止前の確定訳",
    "継続中の文章",
    "確定訳の原文を併記",
    "手前に固定",
    "範囲を翻訳",
    "ライブ新着",
    "ライブへ戻る",
    "入力を選ぶ",
    "一時停止",
    "再開",
    "入力中",
    "入力なし",
    "翻訳停止",
    "サービス停止",
    "翻訳失敗",
    "保存失敗",
    "読み上げ失敗",
    "手動翻訳失敗",
    "通常表示で確認",
    "お知らせ",
    "手動 ·",
];

/// The mini view over the chat of `chat_without_speech`: the model is ready,
/// nothing listens, and nothing can be installed or captured.
fn mini() -> App {
    let mut app = chat_without_speech();
    app.window_mode.enter(&egui::ViewportInfo::default());
    app
}

fn drawn(app: &mut App, (width, height): (f32, f32)) -> Window {
    let mut window = Window::new(width, height);
    window.settle(app);
    window
}

/// A fixed message of the talk.
fn row(id: u64, kind: &str, translation: &str) -> Row {
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

/// The same message while it is still being said.
fn forming(mut row: Row) -> Row {
    row.record.paragraph = Some(ParagraphMeta {
        revision: 1,
        state: ParagraphState::Open,
        close_reason: None,
    });
    row
}

/// A talk of `count` messages of one line each.
fn talk(count: u64) -> Vec<Row> {
    (1..=count)
        .map(|id| row(id, "音声", &format!("{id}番目の訳文です。")))
        .collect()
}

/// The chat is what is shown, with the newest of `count` messages in view.
fn shows_talk(window: &Window, count: u64) -> bool {
    !bubbles(window).is_empty() && window.has(&format!("{count}番目の訳文です。"))
}

/// The message `id` of `talk` is drawn inside the chat, where it can be read.
fn in_view(window: &Window, app: &App, id: u64) -> bool {
    let body = app.live_chat.body().expect("the chat is drawn");
    let words = format!("{id}番目の訳文です。");
    texts(&window.output)
        .iter()
        .any(|(text, rect, _)| *text == words && body.contains_rect(*rect))
}

fn key(key: egui::Key, modifiers: egui::Modifiers) -> egui::Event {
    egui::Event::Key {
        key,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers,
    }
}

fn escape() -> egui::Event {
    key(egui::Key::Escape, egui::Modifiers::NONE)
}

/// Ctrl+Shift+M, which enters the mini view and leaves it.
fn mini_key() -> egui::Event {
    key(egui::Key::M, egui::Modifiers::CTRL | egui::Modifiers::SHIFT)
}

/// A click on the one text drawn as `label`. The frame of the release stays
/// in `window.output`, with what the click sent out of the app.
fn release_on(window: &mut Window, app: &mut App, label: &str) {
    let at = window.at(label).center();
    window.draw(app, vec![egui::Event::PointerMoved(at)]);
    window.draw(app, vec![press(at, egui::PointerButton::Primary, true)]);
    window.draw(app, vec![press(at, egui::PointerButton::Primary, false)]);
}

/// Opens the 「⋯」 menu and waits until it has faded in, so that its colours
/// are the ones it keeps.
fn open_menu(window: &mut Window, app: &mut App) {
    window.click(app, "⋯");
    window.settle(app);
}

/// Rests the pointer on `label` after it was used elsewhere. The hover text
/// comes up a tenth of a second after the last click, once the pointer has
/// been still for as long.
fn hover_later(window: &mut Window, app: &mut App, label: &str) {
    window.settle(app);
    window.settle(app);
    window.hover(app, label);
    window.settle(app);
}

/// A turn of the wheel over `at`, then frames until the scrolling has stopped.
fn scroll(window: &mut Window, app: &mut App, at: egui::Pos2, delta: f32) {
    window.draw(app, vec![egui::Event::PointerMoved(at)]);
    window.draw(app, vec![wheel(delta)]);
    for _ in 0..8 {
        window.settle(app);
    }
}

/// What the last frame asked the system to copy.
fn copied(window: &Window) -> Vec<&str> {
    window
        .output
        .platform_output
        .commands
        .iter()
        .filter_map(|command| match command {
            egui::OutputCommand::CopyText(text) => Some(text.as_str()),
            _ => None,
        })
        .collect()
}

/// The words in each bubble and their colour, from the top of the chat down.
fn bubbles(window: &Window) -> Vec<(String, Color32)> {
    let mut rects = filled(&window.output, theme::CHAT_BUBBLE);
    rects.sort_by(|a, b| a.top().total_cmp(&b.top()));
    rects
        .iter()
        .map(|bubble| {
            texts(&window.output)
                .into_iter()
                .find(|(_, words, _)| bubble.contains_rect(*words))
                .map(|(text, _, color)| (text, color))
                .expect("a bubble has words")
        })
        .collect()
}

/// The status line: the one text on the row of the credit and to its left,
/// part by part with the colour of each. The second value is true when the
/// line was too long for its room and was cut short.
fn status_line(window: &Window) -> (Vec<(String, Color32)>, bool) {
    let credit = window.at(credit::LABEL);
    let lines: Vec<_> = shapes(&window.output)
        .into_iter()
        .filter_map(|(_, shape)| match shape {
            egui::Shape::Text(text) if text.galley.text() != credit::LABEL => {
                let rect = egui::Rect::from_min_size(
                    text.pos + text.galley.rect.min.to_vec2(),
                    text.galley.size(),
                );
                ((rect.center().y - credit.center().y).abs() < 6.0 && rect.right() < credit.left())
                    .then(|| text.galley.clone())
            }
            _ => None,
        })
        .collect();
    assert_eq!(lines.len(), 1, "one status line beside the credit");
    let job = &lines[0].job;
    let parts = job
        .sections
        .iter()
        .map(|section| {
            (
                job.text[section.byte_range.clone()].to_owned(),
                section.format.color,
            )
        })
        .filter(|(text, _)| text.trim() != "·")
        .collect();
    (parts, lines[0].elided)
}

/// The words in the middle of the chat while it has no message.
fn hint(app: &mut App, size: (f32, f32)) -> String {
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

/// A text translation in flight, as `translate` leaves it once its worker has
/// started. The test answers for the worker with `answer`.
fn in_flight(app: &mut App, id: u64) {
    app.manual_busy = Some(id);
    app.output.clear();
    let mut job = row(id, "文章", "");
    job.pending = true;
    app.rows.push(job);
}

fn answer(app: &mut App, event: Event) {
    app.tx.send(event).unwrap();
    app.process(&egui::Context::default());
}

fn done(id: u64, result: std::result::Result<&str, &str>) -> Event {
    Event::Done {
        id,
        lane: Lane::Manual,
        result: result.map(Into::into).map_err(Into::into),
        elapsed: 0.1,
        generation: 0,
    }
}

#[test]
fn the_mini_view_draws_the_talk_as_the_chat_does_with_the_oldest_message_at_the_top() {
    let said = || {
        let mut app = chat_without_speech();
        app.rows = vec![
            row(1, "音声", "最初の訳文です。"),
            row(2, "音声", "次の訳文です。"),
            forming(row(3, "音声", "いま話している途中")),
        ];
        app.current_live_id = Some(3);
        // Speech recognised since then belongs to the message still forming.
        app.asr_partial = "NEW-UNCONFIRMED-DRAFT".into();
        app
    };
    let mut large = said();
    let main = drawn(&mut large, MAIN);
    let mut small = said();
    small.window_mode.enter(&egui::ViewportInfo::default());
    let window = drawn(&mut small, USUAL);

    assert_eq!(
        bubbles(&window),
        [
            ("最初の訳文です。".to_owned(), theme::CHAT_TEXT),
            ("次の訳文です。".to_owned(), theme::CHAT_TEXT),
            ("いま話している途中".to_owned(), theme::CHAT_TEXT_FORMING),
        ],
        "the oldest message is at the top and the one still forming is paler"
    );
    assert_eq!(bubbles(&window), bubbles(&main));
    assert_eq!(
        dots(&window.output).len(),
        3,
        "the forming message has its dots"
    );
    assert_eq!(dots(&main.output).len(), 3);
    assert!(window.has("PC音声"), "the speaker's name is above the talk");
    for hidden in ["Original", "NEW-UNCONFIRMED-DRAFT"] {
        assert!(
            !has_text_containing(&window.output, hidden),
            "{hidden} is drawn, and the chat does not draw it"
        );
    }
}

#[test]
fn the_button_listens_in_the_chosen_input_and_stops_every_input_as_in_the_main_window() {
    // What the app holds after 聞く, then after 止める.
    let pressed = |mode: LiveSourceMode, small: bool| {
        let mut app = chat_without_speech();
        app.listen_input = mode;
        let size = if small {
            app.window_mode.enter(&egui::ViewportInfo::default());
            USUAL
        } else {
            MAIN
        };
        let mut window = drawn(&mut app, size);
        window.click(&mut app, "● 聞く");
        let heard = (armed(&app), app.audio_confirmation, app.status.clone());
        let stops = window.has("■ 止める");
        if stops {
            window.click(&mut app, "■ 止める");
        }
        (heard, stops, armed(&app), window.has("● 聞く"))
    };
    for (label, mode) in INPUTS {
        let small = pressed(mode, true);
        assert_eq!(
            small,
            pressed(mode, false),
            "{label}: the button of the mini view and the button of the main window differ"
        );
        let ((heard, asks, status), stops, after, listens_again) = small;
        let expected = match mode {
            LiveSourceMode::Auto => (true, true),
            LiveSourceMode::Captions => (true, false),
            LiveSourceMode::Audio => (false, true),
            // This PC has no screen to read, and says so.
            LiveSourceMode::Screen => (false, false),
        };
        assert_eq!(heard, expected, "{label}: 聞く");
        assert_eq!(
            asks, expected.1,
            "{label}: speech is not prepared, so it asks"
        );
        assert_eq!(stops, mode != LiveSourceMode::Screen, "{label}");
        if mode == LiveSourceMode::Screen {
            assert_eq!(status, NO_SCREEN);
        }
        assert_eq!(after, (false, false), "{label}: 止める");
        assert!(listens_again, "{label}: the button listens again");
    }

    // PC audio that is really on, and the screen feed.
    let mut app = mini();
    app.audio_enabled = true;
    app.asr_ready = true;
    let mut window = drawn(&mut app, USUAL);
    window.click(&mut app, "■ 止める");
    assert!(!app.audio_enabled && !app.caption_enabled);
    assert!(window.has("● 聞く"));

    let mut app = mini();
    app.start_screen_feed(1, (0, 0, 120, 40));
    assert!(app.screen_feed.is_some());
    let mut window = drawn(&mut app, USUAL);
    window.click(&mut app, "■ 止める");
    assert!(app.screen_feed.is_none() && !app.caption_enabled);
}

#[test]
fn without_a_translation_model_the_button_cannot_start_but_it_can_always_stop() {
    let ink = |ready: bool, listening: bool| {
        let mut app = mini();
        app.backend_ready = ready;
        app.caption_enabled = listening;
        let label = if listening {
            "■ 止める"
        } else {
            "● 聞く"
        };
        drawn(&mut app, USUAL).ink(label)
    };
    assert_ne!(ink(true, false), ink(false, false), "聞く is not faded");
    assert_eq!(ink(true, true), ink(false, true), "止める is faded");

    let mut app = mini();
    app.backend_ready = false;
    let mut window = drawn(&mut app, USUAL);
    window.click(&mut app, "● 聞く");
    assert_eq!(armed(&app), (false, false));
    assert!(!app.audio_confirmation);

    // A model that went away while listening.
    let mut app = mini();
    app.caption_enabled = true;
    app.backend_ready = false;
    let mut window = drawn(&mut app, USUAL);
    window.click(&mut app, "■ 止める");
    assert!(!app.caption_enabled);
}

#[test]
fn the_hover_text_of_the_button_is_the_main_window_s_with_its_shortcut() {
    let hover = |small: bool, listening: bool| {
        let mut app = chat_without_speech();
        app.caption_enabled = listening;
        let size = if small {
            app.window_mode.enter(&egui::ViewportInfo::default());
            USUAL
        } else {
            MAIN
        };
        let label = if listening {
            "■ 止める"
        } else {
            "● 聞く"
        };
        let mut window = drawn(&mut app, size);
        window.hover(&mut app, label);
        texts(&window.output)
            .into_iter()
            .map(|(text, _, _)| text)
            .find(|text| text.contains("Ctrl + Alt + L"))
            .expect("the hover text names the shortcut")
    };
    for listening in [false, true] {
        assert_eq!(hover(true, listening), hover(false, listening));
    }
    assert_ne!(hover(true, false), hover(true, true));
}

#[test]
fn the_status_line_says_what_the_line_beside_the_main_button_says_in_its_colour() {
    type Change = fn(&mut App);
    let states: [(&str, Change, Color32); 9] = [
        ("止まっています", |_| {}, theme::TEXT_SECONDARY),
        (
            "YouTube字幕を受け取っています",
            |app| {
                app.caption_enabled = true;
                app.caption_paused = false;
                app.last_caption = Some(Instant::now());
            },
            theme::ACCENT,
        ),
        (
            "PC音声を聞いています",
            |app| {
                app.audio_enabled = true;
                app.asr_ready = true;
            },
            theme::ACCENT,
        ),
        (
            "YouTube字幕を待っています",
            |app| app.caption_enabled = true,
            theme::TEXT_SECONDARY,
        ),
        (
            "音声認識を準備しています",
            |app| app.audio_start_requested = true,
            theme::TEXT_SECONDARY,
        ),
        (
            "画面の文字を読み取っています",
            |app| app.start_screen_feed(1, (0, 0, 120, 40)),
            theme::ACCENT,
        ),
        (
            "入力を待っています",
            |app| app.audio_enabled = true,
            theme::TEXT_SECONDARY,
        ),
        (
            "翻訳モデルを準備しています",
            |app| {
                app.backend_ready = false;
                app.backend_loading = true;
            },
            theme::WARNING,
        ),
        (
            "翻訳モデルの準備が必要です",
            |app| app.backend_ready = false,
            theme::TEXT,
        ),
    ];
    for (words, change, color) in states {
        for size in SIZES {
            let mut app = mini();
            change(&mut app);
            let window = drawn(&mut app, size);
            assert_eq!(
                status_line(&window),
                (vec![(words.to_owned(), color)], false),
                "at {size:?}"
            );

            // The main window says the same beside its button.
            let mut app = chat_without_speech();
            change(&mut app);
            let main = drawn(&mut app, MAIN);
            assert_eq!(find_text(&main.output, words).1, color, "{words}");
        }
    }
}

#[test]
fn a_failure_to_save_a_service_error_and_a_failed_readout_are_on_the_status_line_in_red() {
    const UNSAVED: &str = "保存できていません（Could not append translation history）";
    const SERVICE: &str = "音声認識を読み込めませんでした。";
    const READOUT: &str = "読み上げは日本語の訳文に対応しています。";
    let stopped = ("止まっています".to_owned(), theme::TEXT_SECONDARY);

    let mut app = mini();
    app.journal_error = "Could not append translation history".into();
    let window = drawn(&mut app, USUAL);
    assert_eq!(
        status_line(&window).0,
        [(UNSAVED.to_owned(), theme::ERROR), stopped.clone()],
        "a failure to save comes first"
    );
    // The live page of the main window says it in the same words.
    let mut main = chat_without_speech();
    main.journal_error = "Could not append translation history".into();
    assert_eq!(
        find_text(&drawn(&mut main, MAIN).output, UNSAVED).1,
        theme::ERROR
    );

    let mut app = mini();
    app.service_error = SERVICE.into();
    app.caption_enabled = true;
    let window = drawn(&mut app, USUAL);
    assert_eq!(
        status_line(&window).0,
        [(SERVICE.to_owned(), theme::ERROR)],
        "the error takes the place of what the inputs are doing"
    );

    let mut app = mini();
    app.readout.error = READOUT.into();
    let window = drawn(&mut app, USUAL);
    assert_eq!(
        status_line(&window).0,
        [stopped, (READOUT.to_owned(), theme::ERROR)],
        "a failure to read aloud comes last"
    );

    let mut app = mini();
    app.journal_error = "Could not append translation history".into();
    app.service_error = SERVICE.into();
    app.readout.error = READOUT.into();
    let mut window = drawn(&mut app, SMALLEST);
    let (parts, cut_short) = status_line(&window);
    assert_eq!(
        parts,
        [
            (UNSAVED.to_owned(), theme::ERROR),
            (SERVICE.to_owned(), theme::ERROR),
            (READOUT.to_owned(), theme::ERROR),
        ]
    );
    assert!(cut_short, "three errors do not fit the smallest window");
    let whole = format!("{UNSAVED}  ·  {SERVICE}  ·  {READOUT}");
    let drawn_whole = |window: &Window| {
        texts(&window.output)
            .iter()
            .filter(|(text, _, _)| *text == whole)
            .count()
    };
    assert_eq!(drawn_whole(&window), 1, "the line itself");
    window.hover(&mut app, &whole);
    assert!(
        drawn_whole(&window) > 1,
        "the hover text does not have the line in full"
    );
}

#[test]
fn nothing_else_is_said_on_the_status_line() {
    let mut app = mini();
    app.caption_enabled = true;
    app.status = "翻訳完了 · 1.2秒".into();
    app.readout.status = "読み上げ中です。".into();
    app.live_busy = Some(3);
    app.rows = talk(3);
    app.rows[2].pending = true;
    let window = drawn(&mut app, USUAL);
    assert_eq!(
        status_line(&window).0,
        [(
            "YouTube字幕を待っています".to_owned(),
            theme::TEXT_SECONDARY
        )]
    );
    for gone in [
        "翻訳完了 · 1.2秒",
        "読み上げ中です。",
        "翻訳中",
        "履歴に自動保存",
    ] {
        assert!(
            !has_text_containing(&window.output, gone),
            "{gone} is drawn in the mini view"
        );
    }
}

#[test]
fn the_empty_chat_of_the_mini_view_points_at_the_button_above_it() {
    for size in SIZES {
        let mut app = mini();
        let stopped = hint(&mut app, size);
        assert!(stopped.starts_with(chat::EMPTY_HINT), "{stopped}");
        assert!(
            stopped.ends_with("\n上の「● 聞く」を押すと始まります。"),
            "{stopped}"
        );

        let mut app = mini();
        app.backend_ready = false;
        app.backend_loading = true;
        assert_eq!(
            hint(&mut app, size),
            "翻訳モデルを準備しています。\n終わると、上の「● 聞く」を押せます。"
        );

        let mut app = mini();
        app.backend_ready = false;
        assert_eq!(
            hint(&mut app, size),
            "翻訳モデルの準備が、まだできていません。\n「通常表示」に戻ると、準備の画面を開けます。"
        );
        assert!(
            drawn(&mut app, size).has("通常表示"),
            "the button the words name is on the screen"
        );

        // What to do while an input listens does not depend on the window.
        let mut app = mini();
        app.caption_enabled = true;
        assert_eq!(
            hint(&mut app, size),
            "YouTubeで動画を開き、字幕（CC）をオンにして再生してください。\n字幕の訳が、ここに出ます。"
        );
    }
}

#[test]
fn a_picture_given_in_the_mini_view_opens_the_card_and_hides_the_result_before_it() {
    for size in SIZES {
        let mut app = mini();
        app.rows = talk(3);
        app.input = "The text typed before.".into();
        app.output = "前に訳した文章です。".into();
        let mut window = drawn(&mut app, size);
        assert!(shows_talk(&window, 3), "the chat is shown at first");

        // The recogniser is not prepared, so the picture waits and nothing
        // runs in this test.
        app.settings.ocr_backend = "indra".into();
        app.do_ocr(image::RgbaImage::new(2, 2));
        window.settle(&mut app);
        for shown in [READING, "← チャットに戻る", "訳文", "コピー"] {
            assert!(window.has(shown), "{shown} is not on the card at {size:?}");
        }
        for old in ["前に訳した文章です。", "The text typed before."] {
            assert!(
                !has_text_containing(&window.output, old),
                "{old} is shown as if it belonged to the new picture"
            );
        }
        assert!(bubbles(&window).is_empty(), "the chat is under the card");
        for kept in ["通常表示", "● 聞く", "止まっています", credit::LABEL] {
            assert!(window.has(kept), "{kept} left with the chat");
        }

        // Nothing is on the card to copy while the picture is read.
        release_on(&mut window, &mut app, "コピー");
        assert!(copied(&window).is_empty());

        // The picture is dropped, as it is when another recogniser is chosen
        // in the settings. The result before it is the last one again.
        app.reset_screen_ocr();
        window.settle(&mut app);
        assert!(window.has("前に訳した文章です。") && !window.has(READING));
    }
}

#[test]
fn text_that_was_read_waits_for_the_translator_without_showing_the_translation_in_its_way() {
    let mut app = chat_without_speech();
    app.tab = Tab::Text;
    in_flight(&mut app, 7);
    let mut window = Window::new(USUAL.0, USUAL.1);
    window.draw(&mut app, vec![mini_key()]);
    answer(
        &mut app,
        Event::Partial {
            id: 7,
            text: "先に頼んだ文章の途中の訳".into(),
        },
    );
    window.settle(&mut app);
    assert!(window.has("先に頼んだ文章の途中の訳"));

    let generation = app.ocr.generation;
    answer(
        &mut app,
        Event::Ocr {
            generation,
            result: Ok("Text read from a picture.".into()),
        },
    );
    window.settle(&mut app);
    assert!(app.manual_busy == Some(7), "the translator is still busy");
    assert!(window.has(READING));
    assert!(!has_text_containing(&window.output, "先に頼んだ文章"));
}

#[test]
fn a_translation_fills_the_card_as_it_comes_and_is_copied_with_the_credit_once_done() {
    for size in SIZES {
        let mut app = chat_without_speech();
        app.tab = Tab::Text;
        in_flight(&mut app, 42);
        let mut window = Window::new(size.0, size.1);
        window.draw(&mut app, vec![mini_key()]);
        window.settle(&mut app);
        assert!(app.window_mode.is_mini());
        assert!(window.has("翻訳中"), "nothing has come yet at {size:?}");
        let waiting = window.ink("コピー");

        answer(
            &mut app,
            Event::Partial {
                id: 42,
                text: "途中までの訳".into(),
            },
        );
        window.settle(&mut app);
        assert!(window.has("途中までの訳") && !window.has("翻訳中"));
        release_on(&mut window, &mut app, "コピー");
        assert!(copied(&window).is_empty(), "half a translation was copied");

        answer(&mut app, done(42, Ok("完成した訳文です。")));
        window.settle(&mut app);
        assert!(window.has("完成した訳文です。") && !window.has("途中までの訳"));
        assert_ne!(window.ink("コピー"), waiting, "コピー is still faded");
        hover_later(&mut window, &mut app, "コピー");
        assert!(has_text_containing(
            &window.output,
            "PLaMoのクレジットを付けてコピーします"
        ));
        release_on(&mut window, &mut app, "コピー");
        assert_eq!(
            copied(&window),
            [credit::published("完成した訳文です。")],
            "at {size:?}"
        );
    }
}

#[test]
fn a_failure_is_on_the_card_in_the_words_the_main_window_was_given() {
    type Fail = fn(&mut App);
    let failures: [(&str, Fail); 5] = [
        (NO_SCREEN, |app| app.capture(&egui::Context::default())),
        ("画面を取得できませんでした。", |app| {
            answer(
                app,
                Event::Screenshot(Err("画面を取得できませんでした。".into())),
            )
        }),
        (
            "文字が見つかりませんでした。範囲や原文言語を確認してください。",
            |app| {
                let generation = app.ocr.generation;
                answer(
                    app,
                    Event::Ocr {
                        generation,
                        result: Ok("  ".into()),
                    },
                )
            },
        ),
        ("文字認識を始められませんでした。", |app| {
            let generation = app.ocr.generation;
            answer(
                app,
                Event::Ocr {
                    generation,
                    result: Err("文字認識を始められませんでした。".into()),
                },
            )
        }),
        ("翻訳サーバーに接続できません。", |app| {
            in_flight(app, 9);
            answer(app, done(9, Err("翻訳サーバーに接続できません。")))
        }),
    ];
    for (words, fail) in failures {
        for size in SIZES {
            let mut app = mini();
            app.rows = talk(2);
            app.output = "前に訳した文章です。".into();
            let mut window = drawn(&mut app, size);
            fail(&mut app);
            assert_eq!(app.status, words, "the status row of the main window");
            window.settle(&mut app);
            assert!(window.has(words), "{words} is not on the card at {size:?}");
            assert!(window.has("← チャットに戻る"));
            assert!(
                !has_text_containing(&window.output, "前に訳した文章です。"),
                "{words}: the result before it is shown under the failure"
            );
            release_on(&mut window, &mut app, "コピー");
            assert!(copied(&window).is_empty(), "{words}: a failure was copied");
        }
    }
}

#[test]
fn a_range_too_small_to_read_says_so_on_the_card() {
    let mut app = mini();
    let overlay = egui::Context::default();
    app.selection = Some(Selection {
        screen: platform::Screen {
            id: 1,
            name: "fixture".into(),
            x: 0,
            y: 0,
            width: 640,
            height: 480,
            scale: 1.0,
        },
        image: image::RgbaImage::new(640, 480),
        texture: overlay.load_texture(
            "fixture",
            egui::ColorImage::filled([2, 2], Color32::WHITE),
            egui::TextureOptions::LINEAR,
        ),
        start: None,
        focus_requested: false,
        continuous: false,
    });
    let from = egui::pos2(120.0, 120.0);
    let to = egui::pos2(122.0, 122.0);
    for events in [
        vec![egui::Event::PointerMoved(from)],
        vec![press(from, egui::PointerButton::Primary, true)],
        vec![egui::Event::PointerMoved(to)],
        vec![press(to, egui::PointerButton::Primary, false)],
    ] {
        let _ = overlay.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(640.0, 480.0),
                )),
                events,
                ..Default::default()
            },
            |ctx| app.selection_ui(ctx),
        );
    }
    assert!(app.selection.is_none(), "the range was given");
    let window = drawn(&mut app, USUAL);
    assert!(window.has("もう少し広い範囲を選択してください。"));
    assert!(window.has("← チャットに戻る"));
}

#[test]
fn the_failure_before_is_over_when_the_next_reading_or_translation_begins() {
    // A picture given next. The recogniser is not prepared, so it waits.
    let mut app = mini();
    app.capture(&egui::Context::default());
    let mut window = drawn(&mut app, USUAL);
    assert!(window.has(NO_SCREEN));
    app.settings.ocr_backend = "indra".into();
    app.do_ocr(image::RgbaImage::new(2, 2));
    app.reset_screen_ocr();
    window.settle(&mut app);
    assert!(window.has(EMPTY_CARD), "nothing is left to show");
    assert!(!window.has(NO_SCREEN));

    // A text translation started next. Its worker cannot reach a translator
    // at this address and gives up at once, and the test answers in its place.
    let mut app = mini();
    app.capture(&egui::Context::default());
    let mut window = drawn(&mut app, USUAL);
    assert!(window.has(NO_SCREEN));
    app.settings.translation.endpoint = "no translator".into();
    app.input = "Hello.".into();
    app.manual_translate();
    let id = app.manual_busy.expect("the translation is in flight");
    assert!(matches!(
        app.rx.recv_timeout(Duration::from_secs(10)),
        Ok(Event::Done { .. })
    ));
    window.settle(&mut app);
    assert!(window.has("翻訳中") && !window.has(NO_SCREEN));
    answer(&mut app, done(id, Ok("こんにちは。")));
    window.settle(&mut app);
    assert!(window.has("こんにちは。"), "the new result is on the card");
    assert!(
        !window.has(NO_SCREEN),
        "the failure before it is still shown"
    );
}

#[test]
fn the_way_back_returns_to_the_chat_and_a_new_result_starts_at_its_first_line() {
    let long = |name: &str| format!("{name}の最初の行です。") + &"長い訳文が続きます。".repeat(60);
    let mut app = chat_without_speech();
    app.tab = Tab::Text;
    app.rows = talk(3);
    app.output = long("前の訳文");
    let mut window = Window::new(SMALLEST.0, SMALLEST.1);
    window.draw(&mut app, vec![mini_key()]);
    window.settle(&mut app);
    let first = find_text(&window.output, &app.output).0.top();

    // The reader scrolls down the long result.
    let body = window.at("コピー").center() + egui::vec2(-80.0, 80.0);
    scroll(&mut window, &mut app, body, -200.0);
    let scrolled = find_text(&window.output, &app.output).0.top();
    assert!(
        scrolled < first - 100.0,
        "the wheel moved the result: {scrolled}"
    );

    // More of the same result does not move the reader.
    app.output.push_str("末尾に足した文です。");
    window.settle(&mut app);
    assert_eq!(find_text(&window.output, &app.output).0.top(), scrolled);

    // The next translation begins, and its result is read from the top.
    in_flight(&mut app, 50);
    window.settle(&mut app);
    answer(&mut app, done(50, Ok(&long("次の訳文"))));
    window.settle(&mut app);
    assert_eq!(find_text(&window.output, &app.output).0.top(), first);

    window.click(&mut app, "← チャットに戻る");
    assert!(shows_talk(&window, 3), "the chat is back");
    assert!(!window.has("← チャットに戻る") && !window.has("コピー"));
    assert!(app.window_mode.is_mini());

    // A result that comes after that does not take the chat away again.
    in_flight(&mut app, 51);
    answer(&mut app, done(51, Ok("あとから届いた訳文です。")));
    window.settle(&mut app);
    assert!(shows_talk(&window, 3) && !window.has("あとから届いた訳文です。"));
}

#[test]
fn the_text_screen_brings_its_result_into_the_mini_view_and_every_other_screen_opens_the_chat() {
    let enter = |tab: Tab, prepare: fn(&mut App)| {
        let mut app = chat_without_speech();
        app.tab = tab;
        app.rows = talk(2);
        prepare(&mut app);
        let mut window = Window::new(USUAL.0, USUAL.1);
        window.draw(&mut app, vec![mini_key()]);
        window.settle(&mut app);
        assert!(app.window_mode.is_mini());
        (app, window)
    };
    let translated: fn(&mut App) = |app| app.output = "文章の画面の訳文です。".into();

    let (_, window) = enter(Tab::Text, translated);
    assert!(window.has("文章の画面の訳文です。") && window.has("← チャットに戻る"));
    assert!(bubbles(&window).is_empty());

    // The 文章 screen with nothing to show opens the chat.
    let (_, window) = enter(Tab::Text, |_| {});
    assert!(shows_talk(&window, 2));
    assert!(!window.has("← チャットに戻る") && !window.has(EMPTY_CARD));

    let (_, window) = enter(Tab::Text, |app| in_flight(app, 5));
    assert!(window.has("翻訳中") && bubbles(&window).is_empty());

    let (_, window) = enter(Tab::Text, |app| app.capture(&egui::Context::default()));
    assert!(window.has(NO_SCREEN) && bubbles(&window).is_empty());

    for tab in [Tab::Live, Tab::Files, Tab::Settings, Tab::YouTube] {
        let (mut app, mut window) = enter(tab, translated);
        assert!(shows_talk(&window, 2), "the chat is not shown");
        assert!(!window.has("文章の画面の訳文です。"));
        // Leaving with the chat shown returns to the screen it was entered from.
        window.draw(&mut app, vec![escape()]);
        assert!(!app.window_mode.is_mini() && app.tab == tab);
    }
}

#[test]
fn escape_and_the_mini_key_return_to_the_normal_view_and_the_card_opens_the_text_screen() {
    for leave in [escape as fn() -> egui::Event, mini_key] {
        // From the chat, the screen behind it is the one it was entered from.
        let mut app = mini();
        app.rows = talk(2);
        let mut window = drawn(&mut app, USUAL);
        window.draw(&mut app, vec![leave()]);
        assert!(!app.window_mode.is_mini());
        assert!(app.tab == Tab::Live);

        // From the card, the 文章 screen has the result.
        let mut app = mini();
        app.capture(&egui::Context::default());
        let mut window = drawn(&mut app, USUAL);
        assert!(window.has(NO_SCREEN));
        window.draw(&mut app, vec![leave()]);
        assert!(!app.window_mode.is_mini());
        assert!(app.tab == Tab::Text, "the result is on the 文章 screen");
    }

    let mut app = mini();
    let mut window = drawn(&mut app, USUAL);
    window.hover(&mut app, "通常表示");
    assert!(has_text_containing(&window.output, "通常表示に戻る（Esc）"));
    window.click(&mut app, "通常表示");
    assert!(!app.window_mode.is_mini());
}

#[test]
fn entering_goes_to_the_newest_message_and_scrolling_up_shows_the_new_messages_pill() {
    for follow in [true, false] {
        let mut app = chat_without_speech();
        app.settings.auto_scroll = follow;
        app.rows = talk(12);
        let mut main = drawn(&mut app, MAIN);
        // The reader goes to the first message in the main window.
        let body = app.live_chat.body().unwrap().center();
        scroll(&mut main, &mut app, body, 4000.0);
        assert_eq!(app.live_chat.offset(), 0.0);
        assert!(in_view(&main, &app, 1) && !in_view(&main, &app, 12));

        let mut window = Window::new(SMALLEST.0, SMALLEST.1);
        window.draw(&mut app, vec![mini_key()]);
        window.settle(&mut app);
        assert!(
            in_view(&window, &app, 12) && !in_view(&window, &app, 1),
            "the newest message is not in view with auto-scroll {follow}"
        );
        let end = app.live_chat.offset();

        let body = app.live_chat.body().unwrap().center();
        scroll(&mut window, &mut app, body, 150.0);
        let reading = app.live_chat.offset();
        assert!(reading < end - 100.0, "the wheel left the end: {reading}");
        assert!(!has_text_containing(&window.output, "新しいメッセージ"));

        app.rows.push(row(13, "音声", "13番目の訳文です。"));
        window.settle(&mut app);
        assert!(window.has("↓ 新しいメッセージ 1件"));
        assert!(
            (app.live_chat.offset() - reading).abs() < 1.0,
            "the new message moved the reader"
        );

        window.click(&mut app, "↓ 新しいメッセージ 1件");
        assert!(in_view(&window, &app, 13));
        assert!(!has_text_containing(&window.output, "新しいメッセージ"));

        // Back at the end, the chat follows the next message when it is set to.
        app.rows.push(row(14, "音声", "14番目の訳文です。"));
        window.settle(&mut app);
        window.settle(&mut app);
        assert_eq!(in_view(&window, &app, 14), follow);
        assert_eq!(window.has("↓ 新しいメッセージ 1件"), !follow);
    }
}

/// Every text but `body`, which scrolls in a room of its own: what each says
/// and where it is.
fn around(window: &Window, body: &str) -> Vec<(String, egui::Rect)> {
    texts(&window.output)
        .into_iter()
        .filter(|(text, _, _)| text != body)
        .map(|(text, rect, _)| (text, rect))
        .collect()
}

/// The face of the button whose words are `label`: the smallest rectangle
/// painted around them.
fn button_of(window: &Window, label: &str) -> egui::Rect {
    let words = window.at(label);
    shapes(&window.output)
        .into_iter()
        .filter_map(|(_, shape)| match shape {
            egui::Shape::Rect(face) if face.rect.contains_rect(words) => Some(face.rect),
            _ => None,
        })
        .min_by(|a, b| a.area().total_cmp(&b.area()))
        .unwrap_or_else(|| panic!("{label} has no button"))
}

fn assert_apart_and_inside(texts: &[(String, egui::Rect)], (width, height): (f32, f32)) {
    let window = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(width, height));
    for (index, (first, a)) in texts.iter().enumerate() {
        assert!(
            window.contains_rect(*a),
            "{first} leaves the window at {width}x{height}: {a:?}"
        );
        for (second, b) in &texts[index + 1..] {
            assert!(
                !a.intersects(*b),
                "{first} overlaps {second} at {width}x{height}"
            );
        }
    }
}

#[test]
fn nothing_leaves_the_window_or_overlaps_and_a_long_error_does_not_move_the_chat() {
    for size in SIZES {
        let listening = |error: &str| {
            let mut app = mini();
            app.settings.readout_enabled = true;
            app.caption_enabled = true;
            app.audio_enabled = true;
            app.asr_ready = true;
            app.rows = talk(8);
            app.service_error = error.into();
            app
        };
        let mut app = listening("");
        let window = drawn(&mut app, size);
        let chat = app.live_chat.panel().expect("the chat is drawn");
        for shown in [
            "通常表示",
            "■ 止める",
            "自動読み上げ",
            "8番目の訳文です。",
            "PC音声を聞いています",
            credit::LABEL,
        ] {
            assert!(window.has(shown), "{shown} is not drawn at {size:?}");
        }
        assert_apart_and_inside(&around(&window, ""), size);
        assert!(
            !cut_at_the_top(&window.output, app.live_chat.body().unwrap()),
            "a message is cut at the top at {size:?}"
        );

        // The toolbar is one row of controls 32 points tall, above the chat.
        let tops: Vec<f32> = ["通常表示", "■ 止める", "自動読み上げ"]
            .iter()
            .map(|label| {
                let button = button_of(&window, label);
                assert!(
                    (button.height() - 32.0).abs() < 0.5,
                    "{label} is {} tall at {size:?}",
                    button.height()
                );
                assert!(button.bottom() <= chat.top());
                button.top()
            })
            .collect();
        assert!(tops.iter().all(|top| *top == tops[0]), "{tops:?}");

        // The credit is at the bottom right, under the chat.
        let credit = window.at(credit::LABEL);
        assert!(credit.top() >= chat.bottom() && credit.right() > size.0 - 12.0);
        assert!(chat.height() > size.1 - 80.0, "the chat is {chat:?}");

        let mut app = listening(&"Long service error. ".repeat(60));
        let window = drawn(&mut app, size);
        assert_eq!(
            app.live_chat.panel(),
            Some(chat),
            "the error moved the chat at {size:?}"
        );
        let (parts, cut_short) = status_line(&window);
        assert_eq!(parts.len(), 1);
        assert!(parts[0].0.starts_with("Long service error. ") && cut_short);
        assert_apart_and_inside(&around(&window, ""), size);

        // Nor does a talk that is cleared.
        app.rows.clear();
        drawn(&mut app, size);
        assert_eq!(
            app.live_chat.panel(),
            Some(chat),
            "the empty talk moved the chat at {size:?}"
        );
    }
}

#[test]
fn the_card_fits_the_window_and_shows_whole_lines_of_a_long_result() {
    for size in SIZES {
        let mut app = chat_without_speech();
        app.settings.readout_enabled = true;
        app.tab = Tab::Text;
        app.service_error = "Long service error. ".repeat(60);
        app.output = "長い訳文が続きます。".repeat(80);
        let mut window = Window::new(size.0, size.1);
        window.draw(&mut app, vec![mini_key()]);
        window.settle(&mut app);

        let (clip, body) = shapes(&window.output)
            .into_iter()
            .find_map(|(clip, shape)| match shape {
                egui::Shape::Text(text) if text.galley.text() == app.output => {
                    Some((clip, text.clone()))
                }
                _ => None,
            })
            .expect("the result is on the card");
        let lines: Vec<_> = body
            .galley
            .rows
            .iter()
            .map(|row| row.rect().translate(body.pos.to_vec2()))
            .filter(|line| line.bottom() > clip.top() + 0.5 && line.top() < clip.bottom() - 0.5)
            .collect();
        assert!(
            lines.len() >= 3,
            "only {} lines of the result are in view at {size:?}",
            lines.len()
        );
        for line in &lines {
            assert!(
                line.top() >= clip.top() - 0.5 && line.bottom() <= clip.bottom() + 0.5,
                "a line is cut at {size:?}: {line:?} in {clip:?}"
            );
        }
        for shown in [
            "通常表示",
            "● 聞く",
            "自動読み上げ",
            "← チャットに戻る",
            "訳文",
            "コピー",
            credit::LABEL,
        ] {
            assert!(window.has(shown), "{shown} is not drawn at {size:?}");
        }
        assert_apart_and_inside(&around(&window, &app.output), size);
        let (back, heading, copy) = (
            window.at("← チャットに戻る"),
            window.at("訳文"),
            window.at("コピー"),
        );
        assert!(back.right() < heading.left() && heading.right() < copy.left());
        assert!(copy.right() > size.0 - 40.0, "コピー is not at the right");
        for label in ["← チャットに戻る", "コピー"] {
            let button = button_of(&window, label);
            assert!((button.height() - 32.0).abs() < 0.5, "{label}: {button:?}");
            assert!(button.top() >= button_of(&window, "通常表示").bottom());
            assert!(button.bottom() <= clip.top());
        }
        assert!(clip.bottom() <= window.at(credit::LABEL).top());
    }
}

#[test]
fn none_of_the_old_words_of_the_mini_view_is_drawn() {
    type Change = fn(&mut App);
    let scenes: [(&str, Change); 9] = [
        ("stopped and empty", |_| {}),
        ("listening", |app| {
            app.caption_enabled = true;
            app.audio_enabled = true;
            app.asr_ready = true;
            app.rows = talk(4);
            app.rows.push(forming(row(5, "音声", "いま話している途中")));
            app.current_live_id = Some(5);
            app.asr_partial = "still being recognised".into();
            app.live_busy = Some(5);
        }),
        ("every error at once", |app| {
            app.backend_ready = false;
            app.caption_enabled = true;
            app.service_error = "翻訳モデルへの接続が切れました。".into();
            app.journal_error = "disk full".into();
            app.readout.error = "voice missing".into();
            app.rows = talk(2);
            app.rows[1].error = Some("stream interrupted".into());
        }),
        ("readout in use", |app| {
            app.settings.readout_enabled = true;
            app.settings.auto_read = true;
            app.readout.status = "読み上げ中です。".into();
            app.rows = talk(2);
        }),
        ("a picture being read", |app| {
            app.settings.ocr_backend = "indra".into();
            app.do_ocr(image::RgbaImage::new(2, 2));
        }),
        ("a translation in flight", |app| {
            open_card(app);
            in_flight(app, 4);
        }),
        ("a failure", |app| app.capture(&egui::Context::default())),
        ("a result", |app| {
            open_card(app);
            app.output = "完成した訳文です。".into();
        }),
        ("an empty card", open_card),
    ];
    let check = |window: &Window, scene: &str, size: (f32, f32)| {
        for (text, _, _) in texts(&window.output) {
            for word in REMOVED {
                assert!(
                    !text.contains(word),
                    "{scene} at {size:?}: {word} is drawn in {text}"
                );
            }
        }
    };
    for (scene, change) in scenes {
        for size in SIZES {
            let mut app = mini();
            change(&mut app);
            let mut window = drawn(&mut app, size);
            check(&window, scene, size);
            for behind in ["入力の切り替え", "表示"] {
                open_menu(&mut window, &mut app);
                assert!(
                    window.has("字幕ウィンドウ"),
                    "{scene}: the menu is not open"
                );
                check(&window, scene, size);
                window.click(&mut app, behind);
                check(&window, scene, size);
                window.draw(&mut app, vec![escape()]);
                window.settle(&mut app);
                assert!(!window.has("字幕ウィンドウ"), "{scene}: the menu is open");
            }
        }
    }
}

/// An empty card: a picture was given, then dropped before it was read, as it
/// is when another recogniser is chosen in the settings. The recogniser is
/// not prepared, so nothing runs.
fn open_card(app: &mut App) {
    app.settings.ocr_backend = "indra".into();
    app.do_ocr(image::RgbaImage::new(2, 2));
    app.reset_screen_ocr();
}

fn two_screens() -> Vec<platform::Screen> {
    [(1, "DISPLAY1"), (2, "DISPLAY2")]
        .map(|(id, name)| platform::Screen {
            id,
            name: name.into(),
            x: 0,
            y: 0,
            width: 1920,
            height: 1080,
            scale: 1.0,
        })
        .into()
}

/// The 「⋯」 menu as it is drawn: its frame, and the words of its rows from
/// the top without the arrow or the shortcut at the end of a row.
fn menu(window: &Window) -> (egui::Rect, Vec<(String, egui::Rect)>) {
    let frame = box_of(&window.output, "入力の切り替え", theme::SURFACE);
    let mut rows: Vec<_> = texts(&window.output)
        .into_iter()
        .filter(|(text, rect, _)| {
            frame.contains_rect(*rect) && text != "⏵" && !text.starts_with("Ctrl")
        })
        .map(|(text, rect, _)| (text, rect))
        .collect();
    rows.sort_by(|a, b| a.1.center().y.total_cmp(&b.1.center().y));
    (frame, rows)
}

/// The inputs behind 入力の切り替え that are drawn as the chosen one.
fn marked(window: &Window) -> Vec<&'static str> {
    INPUTS
        .iter()
        .filter(|(label, _)| {
            filled(&window.output, theme::ACCENT_SOFT)
                .iter()
                .any(|rect| rect.contains_rect(window.at(label)))
        })
        .map(|(label, _)| *label)
        .collect()
}

/// Chooses `label` behind 入力の切り替え in the 「⋯」 menu, the way a pointer does.
fn choose(window: &mut Window, app: &mut App, label: &str) {
    open_menu(window, app);
    window.click(app, "入力の切り替え");
    window.click(app, label);
}

#[test]
fn the_menu_has_its_rows_in_order_32_points_apart_and_fits_the_smallest_window() {
    for size in SIZES {
        for screens in [1, 2] {
            let mut app = mini();
            app.rows = talk(2);
            app.screens = two_screens().into_iter().take(screens).collect();
            let mut window = drawn(&mut app, size);
            assert!(!window.has("字幕ウィンドウ"), "the menu opens unasked");
            assert!(
                window.at("⋯").right() > size.0 - 30.0,
                "⋯ is not at the right"
            );
            open_menu(&mut window, &mut app);

            let (frame, rows) = menu(&window);
            let mut expected = vec![
                "入力の切り替え",
                "表示",
                "最前面",
                "全文コピー",
                "範囲翻訳",
                "画面選択",
                "字幕ウィンドウ",
            ];
            if screens == 1 {
                expected.retain(|row| *row != "画面選択");
            }
            let words: Vec<&str> = rows.iter().map(|(text, _)| text.as_str()).collect();
            assert_eq!(words, expected, "{screens} screens at {size:?}");
            assert!(
                egui::Rect::from_min_size(egui::Pos2::ZERO, size.into()).contains_rect(frame),
                "the menu leaves the window with {screens} screens at {size:?}: {frame:?}"
            );
            for pair in rows.windows(2) {
                let step = pair[1].1.center().y - pair[0].1.center().y;
                if pair[0].0 == "全文コピー" {
                    assert!(
                        step > 33.0 && step < 48.0,
                        "no separator under 全文コピー: {step}"
                    );
                } else {
                    assert!(
                        (step - 32.0).abs() < 0.5,
                        "{} is {step} under {}",
                        pair[1].0,
                        pair[0].0
                    );
                }
            }
            let (range, shortcut) = (window.at("範囲翻訳"), window.at("Ctrl + Alt + T"));
            assert!(
                shortcut.left() > range.right()
                    && (shortcut.center().y - range.center().y).abs() < 2.0,
                "the shortcut is not at the end of the 範囲翻訳 row"
            );
        }
    }
}

#[test]
fn the_menu_button_is_called_other_and_escape_closes_the_menu_before_the_mini_view() {
    let mut app = mini();
    let mut window = drawn(&mut app, USUAL);
    window.hover(&mut app, "⋯");
    assert!(window.has("その他"), "the hover text");
    release_on(&mut window, &mut app, "⋯");
    assert!(
        window
            .output
            .platform_output
            .events
            .iter()
            .any(|event| matches!(
                event,
                egui::output::OutputEvent::Clicked(info) if info.label.as_deref() == Some("その他")
            )),
        "a screen reader is not told the name of the button"
    );
    window.settle(&mut app);
    assert!(window.has("字幕ウィンドウ"));

    window.draw(&mut app, vec![escape()]);
    window.settle(&mut app);
    assert!(!window.has("字幕ウィンドウ"), "Esc did not close the menu");
    assert!(app.window_mode.is_mini(), "Esc left the mini view as well");
    window.draw(&mut app, vec![escape()]);
    assert!(
        !app.window_mode.is_mini(),
        "the next Esc leaves the mini view"
    );
}

#[test]
fn the_inputs_are_behind_one_row_in_order_with_the_chosen_one_marked() {
    for size in SIZES {
        let mut app = mini();
        let mut window = drawn(&mut app, size);
        open_menu(&mut window, &mut app);
        assert!(!window.has("自動（字幕を優先）"), "they are behind the row");
        window.click(&mut app, "入力の切り替え");
        let inside = egui::Rect::from_min_size(egui::Pos2::ZERO, size.into());
        let tops: Vec<f32> = INPUTS
            .iter()
            .map(|(label, _)| {
                let words = window.at(label);
                assert!(inside.contains_rect(words), "{label} leaves the window");
                words.top()
            })
            .collect();
        assert!(
            tops.windows(2).all(|pair| pair[0] < pair[1]),
            "the inputs are out of order at {size:?}: {tops:?}"
        );
        assert_eq!(marked(&window), ["自動（字幕を優先）"], "the default");

        window.click(&mut app, "PC音声だけ");
        open_menu(&mut window, &mut app);
        window.click(&mut app, "入力の切り替え");
        assert_eq!(
            marked(&window),
            ["PC音声だけ"],
            "what the next 聞く turns on"
        );

        app.caption_enabled = true;
        window.settle(&mut app);
        assert_eq!(marked(&window), ["YouTube字幕だけ"], "what is on now");
    }
}

#[test]
fn an_input_chosen_while_stopped_waits_for_the_button_and_one_chosen_while_listening_is_on_at_once()
{
    for (label, mode) in INPUTS {
        let mut app = mini();
        let mut window = drawn(&mut app, USUAL);
        choose(&mut window, &mut app, label);
        assert!(app.listen_input == mode, "{label} is not remembered");
        assert_eq!(armed(&app), (false, false), "{label} started by itself");
        assert!(app.screen_feed.is_none() && !app.audio_confirmation);
        assert!(app.status.is_empty(), "{label}: {}", app.status);

        window.click(&mut app, "● 聞く");
        let expected = match mode {
            LiveSourceMode::Auto => (true, true),
            LiveSourceMode::Captions => (true, false),
            LiveSourceMode::Audio => (false, true),
            LiveSourceMode::Screen => (false, false),
        };
        assert_eq!(armed(&app), expected, "{label}: 聞く");
    }

    for (label, mode) in INPUTS {
        let mut app = mini();
        let mut window = drawn(&mut app, USUAL);
        window.click(&mut app, "● 聞く");
        assert_eq!(armed(&app), (true, true));
        let generation = app.caption_generation;
        choose(&mut window, &mut app, label);
        assert!(app.listen_input == mode);
        match mode {
            LiveSourceMode::Auto => {
                assert_eq!(armed(&app), (true, true), "{label}");
                assert_eq!(app.caption_generation, generation, "{label} restarted");
            }
            LiveSourceMode::Captions => assert_eq!(armed(&app), (true, false), "{label}"),
            LiveSourceMode::Audio => assert_eq!(armed(&app), (false, true), "{label}"),
            LiveSourceMode::Screen => {
                assert_eq!(
                    app.status, NO_SCREEN,
                    "the range to read is asked for first"
                );
                assert_eq!(armed(&app), (true, true), "what runs goes on until then");
            }
        }
    }
}

#[test]
fn the_display_options_of_the_menu_are_the_chat_s_own_settings() {
    let mut app = mini();
    app.rows = talk(2);
    assert!(!app.settings.live_show_original && app.settings.auto_scroll);
    let mut window = drawn(&mut app, USUAL);
    assert!(!has_text_containing(&window.output, "Original 1."));
    open_menu(&mut window, &mut app);
    window.click(&mut app, "表示");
    for option in [
        "原文をすべて表示",
        "末尾へ自動スクロール",
        "訳文の文字サイズ",
    ] {
        assert!(window.has(option), "{option} is not behind 表示");
    }
    window.click(&mut app, "原文をすべて表示");
    assert!(app.settings.live_show_original);
    assert!(
        window.has("Original 1.") && window.has("Original 2."),
        "the chat does not show the originals"
    );

    open_menu(&mut window, &mut app);
    window.click(&mut app, "表示");
    window.click(&mut app, "末尾へ自動スクロール");
    assert!(!app.settings.auto_scroll);
}

#[test]
fn the_topmost_switch_of_the_menu_raises_the_window_and_lowers_it_again() {
    let level = |window: &Window| -> Vec<egui::WindowLevel> {
        window.output.viewport_output[&egui::ViewportId::ROOT]
            .commands
            .iter()
            .filter_map(|command| match command {
                egui::ViewportCommand::WindowLevel(level) => Some(*level),
                _ => None,
            })
            .collect()
    };
    let mut app = mini();
    let mut window = drawn(&mut app, USUAL);
    open_menu(&mut window, &mut app);
    release_on(&mut window, &mut app, "最前面");
    assert_eq!(level(&window), [egui::WindowLevel::AlwaysOnTop]);
    assert!(app.window_mode.always_on_top());

    window.settle(&mut app);
    open_menu(&mut window, &mut app);
    release_on(&mut window, &mut app, "最前面");
    assert_eq!(level(&window), [egui::WindowLevel::Normal]);
    assert!(!app.window_mode.always_on_top());
}

#[test]
fn copy_all_in_the_menu_copies_the_talk_with_the_credit_also_from_behind_the_card() {
    for card in [false, true] {
        let mut app = mini();
        if card {
            app.capture(&egui::Context::default());
        }
        let mut window = drawn(&mut app, USUAL);
        open_menu(&mut window, &mut app);
        let empty = window.ink("全文コピー");
        window.click(&mut app, "全文コピー");
        assert_eq!(app.live_chat.copied(), None, "an empty talk was copied");

        app.rows = talk(2);
        open_menu(&mut window, &mut app);
        assert_ne!(window.ink("全文コピー"), empty, "全文コピー is still faded");
        hover_later(&mut window, &mut app, "全文コピー");
        assert!(has_text_containing(
            &window.output,
            "表示中の会話をすべて、PLaMoのクレジットを付けてコピーします。"
        ));
        window.click(&mut app, "全文コピー");
        assert_eq!(
            app.live_chat.copied(),
            Some(credit::published("1番目の訳文です。\n\n2番目の訳文です。").as_str()),
            "behind the card: {card}"
        );
        assert_eq!(
            window.has(NO_SCREEN),
            card,
            "the copy changed what is shown"
        );
    }
}

#[test]
fn reading_a_range_from_the_menu_stays_in_the_mini_view_and_reports_on_the_card() {
    let ink = |ready: bool| {
        let mut app = mini();
        app.backend_ready = ready;
        let mut window = drawn(&mut app, USUAL);
        open_menu(&mut window, &mut app);
        window.ink("範囲翻訳")
    };
    assert_ne!(
        ink(true),
        ink(false),
        "範囲翻訳 is not faded without the model"
    );

    let mut app = mini();
    app.tab = Tab::Files;
    app.rows = talk(2);
    let mut window = drawn(&mut app, USUAL);
    open_menu(&mut window, &mut app);
    window.click(&mut app, "範囲翻訳");
    // This PC has no screen to read, and says so.
    assert_eq!(app.status, NO_SCREEN);
    assert!(window.has(NO_SCREEN) && window.has("← チャットに戻る"));
    assert!(app.window_mode.is_mini());
    assert!(
        app.tab == Tab::Files,
        "the screen behind the mini view changed"
    );
}

#[test]
fn the_screen_to_read_is_chosen_in_the_menu_and_the_subtitle_window_opens_from_it() {
    let mut app = mini();
    app.screens = two_screens();
    let mut window = drawn(&mut app, USUAL);
    open_menu(&mut window, &mut app);
    window.click(&mut app, "画面選択");
    window.click(&mut app, "2: DISPLAY2");
    assert_eq!(app.screen_index, 1);

    assert!(!app.subtitle_window.open);
    open_menu(&mut window, &mut app);
    window.click(&mut app, "字幕ウィンドウ");
    assert!(app.subtitle_window.open);
    assert!(app.window_mode.is_mini());
}
