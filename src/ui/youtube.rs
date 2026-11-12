//! The screen used only to connect the browser extension that sends YouTube
//! captions: how far the extension has got, and the three steps to get there.
use super::*;
use plamo_translate::bridge::{Contact, Heard};

/// Captions count as arriving while the last one is at most this old.
const ARRIVING_WITHIN: Duration = Duration::from_secs(8);
/// How often the screen redraws, so that a new contact shows without input.
const REDRAW: Duration = Duration::from_millis(500);

/// How far the browser extension has got, from what the bridge last heard.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Link {
    NotYet,
    WrongKey,
    Connected { since: Duration },
    Captions,
}

/// The latest contact decides, except that a check between two lines of
/// captions keeps them arriving.
fn link(heard: Heard, now: Instant) -> Link {
    let Some((contact, at)) = heard.last else {
        return Link::NotYet;
    };
    let arriving = heard
        .caption
        .is_some_and(|caption| now.saturating_duration_since(caption) <= ARRIVING_WITHIN);
    match contact {
        Contact::WrongKey => Link::WrongKey,
        _ if arriving => Link::Captions,
        Contact::Caption | Contact::Checked => Link::Connected {
            since: now.saturating_duration_since(at),
        },
    }
}

impl Link {
    /// The word on the badge, its colour, and the fill behind it. Only a
    /// state that works gets the accent fill.
    fn badge(self) -> (&'static str, Color32, Color32) {
        match self {
            Self::NotYet => ("未接続", TEXT_MUTED, SURFACE_RAISED),
            Self::WrongKey => ("接続キーが違います", ERROR, SURFACE_RAISED),
            Self::Connected { .. } => ("接続済み", theme::SUCCESS, ACCENT_SOFT),
            Self::Captions => ("字幕を受信中", theme::SUCCESS, ACCENT_SOFT),
        }
    }

    /// How many of the three steps are behind the extension. A wrong key
    /// still means it is installed: it sent a key.
    fn steps_done(self) -> usize {
        match self {
            Self::NotYet => 0,
            Self::WrongKey => 1,
            Self::Connected { .. } => 2,
            Self::Captions => 3,
        }
    }
}

/// How long ago, in the words of the status line.
fn ago(since: Duration) -> String {
    match since.as_secs() / 60 {
        0 => "1分以内".to_owned(),
        minutes @ 1..60 => format!("{minutes}分前"),
        minutes => format!("{}時間前", minutes / 60),
    }
}

/// One numbered step: its marker, its title and what to do, then its controls.
/// The marker turns into a check once `done` steps include it.
fn step(
    ui: &mut egui::Ui,
    number: usize,
    done: usize,
    title: &str,
    text: &str,
    controls: impl FnOnce(&mut egui::Ui),
) {
    ui.horizontal_top(|ui| {
        let (mark, color) = if number <= done {
            ("✓".to_owned(), theme::SUCCESS)
        } else {
            (number.to_string(), ACCENT)
        };
        // As tall as the title's row, so the marker sits on its line.
        let row = ui.text_style_height(&egui::TextStyle::Body);
        ui.add_sized(
            [theme::SPACE_6, row],
            egui::Label::new(RichText::new(mark).strong().color(color)),
        );
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = theme::SPACE_2;
            ui.label(RichText::new(title).strong());
            ui.add(egui::Label::new(theme::label(text)).wrap());
            controls(ui);
        });
    });
}

impl App {
    /// How far the extension has got, as of now.
    fn extension_link(&self) -> Link {
        link(self.bridge_contact.get(), Instant::now())
    }

    /// The badge of the status card. The settings card shows it too.
    pub(super) fn link_badge(&self, ui: &mut egui::Ui) {
        let (word, color, fill) = self.extension_link().badge();
        badge(ui, word, color, fill);
    }

