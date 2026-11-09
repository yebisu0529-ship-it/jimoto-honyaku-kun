"""Counts wrong and expected renderings of the terms the preset targets, in the
ordinary (standard) and shown (easy) translations of each run."""
import json, re, sys

sys.stdout.reconfigure(encoding="utf-8")
CHECKS = [
    ("Federal Reserve", r"連邦準備制度理事会", r"連邦準備制度(?!理事会)"),
    ("federal funds rate / funds rate", r"連邦資金(?:金利|率)|フェデラル・?ファンド金利|(?<!FF)(?<!・)ファンド金利", r"FF金利"),
    ("Chair / Chairman", r"委員長", r"議長"),
    ("dual mandate / mandate", r"マンデート", r"責務|責任"),
    ("anchored (inflation expectations)", r"据え置かれる|固定され", r"インフレ期待が安定"),
    ("price stability", r"価格の安定|価格安定|物価が変わらない", r"物価の?安定|物価を安定|物価が安定"),
    ("balance sheet", r"貸借対照表", r"バランスシート"),
    ("3 and 3/4", r"3と3/4|3 3/4|3～3/4", r"3\.75"),
]


def count(rows, field, pattern):
    return sum(len(re.findall(pattern, r[field] or "")) for r in rows)


for path in sys.argv[1:]:
    rows = [json.loads(line) for line in open(path, encoding="utf-8")]
    print(path)
    for name, wrong, right in CHECKS:
        print(f"  {name:36s} standard wrong {count(rows, 'standard', wrong):2d} right {count(rows, 'standard', right):2d}"
              f" | shown wrong {count(rows, 'easy', wrong):2d} right {count(rows, 'easy', right):2d}")
