"""A/B test: sogou/weixin work from the local cookie cache alone.

Deliberately strips FETCH_COOKIES from the environment, to prove the engines no
longer depend on it. Run `seed_cookies.py` first.

Usage: python tests/manual/cache_cookie_test.py [bin]
"""
import json
import os
import pathlib
import shutil
import subprocess
import sys
import time

BIN = sys.argv[1] if len(sys.argv) > 1 else r"target\debug\advent-999-search-mcp.exe"
CACHE = pathlib.Path(os.environ.get("LOCALAPPDATA", ".")) / "advent-mcp" / "cookie-cache.json"


def base_env():
    """Env WITHOUT FETCH_COOKIES — only the non-secret settings."""
    env = dict(os.environ)
    env.pop("FETCH_COOKIES", None)
    env["DIRECT_DOMAINS"] = "sogou,weixin,baidu,bilibili,hdslb,zhihu,csdn,juejin,xiaohongshu,cnki,dblp"
    env["USE_PROXY"] = "true"
    env["PROXY_URL"] = "http://127.0.0.1:7890"
    env["ALLOWED_SEARCH_ENGINES"] = "sogou,weixin"
    return env


def call(engine, query):
    req = json.dumps({
        "jsonrpc": "2.0", "id": 1, "method": "tools/call",
        "params": {"name": "web", "arguments": {"query": query, "engines": [engine], "limit": 3}},
    })
    p = subprocess.run([BIN], input=req, capture_output=True, text=True,
                       encoding="utf-8", errors="replace", env=base_env(), timeout=180)
    d = json.loads(p.stdout.strip().splitlines()[0])
    return json.loads(d["result"]["content"][0]["text"]), p.stderr


print(f"cache file : {CACHE}")
print(f"exists     : {CACHE.exists()}")
print(f"FETCH_COOKIES set in shell: {'FETCH_COOKIES' in os.environ} "
      f"(the test strips it either way)\n")

print("=== A. 缓存存在（应通过）===")
for e, q in (("sogou", "深度学习"), ("weixin", "大模型")):
    r, err = call(e, q)
    note = [l.strip() for l in err.splitlines() if "cookie" in l.lower()]
    print(f"  {e:<8} n={r['totalResults']}  "
          + (r["results"][0]["title"][:44] if r["totalResults"]
             else str([f["message"][:50] for f in r.get("partialFailures", [])])))
    for n in note:
        print(f"           {n}")
    time.sleep(4)
print()

backup = CACHE.with_suffix(".json.bak")
print("=== B. 缓存移除（应失败）===")
if CACHE.exists():
    shutil.move(str(CACHE), str(backup))
try:
    for e in ("sogou",):
        r, err = call(e, "深度学习")
        print(f"  {e:<8} n={r['totalResults']}  "
              + str([f["message"][:56] for f in r.get("partialFailures", [])]))
finally:
    if backup.exists():
        shutil.move(str(backup), str(CACHE))
        print("\ncache restored")
