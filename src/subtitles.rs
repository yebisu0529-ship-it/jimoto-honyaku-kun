use anyhow::{Context, Result, bail};

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct Cue {
    pub start_ms: u64,
    pub end_ms: u64,
    pub text: String,
}

pub fn parse_srt(input: &str) -> Result<Vec<Cue>> {
    parse(input, false)
}

pub fn parse_vtt(input: &str) -> Result<Vec<Cue>> {
    parse(input, true)
}

fn parse(input: &str, vtt: bool) -> Result<Vec<Cue>> {
    let normalized = input
        .trim_start_matches('\u{feff}')
        .replace("\r\n", "\n")
        .replace('\r', "\n");
    let mut cues = Vec::new();
    let mut block = Vec::new();
    let mut header = true;
    for line in normalized.lines().chain(std::iter::once("")) {
        if header && vtt {
            header = false;
            if line == "WEBVTT" || line.starts_with("WEBVTT ") || line.starts_with("WEBVTT\t") {
                continue;
            }
        }
        if line.trim().is_empty() {
            if !block.is_empty() {
                parse_block(&block, vtt, &mut cues)?;
                block.clear();
            }
        } else {
            block.push(line);
        }
    }
    anyhow::ensure!(
        !cues.is_empty(),
        "No subtitle cues were found in the {} file",
        if vtt { "WebVTT" } else { "SRT" }
    );
    Ok(cues)
}

fn parse_block(lines: &[&str], vtt: bool, cues: &mut Vec<Cue>) -> Result<()> {
    let first = lines[0].trim();
    if vtt
        && (first == "NOTE"
            || first.starts_with("NOTE ")
            || first.starts_with("NOTE\t")
            || first == "STYLE"
            || first == "REGION")
    {
        return Ok(());
    }
    // The optional first line is an SRT index or a WebVTT cue identifier.
    let timing_index = if first.contains("-->") { 0 } else { 1 };
    let number = cues.len() + 1;
    let timing = lines
        .get(timing_index)
        .with_context(|| format!("Cue {number} is missing its timestamp line"))?;
    if !vtt && timing_index == 1 && !first.chars().all(|c| c.is_ascii_digit()) {
        bail!("Cue {number} has an invalid SRT index: {first}");
    }
    let (start, end_and_settings) = timing
        .split_once("-->")
        .with_context(|| format!("Cue {number} is missing the '-->' timestamp separator"))?;
    anyhow::ensure!(
        !end_and_settings.contains("-->"),
        "Cue {number} contains more than one timestamp separator"
    );
    let mut end_parts = end_and_settings.split_whitespace();
    let end = end_parts
        .next()
        .with_context(|| format!("Cue {number} is missing its end timestamp"))?;
    anyhow::ensure!(
        vtt || end_parts.next().is_none(),
        "Cue {number} has unexpected text after its end timestamp"
    );
    let start_ms = timestamp(start.trim(), vtt)
        .with_context(|| format!("Cue {number}: invalid start timestamp"))?;
    let end_ms =
        timestamp(end, vtt).with_context(|| format!("Cue {number}: invalid end timestamp"))?;
    anyhow::ensure!(end_ms >= start_ms, "Cue {number} ends before it starts");
    let text = lines[timing_index + 1..].join("\n");
    anyhow::ensure!(!text.trim().is_empty(), "Cue {number} has no subtitle text");
    let text = if vtt {
        unescape(&strip_vtt_tags(&text))
    } else {
        text
    };
    cues.push(Cue {
        start_ms,
        end_ms,
        text,
    });
    Ok(())
}

