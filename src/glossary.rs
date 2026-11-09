//! The glossary as the settings box holds it: one 「原語 = 訳語」 per line.
use std::collections::HashSet;

/// The most entries a glossary can have. The box and the translation prompt
/// both stop here.
pub const MAX_ENTRIES: usize = 100;

/// The two sides of a line: split at its first tab, or else at its first `=`.
fn split(line: &str) -> Option<(&str, &str)> {
    line.split_once('\t').or_else(|| line.split_once('='))
}

/// Reads the box. Blank lines are skipped.
pub fn parse(input: &str) -> Result<Vec<(String, String)>, String> {
    let mut entries = Vec::new();
    for (index, line) in input.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let Some((source, target)) = split(line) else {
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

/// The terms of a US Federal Reserve (FOMC) press conference, most important
/// first, because `add` leaves out the end of the list when the box is full.
pub const FOMC: &[(&str, &str)] = &[
    ("Federal Reserve", "連邦準備制度"),
    ("federal funds rate", "FF金利"),
    ("Fed funds rate", "FF金利"),
    ("funds rate", "FF金利"),
    ("Fed funds futures", "FF金利先物"),
    ("target range", "誘導目標"),
    ("Chair", "議長"),
    ("Chairman", "議長"),
    ("Vice Chair", "副議長"),
    ("Governor", "理事"),
    ("Board of Governors", "理事会"),
    ("dual mandate", "2つの責務"),
    ("mandate", "責務"),
    ("price stability", "物価の安定"),
    ("stable prices", "物価の安定"),
    ("inflation expectations", "インフレ期待"),
    ("anchored", "安定"),
    ("restrictive", "引き締め的"),
    ("accommodative", "緩和的"),
    ("meeting by meeting", "会合ごとに"),
    ("balance sheet", "バランスシート"),
    ("balance sheet runoff", "バランスシートの縮小"),
    ("reserves", "準備預金"),
    ("discount rate", "公定歩合"),
    ("Summary of Economic Projections", "経済見通し（SEP）"),
    ("SEP", "経済見通し（SEP）"),
    ("dot plot", "ドットプロット"),
    ("headline inflation", "総合インフレ率"),
    ("total PCE", "総合PCE"),
    ("payroll", "雇用者数"),
];

/// What `add` changed.
#[derive(Debug, PartialEq)]
pub struct Added {
    /// Lines written to the box.
    pub entries: usize,
    /// Entries that would have taken the box past `MAX_ENTRIES`.
    pub left_out: usize,
}

/// Adds the entries to the end of the box. What the user wrote stays as it is:
/// a term the box already has, in any letter case, keeps the user's
/// translation. Once the box holds `MAX_ENTRIES` lines, the entries still to
/// come are left out.
pub fn add(glossary: &mut String, entries: &[(&str, &str)]) -> Added {
    let mut known: HashSet<String> = glossary
        .lines()
        .filter_map(split)
        .map(|(source, _)| source.trim().to_lowercase())
        .collect();
    let used = glossary
        .lines()
        .filter(|line| !line.trim().is_empty())
        .count();
    let room = MAX_ENTRIES.saturating_sub(used);
    let mut lines = Vec::new();
    let mut left_out = 0;
    for &(source, target) in entries {
        if !known.insert(source.trim().to_lowercase()) {
            continue;
        }
        if lines.len() < room {
            lines.push(format!("{source} = {target}"));
        } else {
            left_out += 1;
        }
    }
    if !lines.is_empty() {
        if !glossary.is_empty() && !glossary.ends_with('\n') {
            glossary.push('\n');
        }
        glossary.push_str(&lines.join("\n"));
    }
    Added {
        entries: lines.len(),
        left_out,
    }
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

    const SMALL: &[(&str, &str)] = &[("alpha", "ア"), ("beta", "イ"), ("Gamma", "ウ")];

    /// The box after `add`, and what it reported.
    fn add_to(glossary: &str, entries: &[(&str, &str)]) -> (String, Added) {
        let mut glossary = glossary.to_owned();
        let report = add(&mut glossary, entries);
        (glossary, report)
    }

    fn added(entries: usize, left_out: usize) -> Added {
        Added { entries, left_out }
    }

    #[test]
    fn an_empty_box_gets_every_entry_in_order() {
        let (glossary, report) = add_to("", SMALL);
        assert_eq!(glossary, "alpha = ア\nbeta = イ\nGamma = ウ");
        assert_eq!(report, added(3, 0));
    }

    #[test]
    fn what_the_user_wrote_stays_as_it_was_and_the_new_lines_start_on_their_own_line() {
        let written = "my term = 私の訳\n\n  second\t二つ目 ";
        let (glossary, _) = add_to(written, SMALL);
        assert_eq!(
            glossary,
            format!("{written}\nalpha = ア\nbeta = イ\nGamma = ウ")
        );

        let written = "my term = 私の訳\n";
        let (glossary, _) = add_to(written, SMALL);
        assert_eq!(
            glossary,
            format!("{written}alpha = ア\nbeta = イ\nGamma = ウ")
        );
    }

    #[test]
    fn a_term_the_box_has_is_not_added_again_in_any_letter_case_or_separator() {
        let written = "ALPHA = 自分の訳\nBeta\t自分の訳2\n  gamma=自分の訳3  ";
        let (glossary, report) = add_to(written, SMALL);
        assert_eq!(glossary, written, "nothing to add, so not even a newline");
        assert_eq!(report, added(0, 0));

        let (glossary, report) = add_to("ALPHA = 自分の訳", SMALL);
        assert_eq!(report, added(2, 0));
        assert_eq!(
            parse(&glossary).unwrap(),
            entries(&[("ALPHA", "自分の訳"), ("beta", "イ"), ("Gamma", "ウ")])
        );
    }

    #[test]
    fn the_term_of_a_line_with_a_tab_and_an_equals_sign_is_the_part_before_the_tab() {
        let (glossary, report) = add_to("alpha=x\tメモ", SMALL);
        assert_eq!(report, added(3, 0));
        assert_eq!(
            parse(&glossary).unwrap()[0],
            ("alpha=x".to_owned(), "メモ".to_owned())
        );
    }

    #[test]
    fn adding_the_same_entries_twice_changes_nothing_the_second_time() {
        let mut glossary = "mine = 自分の訳".to_owned();
        add(&mut glossary, SMALL);
        let once = glossary.clone();

        let report = add(&mut glossary, SMALL);
        assert_eq!(report, added(0, 0));
        assert_eq!(glossary, once);
    }

    #[test]
    fn the_box_takes_what_fits_under_the_limit_in_list_order_and_counts_the_rest() {
        let others = |count: usize| text_of(&numbered(count));

        let (glossary, report) = add_to(&others(MAX_ENTRIES - 1), SMALL);
        assert_eq!(report, added(1, 2));
        assert!(glossary.ends_with("\nalpha = ア"));
        assert_eq!(parse(&glossary).unwrap().len(), MAX_ENTRIES);

        let spaced = others(MAX_ENTRIES - 1).replace('\n', "\n\n");
        assert_eq!(
            add_to(&spaced, SMALL).1,
            added(1, 2),
            "blank lines are not entries"
        );
    }

    #[test]
    fn a_full_box_and_one_already_past_the_limit_take_nothing() {
        for count in [MAX_ENTRIES, MAX_ENTRIES + 5] {
            let text = text_of(&numbered(count));
            let (glossary, report) = add_to(&text, SMALL);
            assert_eq!(glossary, text, "{count} entries");
            assert_eq!(report, added(0, 3), "{count} entries");
        }
    }

    #[test]
    fn terms_the_box_has_take_no_room_and_are_not_counted_as_left_out() {
        // Room for one. The list has alpha before the term that fits, and Gamma after.
        let text = text_of(&numbered(MAX_ENTRIES - 3)) + "alpha = 自分の訳\nGamma = 自分の訳\n";
        let (glossary, report) = add_to(&text, SMALL);
        assert_eq!(report, added(1, 0));
        assert!(glossary.ends_with("\nbeta = イ"));
    }

    #[test]
    fn the_fomc_terms_are_all_different_whatever_their_letter_case() {
        let terms: HashSet<String> = FOMC
            .iter()
            .map(|(source, _)| source.trim().to_lowercase())
            .collect();
        assert_eq!(terms.len(), FOMC.len());
    }

    #[test]
    fn the_fomc_terms_fit_the_box() {
        assert!(FOMC.len() <= MAX_ENTRIES);
    }

    #[test]
    fn the_fomc_terms_read_back_from_the_box_as_they_were_written() {
        let (glossary, report) = add_to("", FOMC);
        assert_eq!(parse(&glossary).unwrap(), entries(FOMC));
        assert_eq!(report, added(FOMC.len(), 0));
    }

    #[test]
    fn the_fomc_terms_are_accepted_by_the_prompt() {
        let options = TranslationOptions {
            source_lang: "en".into(),
            target_lang: "ja".into(),
            style: "polite".into(),
            glossary: entries(FOMC),
            ..Default::default()
        };
        engine::build_prompt("Inflation has eased.", &options).expect("the prompt takes the terms");
    }
}
