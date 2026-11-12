//! The mini view is the chat in a small window, with the main window's words
//! and its one listen button. A reading or a text translation asked for there
//! shows its result on a card in the chat's place.
use super::home_tests::{Window, chat, dark_chat_saved, key, row, with_dark_chat};
use super::listen_tests::{
    CAPTIONS_ARRIVING, INPUTS, SCREEN, armed, caption, chat_without_speech, drawn, extension, hint,
    marked, with_youtube_captions,
};
use super::*;
use crate::chat::testing::{
    assert_drawn_dark, background, box_of, dots, fill_behind, filled, find_text,
    has_text_containing, light_colours, line_cut_at_the_top, press, selected, shapes, texts, wheel,
};
use plamo_translate::journal::{ParagraphMeta, ParagraphState};

/// The smallest size of the mini view, and the size it opens at.
const SMALLEST: (f32, f32) = (400.0, 260.0);
const USUAL: (f32, f32) = (520.0, 360.0);
const SIZES: [(f32, f32); 2] = [SMALLEST, USUAL];
/// The main window at its smallest.
const MAIN: (f32, f32) = (860.0, 620.0);

/// The controls that are in the toolbar while it has room for them and in
/// the 「⋯」 menu while it has not, from the left in the toolbar and from the
/// top in the menu.
const MOVABLE: [&str; 3] = ["クリア", "全文コピー", "最前面"];
/// The same controls in the order they keep their place in the toolbar:
/// 全文コピー is the first to go into the menu and クリア the last.
const KEEP: [&str; 3] = ["クリア", "最前面", "全文コピー"];

/// What the toolbar holds from the left and the 「⋯」 menu from the top, at
/// one size with or without the read-aloud switch.
struct Room {
    size: (f32, f32),
    readout: bool,
    toolbar: &'static [&'static str],
    menu: &'static [&'static str],
}

const ROOM: [Room; 4] = [
    Room {
        size: USUAL,
        readout: false,
        toolbar: &["通常表示", "● 聞く", "クリア", "全文コピー", "最前面", "⋯"],
        menu: &["表示", "入力の切り替え", OTHER_FEATURES],
    },
    Room {
        size: USUAL,
        readout: true,
        toolbar: &[
            "通常表示",
            "● 聞く",
            "自動読み上げ",
            "クリア",
            "最前面",
            "⋯",
        ],
        menu: &["表示", "入力の切り替え", "全文コピー", OTHER_FEATURES],
    },
    Room {
        size: SMALLEST,
        readout: false,
        toolbar: &["通常表示", "● 聞く", "クリア", "最前面", "⋯"],
        menu: &["表示", "入力の切り替え", "全文コピー", OTHER_FEATURES],
    },
    Room {
        size: SMALLEST,
        readout: true,
        toolbar: &["通常表示", "● 聞く", "自動読み上げ", "⋯"],
        menu: &[
            "表示",
            "入力の切り替え",
            "クリア",
            "全文コピー",
            "最前面",
            OTHER_FEATURES,
        ],
    },
];

/// The words of the main window's クリア and 全文コピー when the pointer
/// rests on them.
const CLEAR_HINT: &str = "表示中の会話を消して、新しく始めます。消した会話は「履歴」に残ります。";
const COPY_ALL_HINT: &str = "表示中の会話をすべて、PLaMoのクレジットを付けてコピーします。";
const TOPMOST_HINT: &str = "ほかの窓より手前に置きます。通常表示に戻ると外れます。";

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

