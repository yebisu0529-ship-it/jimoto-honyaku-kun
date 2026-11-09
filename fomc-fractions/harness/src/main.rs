//! Translates FOMC caption paragraphs the way live translation does
//! (src/document.rs document_translate): the source and previous sources
//! without fillers, up to three previous pairs within 1,600 characters
//! (live_text Policy defaults), compose_context with the user context, the
//! glossary on the first pass, then the やさしい日本語 rewrite of the result.
use anyhow::{Context, Result, bail};
use plamo_translate::{
    engine::{TranslationOptions, Translator},
    live_text,
};
use serde::Deserialize;
use serde_json::json;
use std::{collections::VecDeque, io::Write, sync::atomic::AtomicBool, time::Instant};

#[derive(Deserialize)]
struct Paragraph {
    set: String,
    start_ms: u64,
    end_ms: u64,
    source: String,
}

fn parse_glossary(text: &str) -> Result<Vec<(String, String)>> {
    let mut out = Vec::new();
    for line in text.lines() {
        if line.trim().is_empty() || line.trim_start().starts_with('#') {
            continue;
        }
        let Some((s, t)) = line.split_once('\t').or_else(|| line.split_once('=')) else {
            bail!("bad glossary line: {line}");
        };
        out.push((s.trim().to_owned(), t.trim().to_owned()));
    }
    Ok(out)
}

fn tail_chars(text: &str, count: usize) -> String {
    let total = text.chars().count();
    text.chars().skip(total.saturating_sub(count)).collect()
}

/// live_text::Accumulator::bound_previous with the default policy.
fn bound(previous: &mut VecDeque<(String, String)>) {
    const CAP: usize = 1_600;
    while previous.len() > 3 {
        previous.pop_front();
    }
    while previous.len() > 1
        && previous
            .iter()
            .map(|(s, t)| s.chars().count() + t.chars().count())
            .sum::<usize>()
            + previous.len().saturating_sub(1) * 2
            > CAP
    {
        previous.pop_front();
    }
    if let Some((s, t)) = previous.front_mut() {
        *s = tail_chars(s, CAP / 2);
        *t = tail_chars(t, CAP.saturating_sub(s.chars().count()));
    }
}

/// What document_translator_input sends PLaMo for English captions.
fn translator_text(text: &str) -> String {
    let text = live_text::without_fillers(text);
    #[cfg(feature = "fractions")]
    let text = live_text::with_decimal_fractions(&text);
    text
}

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let opt = |name: &str| {
        args.iter()
            .position(|a| a == name)
            .and_then(|i| args.get(i + 1))
            .cloned()
    };
    let corpus_path = opt("--corpus").context("--corpus")?;
    let out_path = opt("--out").context("--out")?;
    let endpoint = opt("--endpoint").unwrap_or("http://127.0.0.1:17870".into());
    let glossary = match opt("--glossary") {
        Some(path) => parse_glossary(&std::fs::read_to_string(path)?)?,
        None => Vec::new(),
    };
    let context = match opt("--context-file") {
        Some(path) => std::fs::read_to_string(path)?.trim().to_owned(),
        None => String::new(),
    };
    let chain = !args.iter().any(|a| a == "--no-chain");
    let skip_easy = args.iter().any(|a| a == "--no-easy");
    let options = TranslationOptions {
        endpoint,
        source_lang: "en".into(),
        target_lang: "ja".into(),
        style: opt("--style").unwrap_or("polite".into()),
        context,
        glossary,
    };
    Translator::new(options.clone())?;
    let corpus: Vec<Paragraph> =
        serde_json::from_str(&std::fs::read_to_string(&corpus_path)?).context("corpus")?;
    let mut out = std::fs::File::create(&out_path)?;
    let cancel = AtomicBool::new(false);
    let mut previous: VecDeque<(String, String)> = VecDeque::new();
    let mut last_end = None;
    for (index, p) in corpus.iter().enumerate() {
        if !chain || last_end.is_none_or(|end: u64| p.start_ms > end + 5_000) {
            previous.clear();
        }
        let joined_source = previous.iter().map(|(s, _)| s.as_str()).collect::<Vec<_>>().join(" ");
        let joined_target = previous.iter().map(|(_, t)| t.as_str()).collect::<Vec<_>>().join(" ");
        let source = translator_text(&p.source);
        let previous_source = translator_text(&joined_source);
        let mut live = options.clone();
        live.context = live_text::compose_context(&options.context, &previous_source, &joined_target, 4_000)?;
        let started = Instant::now();
        let standard = Translator::new(live)?.translate_live(&source, &cancel, |_| {})?;
        let standard_ms = started.elapsed().as_millis();
        let (easy, easy_ms) = if skip_easy {
            (None, 0)
        } else {
            let started = Instant::now();
            let easy = Translator::new(options.easy_japanese())?
                .translate_live(&standard, &cancel, |_| {})
                .ok();
            (easy, started.elapsed().as_millis())
        };
        writeln!(
            out,
            "{}",
            json!({"index": index, "input": source, "set": p.set, "start_ms": p.start_ms, "end_ms": p.end_ms,
                "source": p.source, "standard": standard, "easy": easy,
                "standard_ms": standard_ms, "easy_ms": easy_ms})
        )?;
        out.flush()?;
        eprintln!("{index}: {standard_ms} ms + {easy_ms} ms");
        previous.push_back((p.source.clone(), standard));
        bound(&mut previous);
        last_end = Some(p.end_ms);
    }
    Ok(())
}
