//! Local-only PLaMo 3 translation and local ASR transport.
use crate::glossary::{self, Fault, MAX_ENTRIES, MAX_SIDE_CHARS};
use anyhow::{Context, Result, bail, ensure};
use reqwest::{Url, blocking::Client, redirect::Policy};
use serde_json::{Value, json};
use std::{
    io::{BufRead, BufReader, Read},
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};

pub const LANGUAGES: &[(&str, &str)] = &[
    ("ja", "日本語"),
    ("en", "英語"),
    ("zh-hans", "中国語（簡体字）"),
    ("zh-hant", "中国語（繁体字）"),
    ("ko", "韓国語"),
    ("ar", "アラビア語"),
    ("az", "アゼルバイジャン語"),
    ("be", "ベラルーシ語"),
    ("bg", "ブルガリア語"),
    ("bn", "ベンガル語"),
    ("ca", "カタルーニャ語"),
    ("ceb", "セブアノ語"),
    ("de", "ドイツ語"),
    ("es", "スペイン語"),
    ("et", "エストニア語"),
    ("eu", "バスク語"),
    ("fa", "ペルシャ語"),
    ("fr", "フランス語"),
    ("gu", "グジャラート語"),
    ("he", "ヘブライ語"),
    ("hi", "ヒンディー語"),
    ("hr", "クロアチア語"),
    ("hu", "ハンガリー語"),
    ("hy", "アルメニア語"),
    ("id", "インドネシア語"),
    ("it", "イタリア語"),
    ("jv", "ジャワ語"),
    ("ka", "ジョージア語"),
    ("kk", "カザフ語"),
    ("km", "クメール語"),
    ("lt", "リトアニア語"),
    ("lv", "ラトビア語"),
    ("ml", "マラヤーラム語"),
    ("mr", "マラーティー語"),
    ("ms", "マレー語"),
    ("my", "ビルマ語"),
    ("ne", "ネパール語"),
    ("nl", "オランダ語"),
    ("pa", "パンジャーブ語"),
    ("pl", "ポーランド語"),
    ("ru", "ロシア語"),
    ("si", "シンハラ語"),
    ("sk", "スロバキア語"),
    ("sl", "スロベニア語"),
    ("sr", "セルビア語"),
    ("sw", "スワヒリ語"),
    ("ta", "タミル語"),
    ("te", "テルグ語"),
    ("th", "タイ語"),
    ("tl", "タガログ語"),
    ("ur", "ウルドゥー語"),
    ("uz", "ウズベク語"),
    ("vi", "ベトナム語"),
];

/// PLaMo 3 Translate's やさしい日本語. It is only a destination: no source
/// language, no entry in LANGUAGES and nothing the user can pick.
pub const EASY_JAPANESE: &str = "ja(easy)";

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct TranslationOptions {
    pub endpoint: String,
    pub source_lang: String,
    pub target_lang: String,
    pub style: String,
    pub context: String,
    pub glossary: Vec<(String, String)>,
}

impl Default for TranslationOptions {
    fn default() -> Self {
        Self {
            endpoint: "http://127.0.0.1:17860".into(),
            source_lang: "auto".into(),
            target_lang: "ja".into(),
            style: "plain".into(),
            context: String::new(),
            glossary: Vec::new(),
        }
    }
}

impl TranslationOptions {
    /// The second pass of a Japanese live translation. It reads the first
    /// pass's Japanese and rewrites it as やさしい日本語, so none of the user's
    /// style, context or glossary applies.
    pub fn easy_japanese(&self) -> Self {
        Self {
            endpoint: self.endpoint.clone(),
            source_lang: "ja".into(),
            target_lang: EASY_JAPANESE.into(),
            style: String::new(),
            context: String::new(),
            glossary: Vec::new(),
        }
    }
}

fn supported(code: &str) -> bool {
    LANGUAGES.iter().any(|(c, _)| *c == code)
}

fn safe_field(name: &str, value: &str, max_chars: usize) -> Result<()> {
    ensure!(
        !value.contains("<|plamo:"),
        "{name}にPLaMo予約トークンは使用できません。"
    );
    ensure!(
        value.chars().count() <= max_chars,
        "{name}が長すぎます（上限{max_chars}文字）。分割してください。"
    );
    Ok(())
}

