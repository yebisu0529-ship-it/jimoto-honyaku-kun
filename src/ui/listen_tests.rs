//! 「聞く」 turns on the input that was chosen, 「止める」 turns off every input,
//! and Ctrl+Alt+L is the same button.
use super::*;
use global_hotkey::hotkey::{Code, HotKey, Modifiers};

/// The chat with the translation model ready and speech recognition not on
/// this PC: Whisper, with no Python to run it. Nothing is installed, no file of
/// this PC is read and no audio is captured.
fn chat_without_speech() -> App {
    let mut app = App::inert();
    app.backend_ready = true;
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

#[test]
fn ctrl_alt_l_listens_in_the_automatic_mode_and_the_next_press_stops_every_input() {
    let ctx = egui::Context::default();
    let mut app = chat_without_speech();
    assert!(app.listen_input == LiveSourceMode::Auto);

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