    /// The line under the badge: what the state means and what to do next.
    fn link_line(&self, link: Link) -> String {
        match link {
            Link::NotYet => "拡張機能からの接続は、まだありません。下の1と2で、拡張機能を入れて接続キーを貼ってください。".into(),
            Link::WrongKey => "拡張機能に貼った接続キーが、このアプリのものと違います。2の「接続キーをコピー」で写し、拡張機能の設定画面に貼り直して「保存」を押してください。".into(),
            Link::Connected { since } => format!(
                "拡張機能とつながりました（{}）。YouTubeで字幕を出すと、ここに「字幕を受信中」と出ます。",
                ago(since)
            ),
            Link::Captions if !self.settings.translate_youtube_captions => {
                "YouTube字幕が届いています。訳すときは、上の「YouTube字幕も訳す」をオンにしてください。".into()
            }
            Link::Captions => match (self.current_input(), self.shown_input()) {
                (Some(_), LiveSourceMode::Auto | LiveSourceMode::Captions) => {
                    "YouTube字幕を受け取り、チャットで訳しています。".into()
                }
                (None, LiveSourceMode::Auto | LiveSourceMode::Captions) => {
                    "YouTube字幕が届いています。チャットの「● 聞く」を押すと訳し始めます。".into()
                }
                (_, other) => format!(
                    "YouTube字幕が届いています。いまの入力は「{}」なので、字幕は訳しません。字幕を訳すときは、「入力の切り替え」で「YouTube字幕だけ」か「自動（字幕を優先）」を選びます。",
                    other.label()
                ),
            },
        }
    }