fn validate_options(options: &TranslationOptions) -> Result<()> {
    ensure!(
        options.source_lang == "auto" || supported(&options.source_lang),
        "未対応の原文言語です。"
    );
    ensure!(
        supported(&options.target_lang) || options.target_lang == EASY_JAPANESE,
        "未対応の翻訳先言語です。"
    );
    ensure!(
        matches!(options.style.as_str(), "" | "default" | "plain" | "polite"),
        "文体には標準・常体・敬体のいずれかを指定してください。"
    );
    safe_field("コンテキスト", &options.context, 4000)?;
    ensure!(
        options.glossary.len() <= MAX_ENTRIES,
        "用語集は{MAX_ENTRIES}項目までです。"
    );
    for (source, target) in &options.glossary {
        for (name, side) in [("用語集の原語", source), ("用語集の訳語", target)] {
            match glossary::fault(side) {
                None => {}
                Some(Fault::Empty) => bail!("用語集に空の原語または訳語があります。"),
                Some(Fault::Reserved) => bail!("{name}にPLaMo予約トークンは使用できません。"),
                Some(Fault::TooLong) => {
                    bail!("{name}が長すぎます（上限{MAX_SIDE_CHARS}文字）。分割してください。")
                }
            }
        }
    }
    Ok(())
}

/// PFML3 deliberately omits BOS: llama.cpp inserts the model's BOS itself.
pub fn build_prompt(text: &str, options: &TranslationOptions) -> Result<String> {
    validate_options(options)?;
    safe_field("原文", text, 12000)?;
    ensure!(!text.trim().is_empty(), "翻訳する原文を入力してください。");
    let mut prompt = String::from("<|plamo:tag|>dataset<|plamo:msg|>translation");
    if !options.context.is_empty() {
        prompt.push_str("<|plamo:tag|>context<|plamo:msg|>");
        prompt.push_str(&options.context);
    }
    if !options.glossary.is_empty() {
        prompt.push_str("<|plamo:tag|>glossary");
        for (source, target) in &options.glossary {
            prompt.push_str("<|plamo:index|>");
            prompt.push_str(source);
            prompt.push_str("<|plamo:item|>");
            prompt.push_str(target);
        }
    }
    prompt.push_str("<|plamo:tag|>input");
    if options.source_lang != "auto" {
        prompt.push_str("<|plamo:key|>lang<|plamo:val|>");
        prompt.push_str(&options.source_lang);
    }
    prompt.push_str("<|plamo:msg|>");
    prompt.push_str(text);
    prompt.push_str("<|plamo:tag|>output<|plamo:key|>lang<|plamo:val|>");
    prompt.push_str(&options.target_lang);
    if options.target_lang == "ja" && matches!(options.style.as_str(), "plain" | "polite") {
        prompt.push_str("<|plamo:key|>form<|plamo:val|>");
        prompt.push_str(&options.style);
    }
    prompt.push_str("<|plamo:msg|>");
    ensure!(
        prompt.chars().count() <= 24000,
        "原文・コンテキスト・用語集の合計が長すぎます。分割してください。"
    );
    Ok(prompt)
}

/// Restrict destination before sending any source text. No proxies or redirects.
fn local_base(endpoint: &str) -> Result<Url> {
    let mut url = Url::parse(endpoint.trim()).context("接続先URLの形式が不正です。")?;
    ensure!(
        matches!(url.scheme(), "http" | "https"),
        "接続先はHTTPのローカルURLを指定してください。"
    );
    ensure!(
        matches!(
            url.host_str(),
            Some("localhost" | "127.0.0.1" | "::1" | "[::1]")
        ),
        "原文を外部送信しないため、接続先はlocalhost・127.0.0.1・::1のみ使用できます。"
    );
    ensure!(
        url.username().is_empty()
            && url.password().is_none()
            && url.fragment().is_none()
            && url.query().is_none(),
        "接続先URLに認証情報・クエリ・フラグメントは指定できません。"
    );
    let path = url.path().trim_end_matches('/');
    ensure!(
        path.is_empty() || path == "/v1",
        "接続先にはサーバーのURL、または末尾が/v1のURLを指定してください。"
    );
    url.set_path("/");
    Ok(url)
}

