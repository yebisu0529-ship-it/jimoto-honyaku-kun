"""Checks the rate numbers in two sets of runs (before = main, after = the
branch) and lists every changed sentence.

  check.py repro   main runs against the fomc-glossary kit's recorded runs
  check.py rates   per run: fraction notation left in the output, and whether
                   each decimal the rewrite put in the input reaches the
                   ordinary (standard) and the shown (easy) translation
  check.py diff    every sentence that differs between main and the branch
"""
import difflib, json, os, re, sys

sys.stdout.reconfigure(encoding="utf-8")
W = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
KIT = os.path.join(W, "kit", "fomc-glossary", "runs")
MODELS = ["8b", "08b"]
SETS = ["corpus", "probe", "interim", "statement", "corpus-glossary", "probe-glossary"]
FRACTION_OUT = re.compile(r"\d+\s*(?:と|-|～|\s)?\s*\d/\d(?!\d)|(?<![\d.])\d/\d(?!\d)")
FRACTION_IN = re.compile(r"(?<![\w.,/-])\d{1,2}(?: and |-)(?:1/4|1/2|3/4)(?![\w/])|(?<![\w.,/-])(?:1/4|1/2|3/4)(?=%| percent)")
DECIMAL = re.compile(r"(?<![\d.])\d+\.\d+(?![\d.])")


def load(path):
    return [json.loads(line) for line in open(path, encoding="utf-8")]


def run(label, model, name):
    return load(os.path.join(W, "runs", f"{label}-{model}-{name}.jsonl"))


def sentences(text):
    if not text:
        return []
    return [p.strip() for p in re.split(r"(?<=[。？！?!])|(?=>>)", text) if p.strip()]


def repro():
    pairs = {"corpus": "corpus-A", "probe": "probe-A", "corpus-glossary": "corpus-V4", "probe-glossary": "probe-V4"}
    for model in MODELS:
        for mine, theirs in pairs.items():
            a = run("main", model, mine)
            b = load(os.path.join(KIT, f"{model}-{theirs}.jsonl"))
            assert [r["source"] for r in a] == [r["source"] for r in b]
            same = sum(x["standard"] == y["standard"] and x["easy"] == y["easy"] for x, y in zip(a, b))
            print(f"{model:4s} main-{mine:16s} vs kit {theirs:10s}: {same}/{len(a)} identical (standard and shown)")


def expected_decimals(row):
    return [d for d in DECIMAL.findall(row["input"]) if d not in DECIMAL.findall(row["source"])]


def rates(verbose):
    print("model set               | rows with a rate fraction | main: fraction left std/shown | branch: fraction left std/shown | branch: all decimals std/shown | branch: other number")
    for model in MODELS:
        for name in SETS:
            before, after = run("main", model, name), run("branch", model, name)
            rows = [i for i, r in enumerate(after) if FRACTION_IN.search(r["source"])]
            easy = after[0]["easy"] is not None
            def left(runs, field):
                return sum(bool(FRACTION_OUT.search(runs[i][field] or "")) for i in rows)
            def complete(field):
                return sum(all(d in (after[i][field] or "") for d in expected_decimals(after[i])) for i in rows)
            other = []
            for i in rows:
                allowed = set(DECIMAL.findall(after[i]["input"])) | {"0.25"}
                for field in ("standard", "easy"):
                    for d in DECIMAL.findall(after[i][field] or ""):
                        if d not in allowed:
                            other.append((i, field, d))
            shown = lambda n: f"{n}" if easy else "-"
            print(f"{model:4s} {name:17s} | {len(rows):2d} | {left(before, 'standard'):2d}/{shown(left(before, 'easy')):>2s} | "
                  f"{left(after, 'standard'):2d}/{shown(left(after, 'easy')):>2s} | {complete('standard'):2d}/{shown(complete('easy')):>2s} | {other or ''}")
            if verbose:
                for i in rows:
                    for field in ("standard", "easy"):
                        if before[i][field] is None:
                            continue
                        rate = lambda t: [s for s in sentences(t) if re.search(r"\d", s)][:3]
                        print(f"   #{i} {field}: main  {rate(before[i][field])}")
                        print(f"   #{i} {field}: branch {rate(after[i][field])}")


def diff():
    for model in MODELS:
        for name in SETS:
            before, after = run("main", model, name), run("branch", model, name)
            inputs = sum(b["input"] != a["input"] for b, a in zip(before, after))
            for field in ("standard", "easy"):
                if before[0][field] is None:
                    continue
                changed = total = rows = 0
                lines = []
                for b, a in zip(before, after):
                    x, y = sentences(b[field]), sentences(a[field])
                    total += len(y)
                    rows += b[field] != a[field]
                    for op, i1, i2, j1, j2 in difflib.SequenceMatcher(a=x, b=y, autojunk=False).get_opcodes():
                        if op == "equal":
                            continue
                        changed += max(i2 - i1, j2 - j1)
                        lines.append(f"#{b['index']} [{b['set']} {b['start_ms'] / 1000:.0f}s]" + (" input changed" if b["input"] != a["input"] else ""))
                        lines += ["   - " + s for s in x[i1:i2]] + ["   + " + s for s in y[j1:j2]]
                print(f"== {model} {name} {field}: inputs changed {inputs}/{len(before)}, outputs changed {rows}/{len(before)}, changed sentences {changed} (branch has {total})")
                print("\n".join(lines))


if __name__ == "__main__":
    {"repro": repro, "rates": lambda: rates("-v" in sys.argv), "diff": diff}[sys.argv[1]]()
