//! The 用語集と文脈 section of the settings and its button for the finance terms.
use super::home_tests::{Window, chat};
use super::*;
use crate::chat::testing::{shapes, texts};
use plamo_translate::glossary::FOMC;

const BUTTON: &str = "金融（FOMC記者会見）の用語を入れる";

/// What the status row tells the user to do after terms went in.
const NEXT_STEPS: &str = "「設定を保存」で使います。ほかの話題では消してください。";

/// The settings screen with 「用語集と文脈」 open, in a window tall enough that
/// nothing scrolls. The page has two columns from 1100 points.
fn open(app: &mut App, width: f32) -> Window {
    app.tab = Tab::Settings;
    let mut window = Window::new(width, 2400.0);
    window.settle(app);
    window.click(app, "用語集と文脈");
    window
}

fn fomc_entries() -> Vec<(String, String)> {
    FOMC.iter()
        .map(|&(source, target)| (source.to_owned(), target.to_owned()))
        .collect()
}

#[test]
fn the_finance_button_sits_under_the_hint_and_adds_its_terms_once() {
    for width in [1280.0, 860.0] {
        let mut app = chat();
        app.glossary_input = "my term = 私の訳".into();
        let mut window = open(&mut app, width);

        let hint = window.at("1行に「原語 = 訳語」を入力します。例: inference = 推論");
        let context = window.at("文脈（翻訳する文章の背景）");
        let button = window.at(BUTTON);
        assert!(
            hint.bottom() <= button.top() && button.bottom() < context.top(),
            "{width}: the button is not between the hint and the box"
        );

        window.hover(&mut app, BUTTON);
        // egui shows a tooltip only once the pointer has been still for a moment.
        window.settle(&mut app);
        assert!(
            texts(&window.output).iter().any(|(text, _, _)| text
                == "FRBの記者会見でよく出る用語を、用語集の最後に足します。今ある行はそのまま残ります。ほかの話題を訳すときは消してください。"),
            "{width}: the hover text is missing"
        );

        window.click(&mut app, BUTTON);
        let mut expected = vec![("my term".to_owned(), "私の訳".to_owned())];
        expected.extend(fomc_entries());
        assert_eq!(
            glossary::parse(&app.glossary_input),
            Ok(expected),
            "{width}"
        );
        assert_eq!(
            app.status,
            format!("金融の用語を{}件入れました。{NEXT_STEPS}", FOMC.len())
        );
        assert!(window.has(&app.status), "{width}: the status row is silent");
        assert!(
            app.settings.translation.glossary.is_empty(),
            "{width}: the terms are used only after 設定を保存"
        );
        assert!(
            app.settings.translation.context.is_empty(),
            "{width}: the button must leave the 文脈 alone"
        );

        let once = app.glossary_input.clone();
        window.click(&mut app, BUTTON);
        assert_eq!(app.glossary_input, once, "{width}: the terms went in twice");
        assert_eq!(app.status, "金融の用語は、すべて用語集にあります。");
        assert!(window.has(&app.status), "{width}: the status row is silent");
    }
}

#[test]
fn a_full_box_takes_nothing_and_the_status_counts_what_was_left_out() {
    let mut app = chat();
    app.glossary_input = (0..glossary::MAX_ENTRIES)
        .map(|index| format!("term {index} = 訳語{index}\n"))
        .collect();
    let full = app.glossary_input.clone();
    let mut window = open(&mut app, 1280.0);
    window.click(&mut app, BUTTON);
    assert_eq!(app.glossary_input, full);
    assert!(
        glossary::parse(&app.glossary_input).is_ok(),
        "the box must stay valid, or 設定を保存 is off"
    );
    assert_eq!(
        app.status,
        format!(
            "{}項目を超える{}件は入れていません。",
            glossary::MAX_ENTRIES,
            FOMC.len()
        )
    );
}

#[test]
fn the_status_after_the_finance_button_tells_what_went_in_and_what_did_not() {
    let max = glossary::MAX_ENTRIES;
    let status = |entries, left_out| glossary_preset_status(&glossary::Added { entries, left_out });
    assert_eq!(
        status(12, 0),
        format!("金融の用語を12件入れました。{NEXT_STEPS}")
    );
    assert_eq!(
        status(3, 9),
        format!("金融の用語を3件入れました。{max}項目を超える9件は入れていません。{NEXT_STEPS}")
    );
    assert_eq!(status(0, 0), "金融の用語は、すべて用語集にあります。");
    assert_eq!(
        status(0, 9),
        format!("{max}項目を超える9件は入れていません。")
    );
}

/// What egui drew in the status row of the smallest window when it cut
/// `message` short, or `None` when it drew all of it. `texts` cannot tell. A
/// galley keeps the whole message even when the drawing ends in 「…」.
fn status_row_cuts_short(message: &str) -> Option<String> {
    let mut app = chat();
    app.tab = Tab::Settings;
    app.status = message.to_owned();
    let mut window = Window::new(860.0, 620.0);
    window.settle(&mut app);

    let galleys: Vec<_> = shapes(&window.output)
        .into_iter()
        .filter_map(|(_, shape)| match shape {
            egui::epaint::Shape::Text(text) if text.galley.text() == message => Some(&text.galley),
            _ => None,
        })
        .collect();
    assert_eq!(
        galleys.len(),
        1,
        "expected one {message}, found {}",
        galleys.len()
    );
    let galley = galleys[0];
    galley
        .elided
        .then(|| galley.rows.iter().map(|row| row.text()).collect())
}

#[test]
fn the_status_after_the_finance_button_is_not_cut_short_at_the_smallest_window() {
    assert!(
        status_row_cuts_short(&"あ".repeat(100)).is_some(),
        "the check cannot see a message that is too long"
    );
    for added in [
        glossary::Added {
            entries: FOMC.len(),
            left_out: 0,
        },
        glossary::Added {
            entries: 0,
            left_out: 0,
        },
    ] {
        let status = glossary_preset_status(&added);
        assert_eq!(
            status_row_cuts_short(&status),
            None,
            "the status row cuts {status} short"
        );
    }
}
