"""Lists the sentences that differ between two runs of the same corpus, and
counts, for each glossary entry whose English term occurs in a paragraph,
whether its Japanese term appears in that paragraph's translation."""
import argparse, difflib, json, re, sys

sys.stdout.reconfigure(encoding="utf-8")


def sentences(text):
    if not text:
        return []
    parts = re.split(r"(?<=[。？！?!])|(?=>>)", text)
    return [p.strip() for p in parts if p.strip()]


def load(path):
    return [json.loads(line) for line in open(path, encoding="utf-8")]


def glossary(path):
    out = []
    for line in open(path, encoding="utf-8"):
        if line.strip() and "=" in line:
            s, t = line.split("=", 1)
            out.append((s.strip(), t.strip()))
    return out


def occurs(term, text):
    return re.search(r"(?<![A-Za-z])" + re.escape(term) + r"(?![A-Za-z])", text, re.IGNORECASE) is not None


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("before")
    ap.add_argument("after")
    ap.add_argument("--glossary")
    ap.add_argument("--field", default="standard", choices=["standard", "easy"])
    ap.add_argument("--quiet", action="store_true")
    a = ap.parse_args()
    before, after = load(a.before), load(a.after)
    changed = total = 0
    for b, c in zip(before, after):
        assert b["source"] == c["source"]
        x, y = sentences(b[a.field]), sentences(c[a.field])
        total += len(y)
        sm = difflib.SequenceMatcher(a=x, b=y, autojunk=False)
        for op, i1, i2, j1, j2 in sm.get_opcodes():
            if op == "equal":
                continue
            changed += max(i2 - i1, j2 - j1)
            if not a.quiet:
                print(f"#{b['index']} [{b['set']} {b['start_ms']/1000:.0f}s]")
                for s in x[i1:i2]:
                    print("   - " + s)
                for s in y[j1:j2]:
                    print("   + " + s)
    print(f"changed sentences: {changed} (after has {total})")
    if a.glossary:
        print("term | paragraphs with the English term | Japanese present before -> after")
        for s, t in glossary(a.glossary):
            hits = [(b, c) for b, c in zip(before, after) if occurs(s, b["source"])]
            if not hits:
                continue
            nb = sum(t in b[a.field] for b, _ in hits)
            nc = sum(t in c[a.field] for _, c in hits)
            print(f"{s} = {t} | {len(hits)} | {nb} -> {nc}")


main()
