#!/usr/bin/env bash
# Usage: run_set.sh <label> <harness.exe>. Runs every input on 8B and 0.8B,
# one fresh server per run, and writes runs/<label>-<model>-<input>.jsonl.
set -euo pipefail
W="M:/JimotoFomc-20261108/fomc-fractions"
K="$W/kit/fomc-glossary"
label=$1; harness=$2
for model in 8b 0.8b; do
  m=${model/./}
  python -I "$W/tools/runner.py" --harness "$harness" --model $model --corpus "$K/corpus.json" --out "$W/runs/$label-$m-corpus.jsonl"
  python -I "$W/tools/runner.py" --harness "$harness" --model $model --corpus "$K/probes.json" --out "$W/runs/$label-$m-probe.jsonl" --no-easy
  python -I "$W/tools/runner.py" --harness "$harness" --model $model --corpus "$W/interims.json" --out "$W/runs/$label-$m-interim.jsonl" --no-chain
  python -I "$W/tools/runner.py" --harness "$harness" --model $model --corpus "$W/statements.json" --out "$W/runs/$label-$m-statement.jsonl" --no-chain
  python -I "$W/tools/runner.py" --harness "$harness" --model $model --corpus "$K/corpus.json" --out "$W/runs/$label-$m-corpus-glossary.jsonl" --glossary "$K/cand/v4.tsv"
  python -I "$W/tools/runner.py" --harness "$harness" --model $model --corpus "$K/probes.json" --out "$W/runs/$label-$m-probe-glossary.jsonl" --no-easy --glossary "$K/cand/v4.tsv"
done
echo "set $label done"
