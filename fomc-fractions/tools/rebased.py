"""Compares the runs made after the rebase onto the current main with the
runs made before it: main against rebased-main, branch against
rebased-branch, every input on both models."""
import json, os, sys
sys.stdout.reconfigure(encoding="utf-8")
W = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
SETS = ["corpus", "probe", "interim", "statement", "corpus-glossary", "probe-glossary"]
total = same = 0
for before, after in [("main", "rebased-main"), ("branch", "rebased-branch")]:
    for model in ["8b", "08b"]:
        for name in SETS:
            a = [json.loads(l) for l in open(f"{W}/runs/{before}-{model}-{name}.jsonl", encoding="utf-8")]
            b = [json.loads(l) for l in open(f"{W}/runs/{after}-{model}-{name}.jsonl", encoding="utf-8")]
            ok = sum(x["input"] == y["input"] and x["standard"] == y["standard"] and x["easy"] == y["easy"] for x, y in zip(a, b))
            total += len(a); same += ok
            print(f"{before:7s} vs {after:15s} {model:4s} {name:16s} {ok}/{len(a)}")
print(f"identical rows (input, standard, shown): {same}/{total}")