fn client(timeout_secs: u64) -> Result<Client> {
    Client::builder()
        .no_proxy()
        .redirect(Policy::none())
        .connect_timeout(Duration::from_secs(3))
        .timeout(Duration::from_secs(timeout_secs))
        .build()
        .context("ローカル接続を初期化できませんでした。")
}

fn checked(
    response: reqwest::blocking::Response,
    operation: &str,
) -> Result<reqwest::blocking::Response> {
    let status = response.status();
    if !status.is_success() {
        let mut detail = String::new();
        let _ = response.take(2048).read_to_string(&mut detail);
        bail!(
            "{operation}に失敗しました（HTTP {status}）: {}",
            detail.trim()
        );
    }
    Ok(response)
}

fn read_json(response: reqwest::blocking::Response) -> Result<Value> {
    let mut body = Vec::new();
    response
        .take(1024 * 1024 + 1)
        .read_to_end(&mut body)
        .context("応答の読み取りに失敗しました。")?;
    ensure!(
        body.len() <= 1024 * 1024,
        "ローカルサーバーの応答が大きすぎます。"
    );
    serde_json::from_slice(&body).context("ローカルサーバーから不正なJSON応答を受信しました。")
}

#[derive(Clone, Debug)]
pub struct Translator {
    options: TranslationOptions,
    base: Url,
    client: Client,
}

impl Translator {
    pub fn new(options: TranslationOptions) -> Result<Self> {
        validate_options(&options)?;
        let base = local_base(&options.endpoint)?;
        Ok(Self {
            options,
            base,
            client: client(180)?,
        })
    }

    fn model(&self) -> Result<String> {
        checked(
            self.client
                .get(self.base.join("health")?)
                .timeout(Duration::from_secs(5))
                .send()
                .context("翻訳サーバーに接続できません。ローカルサーバーを起動してください。")?,
            "起動状態の確認",
        )?;
        let data = read_json(checked(
            self.client
                .get(self.base.join("v1/models")?)
                .timeout(Duration::from_secs(5))
                .send()
                .context("翻訳モデルを確認できませんでした。")?,
            "モデルの確認",
        )?)?;
        let models = data["data"]
            .as_array()
            .context("モデル一覧にdata配列がありません。")?;
        let ids: Vec<&str> = models.iter().filter_map(|m| m["id"].as_str()).collect();
        ensure!(
            !ids.is_empty(),
            "翻訳サーバーにモデルが読み込まれていません。"
        );
        // The launcher gives llama-server the model's file stem as its alias. Also
        // accept plamo3 and the other PLaMo 3 Translate file IDs.
        let model = ids
            .iter()
            .find(|id| {
                let id = id.to_ascii_lowercase();
                id == "plamo3"
                    || ((id.contains("plamo-3") || id.contains("plamo3"))
                        && id.contains("translate"))
            })
            .context(
                "PLaMo 3 Translateを確認できません。専用サーバーとモデルを使用してください。",
            )?;
        Ok((*model).to_owned())
    }

    pub fn health(&self) -> Result<String> {
        Ok(format!("接続済み: {}", self.model()?))
    }

    pub fn translate(&self, text: &str, on_partial: impl FnMut(&str)) -> Result<String> {
        self.translate_inner(text, None, on_partial)
    }

    /// Live revisions can be superseded without publishing or completing old work.
    pub fn translate_live(
        &self,
        text: &str,
        cancelled: &AtomicBool,
        on_partial: impl FnMut(&str),
    ) -> Result<String> {
        self.translate_inner(text, Some(cancelled), on_partial)
    }

