use super::home_tests::{Window, chat, row};
use super::*;
use crate::chat::testing::texts;
use plamo_translate::journal::Talk;

const READING: &str = "これまでの会話を読み込んでいます…";

fn said(id: u64, created_at: &str, translation: &str) -> Row {
    let mut said = row(id, "音声", translation);
    said.record.created_at = created_at.into();
    said
}

fn saved_talk(folder: &Path, rows: &[Row]) {
    let journal = Journal::new(folder).unwrap();
    for row in rows {
        journal.append(&row.record).unwrap();
    }
}

fn saving(folder: &Path, rows: Vec<Row>) -> App {
    let mut app = chat();
    let journal = Journal::new(folder).unwrap();
    for row in &rows {
        journal.append(&row.record).unwrap();
    }
    app.journal = Some(journal);
    app.rows = rows;
    app
}

fn reading(app: &mut App) -> Event {
    app.rx
        .recv_timeout(Duration::from_secs(10))
        .expect("the earlier talks are not read")
}

fn take_in(app: &mut App, event: Event) {
    app.tx.send(event).unwrap();
    app.process(&egui::Context::default());
}

fn read_earlier(app: &mut App) {
    let event = reading(app);
    take_in(app, event);
}

#[test]
fn after_clear_the_history_shows_the_removed_talk_in_a_fold_that_opens_on_a_click() {
    let folder = tempfile::tempdir().unwrap();
    let mut app = saving(
        folder.path(),
        vec![said(
            1,
            "2026-11-08T10:02:00+09:00",
            "消した会話の訳文です。",
        )],
    );
    let mut window = Window::new(860.0, 620.0);
    window.settle(&mut app);
    window.click(&mut app, "クリア");
    window.open_menu(&mut app);
    window.click(&mut app, "履歴");
    read_earlier(&mut app);
    window.settle(&mut app);

    let fold = "2026/11/08 10:02 からの会話 · 1件";
    assert!(window.has("これまでの会話"), "the heading is not drawn");
    assert!(window.has(fold), "the removed talk has no fold");
    assert!(
        !window.has("消した会話の訳文です。"),
        "the fold is open at first"
    );
    window.click(&mut app, fold);
    window.settle(&mut app);
    assert!(
        window.has("消した会話の訳文です。"),
        "the opened fold does not show the translation"
    );
}

#[test]
fn a_search_word_finds_a_record_of_an_earlier_talk_and_hides_a_talk_without_a_match() {
    let folder = tempfile::tempdir().unwrap();
    saved_talk(
        folder.path(),
        &[said(1, "2026-11-07T09:00:00+09:00", "金利は据え置きです。")],
    );
    saved_talk(
        folder.path(),
        &[said(2, "2026-11-08T09:00:00+09:00", "天気の話です。")],
    );
    let mut app = saving(folder.path(), vec![row(3, "音声", "いまの会話です。")]);
    app.tab = Tab::Files;
    let mut window = Window::new(860.0, 620.0);
    window.settle(&mut app);
    read_earlier(&mut app);
    window.settle(&mut app);
    let (with_match, without) = (
        "2026/11/07 09:00 からの会話 · 1件",
        "2026/11/08 09:00 からの会話 · 1件",
    );
    assert!(window.has(with_match) && window.has(without));
    assert!(
        !window.has("金利は据え置きです。"),
        "a fold is open at first"
    );

    app.search = "金利".into();
    window.settle(&mut app);
    window.settle(&mut app);
    assert!(window.has(with_match), "the talk with a match is not shown");
    assert!(
        window.has("金利は据え置きです。"),
        "the talk with a match is not open on it"
    );
    assert!(!window.has(without), "the talk without a match is shown");
    assert!(!window.has("いまの会話です。"));
    assert!(!window.has("一致する履歴がありません。"));

    app.search = "どこにもない言葉".into();
    window.settle(&mut app);
    assert!(
        window.has("一致する履歴がありません。"),
        "nothing matches, and the screen does not say so"
    );
}