fn open_other_features(window: &mut Window, app: &mut App) {
    open_menu(window, app);
    window.click(app, OTHER_FEATURES);
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

/// Starts a text translation the way the app does. Its worker cannot reach a
/// translator at this address and gives up at once. That answer is taken
/// away here, and the test gives its own with `answer`.
fn translation_begun(app: &mut App) -> u64 {
    app.settings.translation.endpoint = "no translator".into();
    app.input = "Hello.".into();
    app.manual_translate();
    let id = app.manual_busy.expect("the translation is in flight");
    assert!(matches!(
        app.rx.recv_timeout(Duration::from_secs(10)),
        Ok(Event::Done { .. })
    ));
    id
}

fn done(id: u64, result: std::result::Result<&str, &str>) -> Event {
    Event::Done {
        id,
        result: result.map(Into::into).map_err(Into::into),
        elapsed: 0.1,
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
    let screen = (SCREEN, LiveSourceMode::Screen, (false, false));
    for (label, mode, expected) in INPUTS.into_iter().chain([screen]) {
        let small = pressed(mode, true);
        assert_eq!(
            small,
            pressed(mode, false),
            "{label}: the button of the mini view and the button of the main window differ"
        );
        let ((heard, asks, status), stops, after, listens_again) = small;
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
fn captions_that_arrive_while_they_are_not_translated_are_told_on_the_status_line_too() {
    for size in SIZES {
        let stopped = ("止まっています".to_owned(), theme::TEXT_SECONDARY);
        let mut app = mini();
        let extension = extension(&mut app);
        extension
            .send(caption("The library opens at nine.", false))
            .unwrap();
        app.process(&egui::Context::default());
        let window = drawn(&mut app, size);
        assert_eq!(
            status_line(&window).0,
            [
                stopped.clone(),
                (CAPTIONS_ARRIVING.to_owned(), theme::TEXT_SECONDARY)
            ],
            "at {size:?}"
        );

        // What the app tells next takes its place, as on the main window's row.
        app.status = "準備ができました。".into();
        let window = drawn(&mut app, size);
        assert_eq!(status_line(&window).0, [stopped], "at {size:?}");
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
    app.settings.readout_enabled = true;
    app.readout.error = READOUT.into();
    let window = drawn(&mut app, USUAL);
    assert_eq!(
        status_line(&window).0,
        [stopped.clone(), (READOUT.to_owned(), theme::ERROR)],
        "a failure to read aloud comes last"
    );
    app.settings.readout_enabled = false;
    let window = drawn(&mut app, USUAL);
    assert_eq!(
        status_line(&window).0,
        [stopped],
        "a failure to read aloud is not said while readout is off"
    );

    let mut app = mini();
    app.journal_error = "Could not append translation history".into();
    app.service_error = SERVICE.into();
    app.settings.readout_enabled = true;
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
fn the_empty_mini_chat_says_that_listen_translates_the_english_playing_on_the_pc() {
    for size in SIZES {
        let mut app = mini();
        assert_eq!(
            hint(&mut app, size),
            "PCで再生中の英語の音声を、ここにやさしい日本語で表示します。\n上の「● 聞く」を押すと始まります。",
            "{size:?}"
        );

        let mut app = with_youtube_captions(mini());
        assert_eq!(
            hint(&mut app, size),
            "PCで再生中の英語の音声やYouTube字幕を、ここにやさしい日本語で表示します。\n上の「● 聞く」を押すと始まります。",
            "{size:?}"
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
fn a_reading_s_translation_takes_the_card_from_its_own_state_not_from_the_translation_before_it() {
    const BEFORE: &str = "前に訳した文章です。";
    let mut app = mini();
    app.output = BEFORE.into();
    app.settings.translation.endpoint = "no translator".into();
    // The recogniser is reading a picture given in the mini view.
    app.ocr.busy = Some(app.ocr.generation);
    app.reading_given();
    let mut window = drawn(&mut app, USUAL);
    assert!(window.has(READING) && !has_text_containing(&window.output, BEFORE));

    let generation = app.ocr.generation;
    answer(
        &mut app,
        Event::Ocr {
            generation,
            result: Ok("Text read from a picture.".into()),
        },
    );
    let id = app.manual_busy.expect("the text that was read is not sent");
    // The worker cannot reach a translator at this address and gives up at
    // once; the test answers in its place.
    assert!(matches!(
        app.rx.recv_timeout(Duration::from_secs(10)),
        Ok(Event::Done { .. })
    ));
    window.settle(&mut app);
    assert!(
        window.has("翻訳中") && !has_text_containing(&window.output, BEFORE),
        "the translation before the reading is shown as the reading's"
    );
    answer(
        &mut app,
        Event::Partial {
            id,
            text: "読み取った文の途中の訳".into(),
        },
    );
    window.settle(&mut app);
    assert!(window.has("読み取った文の途中の訳") && !window.has("翻訳中"));
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

    // A translation that is still coming leaves the chat in view. Once it is
    // done the card shows it: what was asked for is not left out of sight.
    in_flight(&mut app, 51);
    answer(
        &mut app,
        Event::Partial {
            id: 51,
            text: "あとから届く訳文の途中".into(),
        },
    );
    window.settle(&mut app);
    assert!(
        shows_talk(&window, 3) && !has_text_containing(&window.output, "あとから届く"),
        "half a translation took the chat away"
    );
    answer(&mut app, done(51, Ok("あとから届いた訳文です。")));
    window.settle(&mut app);
    assert!(
        window.has("あとから届いた訳文です。") && window.has("← チャットに戻る"),
        "the result is not shown once it is done"
    );
    assert!(bubbles(&window).is_empty(), "the chat is under the card");
}

#[test]
fn messages_that_come_while_the_card_is_shown_are_in_view_on_the_way_back() {
    let mut app = mini();
    app.rows = talk(6);
    let mut window = drawn(&mut app, SMALLEST);
    assert!(in_view(&window, &app, 6) && !in_view(&window, &app, 1));

    // This PC has no screen to read, so the card comes up with that.
    app.capture(&egui::Context::default());
    window.settle(&mut app);
    assert!(window.has(NO_SCREEN) && bubbles(&window).is_empty());
    for id in 7..=9 {
        app.rows
            .push(row(id, "音声", &format!("{id}番目の訳文です。")));
    }
    window.settle(&mut app);

    window.click(&mut app, "← チャットに戻る");
    assert!(
        in_view(&window, &app, 9),
        "the chat stayed where it was when the card came up"
    );
    assert!(!has_text_containing(&window.output, "新しいメッセージ"));
}

#[test]
fn a_result_that_comes_while_the_chat_is_shown_is_read_from_its_first_line() {
    let long = |name: &str| format!("{name}の最初の行です。") + &"長い訳文が続きます。".repeat(60);
    let mut app = chat_without_speech();
    app.tab = Tab::Text;
    app.rows = talk(3);
    app.output = long("前の訳文");
    let mut window = Window::new(SMALLEST.0, SMALLEST.1);
    window.draw(&mut app, vec![mini_key()]);
    window.settle(&mut app);
    let first = find_text(&window.output, &app.output).0.top();
    let body = window.at("コピー").center() + egui::vec2(-80.0, 80.0);
    scroll(&mut window, &mut app, body, -200.0);
    assert!(
        find_text(&window.output, &app.output).0.top() < first - 100.0,
        "the wheel did not move the result"
    );

    // The reader goes back to the chat. The next translation begins and is
    // done without the card being drawn in between.
    window.click(&mut app, "← チャットに戻る");
    let id = translation_begun(&mut app);
    window.settle(&mut app);
    assert!(shows_talk(&window, 3));
    answer(&mut app, done(id, Ok(&long("次の訳文"))));
    window.settle(&mut app);
    assert!(app.output.starts_with("次の訳文"));
    assert_eq!(
        find_text(&window.output, &app.output).0.top(),
        first,
        "the new result opens where the reader was in the one before"
    );
}

#[test]
fn a_failure_while_a_translation_is_in_flight_gives_way_once_the_translation_is_done() {
    const NOTHING_READ: &str = "文字が見つかりませんでした。範囲や原文言語を確認してください。";
    let mut app = mini();
    open_card(&mut app);
    let mut window = drawn(&mut app, USUAL);
    let id = translation_begun(&mut app);
    answer(
        &mut app,
        Event::Partial {
            id,
            text: "先に頼んだ文章の途中の訳".into(),
        },
    );
    window.settle(&mut app);
    assert!(window.has("先に頼んだ文章の途中の訳"));

    // A picture given meanwhile has no text in it. That is the newest thing
    // that happened, so the card says it at once.
    let generation = app.ocr.generation;
    answer(
        &mut app,
        Event::Ocr {
            generation,
            result: Ok("  ".into()),
        },
    );
    window.settle(&mut app);
    assert!(window.has(NOTHING_READ), "the failure waits behind 翻訳中");
    assert!(!has_text_containing(&window.output, "先に頼んだ文章"));

    answer(&mut app, done(id, Ok("完成した訳文です。")));
    window.settle(&mut app);
    assert!(
        window.has("完成した訳文です。") && !window.has(NOTHING_READ),
        "the older failure hides the translation that is done"
    );
    release_on(&mut window, &mut app, "コピー");
    assert_eq!(copied(&window), [credit::published("完成した訳文です。")]);
}

#[test]
fn the_menu_row_enters_at_the_newest_message_and_the_main_chat_keeps_the_reader_s_place() {
    for follow in [true, false] {
        let mut app = chat_without_speech();
        app.settings.auto_scroll = follow;
        app.rows = talk(12);
        let mut window = drawn(&mut app, MAIN);
        // The reader goes to the first message in the main window.
        let body = app.live_chat.body().unwrap().center();
        scroll(&mut window, &mut app, body, 4000.0);
        assert!(in_view(&window, &app, 1) && !in_view(&window, &app, 12));

        // その他 → ミニ表示. It is the same window, made small, and the main
        // window still draws its chat on the frame of the click.
        window.open_menu(&mut app);
        window.click(&mut app, "ミニ表示");
        assert!(app.window_mode.is_mini());
        window.resize(SMALLEST.0, SMALLEST.1);
        window.settle(&mut app);
        assert!(
            in_view(&window, &app, 12) && !in_view(&window, &app, 1),
            "the newest message is not in view with auto-scroll {follow}"
        );

        window.click(&mut app, "通常表示");
        window.resize(MAIN.0, MAIN.1);
        window.settle(&mut app);
        assert!(!app.window_mode.is_mini());
        assert!(
            in_view(&window, &app, 1) && !in_view(&window, &app, 12),
            "the main chat lost the reader's place with auto-scroll {follow}"
        );
    }
}

#[test]
fn a_visit_that_showed_only_the_card_leaves_the_main_chat_where_the_reader_was() {
    let mut app = chat_without_speech();
    app.rows = talk(12);
    let mut window = drawn(&mut app, MAIN);
    let body = app.live_chat.body().unwrap().center();
    scroll(&mut window, &mut app, body, 4000.0);
    assert!(in_view(&window, &app, 1) && !in_view(&window, &app, 12));

    // The シンプル翻訳 screen has a result, so the mini view opens on the card.
    app.tab = Tab::Text;
    app.output = "シンプル翻訳の訳文です。".into();
    window.draw(&mut app, vec![mini_key()]);
    window.resize(USUAL.0, USUAL.1);
    window.settle(&mut app);
    assert!(window.has("シンプル翻訳の訳文です。") && bubbles(&window).is_empty());
    window.draw(&mut app, vec![escape()]);
    window.resize(MAIN.0, MAIN.1);
    assert!(!app.window_mode.is_mini() && app.tab == Tab::Text);

    app.tab = Tab::Live;
    window.settle(&mut app);
    assert!(
        in_view(&window, &app, 1) && !in_view(&window, &app, 12),
        "the chat that the mini view never showed was moved to its end"
    );
}

#[test]
fn escape_lets_go_of_selected_text_before_it_leaves_the_mini_view() {
    let mut app = mini();
    app.rows = talk(2);
    let mut window = drawn(&mut app, USUAL);
    let words = window.at("1番目の訳文です。");
    let from = words.left_center() + egui::vec2(2.0, 0.0);
    let to = words.right_center() - egui::vec2(2.0, 0.0);
    window.draw(&mut app, vec![egui::Event::PointerMoved(from)]);
    window.draw(
        &mut app,
        vec![press(from, egui::PointerButton::Primary, true)],
    );
    for step in 1..=4 {
        let at = from + (to - from) * (step as f32 / 4.0);
        window.draw(&mut app, vec![egui::Event::PointerMoved(at)]);
    }
    window.draw(
        &mut app,
        vec![press(to, egui::PointerButton::Primary, false)],
    );
    window.settle(&mut app);
    assert_eq!(
        selected(&window.output),
        ["1番目の訳文です。"],
        "the drag selected the words of the bubble"
    );

    window.draw(&mut app, vec![escape()]);
    window.settle(&mut app);
    assert!(
        selected(&window.output).is_empty(),
        "Esc did not let go of the selection"
    );
    assert!(app.window_mode.is_mini(), "Esc left the mini view as well");
    window.draw(&mut app, vec![escape()]);
    assert!(
        !app.window_mode.is_mini(),
        "the next Esc leaves the mini view"
    );
}

#[test]
fn escape_closes_the_menu_before_it_lets_go_of_selected_text() {
    let mut app = mini();
    app.rows = talk(2);
    let mut window = drawn(&mut app, USUAL);
    let words = window.at("1番目の訳文です。");
    let from = words.left_center() + egui::vec2(2.0, 0.0);
    let to = words.right_center() - egui::vec2(2.0, 0.0);
    window.draw(&mut app, vec![egui::Event::PointerMoved(from)]);
    window.draw(
        &mut app,
        vec![press(from, egui::PointerButton::Primary, true)],
    );
    for step in 1..=4 {
        let at = from + (to - from) * (step as f32 / 4.0);
        window.draw(&mut app, vec![egui::Event::PointerMoved(at)]);
    }
    window.draw(
        &mut app,
        vec![press(to, egui::PointerButton::Primary, false)],
    );
    open_menu(&mut window, &mut app);
    assert!(window.has("入力の切り替え") && !selected(&window.output).is_empty());

    window.draw(&mut app, vec![escape()]);
    window.settle(&mut app);
    assert!(!window.has("入力の切り替え"), "Esc did not close the menu");
    assert_eq!(
        selected(&window.output),
        ["1番目の訳文です。"],
        "the Esc that closed the menu let go of the selection too"
    );
    window.draw(&mut app, vec![escape()]);
    window.settle(&mut app);
    assert!(selected(&window.output).is_empty());
    assert!(app.window_mode.is_mini());
}

#[test]
fn the_model_screen_has_no_way_into_the_mini_view_and_says_why() {
    let row_ink = |ready: bool, tab: Tab| {
        let mut app = chat_without_speech();
        app.backend_ready = ready;
        app.tab = tab;
        let mut window = Window::new(MAIN.0, MAIN.1);
        window.open_menu(&mut app);
        window.ink("ミニ表示")
    };
    let faded = row_ink(false, Tab::Files);
    assert_ne!(row_ink(true, Tab::Files), faded);
    assert_eq!(
        row_ink(true, Tab::Setup),
        faded,
        "the row is not faded on the model screen"
    );

    for (ready, words) in [
        (true, "モデルの画面では、ミニ表示を使えません。"),
        (false, "モデルの準備ができると、ミニ表示を使えます。"),
    ] {
        let mut app = chat_without_speech();
        app.backend_ready = ready;
        app.tab = Tab::Setup;
        let mut window = drawn(&mut app, MAIN);
        window.draw(&mut app, vec![mini_key()]);
        assert!(!app.window_mode.is_mini());
        assert_eq!(app.status, words);
    }
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
    let translated: fn(&mut App) = |app| app.output = "シンプル翻訳の訳文です。".into();

    let (_, window) = enter(Tab::Text, translated);
    assert!(window.has("シンプル翻訳の訳文です。") && window.has("← チャットに戻る"));
    assert!(bubbles(&window).is_empty());

    // The シンプル翻訳 screen with nothing to show opens the chat.
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
        assert!(!window.has("シンプル翻訳の訳文です。"));
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

        // From the card, the シンプル翻訳 screen has the result.
        let mut app = mini();
        app.capture(&egui::Context::default());
        let mut window = drawn(&mut app, USUAL);
        assert!(window.has(NO_SCREEN));
        window.draw(&mut app, vec![leave()]);
        assert!(!app.window_mode.is_mini());
        assert!(
            app.tab == Tab::Text,
            "the result is on the シンプル翻訳 screen"
        );
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

/// The words of the toolbar from the left: every text above the chat.
fn toolbar(window: &Window, app: &App) -> Vec<String> {
    let chat = app.live_chat.panel().expect("the chat is drawn");
    let mut above: Vec<_> = texts(&window.output)
        .into_iter()
        .filter(|(_, rect, _)| rect.bottom() <= chat.top())
        .map(|(text, rect, _)| (text, rect.left()))
        .collect();
    above.sort_by(|a, b| a.1.total_cmp(&b.1));
    above.into_iter().map(|(text, _)| text).collect()
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
    for room in ROOM {
        let (size, readout) = (room.size, room.readout);
        let listening = |error: &str| {
            let mut app = mini();
            app.settings.readout_enabled = readout;
            app.caption_enabled = true;
            app.audio_enabled = true;
            app.asr_ready = true;
            app.rows = talk(8);
            app.service_error = error.into();
            app
        };
        // While it listens, 聞く is 止める.
        let controls: Vec<String> = room
            .toolbar
            .iter()
            .map(|label| label.replace("● 聞く", "■ 止める"))
            .collect();
        let mut app = listening("");
        let window = drawn(&mut app, size);
        let chat = app.live_chat.panel().expect("the chat is drawn");
        for shown in controls.iter().map(String::as_str).chain([
            "8番目の訳文です。",
            "PC音声を聞いています",
            credit::LABEL,
        ]) {
            assert!(
                window.has(shown),
                "{shown} is not drawn at {size:?}, read-aloud {readout}"
            );
        }
        assert_apart_and_inside(&around(&window, ""), size);
        assert!(
            !line_cut_at_the_top(&window.output, app.live_chat.body().unwrap()),
            "a line of a message is cut at the top at {size:?}"
        );

        // The toolbar is one row of controls 32 points tall, above the chat,
        // and no control is drawn over another.
        let faces: Vec<(String, egui::Rect)> = controls
            .iter()
            .map(|label| (label.clone(), button_of(&window, label)))
            .collect();
        assert_apart_and_inside(&faces, size);
        for (label, face) in &faces {
            assert!(
                (face.height() - 32.0).abs() < 0.5,
                "{label} is {} tall at {size:?}",
                face.height()
            );
            assert!(face.bottom() <= chat.top());
        }
        assert!(
            faces.iter().all(|(_, face)| face.top() == faces[0].1.top()),
            "{faces:?}"
        );

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
fn a_control_is_in_the_menu_only_while_the_toolbar_has_no_room_for_it_at_every_width_and_scale() {
    // At 1.1, one zoom-in step above 100%, the words of the controls are a
    // fraction of a point wider than at 1 and 1.25.
    for scale in [1.0, 1.1, 1.25] {
        // How wide each control and the gap between two are drawn, from a
        // toolbar that holds all three.
        let mut app = mini();
        let mut window = Window::new(USUAL.0, USUAL.1);
        window.scale(scale);
        window.settle(&mut app);
        let widths = KEEP.map(|label| button_of(&window, label).width());
        let gap = button_of(&window, "● 聞く").left() - button_of(&window, "通常表示").right();

        for readout in [false, true] {
            let mut app = mini();
            app.settings.readout_enabled = readout;
            let mut window = Window::new(SMALLEST.0, SMALLEST.1);
            window.scale(scale);
            // How many the toolbar held when the menu was last opened.
            let mut opened_at = None;
            // Every width the window can have, in whole pixels.
            let pixels = |points: f32| (points * scale).round() as u32;
            for wide in pixels(SMALLEST.0)..=pixels(USUAL.0) {
                let size = (wide as f32 / scale, SMALLEST.1);
                let at = format!("at {wide} pixels wide, {scale}x, read-aloud {readout}");
                window.resize(size.0, size.1);
                window.settle(&mut app);
                let shown = toolbar(&window, &app);
                let held: Vec<&str> = shown
                    .iter()
                    .map(String::as_str)
                    .filter(|label| MOVABLE.contains(label))
                    .collect();
                // The first ones of KEEP stay, and the toolbar and the menu
                // have theirs in the order of MOVABLE.
                let (stay, rest): (Vec<&str>, Vec<&str>) = MOVABLE
                    .into_iter()
                    .partition(|label| KEEP[..held.len()].contains(label));
                assert_eq!(held, stay, "{at}");
                let faces: Vec<(String, egui::Rect)> = shown
                    .iter()
                    .map(|label| (label.clone(), button_of(&window, label)))
                    .collect();
                assert_apart_and_inside(&faces, size);
                for pair in faces.windows(2) {
                    let apart = pair[1].1.left() - pair[0].1.right();
                    assert!(
                        apart > gap - 0.01,
                        "{} is {apart} from {} {at}",
                        pair[1].0,
                        pair[0].0
                    );
                }
                if let Some(next) = KEEP.get(held.len()) {
                    let [.., (_, last), (_, menu)] = faces.as_slice() else {
                        panic!("the toolbar is {shown:?} {at}");
                    };
                    let room = menu.left() - last.right() - 2.0 * gap;
                    let needs = widths[held.len()];
                    assert!(
                        needs > room - 0.01,
                        "{next} needs {needs} and is in the menu with {room} of room {at}"
                    );
                }
                if opened_at != Some(held.len()) {
                    opened_at = Some(held.len());
                    open_menu(&mut window, &mut app);
                    let (_, rows) = menu(&window, "入力の切り替え");
                    let listed: Vec<&str> = rows
                        .iter()
                        .map(|(text, _)| text.as_str())
                        .filter(|text| MOVABLE.contains(text))
                        .collect();
                    assert_eq!(listed, rest, "the menu {at}");
                    window.draw(&mut app, vec![escape(), egui::Event::PointerGone]);
                    window.settle(&mut app);
                }
            }
        }
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
fn a_forming_paragraph_taller_than_the_chat_is_drawn_from_a_whole_line() {
    // The paragraph still forming in the preview 13-continuous-mini.
    const FORMING: &str = "来館者はいつでも展示リストに戻れます。画面のタップ数を数えるだけでなく、同じ資料をもう一度見つけられるかどうかを確認します。フィードバックカードでは、印象に残った内容、分かりにくい箇所、今後の改善について3つの質問をします。";
    for size in SIZES {
        let mut app = mini();
        app.settings.readout_enabled = true;
        app.audio_enabled = true;
        app.asr_ready = true;
        app.rows = vec![
            row(1, "音声", "各説明文は英語で約150語で構成されています。"),
            forming(row(2, "音声", FORMING)),
        ];
        app.current_live_id = Some(2);
        let window = drawn(&mut app, size);
        let body = app.live_chat.body().expect("the chat is drawn");
        let (words, color) = find_text(&window.output, FORMING);
        assert_eq!(color, theme::CHAT_TEXT_FORMING);
        assert!(
            words.top() < body.top(),
            "the paragraph starts inside the chat at {size:?}, so nothing is tested"
        );
        assert!(
            !line_cut_at_the_top(&window.output, body),
            "a line is cut at the top of the chat at {size:?}"
        );
        let end = dots(&window.output);
        assert_eq!(end.len(), 3);
        assert!(
            end.iter().all(|(dot, _)| body.contains(*dot)),
            "the end of the paragraph is not in view at {size:?}"
        );
    }
}

/// The room a bubble keeps above its first line.
const BUBBLE_MARGIN: f32 = 9.0;

/// Each line of text drawn whole inside the chat, with the text it is part of.
fn whole_lines(output: &egui::FullOutput, body: egui::Rect) -> Vec<(&str, egui::Rect)> {
    shapes(output)
        .into_iter()
        .filter_map(|(clip, shape)| match shape {
            egui::Shape::Text(text) => Some((clip.intersect(body).expand(0.5), text)),
            _ => None,
        })
        .flat_map(|(seen, text)| {
            text.galley.rows.iter().filter_map(move |row| {
                let line = row.rect().translate(text.pos.to_vec2());
                seen.contains_rect(line)
                    .then_some((text.galley.text(), line))
            })
        })
        .collect()
}

/// `earlier`, a bubble that starts above the top of the chat, is drawn from
/// its first whole line, so no more of the top is empty than a line and the
/// margin above it, and the newest message `newest` is in view.
fn assert_drawn_from_a_whole_line(
    window: &Window,
    app: &App,
    (earlier, newest): (&str, &str),
    size: (f32, f32),
) {
    let body = app.live_chat.body().expect("the chat is drawn");
    let lines = whole_lines(&window.output, body);
    let first = lines
        .iter()
        .map(|(_, line)| *line)
        .min_by(|a, b| a.top().total_cmp(&b.top()))
        .expect("a line is in view");
    let room = first.top() - body.top();
    assert!(
        room < first.height() + BUBBLE_MARGIN,
        "the top {room} points of the chat are empty at {size:?}"
    );
    assert!(
        box_of(&window.output, earlier, theme::CHAT_BUBBLE).top() < body.top(),
        "the earlier bubble starts inside the chat at {size:?}, so nothing is tested"
    );
    assert!(
        lines.iter().any(|(text, _)| *text == earlier),
        "no line of the earlier bubble is in view at {size:?}"
    );
    assert!(
        !line_cut_at_the_top(&window.output, body),
        "a line is cut at the top of the chat at {size:?}"
    );
    let (words, _) = find_text(&window.output, newest);
    assert!(
        body.contains_rect(words),
        "the newest message is not in view at {size:?}"
    );
}

#[test]
fn the_bubble_above_a_growing_message_is_drawn_from_a_whole_line_instead_of_leaving_the_top_empty()
{
    // One sentence longer than a bubble takes, so it is a bubble of its own.
    const LAST: &str = "直訳では分かりにくい場合は、スタッフが平易な言葉で意味を説明し、来館者がその場で質問できるように、展示室の入り口と出口に案内係を一人ずつ置きます。";
    const GROWING: &str = "来館者はいつでも展示リストに戻れます。画面のタップ数を数えます。";
    for size in SIZES {
        let mut app = mini();
        app.rows = vec![
            row(1, "音声", &format!("試行期間は6週間です。{LAST}")),
            forming(row(2, "音声", GROWING)),
        ];
        app.current_live_id = Some(2);
        let window = drawn(&mut app, size);
        assert_drawn_from_a_whole_line(&window, &app, (LAST, GROWING), size);
    }
}

#[test]
fn a_paragraph_that_ended_but_is_still_one_forming_bubble_is_drawn_from_a_whole_line() {
    const ENDED: &str = "来館者はいつでも展示リストに戻れます。画面のタップ数を数えるだけでなく、同じ資料をもう一度見つけられるかどうかを確認します。フィードバックカードでは、印象に残った内容、分かりにくい箇所、今後の改善について3つの質問をします。回答は毎週金曜日にスタッフが確認します。";
    const NEXT: &str = "次は展示室の地図です。";
    for size in SIZES {
        let mut app = mini();
        // The paragraph waits for its final translation, which cuts it into
        // bubbles. Until then it is one bubble, still forming.
        let mut ended = forming(row(1, "音声", ENDED));
        ended.record.paragraph.as_mut().unwrap().state = ParagraphState::Closing;
        app.rows = vec![ended, forming(row(2, "音声", NEXT))];
        app.current_live_id = Some(2);
        let window = drawn(&mut app, size);
        assert_drawn_from_a_whole_line(&window, &app, (ENDED, NEXT), size);
        let body = app.live_chat.body().expect("the chat is drawn");
        assert!(
            box_of(&window.output, ENDED, theme::CHAT_BUBBLE).height() > body.height(),
            "the paragraph that ended fits in the chat at {size:?}"
        );
        assert_eq!(
            find_text(&window.output, ENDED).1,
            theme::CHAT_TEXT_FORMING,
            "the paragraph that ended is not drawn as forming at {size:?}"
        );
    }
}

/// A bubble that starts above the top of the chat, as drawn where the chat
/// does not follow the newest message and so draws every bubble whole.
struct Cut {
    /// The texts painted in the bubble.
    texts: Vec<String>,
    /// The texts of its lines that are whole in view.
    lines: Vec<String>,
    /// Its dots are in view, on a line of their own under its text.
    dots_alone: bool,
    /// The chat that follows the newest message draws it.
    drawn: bool,
}

/// What reached above the top of the chat while the window grew.
#[derive(Default)]
struct Top {
    bubbles: Vec<Cut>,
    /// Dates that started above the top.
    dates: usize,
}

/// The window grows a point at a time, so the top of the chat runs through
/// each place of a bubble of one line: the margin above the line, the line,
/// and the margin below it.
fn heights() -> impl Iterator<Item = f32> {
    (SMALLEST.1 as u16..=USUAL.1 as u16).map(f32::from)
}

/// Draws the chat at `width` and each of `heights`, at `pixels_per_point`,
/// while it follows the newest message, and again at the same place while it
/// does not, which draws every bubble in view whole. A bubble that starts
/// above the top is drawn only with a whole line of its text in view, as an
/// empty strip of bubble reads as an empty message; no line or date is drawn
/// cut; and following leaves out no line of a message that is whole in view.
fn sweep(
    app: &mut App,
    width: f32,
    pixels_per_point: f32,
    heights: impl IntoIterator<Item = f32>,
) -> Top {
    let mut window = Window::new(width, SMALLEST.1);
    window.scale(pixels_per_point);
    // The scroll bar comes in over a few frames and narrows the chat.
    for _ in 0..4 {
        window.settle(app);
    }
    let mut top = Top::default();
    for height in heights {
        let size = (width, height);
        window.resize(width, height);
        app.settings.auto_scroll = true;
        window.settle(app);
        let body = app.live_chat.body().expect("the chat is drawn");
        let offset = app.live_chat.offset();
        let following = window.output.clone();
        let lines = whole_lines(&following, body);
        let drawn = filled(&following, theme::CHAT_BUBBLE);
        for bubble in &drawn {
            if bubble.top() < body.top() - 0.5 {
                assert!(
                    lines.iter().any(|(_, line)| bubble.contains_rect(*line)),
                    "a bubble is drawn at the top without a whole line of its text at {size:?}"
                );
            }
        }
        assert!(
            !line_cut_at_the_top(&following, body),
            "a line is drawn cut at the top at {size:?}"
        );
        assert!(
            filled(&following, theme::CHAT_PILL)
                .iter()
                .all(|pill| pill.top() >= body.top() - 0.5),
            "a date is drawn cut at the top at {size:?}"
        );

        app.settings.auto_scroll = false;
        window.draw(app, vec![]);
        assert!(
            app.live_chat.offset() == offset && app.live_chat.body() == Some(body),
            "the chat moved when it stopped following at {size:?}, so nothing is compared"
        );
        let still = &window.output;
        let bubbles = filled(still, theme::CHAT_BUBBLE);
        let in_view: Vec<_> = whole_lines(still, body)
            .into_iter()
            .filter(|(_, line)| bubbles.iter().any(|bubble| bubble.contains_rect(*line)))
            .collect();
        for (text, line) in &in_view {
            assert!(
                lines.contains(&(*text, *line)),
                "a line of {text} is whole in view but is not drawn while the chat follows at {size:?}"
            );
        }
        let painted: Vec<_> = shapes(still)
            .into_iter()
            .filter_map(|(_, shape)| match shape {
                egui::Shape::Text(text) => Some(text),
                _ => None,
            })
            .collect();
        for bubble in bubbles {
            if bubble.top() >= body.top() - 0.5 || bubble.bottom() <= body.top() + 0.5 {
                continue;
            }
            let inside: Vec<_> = painted
                .iter()
                .filter(|text| bubble.contains(text.pos))
                .collect();
            let text_bottom = inside
                .iter()
                .flat_map(|text| {
                    let y = text.pos.y;
                    text.galley
                        .rows
                        .iter()
                        .map(move |row| y + row.rect().bottom())
                })
                .fold(f32::MIN, f32::max);
            top.bubbles.push(Cut {
                texts: inside
                    .iter()
                    .map(|text| text.galley.text().to_owned())
                    .collect(),
                lines: in_view
                    .iter()
                    .filter(|(_, line)| bubble.contains_rect(*line))
                    .map(|(text, _)| (*text).to_owned())
                    .collect(),
                dots_alone: dots(still).iter().any(|(dot, _)| {
                    bubble.contains(*dot) && body.contains(*dot) && dot.y > text_bottom
                }),
                drawn: drawn.contains(&bubble),
            });
        }
        top.dates += filled(still, theme::CHAT_PILL)
            .iter()
            .filter(|pill| pill.top() < body.top() - 0.5 && pill.bottom() > body.top() + 0.5)
            .count();
    }
    top
}

#[test]
fn no_bubble_is_drawn_at_the_top_without_a_whole_line_of_its_text_in_view() {
    for (width, _) in SIZES {
        let mut app = mini();
        app.rows = talk(8);
        let top = sweep(&mut app, width, 1.0, heights());
        assert!(
            top.bubbles.iter().any(|bubble| bubble.drawn),
            "no bubble is drawn cut at the top at the width {width}, so nothing is tested"
        );
    }
}

#[test]
fn the_newest_message_cut_into_bubbles_leaves_no_empty_strip_at_the_top() {
    // A finished paragraph is cut at the ends of its sentences into bubbles,
    // and all of them are the newest message.
    const PARAGRAPH: &str = "来館者はいつでも展示リストに戻れます。画面のタップ数を数えるだけでなく、同じ資料をもう一度見つけられるかどうかを確認します。回答は毎週金曜日にスタッフが確認し、分かりにくいと書かれた説明文は、翌週の月曜日までに平易な言葉で書き直します。";
    for (width, _) in SIZES {
        let mut app = mini();
        app.rows = vec![
            row(1, "音声", "試行期間は6週間です。"),
            row(2, "音声", PARAGRAPH),
        ];
        let top = sweep(&mut app, width, 1.0, heights());
        assert!(
            top.bubbles.iter().any(|bubble| {
                bubble.lines.is_empty()
                    && bubble
                        .texts
                        .iter()
                        .any(|text| PARAGRAPH.contains(text.as_str()))
            }),
            "no bubble of the newest message is cut at the top without a whole line in view at the width {width}, so nothing is tested"
        );
    }
}

/// Words still being said that fill each line they take at `width`, so the
/// dots after them start a line of their own. With the scroll bar in, a line
/// of a bubble holds 11 letters at the smallest width and 16 at the usual one.
fn filling(width: f32) -> String {
    let letters = if width == SMALLEST.0 { 11 } else { 16 };
    "展示室の入り口と出口に案内係を置きます"
        .chars()
        .cycle()
        .take(2 * letters)
        .collect()
}

#[test]
fn a_message_still_forming_is_not_drawn_at_the_top_with_only_its_dots_in_view() {
    for (width, _) in SIZES {
        let mut app = mini();
        let words = filling(width);
        // The third has no words yet, only dots.
        let mut listening = forming(row(3, "音声", ""));
        listening.record.source.clear();
        app.rows = vec![
            forming(row(1, "音声", &words)),
            forming(row(2, "音声", &words)),
            listening,
            forming(row(4, "音声", &words)),
            row(5, "音声", "5番目の訳文です。"),
        ];
        let top = sweep(&mut app, width, 1.0, heights());
        assert!(
            top.bubbles.iter().any(|bubble| {
                bubble.dots_alone && bubble.lines.is_empty() && !bubble.texts.is_empty()
            }),
            "no message still forming is cut at the top with only its dots in view at the width {width}, so nothing is tested"
        );
        assert!(
            top.bubbles
                .iter()
                .any(|bubble| bubble.dots_alone && bubble.texts.is_empty()),
            "no message of dots alone is cut at the top with its dots in view at the width {width}, so nothing is tested"
        );
    }
}

#[test]
fn a_bubble_is_drawn_at_the_top_while_its_original_or_video_time_is_whole_in_view() {
    for (width, _) in SIZES {
        let mut app = mini();
        app.settings.live_show_original = true;
        app.rows = (1..=6)
            .map(|id| {
                let mut row = row(id, "YouTube", &format!("{id}番目の字幕の訳文です。"));
                row.record.source =
                    format!("Caption {id} of the talk, which goes on for a while in English.");
                row.record.start_ms = id * 7_000;
                row
            })
            .collect();
        let top = sweep(&mut app, width, 1.0, heights());
        let video = |line: &String| line.starts_with("動画 ");
        let original = |line: &String| line.starts_with("Caption ");
        assert!(
            top.bubbles
                .iter()
                .any(|bubble| !bubble.lines.is_empty() && bubble.lines.iter().all(video)),
            "no bubble is cut at the top with only its video time whole in view at the width {width}, so nothing is tested"
        );
        assert!(
            top.bubbles.iter().any(|bubble| {
                bubble.lines.iter().any(original)
                    && bubble
                        .lines
                        .iter()
                        .all(|line| original(line) || video(line))
            }),
            "no bubble is cut at the top with only its original in view at the width {width}, so nothing is tested"
        );
    }
}

#[test]
fn a_date_cut_at_the_top_is_not_drawn() {
    for (width, _) in SIZES {
        let mut app = mini();
        app.rows = (1..=8)
            .map(|id| {
                let mut row = row(id, "音声", &format!("{id}日の訳文です。"));
                row.record.created_at = format!("2026-11-{id:02}T10:00:00+09:00");
                row
            })
            .collect();
        let top = sweep(&mut app, width, 1.0, heights());
        assert!(
            top.dates > 0,
            "no date is cut at the top at the width {width}, so nothing is tested"
        );
    }
}

#[test]
fn at_125_percent_no_bubble_is_drawn_at_the_top_without_a_whole_line_of_its_text_in_view() {
    for (width, _) in SIZES {
        let mut app = mini();
        app.rows = talk(8);
        // A window is a whole number of pixels, and at 125% a pixel is 0.8
        // points.
        let heights = (325..=450_u16).map(|pixels| f32::from(pixels) / 1.25);
        let top = sweep(&mut app, width, 1.25, heights);
        assert!(
            top.bubbles.iter().any(|bubble| bubble.drawn),
            "no bubble is drawn cut at the top at the width {width}, so nothing is tested"
        );
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
            for behind in ["入力の切り替え", "表示", OTHER_FEATURES] {
                open_menu(&mut window, &mut app);
                assert!(
                    window.has("入力の切り替え"),
                    "{scene}: the menu is not open"
                );
                check(&window, scene, size);
                window.click(&mut app, behind);
                check(&window, scene, size);
                window.draw(&mut app, vec![escape()]);
                window.settle(&mut app);
                assert!(!window.has("入力の切り替え"), "{scene}: the menu is open");
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

fn menu(window: &Window, first: &str) -> (egui::Rect, Vec<(String, egui::Rect)>) {
    let frame = box_of(&window.output, first, theme::SURFACE);
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

/// Chooses `label` behind 入力の切り替え in the 「⋯」 menu, the way a pointer does.
fn choose(window: &mut Window, app: &mut App, label: &str) {
    open_menu(window, app);
    window.click(app, "入力の切り替え");
    window.click(app, label);
}

#[test]
fn the_toolbar_keeps_clear_then_topmost_then_copy_all_while_they_fit_and_the_menu_lists_the_rest() {
    for room in ROOM {
        let (size, readout) = (room.size, room.readout);
        let mut app = mini();
        app.settings.readout_enabled = readout;
        app.rows = talk(2);
        let mut window = drawn(&mut app, size);
        assert_eq!(
            toolbar(&window, &app),
            room.toolbar,
            "the toolbar at {size:?}, read-aloud {readout}"
        );
        open_menu(&mut window, &mut app);
        let (_, rows) = menu(&window, "入力の切り替え");
        let words: Vec<&str> = rows.iter().map(|(text, _)| text.as_str()).collect();
        assert_eq!(
            words, room.menu,
            "the menu at {size:?}, read-aloud {readout}"
        );
    }
}

#[test]
fn the_menu_has_its_rows_in_order_32_points_apart_and_fits_the_smallest_window() {
    for room in ROOM {
        let size = room.size;
        for screens in [1, 2] {
            let mut app = mini();
            app.settings.readout_enabled = room.readout;
            app.rows = talk(2);
            app.screens = two_screens().into_iter().take(screens).collect();
            let mut window = drawn(&mut app, size);
            assert!(!window.has("入力の切り替え"), "the menu opens unasked");
            assert!(
                window.at("⋯").right() > size.0 - 30.0,
                "⋯ is not at the right"
            );
            open_menu(&mut window, &mut app);
            let inside = egui::Rect::from_min_size(egui::Pos2::ZERO, size.into());
            let steps = |rows: &[(String, egui::Rect)], line_under: Option<&str>| {
                for pair in rows.windows(2) {
                    let step = pair[1].1.center().y - pair[0].1.center().y;
                    if Some(pair[0].0.as_str()) == line_under {
                        assert!(
                            step > 33.0 && step < 48.0,
                            "no separator under {}: {step}",
                            pair[0].0
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
            };

            let (frame, rows) = menu(&window, "入力の切り替え");
            assert!(
                inside.contains_rect(frame),
                "the menu leaves the window with {screens} screens at {size:?}: {frame:?}"
            );
            let words: Vec<&str> = rows.iter().map(|(text, _)| text.as_str()).collect();
            assert_eq!(
                words, room.menu,
                "{screens} screens at {size:?}, read-aloud {}",
                room.readout
            );
            steps(&rows, Some(room.menu[room.menu.len() - 2]));

            window.click(&mut app, OTHER_FEATURES);
            let (frame, rows) = menu(&window, "範囲翻訳");
            assert!(
                inside.contains_rect(frame),
                "{OTHER_FEATURES} leaves the window with {screens} screens at {size:?}: {frame:?}"
            );
            let mut expected = vec!["範囲翻訳", "画面選択", "字幕ウィンドウ"];
            if screens == 1 {
                expected.retain(|row| *row != "画面選択");
            }
            let words: Vec<&str> = rows.iter().map(|(text, _)| text.as_str()).collect();
            assert_eq!(words, expected, "{screens} screens at {size:?}");
            steps(&rows, None);
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
fn the_display_and_input_submenus_open_left_of_the_menu_inside_the_window_at_every_small_size() {
    // A submenu opens as soon as the pointer is on its row, so one drawn over
    // the menu stands in the way of the rows under that row.
    let submenus = [
        (
            "表示",
            [
                "原文をすべて表示",
                "末尾へ自動スクロール",
                "ダークモード",
                "訳文の文字サイズ",
            ],
        ),
        // With the captions on, and 画面の文字 listed while it is the input,
        // 入力の切り替え has its most rows.
        (
            "入力の切り替え",
            [
                "PC音声だけ",
                "YouTube字幕だけ",
                "自動（字幕を優先）",
                SCREEN,
            ],
        ),
    ];
    let mut over = Vec::new();
    for width in [400.0, 520.0] {
        for height in [260.0, 270.0, 280.0, 300.0, 360.0] {
            for readout in [false, true] {
                for (row, options) in submenus {
                    let mut app = with_youtube_captions(mini());
                    app.listen_input = LiveSourceMode::Screen;
                    app.settings.readout_enabled = readout;
                    let mut window = drawn(&mut app, (width, height));
                    open_menu(&mut window, &mut app);
                    let (frame, _) = menu(&window, "入力の切り替え");
                    window.click(&mut app, row);
                    let at = format!("{row} at {width}x{height}, read-aloud {readout}");
                    for option in options {
                        assert!(window.has(option), "{option} is not behind {at}");
                    }
                    let first = window.at(options[0]);
                    let submenu = filled(&window.output, theme::SURFACE)
                        .into_iter()
                        .filter(|rect| *rect != frame && rect.contains_rect(first))
                        .min_by(|a, b| a.area().total_cmp(&b.area()))
                        .unwrap_or_else(|| panic!("{at} opens no submenu"));
                    let inside =
                        egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(width, height));
                    if submenu.right() > frame.left() || !inside.contains_rect(submenu) {
                        over.push(format!("{at}: {submenu:?}, the menu {frame:?}"));
                    }
                }
            }
        }
    }
    assert_eq!(
        over,
        Vec::<String>::new(),
        "these submenus are drawn over the menu or leave the window"
    );
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
    assert!(window.has("入力の切り替え"));

    window.draw(&mut app, vec![escape()]);
    window.settle(&mut app);
    assert!(!window.has("入力の切り替え"), "Esc did not close the menu");
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
        let mut app = with_youtube_captions(mini());
        let mut window = drawn(&mut app, size);
        open_menu(&mut window, &mut app);
        assert!(!window.has("自動（字幕を優先）"), "they are behind the row");
        window.click(&mut app, "入力の切り替え");
        let inside = egui::Rect::from_min_size(egui::Pos2::ZERO, size.into());
        let tops: Vec<f32> = INPUTS
            .iter()
            .map(|(label, _, _)| {
                let words = window.at(label);
                assert!(inside.contains_rect(words), "{label} leaves the window");
                words.top()
            })
            .collect();
        assert!(
            tops.windows(2).all(|pair| pair[0] < pair[1]),
            "the inputs are out of order at {size:?}: {tops:?}"
        );
        assert!(
            !window.has(SCREEN),
            "{SCREEN} is listed while it is not the input"
        );
        assert_eq!(marked(&window), ["PC音声だけ"], "the default");

        window.click(&mut app, "自動（字幕を優先）");
        open_menu(&mut window, &mut app);
        window.click(&mut app, "入力の切り替え");
        assert_eq!(
            marked(&window),
            ["自動（字幕を優先）"],
            "what the next 聞く turns on"
        );

        app.caption_enabled = true;
        window.settle(&mut app);
        assert_eq!(marked(&window), ["YouTube字幕だけ"], "what is on now");
    }
}

#[test]
fn the_mini_inputs_are_pc_audio_alone_until_youtube_captions_are_turned_on() {
    for size in SIZES {
        let mut app = mini();
        let mut window = drawn(&mut app, size);
        open_menu(&mut window, &mut app);
        window.click(&mut app, "入力の切り替え");
        let listed: Vec<&str> = ["PC音声だけ", "YouTube字幕だけ", "自動（字幕を優先）"]
            .into_iter()
            .filter(|label| window.has(label))
            .collect();
        assert_eq!(
            listed,
            ["PC音声だけ"],
            "the captions are an option at {size:?}"
        );
        assert_eq!(marked(&window), ["PC音声だけ"]);

        let mut app = with_youtube_captions(mini());
        let mut window = drawn(&mut app, size);
        open_menu(&mut window, &mut app);
        window.click(&mut app, "入力の切り替え");
        let tops: Vec<f32> = ["PC音声だけ", "YouTube字幕だけ", "自動（字幕を優先）"]
            .iter()
            .map(|label| window.at(label).top())
            .collect();
        assert!(
            tops.windows(2).all(|pair| pair[0] < pair[1]),
            "the inputs are out of order at {size:?}: {tops:?}"
        );
        assert_eq!(
            marked(&window),
            ["PC音声だけ"],
            "the first input is still the PC audio"
        );
    }
}

#[test]
fn an_input_chosen_while_stopped_waits_for_the_button_and_one_chosen_while_listening_is_on_at_once()
{
    for (label, mode, on) in INPUTS {
        let mut app = with_youtube_captions(mini());
        let mut window = drawn(&mut app, USUAL);
        choose(&mut window, &mut app, label);
        assert!(app.listen_input == mode, "{label} is not remembered");
        assert_eq!(armed(&app), (false, false), "{label} started by itself");
        assert!(app.screen_feed.is_none() && !app.audio_confirmation);
        assert!(app.status.is_empty(), "{label}: {}", app.status);

        window.click(&mut app, "● 聞く");
        assert_eq!(armed(&app), on, "{label}: 聞く");
    }

    for (label, mode, on) in INPUTS {
        let mut app = with_youtube_captions(mini());
        app.listen_input = LiveSourceMode::Auto;
        let mut window = drawn(&mut app, USUAL);
        window.click(&mut app, "● 聞く");
        assert_eq!(armed(&app), (true, true));
        let generation = app.caption_generation;
        choose(&mut window, &mut app, label);
        assert!(app.listen_input == mode);
        assert_eq!(armed(&app), on, "{label}");
        if mode == LiveSourceMode::Auto {
            assert_eq!(app.caption_generation, generation, "{label} restarted");
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
fn the_display_menu_of_the_mini_view_has_dark_mode_right_after_auto_scroll_inside_the_window() {
    for size in SIZES {
        let mut app = mini();
        let mut window = drawn(&mut app, size);
        open_menu(&mut window, &mut app);
        window.click(&mut app, "表示");
        assert!(
            window.has("ダークモード"),
            "{size:?}: ダークモード is not behind 表示"
        );
        let options = [
            "原文をすべて表示",
            "末尾へ自動スクロール",
            "ダークモード",
            "訳文の文字サイズ",
        ];
        let tops: Vec<f32> = options
            .iter()
            .map(|option| window.at(option).top())
            .collect();
        assert!(
            tops.windows(2).all(|pair| pair[0] < pair[1]),
            "{size:?}: ダークモード is not right after 末尾へ自動スクロール: {tops:?}"
        );
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(size.0, size.1));
        for option in options {
            assert!(
                screen.contains_rect(window.at(option)),
                "{size:?}: {option} leaves the window"
            );
        }
        assert_eq!(
            dark_chat_saved(&app),
            serde_json::json!(false),
            "{size:?}: it does not start off"
        );
        window.click(&mut app, "ダークモード");
        assert_eq!(
            dark_chat_saved(&app),
            serde_json::json!(true),
            "{size:?}: it is not the setting the main window saves"
        );
    }
}

#[test]
fn dark_mode_draws_the_mini_chat_dark_and_turning_it_off_brings_today_s_colours_back_at_once() {
    // The talk of the mini view's preview: the last two messages are in view,
    // with a time, a name, a message still forming and the scroll bar.
    let words = "翻訳は保存されるので、あとから読み返せます。";
    let mut app = with_dark_chat(mini());
    app.rows = crate::demo_talk(4);
    app.current_live_id = Some(4);
    let mut window = drawn(&mut app, USUAL);
    let panel = app.live_chat.panel().expect("the chat is drawn");
    assert_drawn_dark(&window.output, panel, words);
    assert_eq!(
        light_colours(&window.output, panel),
        Vec::<&str>::new(),
        "parts of the mini chat are still light"
    );

    open_menu(&mut window, &mut app);
    window.click(&mut app, "表示");
    window.click(&mut app, "ダークモード");
    assert_eq!(background(&window.output, panel), theme::CHAT_BACKGROUND);
    assert_eq!(fill_behind(&window.output, words), theme::CHAT_BUBBLE);
    assert_eq!(find_text(&window.output, words).1, theme::CHAT_TEXT);
}

#[test]
fn the_topmost_switch_of_the_menu_raises_the_window_and_lowers_it_again() {
    // At its smallest with the read-aloud switch, the toolbar has no room for
    // 最前面.
    let mut app = mini();
    app.settings.readout_enabled = true;
    let mut window = drawn(&mut app, SMALLEST);
    open_menu(&mut window, &mut app);
    release_on(&mut window, &mut app, "最前面");
    assert_eq!(window_level(&window), [egui::WindowLevel::AlwaysOnTop]);
    assert!(app.window_mode.always_on_top());

    window.settle(&mut app);
    open_menu(&mut window, &mut app);
    release_on(&mut window, &mut app, "最前面");
    assert_eq!(window_level(&window), [egui::WindowLevel::Normal]);
    assert!(!app.window_mode.always_on_top());
}

#[test]
fn copy_all_in_the_menu_copies_the_talk_with_the_credit_also_from_behind_the_card() {
    for card in [false, true] {
        let mut app = mini();
        if card {
            app.capture(&egui::Context::default());
        }
        // At its smallest, the toolbar has no room for 全文コピー.
        let mut window = drawn(&mut app, SMALLEST);
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

/// What the last frame asked of the window's level: on top of the others or
/// back among them.
fn window_level(window: &Window) -> Vec<egui::WindowLevel> {
    window.output.viewport_output[&egui::ViewportId::ROOT]
        .commands
        .iter()
        .filter_map(|command| match command {
            egui::ViewportCommand::WindowLevel(level) => Some(*level),
            _ => None,
        })
        .collect()
}

/// The control is drawn as on: filled in the colour of what is chosen.
fn drawn_on(window: &Window, label: &str) -> bool {
    let words = window.at(label);
    filled(&window.output, theme::ACCENT_SOFT)
        .iter()
        .any(|face| face.contains_rect(words))
}

#[test]
fn the_topmost_switch_of_the_toolbar_raises_the_window_lowers_it_again_and_shows_which() {
    let mut app = mini();
    let mut window = drawn(&mut app, USUAL);
    assert!(!drawn_on(&window, "最前面"), "it starts on");

    release_on(&mut window, &mut app, "最前面");
    assert_eq!(window_level(&window), [egui::WindowLevel::AlwaysOnTop]);
    assert!(app.window_mode.always_on_top());
    window.settle(&mut app);
    assert!(
        drawn_on(&window, "最前面"),
        "it does not show that it is on"
    );

    release_on(&mut window, &mut app, "最前面");
    assert_eq!(window_level(&window), [egui::WindowLevel::Normal]);
    assert!(!app.window_mode.always_on_top());
    window.settle(&mut app);
    assert!(!drawn_on(&window, "最前面"), "it still shows that it is on");
}

#[test]
fn copy_all_in_the_toolbar_copies_the_talk_with_the_credit_also_from_behind_the_card() {
    for card in [false, true] {
        let mut app = mini();
        app.rows = talk(2);
        if card {
            app.capture(&egui::Context::default());
        }
        let mut window = drawn(&mut app, USUAL);
        hover_later(&mut window, &mut app, "全文コピー");
        assert!(
            has_text_containing(&window.output, COPY_ALL_HINT),
            "the hover text is not the chat header's"
        );
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
fn clear_in_the_toolbar_and_in_the_menu_starts_a_new_talk_as_the_chat_header_s_does() {
    for (size, readout, place) in [(USUAL, false, "toolbar"), (SMALLEST, true, "menu")] {
        let history = tempfile::tempdir().unwrap();
        let earlier = Journal::new(history.path()).unwrap();
        let message = row(1, "音声", "こんにちは。");
        earlier.append(&message.record).unwrap();
        let earlier_talk = earlier.path.clone();
        let mut app = mini();
        app.settings.readout_enabled = readout;
        app.journal = Some(earlier);
        app.rows = vec![message];
        let mut window = drawn(&mut app, size);
        if place == "menu" {
            open_menu(&mut window, &mut app);
        }
        hover_later(&mut window, &mut app, "クリア");
        assert!(
            has_text_containing(&window.output, CLEAR_HINT),
            "the hover text in the {place} is not the chat header's"
        );

        window.click(&mut app, "クリア");
        assert!(app.rows.is_empty(), "the {place} left the chat as it was");
        let journal = app.journal.as_ref().expect("a new talk is begun");
        assert_ne!(journal.path, earlier_talk, "the {place} kept the talk");
        assert_eq!(journal.path.parent(), Some(history.path()));
        assert!(
            fs::read_to_string(&earlier_talk)
                .unwrap()
                .contains("Original 1."),
            "the earlier talk lost its messages"
        );
        assert!(app.window_mode.is_mini());
    }
}

#[test]
fn clear_waits_while_the_card_covers_the_chat_and_can_be_pressed_again_back_on_the_chat() {
    for (size, readout, place) in [(USUAL, false, "toolbar"), (SMALLEST, true, "menu")] {
        let open = |window: &mut Window, app: &mut App| {
            if place == "menu" {
                open_menu(window, app);
            }
        };
        let faded = {
            let mut app = mini();
            app.settings.readout_enabled = readout;
            let mut window = drawn(&mut app, size);
            open(&mut window, &mut app);
            window.ink("クリア")
        };
        let mut app = mini();
        app.settings.readout_enabled = readout;
        app.rows = talk(2);
        app.capture(&egui::Context::default());
        let mut window = drawn(&mut app, size);
        open(&mut window, &mut app);
        assert_eq!(
            window.ink("クリア"),
            faded,
            "クリア in the {place} can be pressed while the card covers the chat"
        );
        window.click(&mut app, "クリア");
        assert_eq!(
            app.rows.len(),
            2,
            "the {place} cleared a talk that is out of sight"
        );

        open(&mut window, &mut app);
        window.click(&mut app, "← チャットに戻る");
        assert!(shows_talk(&window, 2), "the chat is back");
        open(&mut window, &mut app);
        assert_ne!(
            window.ink("クリア"),
            faded,
            "クリア in the {place} is still faded back on the chat"
        );
    }
}

#[test]
fn the_chat_header_s_clear_is_not_held_back_by_a_card_the_mini_view_was_left_on() {
    let enabled = {
        let mut app = chat();
        app.rows = talk(2);
        drawn(&mut app, MAIN).ink("クリア")
    };
    let mut app = mini();
    app.rows = talk(2);
    app.capture(&egui::Context::default());
    let mut window = drawn(&mut app, USUAL);
    assert!(window.has(NO_SCREEN));
    window.draw(&mut app, vec![escape()]);
    assert!(!app.window_mode.is_mini());

    let mut window = drawn(&mut app, MAIN);
    window.click(&mut app, "← チャットに戻る");
    window.settle(&mut app);
    assert!(app.tab == Tab::Live);
    assert_eq!(
        window.ink("クリア"),
        enabled,
        "the chat header's クリア is faded after the mini view was left on the card"
    );
}

#[test]
fn the_topmost_switch_says_what_it_does_in_the_toolbar_and_in_the_menu() {
    for (size, readout, place) in [(USUAL, false, "toolbar"), (SMALLEST, true, "menu")] {
        let mut app = mini();
        app.settings.readout_enabled = readout;
        let mut window = drawn(&mut app, size);
        if place == "menu" {
            open_menu(&mut window, &mut app);
        }
        hover_later(&mut window, &mut app, "最前面");
        assert!(
            has_text_containing(&window.output, TOPMOST_HINT),
            "最前面 in the {place} does not say what it does"
        );
    }
}

#[test]
fn clear_and_copy_all_of_the_mini_view_can_be_pressed_exactly_when_the_chat_header_s_can() {
    type Change = fn(&mut App);
    let talks: [(&str, Change); 6] = [
        ("an empty talk", |_| {}),
        ("text and range translations", |app| {
            app.rows = vec![
                row(1, "文章", "文章の訳文です。"),
                row(2, "画面OCR", "範囲の訳文です。"),
            ]
        }),
        ("a PC-audio message", |app| {
            app.rows = vec![row(1, "音声", "こんにちは。")]
        }),
        ("a message not translated yet", |app| {
            app.rows = vec![row(1, "音声", "")]
        }),
        ("speech being recognised", |app| {
            app.asr_partial = " recognizing... ".into()
        }),
        ("blank speech", |app| app.asr_partial = "   ".into()),
    ];
    for (talk, change) in talks {
        let mut main = chat();
        change(&mut main);
        let header = drawn(&mut main, MAIN);
        for room in ROOM {
            let (size, readout) = (room.size, room.readout);
            for label in ["クリア", "全文コピー"] {
                let mut app = mini();
                app.settings.readout_enabled = readout;
                change(&mut app);
                let mut window = drawn(&mut app, size);
                if !room.toolbar.contains(&label) {
                    open_menu(&mut window, &mut app);
                }
                assert_eq!(
                    window.ink(label),
                    header.ink(label),
                    "{label} with {talk} at {size:?}, read-aloud {readout}"
                );
            }
        }
    }
}

#[test]
fn reading_a_range_from_the_menu_stays_in_the_mini_view_and_reports_on_the_card() {
    let ink = |ready: bool| {
        let mut app = mini();
        app.backend_ready = ready;
        let mut window = drawn(&mut app, USUAL);
        open_other_features(&mut window, &mut app);
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
    open_other_features(&mut window, &mut app);
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
    open_other_features(&mut window, &mut app);
    window.click(&mut app, "画面選択");
    window.click(&mut app, "2: DISPLAY2");
    assert_eq!(app.screen_index, 1);

    assert!(!app.subtitle_window.open);
    open_other_features(&mut window, &mut app);
    window.click(&mut app, "字幕ウィンドウ");
    assert!(app.subtitle_window.open);
    assert!(app.window_mode.is_mini());
}