    fn translate_inner(
        &self,
        text: &str,
        cancelled: Option<&AtomicBool>,
        on_partial: impl FnMut(&str),
    ) -> Result<String> {
        check_translation_cancelled(cancelled)?;
        let prompt = build_prompt(text, &self.options)?;
        let model = self.model()?;
        check_translation_cancelled(cancelled)?;
        let max_tokens = (text.chars().count().saturating_mul(2) + 256).clamp(2048, 8192);
        let response = self
            .client
            .post(self.base.join("v1/completions")?)
            .json(&json!({"model": model, "prompt": prompt, "temperature": 0,
                "repeat_penalty": 1.0, "max_tokens": max_tokens, "stream": true,
                "stop": ["<|plamo:tag|>", "<|plamo:bos|>"]}))
            .send()
            .context("翻訳サーバーへの接続が切れました。サーバーの状態を確認してください。")?;
        parse_sse_interruptible(
            BufReader::new(checked(response, "翻訳")?),
            on_partial,
            cancelled,
        )
    }
}

#[cfg(test)]
fn parse_sse(mut reader: impl BufRead, mut on_partial: impl FnMut(&str)) -> Result<String> {
    parse_sse_interruptible(&mut reader, &mut on_partial, None)
}

fn check_translation_cancelled(cancelled: Option<&AtomicBool>) -> Result<()> {
    ensure!(
        !cancelled.is_some_and(|flag| flag.load(Ordering::Acquire)),
        "文章の翻訳を中止しました。"
    );
    Ok(())
}

fn parse_sse_interruptible(
    mut reader: impl BufRead,
    mut on_partial: impl FnMut(&str),
    cancelled: Option<&AtomicBool>,
) -> Result<String> {
    let mut output = String::new();
    let mut event = String::new();
    let mut terminal = false;
    let mut done = false;
    loop {
        check_translation_cancelled(cancelled)?;
        let mut line = String::new();
        let count = reader
            .by_ref()
            .take(256 * 1024 + 1)
            .read_line(&mut line)
            .context("翻訳の受信が中断されました。未完了の訳文は確定されません。")?;
        ensure!(
            line.len() <= 256 * 1024,
            "翻訳サーバーのストリーム行が大きすぎます。"
        );
        if count == 0 {
            break;
        }
        let line = line.trim_end_matches(['\r', '\n']);
        if line.is_empty() {
            if event.is_empty() {
                continue;
            }
            if event.trim() == "[DONE]" {
                event.clear();
                done = true;
                break;
            }
            let value: Value =
                serde_json::from_str(&event).context("翻訳ストリームのJSONが不正です。")?;
            event.clear();
            if let Some(error) = value.get("error") {
                bail!("翻訳サーバーエラー: {error}");
            }
            // OpenAI-compatible llama.cpp streaming chunks.
            let choices = value["choices"]
                .as_array()
                .context("翻訳ストリームにchoices配列がありません。")?;
            if let Some(choice) = choices.first() {
                if let Some(text) = choice["text"].as_str() {
                    ensure!(
                        !terminal || text.is_empty(),
                        "終了後に翻訳データを受信しました。"
                    );
                    output.push_str(text);
                    ensure!(output.len() <= 1024 * 1024, "翻訳結果が大きすぎます。");
                    if !text.is_empty() {
                        on_partial(&output);
                    }
                }
                if let Some(reason) = choice["finish_reason"].as_str() {
                    ensure!(
                        reason != "length",
                        "出力上限に達して翻訳が途中で止まりました。原文を分割してください。"
                    );
                    ensure!(
                        reason == "stop",
                        "翻訳が正常終了しませんでした（{reason}）。"
                    );
                    terminal = true;
                }
            }
        } else if let Some(data) = line.strip_prefix("data:") {
            if !event.is_empty() {
                event.push('\n');
            }
            event.push_str(data.strip_prefix(' ').unwrap_or(data));
            ensure!(
                event.len() <= 256 * 1024,
                "翻訳サーバーのイベントが大きすぎます。"
            );
        } else if line.starts_with(':')
            || line.starts_with("event:")
            || line.starts_with("id:")
            || line.starts_with("retry:")
        {
            // SSE comments/metadata are not model output.
        } else {
            bail!("翻訳サーバーからSSE形式ではない応答を受信しました。");
        }
    }
    ensure!(
        event.is_empty(),
        "翻訳の受信がイベントの途中で切れました。もう一度実行してください。"
    );
    ensure!(
        terminal || done,
        "翻訳の受信が途中で切れました。もう一度実行してください。"
    );
    let result = output.trim().to_owned();
    ensure!(
        !result.is_empty(),
        "翻訳結果が空です。モデルと原文を確認してください。"
    );
    ensure!(
        !result.contains("<|plamo:"),
        "翻訳に予約トークンが混入しました。原文を分割して再実行してください。"
    );
    if result != output {
        on_partial(&result);
    }
    Ok(result)
}