fn timestamp(input: &str, short: bool) -> Result<u64> {
    let (clock, fraction) = input
        .rsplit_once(['.', ','])
        .context("Expected milliseconds, for example 00:01:02.345")?;
    anyhow::ensure!(
        fraction.len() == 3 && fraction.bytes().all(|b| b.is_ascii_digit()),
        "Milliseconds must contain exactly three digits"
    );
    let parts: Vec<_> = clock.split(':').collect();
    anyhow::ensure!(
        parts.len() == 3 || (short && parts.len() == 2),
        "Expected hours:minutes:seconds or WebVTT minutes:seconds"
    );
    let mut values = Vec::new();
    for part in &parts {
        anyhow::ensure!(
            part.len() >= 2 && part.bytes().all(|b| b.is_ascii_digit()),
            "Timestamp components must be numeric with at least two digits"
        );
        values.push(
            part.parse::<u64>()
                .context("Timestamp component is too large")?,
        );
    }
    let (hours, minutes, seconds) = if values.len() == 3 {
        (values[0], values[1], values[2])
    } else {
        (0, values[0], values[1])
    };
    anyhow::ensure!(
        parts[parts.len() - 1].len() == 2
            && parts[parts.len() - 2].len() == 2
            && minutes < 60
            && seconds < 60,
        "Minutes and seconds must be between 00 and 59"
    );
    hours
        .checked_mul(3_600_000)
        .and_then(|n| {
            n.checked_add(minutes * 60_000 + seconds * 1_000 + fraction.parse::<u64>().ok()?)
        })
        .context("Timestamp exceeds the supported range")
}

fn strip_vtt_tags(input: &str) -> String {
    let mut result = String::new();
    let mut rest = input;
    while let Some(start) = rest.find('<') {
        result.push_str(&rest[..start]);
        rest = &rest[start..];
        if let Some(end) = rest.find('>') {
            let tag = &rest[1..end];
            let name = tag
                .trim_start_matches('/')
                .split([' ', '\t', '.'])
                .next()
                .unwrap_or("");
            if matches!(name, "c" | "v" | "lang" | "b" | "i" | "u" | "ruby" | "rt")
                || timestamp(tag, true).is_ok()
            {
                rest = &rest[end + 1..];
                continue;
            }
        }
        result.push('<');
        rest = &rest[1..];
    }
    result.push_str(rest);
    result
}

fn unescape(input: &str) -> String {
    let mut result = String::new();
    let mut rest = input;
    while let Some(start) = rest.find('&') {
        result.push_str(&rest[..start]);
        rest = &rest[start..];
        if let Some(end) = rest.find(';').filter(|&n| n <= 16) {
            let entity = &rest[1..end];
            let decoded = match entity {
                "amp" => Some('&'),
                "lt" => Some('<'),
                "gt" => Some('>'),
                "quot" => Some('"'),
                "apos" => Some('\''),
                "nbsp" => Some('\u{a0}'),
                "lrm" => Some('\u{200e}'),
                "rlm" => Some('\u{200f}'),
                _ => entity
                    .strip_prefix("#x")
                    .or_else(|| entity.strip_prefix("#X"))
                    .and_then(|n| u32::from_str_radix(n, 16).ok())
                    .and_then(char::from_u32)
                    .or_else(|| {
                        entity
                            .strip_prefix('#')
                            .and_then(|n| n.parse().ok())
                            .and_then(char::from_u32)
                    }),
            };
            if let Some(c) = decoded {
                result.push(c);
                rest = &rest[end + 1..];
                continue;
            }
        }
        result.push('&');
        rest = &rest[1..];
    }
    result.push_str(rest);
    result
}

fn format_time(ms: u64, separator: char) -> String {
    format!(
        "{:02}:{:02}:{:02}{}{:03}",
        ms / 3_600_000,
        ms / 60_000 % 60,
        ms / 1_000 % 60,
        separator,
        ms % 1_000
    )
}

pub fn to_srt(cues: &[Cue]) -> String {
    cues.iter()
        .enumerate()
        .map(|(i, cue)| {
            format!(
                "{}\n{} --> {}\n{}\n\n",
                i + 1,
                format_time(cue.start_ms, ','),
                format_time(cue.end_ms, ','),
                cue.text
            )
        })
        .collect()
}

