"""Collects every distinct caption text containing a fraction that the app
recorded for hs8YWtNQQ40 since 2026-11-08 (older records carry YouTube's
settings label in the text). Each is a real interim or final paragraph that
the app sent to PLaMo."""
import glob, json, os, re, sys
sessions, out = sys.argv[1], sys.argv[2]
FRACTION = re.compile(r"\d(?: and |-)\d/\d|\d/\d (?:percent|%)")
rows, seen = [], set()
for path in sorted(glob.glob(sessions + "/plamo-timeline-2026110[89]*.jsonl")):
    for line in open(path, encoding="utf-8"):
        try:
            r = json.loads(line)
        except ValueError:
            continue
        src = r.get("source") or ""
        if r.get("video_id") != "hs8YWtNQQ40" or not FRACTION.search(src) or src in seen:
            continue
        seen.add(src)
        rows.append({"set": "interim", "session": os.path.basename(path)[15:-6], "id": r.get("id"),
                     "revision": (r.get("paragraph") or {}).get("revision"),
                     "start_ms": r["start_ms"], "end_ms": r["end_ms"], "source": src,
                     "recorded_standard": r.get("standard_translation"),
                     "recorded_shown": r.get("translation")})
json.dump(rows, open(out, "w", encoding="utf-8"), ensure_ascii=False, indent=1)
print(len(rows), "texts")
for r in rows:
    print(r["session"][:8], r["revision"], len(r["source"]), r["source"][:60].encode("ascii", "replace").decode())
