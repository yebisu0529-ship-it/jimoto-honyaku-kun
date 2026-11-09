//! The glossary as the settings box holds it: one 「原語 = 訳語」 per line.

/// The most entries a glossary can have. The box and the translation prompt
/// both stop here.
pub const MAX_ENTRIES: usize = 100;

/// Reads the box. The two sides of a line are split at its first tab, or else
/// at its first `=`. Blank lines are skipped.
pub fn parse(input: &str) -> Result<Vec<(String, String)>, String> {
    let mut entries = Vec::new();
    for (index, line) in input.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let Some((source, target)) = line.split_once('\t').or_else(|| line.split_once('=')) else {
            return Err(format!(
                "{}行目: 原語と訳語を = またはタブで区切ってください。",
                index + 1
            ));
        };
        if source.trim().is_empty() || target.trim().is_empty() {
            return Err(format!(
                "{}行目: 原語と訳語の両方を入力してください。",
                index + 1
            ));
        }
        entries.push((source.trim().to_owned(), target.trim().to_owned()));
    }
    if entries.len() > MAX_ENTRIES {
        return Err(format!(
            "用語集は{MAX_ENTRIES}項目までです（今は{}項目）。",
            entries.len()
        ));
    }
    Ok(entries)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::{self, TranslationOptions};

    fn entries(list: &[(&str, &str)]) -> Vec<(String, String)> {
        list.iter()
            .map(|&(source, target)| (source.to_owned(), target.to_owned()))
            .collect()
    }

    fn numbered(count: usize) -> Vec<(String, String)> {
        (0..count)
            .map(|index| (format!("term {index}"), format!("訳語{index}")))
            .collect()
    }

    fn text_of(entries: &[(String, String)]) -> String {
        entries
            .iter()
            .map(|(source, target)| format!("{source} = {target}\n"))
            .collect()
    }

    #[test]
    fn a_line_is_split_at_its_tab_or_else_at_its_first_equals_sign() {
        let text = "inference = 推論\n  latency\t 遅延 \nA=B=C\nE=mc2\t質量とエネルギー\n";
        assert_eq!(
            parse(text).unwrap(),
            entries(&[
                ("inference", "推論"),
                ("latency", "遅延"),
                ("A", "B=C"),
                ("E=mc2", "質量とエネルギー"),
            ])
        );
    }

    #[test]
    fn blank_lines_are_skipped_but_still_counted_in_line_numbers() {
        assert_eq!(parse(""), Ok(vec![]));
        assert_eq!(parse("\n  \n\t\n"), Ok(vec![]));
        assert_eq!(
            parse("a = あ\n\n   \nb = い"),
            Ok(entries(&[("a", "あ"), ("b", "い")]))
        );
        assert_eq!(
            parse("a = あ\n\n\nb"),
            Err("4行目: 原語と訳語を = またはタブで区切ってください。".to_owned())
        );
    }

    #[test]
    fn a_line_with_only_one_side_is_refused_with_its_line_number() {
        for line in ["= あ", "a =", "a =   ", "\t訳", "用語\t", "="] {
            assert_eq!(
                parse(&format!("ok = 良い\n{line}")),
                Err("2行目: 原語と訳語の両方を入力してください。".to_owned()),
                "{line:?}"
            );
        }
    }

    #[test]
    fn the_limit_counts_entries_not_lines_and_one_more_is_refused_with_the_count() {
        assert_eq!(
            parse(&text_of(&numbered(MAX_ENTRIES))).unwrap().len(),
            MAX_ENTRIES
        );
        let spaced = text_of(&numbered(MAX_ENTRIES)).replace('\n', "\n\n");
        assert_eq!(parse(&spaced).unwrap().len(), MAX_ENTRIES);
        assert_eq!(
            parse(&text_of(&numbered(MAX_ENTRIES + 1))).err(),
            Some(format!(
                "用語集は{MAX_ENTRIES}項目までです（今は{}項目）。",
                MAX_ENTRIES + 1
            ))
        );
    }

    #[test]
    fn the_box_takes_as_many_entries_as_the_prompt_does() {
        for (count, accepted) in [(MAX_ENTRIES, true), (MAX_ENTRIES + 1, false)] {
            let glossary = numbered(count);
            assert_eq!(
                parse(&text_of(&glossary)).is_ok(),
                accepted,
                "the box with {count} entries"
            );
            let options = TranslationOptions {
                glossary,
                ..Default::default()
            };
            assert_eq!(
                engine::build_prompt("Hello.", &options).is_ok(),
                accepted,
                "the prompt with {count} entries"
            );
        }
    }
}
