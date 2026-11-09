"""Breaks the fraction rewrite one way at a time, runs the tests that cover it,
and restores the file. Every mutation must compile and must make at least one
test fail. Usage: mutate.py <worktree> (cargo and PLAMO_DATA_DIR from the
environment). Writes the table to stdout."""
import os, re, subprocess, sys

sys.stdout.reconfigure(encoding="utf-8")
TREE = sys.argv[1]
LIB = ("src/live_text.rs", ["cargo", "test", "--locked", "--lib", "--", "live_text::tests::"])
GUI = ("src/document.rs", ["cargo", "test", "--locked", "--bin", "plamo-translate", "--", "document::tests::"])
MUTATIONS = [
    (LIB, "drop the ' and ' joiner", 'strip_prefix(" and ")', 'strip_prefix(" und ")'),
    (LIB, "drop the '-' joiner", "tail.strip_prefix('-')", "tail.strip_prefix('\\u{1}')"),
    (LIB, "drop the percent rule", "let percent = after.starts_with('%') || after.starts_with(\" percent\");", "let percent = false;"),
    (LIB, "percent rule without ' percent'", " || after.starts_with(\" percent\")", ""),
    (LIB, "write '' instead of '0' before a bare fraction", 'percent.then_some(("0",', 'percent.then_some(("",'),
    (LIB, "accept a whole number 0", " && !number.starts_with('0')", ""),
    (LIB, "accept 3 and 4 digit whole numbers", "(1..=2).contains(&digits)", "(1..=4).contains(&digits)"),
    (LIB, "no left boundary", "let runs_into = |c: char| c.is_ascii_alphanumeric() || matches!(c, '.' | ',' | '/' | '-');", "let runs_into = |_: char| false;"),
    (LIB, "left boundary without '/'", "matches!(c, '.' | ',' | '/' | '-')", "matches!(c, '.' | ',' | '-')"),
    (LIB, "left boundary without '-'", "matches!(c, '.' | ',' | '/' | '-')", "matches!(c, '.' | ',' | '/')"),
    (LIB, "left boundary without '.' and ','", "matches!(c, '.' | ',' | '/' | '-')", "matches!(c, '/' | '-')"),
    (LIB, "no right boundary", "Some(c) if c.is_ascii_alphanumeric() || c == '/' => false,", "Some(c) if c == '\\u{1}' => false,"),
    (LIB, "right boundary lets '/' through", "Some(c) if c.is_ascii_alphanumeric() || c == '/' => false,", "Some(c) if c.is_ascii_alphanumeric() => false,"),
    (LIB, "'.' or ',' then a digit ends the fraction", "Some('.' | ',') => !next.next().is_some_and(|c| c.is_ascii_digit()),", "Some('.' | ',') => true,"),
    (LIB, "swap the 1/4 and 3/4 decimals", '("1/4", ".25"), ("1/2", ".5"), ("3/4", ".75")', '("1/4", ".75"), ("1/2", ".5"), ("3/4", ".25")'),
    (LIB, "re-join the output on whitespace", "out.push_str(&text[copied..]);\n    out\n}", "out.push_str(&text[copied..]);\n    out.split_whitespace().collect::<Vec<_>>().join(\" \")\n}"),
    (GUI, "fractions before fillers", "live_text::with_decimal_fractions(&live_text::without_fillers(source)),", "live_text::without_fillers(&live_text::with_decimal_fractions(source)),"),
    (GUI, "no fractions for spoken text (main)", "live_text::with_decimal_fractions(&live_text::without_fillers(source)),", "live_text::without_fillers(source),"),
    (GUI, "no fractions in the previous source", "live_text::with_decimal_fractions(&live_text::without_fillers(previous)),", "live_text::without_fillers(previous),"),
    (GUI, "no fractions for screen text", "live_text::with_decimal_fractions(source),\n                live_text::with_decimal_fractions(previous),", "source.clone(),\n                previous.clone(),"),
    (GUI, "screen text loses its fillers too", "live_text::with_decimal_fractions(source),\n                live_text::with_decimal_fractions(previous),", "live_text::with_decimal_fractions(&live_text::without_fillers(source)),\n                live_text::with_decimal_fractions(&live_text::without_fillers(previous)),"),
    (GUI, "fractions in text that is not English", "(source.clone(), previous.clone())\n        } else if screen {", "(\n                live_text::with_decimal_fractions(source),\n                live_text::with_decimal_fractions(previous),\n            )\n        } else if screen {"),
]


def run(cmd):
    p = subprocess.run(cmd, cwd=TREE, capture_output=True, text=True, encoding="utf-8", errors="replace")
    out = p.stdout + p.stderr
    failed = sorted(set(re.findall(r"^test (\S+) \.\.\. FAILED", out, re.M)))
    compiled = "error[E" not in out and "could not compile" not in out
    passed = re.findall(r"test result: ok\. (\d+) passed", out)
    return p.returncode, compiled, failed, passed, out


def main():
    killed = 0
    print("| mutation | file | result | failing tests |")
    print("|---|---|---|---|")
    first = int(sys.argv[2]) if len(sys.argv) > 2 else 0
    for (path, cmd), name, old, new in MUTATIONS[first:]:
        full = os.path.join(TREE, path)
        original = open(full, encoding="utf-8", newline="").read()
        if "\r\n" in original:
            old, new = old.replace("\n", "\r\n"), new.replace("\n", "\r\n")
        assert original.count(old) == 1, f"{name}: the text to replace occurs {original.count(old)} times"
        try:
            open(full, "w", encoding="utf-8", newline="").write(original.replace(old, new))
            code, compiled, failed, passed, out = run(cmd)
        finally:
            open(full, "w", encoding="utf-8", newline="").write(original)
        if not compiled:
            result = "DOES NOT COMPILE"
        elif failed:
            result = "caught"
            killed += 1
        else:
            result = "SURVIVED"
        print(f"| {name} | {path} | {result} | {', '.join(t.split('::')[-1] for t in failed)} |", flush=True)
        if result != "caught":
            print(out[-3000:], file=sys.stderr)
    print(f"\n{killed} of {len(MUTATIONS) - first} mutations caught")
    code, compiled, failed, passed, _ = run(LIB[1])
    code2, compiled2, failed2, passed2, _ = run(GUI[1])
    print(f"restored tree: lib live_text tests {passed} failed {failed}; GUI document tests {passed2} failed {failed2}")


main()
