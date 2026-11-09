"""Build the FOMC caption corpus from the demo journals (video hs8YWtNQQ40).

take1: the five final paragraphs of the take1 session, checked against
take1/meta.json captions. extra: final paragraphs from other sessions that do
not overlap take1 or each other, preferring the longest continuous session.
"""
import glob, json, os, sys

SESSIONS = r"M:/JimotoDemo-20261105/data/sessions"
TAKE1 = "20261108T130106060130700Z"
VIDEO = "hs8YWtNQQ40"


def finals(path):
    out = {}
    for line in open(path, encoding="utf-8"):
        try:
            d = json.loads(line)
        except ValueError:
            continue
        if d.get("video_id") != VIDEO or d.get("paragraph", {}).get("state") != "final":
            continue
        out[d["id"]] = d
    return sorted(out.values(), key=lambda d: d["start_ms"])


def main():
    sessions = {}
    for p in sorted(glob.glob(os.path.join(SESSIONS, "*.jsonl"))):
        rows = finals(p)
        if rows:
            sessions[os.path.basename(p)[15:40]] = rows
    take1 = sessions[TAKE1]
    meta = json.load(open(r"M:/JimotoFomc-20261108/take1/meta.json", encoding="utf-8"))
    last_caption = meta["captions"][-1][2]
    assert take1[-1]["source"].endswith(last_caption), "take1 journal and captions differ"
    chosen = [("take1", d) for d in take1]
    covered = [(take1[0]["start_ms"], take1[-1]["end_ms"])]
    order = sorted(sessions, key=lambda s: -sum(d["end_ms"] - d["start_ms"] for d in sessions[s]))
    for s in order:
        if s == TAKE1:
            continue
        for d in sessions[s]:
            if any(d["start_ms"] < b and d["end_ms"] > a for a, b in covered):
                continue
            chosen.append((s, d))
            covered.append((d["start_ms"], d["end_ms"]))
    chosen.sort(key=lambda x: x[1]["start_ms"])
    corpus = []
    for s, d in chosen:
        corpus.append({
            "set": "take1" if s == "take1" else "extra",
            "session": s,
            "id": d["id"],
            "start_ms": d["start_ms"],
            "end_ms": d["end_ms"],
            "source": d["source"],
            "recorded_standard": d.get("standard_translation"),
            "recorded_shown": d.get("translation"),
        })
    out = os.path.join(os.path.dirname(os.path.abspath(__file__)), "corpus.json")
    json.dump(corpus, open(out, "w", encoding="utf-8"), ensure_ascii=False, indent=1)
    sys.stdout.reconfigure(encoding="utf-8")
    for c in corpus:
        print(f"[{c['set']} {c['start_ms']/1000:6.1f}-{c['end_ms']/1000:6.1f}] {c['source']}")
    print("paragraphs", len(corpus), "chars", sum(len(c["source"]) for c in corpus))


main()
