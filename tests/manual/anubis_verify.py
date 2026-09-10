"""Compare engine results with a fixed list of engines, showing pass/fail and
timing — used to validate the Anubis solver (dblp / startpage).

Usage: python tests/manual/anubis_verify.py [bin] [engine ...]
"""
import json
import os
import pathlib
import subprocess
import sys
import time

BIN = sys.argv[1] if len(sys.argv) > 1 else r"target\debug\advent-999-search-mcp.exe"
WANT = sys.argv[2:] or ["dblp", "startpage", "cnki", "sogou"]

QUERIES = {
    "dblp": "transformer",
    "startpage": "python asyncio tutorial",
    "cnki": "知识图谱",
    "sogou": "深度学习",
}


def load_env():
    p = pathlib.Path(os.environ.get("APPDATA", "")) / "Code" / "User" / "mcp.json"
    try:
        return dict(json.loads(p.read_text(encoding="utf-8"))["servers"]["advent"]["env"])
    except Exception:  # noqa: BLE001
        return {}


env = dict(os.environ)
env.update(load_env())
env["DIRECT_DOMAINS"] = "sogou,weixin,baidu,bilibili,hdslb,zhihu,csdn,juejin,xiaohongshu,cnki,dblp"
env["ALLOWED_SEARCH_ENGINES"] = ",".join(WANT)

for e in WANT:
    req = json.dumps({
        "jsonrpc": "2.0", "id": 1, "method": "tools/call",
        "params": {"name": "web", "arguments": {"query": QUERIES.get(e, e), "engines": [e], "limit": 3}},
    })
    t0 = time.time()
    p = subprocess.run([BIN], input=req, capture_output=True, text=True,
                       encoding="utf-8", errors="replace", env=env, timeout=300)
    secs = time.time() - t0

    notes = [l.strip() for l in p.stderr.splitlines()
             if "anubis" in l.lower() or "solving" in l.lower()]
    d = json.loads(p.stdout.strip().splitlines()[0])
    r = json.loads(d["result"]["content"][0]["text"])

    state = "OK  " if r["totalResults"] else "FAIL"
    print(f"{state} {e:<10} n={r['totalResults']}  {secs:5.1f}s")
    for n in notes:
        print(f"       {n}")
    if r["totalResults"]:
        print(f"       e.g. {r['results'][0]['title'][:60]}")
    elif r.get("partialFailures"):
        print(f"       {r['partialFailures'][0]['message'][:70]}")
    print()