#[test]
fn a_reading_that_arrives_after_another_clear_is_ignored() {
    let folder = tempfile::tempdir().unwrap();
    saved_talk(
        folder.path(),
        &[said(1, "2026-11-07T09:00:00+09:00", "最初の会話です。")],
    );
    let mut app = saving(
        folder.path(),
        vec![said(2, "2026-11-08T09:00:00+09:00", "二つめの会話です。")],
    );
    app.tab = Tab::Files;
    let mut window = Window::new(860.0, 620.0);
    window.settle(&mut app);
    let early = reading(&mut app);
    app.tab = Tab::Live;
    window.settle(&mut app);
    window.click(&mut app, "クリア");
    take_in(&mut app, early);

    app.tab = Tab::Files;
    window.settle(&mut app);
    read_earlier(&mut app);
    window.settle(&mut app);
    assert!(
        window.has("2026/11/08 09:00 からの会話 · 1件"),
        "the talk 「クリア」 removed is missing: the early reading was taken"
    );
    assert!(window.has("2026/11/07 09:00 からの会話 · 1件"));
}

#[test]
fn the_history_says_while_it_reads_and_with_no_earlier_talk_shows_no_heading() {
    let folder = tempfile::tempdir().unwrap();
    let mut app = saving(folder.path(), vec![row(1, "音声", "いまの会話です。")]);
    app.tab = Tab::Files;
    let mut window = Window::new(860.0, 620.0);
    window.settle(&mut app);
    assert!(window.has(READING), "the screen does not say it reads");
    read_earlier(&mut app);
    window.settle(&mut app);
    assert!(!window.has(READING));
    assert!(!window.has("これまでの会話"), "a heading over no talk");
    assert!(window.has("いまの会話です。"));
}

#[test]
fn the_history_says_what_it_could_not_read_and_what_it_left_out() {
    let folder = tempfile::tempdir().unwrap();
    let mut app = chat();
    app.journal = Some(Journal {
        path: folder.path().join("gone").join(
            "plamo-timeline-20261108T100000000000000Z-00000000-0000-0000-0000-000000000000.jsonl",
        ),
    });
    app.tab = Tab::Files;
    let mut window = Window::new(860.0, 620.0);
    window.settle(&mut app);
    read_earlier(&mut app);
    window.settle(&mut app);
    let failure = texts(&window.output)
        .into_iter()
        .find(|(text, _, _)| text.starts_with("これまでの会話を読めませんでした（"))
        .expect("the failure is not drawn");
    assert!(
        failure.0.ends_with('）') && failure.0.len() > "これまでの会話を読めませんでした（）".len()
    );
    assert_eq!(
        failure.2,
        theme::ERROR,
        "the failure is not in the error colour"
    );
    assert!(
        window.has("翻訳履歴はここに表示されます。"),
        "with nothing to show, the screen does not say where talks go"
    );

    app.earlier = Earlier::Read(EarlierTalks {
        talks: vec![Talk {
            file: folder.path().join("older.jsonl"),
            records: vec![said(1, "2026-11-07T09:00:00+09:00", "前の会話です。").record],
        }],
        older_left_out: true,
        unreadable: 2,
    });
    window.settle(&mut app);
    assert!(window.has("これより前の会話は、保存フォルダーのファイルにあります。"));
    assert!(window.has("読めない会話ファイルが2件あります。"));
    assert!(
        !window.has("翻訳履歴はここに表示されます。"),
        "an earlier talk is there to show"
    );
}

fn translations(window: &Window) -> Vec<String> {
    texts(&window.output)
        .into_iter()
        .map(|(text, _, _)| text)
        .filter(|text| text.starts_with("訳文"))
        .collect()
}