pub fn to_vtt(cues: &[Cue]) -> String {
    let mut output = String::from("WEBVTT\n\n");
    for cue in cues {
        let text = cue
            .text
            .replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;");
        output.push_str(&format!(
            "{} --> {}\n{}\n\n",
            format_time(cue.start_ms, '.'),
            format_time(cue.end_ms, '.'),
            text
        ));
    }
    output
}

/// Split by Unicode scalar count, preserving every input byte and preferring spaces.
pub fn split_text(input: &str, max_chars: usize) -> Vec<String> {
    let limit = max_chars.max(1);
    let mut rest = input;
    let mut output = Vec::new();
    while !rest.is_empty() {
        let mut end = rest.len();
        let mut boundary = None;
        for (count, (index, character)) in rest.char_indices().enumerate() {
            if count == limit {
                end = index;
                break;
            }
            if character.is_whitespace() && count >= limit / 2 {
                boundary = Some(index + character.len_utf8());
            }
        }
        if end < rest.len()
            && let Some(space) = boundary
        {
            end = space;
        }
        output.push(rest[..end].to_owned());
        rest = &rest[end..];
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn srt_bom_crlf_and_final_cue_without_newline() {
        let cues = parse_srt("\u{feff}1\r\n00:00:00,010 --> 00:00:01,200\r\nfirst\r\nsecond\r\n\r\n2\r\n00:00:02,000 --> 00:00:03,000\r\n最後").unwrap();
        assert_eq!(cues.len(), 2);
        assert_eq!(cues[0].text, "first\nsecond");
        assert_eq!(cues[0].start_ms, 10);
        let roundtrip = parse_srt(&to_srt(&cues)).unwrap();
        assert_eq!(roundtrip[1].end_ms, 3_000);
    }
    #[test]
    fn vtt_identifiers_settings_metadata_and_inline_markup() {
        let input = "WEBVTT\n\nNOTE a note\nignore me\n\nSTYLE\n::cue { color: red }\n\nREGION\nid:a\n\nintro\n00:01.010 --> 00:02.200 align:start\n<v Alice><c.red>Hello</c> &amp; <00:01.400>2 < 5 &lt;b&gt; &#x65e5;&#26412;</v>";
        let cues = parse_vtt(input).unwrap();
        assert_eq!(cues.len(), 1);
        assert_eq!(cues[0].text, "Hello & 2 < 5 <b> 日本");
        assert_eq!(parse_vtt(&to_vtt(&cues)).unwrap()[0].text, cues[0].text);
    }
    #[test]
    fn bad_timestamps_are_rejected() {
        for time in [
            "00:60:00,000 --> 00:61:00,000",
            "00:00:60,000 --> 00:01:01,000",
            "00:00:01,00 --> 00:00:02,000",
            "00:00:02,000 --> 00:00:01,000",
            "-1:00:00,000 --> 00:00:02,000",
        ] {
            assert!(parse_srt(&format!("1\n{time}\ntext")).is_err(), "{time}");
        }
        assert!(parse_vtt("WEBVTT\n\nNOTE nothing here").is_err());
        assert!(parse_srt("").is_err());
    }
    #[test]
    fn unicode_chunks_preserve_every_byte_and_limit() {
        let input = "  hello world\r\n日本語 🦀e\u{301} abcdefghijklmnop\t末尾  ";
        for max in [0, 1, 2, 7, 12, 200] {
            let chunks = split_text(input, max);
            assert_eq!(chunks.concat(), input);
            assert!(
                chunks
                    .iter()
                    .all(|s| !s.is_empty() && s.chars().count() <= max.max(1))
            );
        }
        assert!(split_text("", 3).is_empty());
        assert_eq!(split_text("hello world", 7), vec!["hello ", "world"]);
    }
}