    pub(super) fn youtube_ui(&mut self, ui: &mut egui::Ui) {
        ui.ctx().request_repaint_after(REDRAW);
        let link = self.extension_link();
        page_column(ui, |ui| {
            self.page_header(
                ui,
                "YouTube字幕の接続",
                "ChromeやEdgeの拡張機能が、表示中のYouTube字幕をこのアプリに送ります。",
                |_| {},
            );
            ui.add_space(theme::SPACE_2);
            egui::ScrollArea::vertical()
                .id_salt("youtube-scroll")
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    // The steps are prose; a wide window keeps them to a readable measure.
                    let measure = theme::prose_width(theme::SIZE_LABEL);
                    ui.set_max_width(ui.available_width().min(measure));
                    ui.spacing_mut().item_spacing.y = theme::SPACE_3;
                    settings_card(ui, |ui| self.youtube_captions_switch(ui));
                    settings_card(ui, |ui| {
                        let (word, color, fill) = link.badge();
                        badge(ui, word, color, fill);
                        ui.add(egui::Label::new(theme::body(self.link_line(link))).wrap());
                    });
                    self.youtube_steps(ui, link.steps_done());
                    for note in [
                        "字幕はこのPC内の 127.0.0.1:17862 にだけ送られ、外部には送信されません。",
                        "YouTubeの窓を最小化したときなどは、YouTubeが字幕の表示を止めるため、字幕が届きません。「入力の切り替え」が「自動（字幕を優先）」なら、そのあいだはPC音声を訳します。",
                        "拡張機能のファイルを新しくしたときは、chrome://extensions（Edgeは edge://extensions）で、この拡張機能の再読み込みボタンを押してください。そのあと、開いているYouTubeのページも再読み込みしてください。",
                    ] {
                        ui.add(egui::Label::new(theme::caption(note)).wrap());
                    }
                });
        });
    }

    /// The three steps; the first `done` of them are checked off.
    fn youtube_steps(&mut self, ui: &mut egui::Ui, done: usize) {
        let folder = app_root().join("extension");
        let mut open_folder = false;
        let mut copy_key = false;
        settings_card(ui, |ui| {
            ui.spacing_mut().item_spacing.y = theme::SPACE_4;
            step(
                ui,
                1,
                done,
                "拡張機能を入れる",
                "Chromeは chrome://extensions、Edgeは edge://extensions を開き、開発者モードをオンにします。「パッケージ化されていない拡張機能を読み込む」を押し、次のフォルダーを選びます。",
                |ui| {
                    ui.add(
                        egui::Label::new(theme::caption(folder.display().to_string()))
                            .selectable(true)
                            .wrap(),
                    );
                    open_folder = ui.add(standard("フォルダーを開く")).clicked();
                },
            );
            step(
                ui,
                2,
                done,
                "接続キーを貼る",
                "ツールバーの拡張機能のアイコンを押すと、拡張機能の設定画面が開きます（アイコンが見えないときは、パズルの形のボタンから開きます）。下の「接続キーをコピー」を押して貼り付け、「保存」→「接続確認」を押します。",
                |ui| copy_key = ui.add(standard("接続キーをコピー")).clicked(),
            );
            step(
                ui,
                3,
                done,
                "YouTubeで字幕を出す",
                "YouTubeで動画を開き、CCをオンにして英語などの字幕を選びます。拡張機能を入れる前から開いていたYouTubeのページは、再読み込みします。字幕が届くと、上に「字幕を受信中」と出ます。訳すときは、いちばん上の「YouTube字幕も訳す」をオンにし、「入力の切り替え」で「YouTube字幕だけ」か「自動（字幕を優先）」を選んで、チャットの「● 聞く」を押します。",
                |_| {},
            );
        });
        if open_folder {
            let mut command = Command::new("explorer.exe");
            command.arg(&folder);
            platform::hide_console(&mut command);
            let _ = command.spawn();
        }
        if copy_key {
            // The key alone: the PLaMo credit that other copies carry would
            // make it a different key.
            ui.ctx().copy_text(self.settings.bridge_token.clone());
            self.status =
                "接続キーをコピーしました。拡張機能の設定画面に貼り付けてください。".into();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chat::testing::{box_of, find_text, press, texts};
    use crate::ui::home_tests::{Window, chat};
    use crate::ui::listen_tests::with_youtube_captions;

    const BADGES: [&str; 4] = ["未接続", "接続キーが違います", "接続済み", "字幕を受信中"];

    /// This screen, after the bridge has heard `heard`, in that order, just now.
    fn screen(heard: &[Contact]) -> App {
        let mut app = chat();
        app.tab = Tab::YouTube;
        for contact in heard {
            app.bridge_contact.note(*contact);
        }
        app
    }

    /// A window tall enough for the whole screen.
    fn drawn(app: &mut App) -> Window {
        let mut window = Window::new(1280.0, 1369.0);
        window.settle(app);
        window
    }

    fn masked(text: &str) -> bool {
        !text.is_empty() && text.chars().all(|c| c == '•')
    }

    /// The bridge heard only `contact`, at `at`.
    fn only(contact: Contact, at: Instant) -> Heard {
        Heard {
            last: Some((contact, at)),
            caption: (contact == Contact::Caption).then_some(at),
        }
    }

    #[test]
    fn the_link_is_how_far_the_last_contact_got() {
        let at = Instant::now();
        let later = |seconds| at + Duration::from_secs(seconds);
        assert_eq!(link(Heard::default(), at), Link::NotYet);
        assert_eq!(link(only(Contact::WrongKey, at), later(3)), Link::WrongKey);
        assert_eq!(
            link(only(Contact::Checked, at), later(90)),
            Link::Connected {
                since: Duration::from_secs(90)
            }
        );
        assert_eq!(link(only(Contact::Caption, at), later(2)), Link::Captions);
    }

    #[test]
    fn a_caption_counts_as_arriving_for_eight_seconds() {
        let at = Instant::now();
        let caption = only(Contact::Caption, at);
        assert_eq!(link(caption, at + Duration::from_secs(8)), Link::Captions);
        let since = Duration::from_millis(8_001);
        assert_eq!(link(caption, at + since), Link::Connected { since });
    }

    #[test]
    fn a_check_between_two_lines_keeps_the_captions_arriving() {
        let at = Instant::now();
        let later = |seconds| at + Duration::from_secs(seconds);
        let cleared = Heard {
            last: Some((Contact::Checked, later(1))),
            caption: Some(at),
        };
        assert_eq!(link(cleared, later(3)), Link::Captions);
        assert_eq!(
            link(cleared, later(9)),
            Link::Connected {
                since: Duration::from_secs(8)
            }
        );
        let wrong = Heard {
            last: Some((Contact::WrongKey, later(1))),
            caption: Some(at),
        };
        assert_eq!(link(wrong, later(3)), Link::WrongKey);
    }

    #[test]
    fn the_time_since_is_told_in_minutes_then_in_hours() {
        for (seconds, words) in [
            (30, "1分以内"),
            (59, "1分以内"),
            (60, "1分前"),
            (5 * 60, "5分前"),
            (3_599, "59分前"),
            (3_600, "1時間前"),
            (2 * 3_600, "2時間前"),
        ] {
            assert_eq!(ago(Duration::from_secs(seconds)), words, "{seconds} s");
        }
    }

    #[test]
    fn each_state_has_its_badge_and_checks_off_the_steps_behind_it() {
        for (heard, word, color, fill, checked) in [
            (vec![], "未接続", TEXT_MUTED, SURFACE_RAISED, 0),
            (
                vec![Contact::WrongKey],
                "接続キーが違います",
                ERROR,
                SURFACE_RAISED,
                1,
            ),
            (
                vec![Contact::Checked],
                "接続済み",
                theme::SUCCESS,
                ACCENT_SOFT,
                2,
            ),
            (
                vec![Contact::Caption],
                "字幕を受信中",
                theme::SUCCESS,
                ACCENT_SOFT,
                3,
            ),
        ] {
            let mut app = screen(&heard);
            let window = drawn(&mut app);
            for other in BADGES {
                assert_eq!(
                    window.has(other),
                    other == word,
                    "{other}, expecting {word}"
                );
            }
            let (text, ink) = find_text(&window.output, word);
            assert_eq!(ink, color, "{word}");
            // The green fill is kept for a state that works.
            let pill = box_of(&window.output, word, fill);
            assert!(
                pill.height() < 2.0 * text.height(),
                "{word} is in a pill of its fill: {pill:?}"
            );
            let checks: Vec<_> = texts(&window.output)
                .into_iter()
                .filter(|(text, _, _)| text == "✓")
                .collect();
            assert_eq!(checks.len(), checked, "{word}");
            assert!(checks.iter().all(|(_, _, ink)| *ink == theme::SUCCESS));
            for number in 1..=3 {
                let mark = number.to_string();
                assert_eq!(window.has(&mark), number > checked, "{word}: step {number}");
                if number > checked {
                    assert_eq!(find_text(&window.output, &mark).1, ACCENT);
                }
            }
        }
    }

    #[test]
    fn the_check_is_a_glyph_of_the_app_fonts_and_not_a_box() {
        let ctx = crate::chat::testing::context();
        let _ = ctx.run(egui::RawInput::default(), |_| {});
        let font = egui::FontId::proportional(theme::SIZE_BODY);
        assert!(ctx.fonts_mut(|fonts| fonts.has_glyph(&font, '✓')));
    }

    #[test]
    fn each_state_says_what_it_means_and_what_to_do_next() {
        for (heard, line) in [
            (
                vec![],
                "拡張機能からの接続は、まだありません。下の1と2で、拡張機能を入れて接続キーを貼ってください。",
            ),
            (
                vec![Contact::WrongKey],
                "拡張機能に貼った接続キーが、このアプリのものと違います。2の「接続キーをコピー」で写し、拡張機能の設定画面に貼り直して「保存」を押してください。",
            ),
            (
                vec![Contact::Checked],
                "拡張機能とつながりました（1分以内）。YouTubeで字幕を出すと、ここに「字幕を受信中」と出ます。",
            ),
            (
                vec![Contact::Caption],
                "YouTube字幕が届いています。チャットの「● 聞く」を押すと訳し始めます。",
            ),
        ] {
            let mut app = with_youtube_captions(screen(&heard));
            app.listen_input = LiveSourceMode::Auto;
            assert!(drawn(&mut app).has(line), "{line}");
        }
        let mut app = with_youtube_captions(screen(&[Contact::Caption]));
        app.caption_enabled = true;
        app.caption_paused = false;
        app.last_caption = Some(Instant::now());
        assert!(drawn(&mut app).has("YouTube字幕を受け取り、チャットで訳しています。"));

        // Between two lines the app counts the captions as paused; it still
        // listens to them.
        let mut app = with_youtube_captions(screen(&[Contact::Caption, Contact::Checked]));
        app.caption_enabled = true;
        app.audio_enabled = true;
        app.caption_paused = true;
        assert!(drawn(&mut app).has("YouTube字幕を受け取り、チャットで訳しています。"));

        // The button under the chat is 「■ 止める」 then, so the line does not
        // send the user there.
        let mut app = with_youtube_captions(screen(&[Contact::Caption]));
        app.audio_enabled = true;
        let window = drawn(&mut app);
        assert!(window.has(
            "YouTube字幕が届いています。いまの入力は「PC音声だけ」なので、字幕は訳しません。字幕を訳すときは、「入力の切り替え」で「YouTube字幕だけ」か「自動（字幕を優先）」を選びます。"
        ));
        assert!(
            !texts(&window.output)
                .iter()
                .any(|(text, _, _)| text.contains("「● 聞く」を押すと訳し始めます"))
        );
    }

    #[test]
    fn captions_the_chosen_input_will_not_translate_are_said_so_before_listening() {
        for (mode, label) in [
            (LiveSourceMode::Audio, "PC音声だけ"),
            (LiveSourceMode::Screen, "画面の文字"),
        ] {
            let mut app = with_youtube_captions(screen(&[Contact::Caption]));
            app.listen_input = mode;
            let window = drawn(&mut app);
            assert!(
                window.has(&format!(
                    "YouTube字幕が届いています。いまの入力は「{label}」なので、字幕は訳しません。字幕を訳すときは、「入力の切り替え」で「YouTube字幕だけ」か「自動（字幕を優先）」を選びます。"
                )),
                "{label}"
            );
            assert!(
                !texts(&window.output)
                    .iter()
                    .any(|(text, _, _)| text.contains("「● 聞く」を押すと訳し始めます")),
                "{label}"
            );
        }
    }

    const SWITCH: &str = "YouTube字幕も訳す";

    #[test]
    fn the_screen_starts_with_the_switch_and_arriving_captions_point_at_it_while_it_is_off() {
        let mut app = screen(&[Contact::Caption]);
        let mut window = drawn(&mut app);
        assert!(window.has(SWITCH), "the switch is not on the screen");
        assert!(
            window.at(SWITCH).bottom() < window.at("字幕を受信中").top(),
            "the switch is not above the status"
        );
        assert!(window.has(
            "YouTube字幕が届いています。訳すときは、上の「YouTube字幕も訳す」をオンにしてください。"
        ));

        window.click(&mut app, SWITCH);
        assert!(window.has(
            "YouTube字幕が届いています。いまの入力は「PC音声だけ」なので、字幕は訳しません。字幕を訳すときは、「入力の切り替え」で「YouTube字幕だけ」か「自動（字幕を優先）」を選びます。"
        ));
    }

    #[test]
    fn the_switch_is_one_setting_saved_with_the_others_in_the_settings_and_on_this_screen() {
        let saved = |app: &App| {
            serde_json::to_value(&app.settings).unwrap()["translate_youtube_captions"].clone()
        };
        let mut app = chat();
        assert_eq!(
            saved(&app),
            serde_json::json!(false),
            "it is not a saved setting that starts off"
        );

        app.tab = Tab::Settings;
        let mut window = Window::new(1280.0, 2400.0);
        window.settle(&mut app);
        assert!(
            !window.has(SWITCH),
            "it is shown before {OTHER_FEATURES} is opened"
        );
        window.click(&mut app, OTHER_FEATURES);
        assert!(window.has(SWITCH), "it is not behind {OTHER_FEATURES}");
        let (heading, switch) = (window.at("YouTube字幕"), window.at(SWITCH));
        assert!(
            window.at(OTHER_FEATURES).bottom() < heading.top() && heading.bottom() <= switch.top(),
            "it is not in a card of its own behind {OTHER_FEATURES}"
        );
        window.click(&mut app, SWITCH);
        assert_eq!(saved(&app), serde_json::json!(true));

        app.tab = Tab::YouTube;
        let mut window = drawn(&mut app);
        window.click(&mut app, SWITCH);
        assert_eq!(
            saved(&app),
            serde_json::json!(false),
            "this screen has the same setting"
        );

        // A settings file from before the switch keeps what it has, and the
        // switch is off.
        let before: Settings =
            serde_json::from_value(serde_json::json!({"bridge_token": "the extension's key"}))
                .unwrap();
        assert_eq!(before.bridge_token, "the extension's key");
        assert_eq!(
            serde_json::to_value(&before).unwrap()["translate_youtube_captions"],
            serde_json::json!(false)
        );
    }

    #[test]
    fn the_steps_say_to_reload_a_youtube_page_that_was_open_before() {
        let window = drawn(&mut screen(&[]));
        let drawn: Vec<String> = texts(&window.output)
            .into_iter()
            .map(|(text, _, _)| text)
            .collect();
        let find = |start: &str| {
            drawn
                .iter()
                .find(|text| text.starts_with(start))
                .unwrap_or_else(|| panic!("{start} is not drawn"))
        };
        let step = find("YouTubeで動画を開き");
        assert!(step.contains("YouTubeのページは、再読み込み"), "{step}");
        let note = find("拡張機能のファイルを新しくしたときは");
        assert!(note.contains("YouTubeのページも再読み込み"), "{note}");
    }

    #[test]
    fn the_latest_contact_wins() {
        let window = drawn(&mut screen(&[Contact::WrongKey, Contact::Checked]));
        assert!(window.has("接続済み") && !window.has("接続キーが違います"));
        let window = drawn(&mut screen(&[Contact::Caption, Contact::WrongKey]));
        assert!(window.has("接続キーが違います") && !window.has("字幕を受信中"));
        let window = drawn(&mut screen(&[Contact::Caption, Contact::Checked]));
        assert!(
            window.has("字幕を受信中") && !window.has("接続済み"),
            "the caption cleared between two lines"
        );
    }

    #[test]
    fn the_screen_redraws_by_itself_so_a_new_contact_shows_without_input() {
        let repaint = |tab| {
            let mut app = chat();
            app.tab = tab;
            drawn(&mut app).output.viewport_output[&egui::ViewportId::ROOT].repaint_delay
        };
        assert!(repaint(Tab::YouTube) <= Duration::from_millis(500));
        assert!(
            repaint(Tab::Settings) > Duration::from_secs(1),
            "another screen asks for no redraw, so the delay is this screen's"
        );
    }

    #[test]
    fn the_menu_row_opens_the_screen_and_the_back_button_returns_to_the_chat() {
        let mut app = chat();
        let mut window = Window::new(860.0, 620.0);
        window.open_other_features(&mut app);
        window.hover(&mut app, "YouTube字幕の接続");
        // egui holds a tooltip back until the click that opened the menu is a
        // moment old and the pointer has rested.
        window.settle(&mut app);
        assert!(window.has("ChromeやEdgeの拡張機能とつなぎ、YouTubeの字幕を受け取ります。"));
        window.click(&mut app, "YouTube字幕の接続");
        assert!(app.tab == Tab::YouTube);
        for shown in [
            "YouTube字幕の接続",
            "ChromeやEdgeの拡張機能が、表示中のYouTube字幕をこのアプリに送ります。",
            "← チャットに戻る",
        ] {
            assert!(window.has(shown), "{shown} is not drawn");
        }
        window.click(&mut app, "← チャットに戻る");
        assert!(app.tab == Tab::Live);
    }

    #[test]
    fn the_settings_card_has_the_same_badge_and_a_way_to_the_screen_and_nothing_else() {
        for listening in [false, true] {
            let mut app = chat();
            app.tab = Tab::Settings;
            app.caption_enabled = listening;
            app.bridge_contact.note(Contact::WrongKey);
            let mut window = Window::new(1280.0, 2000.0);
            window.settle(&mut app);
            for shown in [
                "YouTube字幕の接続",
                "接続キーが違います",
                "拡張機能の入れ方と接続キーは、専用の画面にまとめました。",
                "接続の画面を開く",
            ] {
                assert!(window.has(shown), "{shown} is not on the card");
            }
            for gone in [
                "字幕を受け取る",
                "接続待ち",
                "停止中",
                "接続キーをコピー",
                "字幕連携フォルダー",
            ] {
                assert!(!window.has(gone), "{gone} is still on the card");
            }
            assert!(
                !texts(&window.output)
                    .iter()
                    .any(|(text, _, _)| masked(text))
            );
            window.click(&mut app, "接続の画面を開く");
            assert!(app.tab == Tab::YouTube);
            assert!(
                window.has("ChromeやEdgeの拡張機能が、表示中のYouTube字幕をこのアプリに送ります。")
            );
        }
    }

    #[test]
    fn copying_the_key_copies_exactly_the_key_and_the_key_is_never_drawn() {
        const KEY: &str = "4a7d1c9e-only-in-this-test";
        let mut app = screen(&[]);
        app.settings.bridge_token = KEY.into();
        let mut window = drawn(&mut app);
        let at = window.at("接続キーをコピー").center();
        window.draw(&mut app, vec![egui::Event::PointerMoved(at)]);
        window.draw(
            &mut app,
            vec![press(at, egui::PointerButton::Primary, true)],
        );
        window.draw(
            &mut app,
            vec![press(at, egui::PointerButton::Primary, false)],
        );
        let copied: Vec<&str> = window
            .output
            .platform_output
            .commands
            .iter()
            .filter_map(|command| match command {
                egui::OutputCommand::CopyText(text) => Some(text.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(copied, [KEY]);
        window.settle(&mut app);
        assert!(window.has("接続キーをコピーしました。拡張機能の設定画面に貼り付けてください。"));
        for (text, _, _) in texts(&window.output) {
            assert!(
                !text.contains(KEY) && !masked(&text),
                "the key is drawn: {text}"
            );
        }
    }

    #[test]
    fn nothing_on_the_screen_leaves_the_window_from_the_smallest_size_to_4k() {
        for (width, height) in [(860.0, 620.0), (860.0, 1600.0), (2560.0, 1369.0)] {
            for heard in [
                vec![],
                vec![Contact::WrongKey],
                vec![Contact::Checked],
                vec![Contact::Caption],
            ] {
                let mut app = screen(&heard);
                let mut window = Window::new(width, height);
                window.settle(&mut app);
                for (text, rect, _) in texts(&window.output) {
                    assert!(
                        rect.left() >= 0.0 && rect.right() <= width,
                        "{text} leaves the window at {width}x{height}: {rect:?}"
                    );
                }
            }
        }
    }
}