#[test]
fn a_long_talk_shows_its_newest_rows_and_more_on_request_until_the_search_changes() {
    let mut app = chat();
    app.tab = Tab::Files;
    app.rows = (1..=250)
        .map(|id| row(id, "音声", &format!("訳文{id}")))
        .collect();
    let mut window = Window::new(860.0, 40_000.0);
    window.settle(&mut app);
    let shown = translations(&window);
    assert_eq!(shown.len(), 200);
    assert_eq!(
        (shown[0].as_str(), shown[199].as_str()),
        ("訳文250", "訳文51")
    );
    window.click(&mut app, "さらに表示（残り50件）");
    assert_eq!(translations(&window).len(), 250);
    assert!(
        !texts(&window.output)
            .iter()
            .any(|(text, _, _)| text.starts_with("さらに表示"))
    );

    app.search = "訳文1".into();
    window.settle(&mut app);
    app.search.clear();
    window.settle(&mut app);
    assert_eq!(
        translations(&window).len(),
        200,
        "the count outlived the search"
    );

    app.rows.truncate(1);
    app.earlier = Earlier::Read(EarlierTalks {
        talks: vec![Talk {
            file: PathBuf::from("earlier.jsonl"),
            records: (1000..1201)
                .map(|id| said(id, "2026-11-07T09:00:00+09:00", &format!("訳文{id}")).record)
                .collect(),
        }],
        older_left_out: false,
        unreadable: 0,
    });
    window.settle(&mut app);
    window.click(&mut app, "2026/11/07 09:00 からの会話 · 201件");
    window.settle(&mut app);
    assert_eq!(translations(&window).len(), 1 + 200);
    window.click(&mut app, "さらに表示（残り1件）");
    assert_eq!(translations(&window).len(), 1 + 201);
}

#[test]
fn a_row_names_its_source_as_the_chat_does() {
    let mut app = chat();
    app.tab = Tab::Files;
    app.rows = [
        (1, "YouTube"),
        (2, "音声"),
        (3, "画面"),
        (4, "文章"),
        (5, "画面OCR"),
    ]
    .into_iter()
    .map(|(id, kind)| {
        let mut row = row(id, kind, "訳文です。");
        row.record.created_at = "2026-11-05T12:00:00+09:00".into();
        row
    })
    .collect();
    let mut window = Window::new(860.0, 2000.0);
    window.settle(&mut app);
    for name in ["YouTube", "PC音声", "画面", "文章", "範囲翻訳"] {
        let label = format!("2026/11/05 12:00 · {name}");
        assert!(window.has(&label), "{label} is not drawn");
    }
    for stored in ["音声", "画面OCR"] {
        let label = format!("2026/11/05 12:00 · {stored}");
        assert!(!window.has(&label), "{label} is drawn");
    }
}

#[test]
fn the_export_button_of_the_history_says_it_writes_the_current_talk() {
    let mut app = chat();
    app.tab = Tab::Files;
    app.rows.push(row(1, "音声", "いまの会話です。"));
    let mut window = Window::new(860.0, 620.0);
    window.settle(&mut app);
    window.hover(&mut app, "書き出し");
    window.settle(&mut app);
    assert!(window.has("いまの会話を、TXT・JSON・SRTに書き出します。"));
}

#[test]
fn clear_says_in_japanese_when_a_new_talk_cannot_be_started_and_keeps_the_talk() {
    let folder = tempfile::tempdir().unwrap();
    let not_a_folder = folder.path().join("a-file");
    fs::write(&not_a_folder, "").unwrap();
    let mut app = chat();
    app.journal = Some(Journal {
        path: not_a_folder.join("plamo-timeline.jsonl"),
    });
    app.rows.push(row(1, "音声", "こんにちは。"));
    let mut window = Window::new(860.0, 620.0);
    window.settle(&mut app);
    window.click(&mut app, "クリア");
    assert!(
        app.status.starts_with("新しい会話を始められませんでした（") && app.status.ends_with('）'),
        "{}",
        app.status
    );
    assert_eq!(app.rows.len(), 1, "the talk was removed");
}