pub fn transcribe(endpoint: &str, wav: Vec<u8>, language: &str) -> Result<String> {
    let base = local_base(endpoint)?;
    ensure!(
        language == "auto" || supported(language),
        "未対応の音声言語です。"
    );
    ensure!(!wav.is_empty(), "音声データが空です。");
    ensure!(
        wav.len() <= 32 * 1024 * 1024,
        "音声データが大きすぎます。短く分割してください。"
    );
    let response = client(30)?
        .post(base.join("transcribe")?)
        .query(&[("language", language)])
        .header("Content-Type", "application/octet-stream")
        .body(wav)
        .send()
        .context(
            "音声認識サーバーに接続できませんでした。ローカルASRの起動状態を確認してください。",
        )?;
    let data = read_json(checked(response, "音声認識")?)?;
    let text = data["text"]
        .as_str()
        .context("音声認識結果にtextがありません。")?
        .trim();
    ensure!(
        !text.is_empty(),
        "音声を認識できませんでした。入力音量と言語を確認してください。"
    );
    Ok(text.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn pfml_keeps_context_glossary_order_and_no_bos() {
        let options = TranslationOptions {
            source_lang: "en".into(),
            context: "説明書".into(),
            glossary: vec![("dashboard".into(), "管理画面".into())],
            ..Default::default()
        };
        assert_eq!(
            build_prompt("Open dashboard.", &options).unwrap(),
            concat!(
                "<|plamo:tag|>dataset<|plamo:msg|>translation",
                "<|plamo:tag|>context<|plamo:msg|>説明書",
                "<|plamo:tag|>glossary<|plamo:index|>dashboard<|plamo:item|>管理画面",
                "<|plamo:tag|>input<|plamo:key|>lang<|plamo:val|>en<|plamo:msg|>Open dashboard.",
                "<|plamo:tag|>output<|plamo:key|>lang<|plamo:val|>ja<|plamo:key|>form<|plamo:val|>plain<|plamo:msg|>"
            )
        );
    }

    #[test]
    fn easy_japanese_rewrites_japanese_and_ignores_the_users_settings() {
        let rewrite = concat!(
            "<|plamo:tag|>dataset<|plamo:msg|>translation",
            "<|plamo:tag|>input<|plamo:key|>lang<|plamo:val|>ja<|plamo:msg|>管理画面を開いてください。",
            "<|plamo:tag|>output<|plamo:key|>lang<|plamo:val|>ja(easy)<|plamo:msg|>"
        );
        let direct = TranslationOptions {
            source_lang: "ja".into(),
            target_lang: EASY_JAPANESE.into(),
            ..Default::default()
        };
        assert_eq!(
            build_prompt("管理画面を開いてください。", &direct).unwrap(),
            rewrite,
            "no form, even with the plain style"
        );

        let user = TranslationOptions {
            source_lang: "en".into(),
            style: "polite".into(),
            context: "説明書".into(),
            glossary: vec![("dashboard".into(), "管理画面".into())],
            ..Default::default()
        };
        assert_eq!(
            build_prompt("管理画面を開いてください。", &user.easy_japanese()).unwrap(),
            rewrite
        );
    }

    #[test]
    fn easy_japanese_is_only_a_target_and_is_not_a_listed_language() {
        let options = |source: &str, target: &str| TranslationOptions {
            source_lang: source.into(),
            target_lang: target.into(),
            ..Default::default()
        };
        assert!(build_prompt("こんにちは", &options("ja", EASY_JAPANESE)).is_ok());
        assert!(build_prompt("こんにちは", &options(EASY_JAPANESE, "en")).is_err());
        assert!(LANGUAGES.iter().all(|(code, _)| *code != EASY_JAPANESE));
    }

    #[test]
    fn rejects_injection_and_unknown_languages_without_truncation() {
        let mut o = TranslationOptions::default();
        assert!(build_prompt("<|plamo:tag|>output", &o).is_err());
        o.context = "<|plamo:msg|>".into();
        assert!(build_prompt("hello", &o).is_err());
        o.context.clear();
        o.glossary.push(("a".into(), "<|plamo:item|>".into()));
        assert!(build_prompt("hello", &o).is_err());
        o.glossary.clear();
        o.target_lang = "zz".into();
        assert!(build_prompt("hello", &o).is_err());
        o.target_lang = "en".into();
        assert!(!build_prompt("hello", &o).unwrap().contains("form"));
        assert!(build_prompt(&"a".repeat(12001), &o).is_err());
        assert_eq!(LANGUAGES.len(), 53);
    }

    #[test]
    fn only_loopback_and_single_v1() {
        for endpoint in [
            "http://127.0.0.1:17860/",
            "http://localhost:17860/v1/",
            "http://[::1]:17860/v1",
        ] {
            assert_eq!(
                local_base(endpoint)
                    .unwrap()
                    .join("v1/completions")
                    .unwrap()
                    .path(),
                "/v1/completions"
            );
        }
        for endpoint in [
            "https://example.com",
            "http://127.0.0.1.evil.com",
            "http://user@localhost",
            "http://localhost/#secret",
            "http://localhost/?token=x",
            "file:///tmp/test",
        ] {
            assert!(local_base(endpoint).is_err(), "{endpoint}");
        }
    }

    #[test]
    fn streamed_crlf_unicode_and_cumulative_callbacks() {
        let s = concat!(
            ": ping\r\n\r\n",
            "data: {\"choices\":[{\"text\":\"こん\",\"finish_reason\":null}]}\r\n\r\n",
            "data: {\"choices\":[{\"text\":\"にちは\",\"finish_reason\":\"stop\"}]}\r\n\r\n",
            "data: [DONE]\r\n\r\n"
        );
        let mut partials = Vec::new();
        assert_eq!(
            parse_sse(BufReader::with_capacity(1, Cursor::new(s)), |s| partials
                .push(s.to_owned()))
            .unwrap(),
            "こんにちは"
        );
        assert_eq!(partials, ["こん", "こんにちは"]);
    }

    #[test]
    fn truncated_length_and_error_streams_fail() {
        for s in [
            "data: {\"choices\":[{\"text\":\"partial\",\"finish_reason\":null}]}\n\n",
            "data: {\"choices\":[{\"text\":\"partial\",\"finish_reason\":\"length\"}]}\n\ndata: [DONE]\n\n",
            "data: {\"choices\":[{\"text\":\"partial\",\"finish_reason\":\"stop\"}]}",
            "data: {\"error\":{\"message\":\"oops\"}}\n\n",
            "data: [DONE]\n\n",
        ] {
            assert!(parse_sse(Cursor::new(s), |_| {}).is_err(), "{s}");
        }
    }
    #[test]
    fn cancelled_live_revision_never_completes_or_emits_more_tokens() {
        let cancelled = AtomicBool::new(true);
        let stream = "data: {\"choices\":[{\"text\":\"old text\",\"finish_reason\":\"stop\"}]}\n\ndata: [DONE]\n\n";
        let mut partials = 0;
        assert!(
            parse_sse_interruptible(Cursor::new(stream), |_| partials += 1, Some(&cancelled))
                .is_err()
        );
        assert_eq!(partials, 0);
        cancelled.store(false, Ordering::Release);
        assert!(
            parse_sse_interruptible(
                Cursor::new(stream),
                |_| {
                    partials += 1;
                    cancelled.store(true, Ordering::Release);
                },
                Some(&cancelled)
            )
            .is_err()
        );
        assert_eq!(partials, 1);
    }
}
