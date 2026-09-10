"""End-to-end test: sogou/weixin engines with session cookies.

NOTE (2026-09-10): cookies now live in the local cookie cache file, not in
FETCH_COOKIES — see `cache_cookie_test.py` for the current test. This script is
kept because it also exercises the env-var fallback path (fetch.rs still uses
FETCH_COOKIES for long-lived login cookies like Zhihu's d_c0).

Usage: python tests/manual/sogou_with_cookies.py [cookie-string]
"""
import json
import os
import pathlib
import subprocess
import sys
import time

BIN = r"target\debug\advent-999-search-mcp.exe"


def load_env():
    p = pathlib.Path(os.environ.get("APPDATA", "")) / "Code" / "User" / "mcp.json"
    try:
        return dict(json.loads(p.read_text(encoding="utf-8"))["servers"]["advent"]["env"])
    except Exception:  # noqa: BLE001
        return {}


def run(label, cookies):
    env = dict(os.environ)
    env.update(load_env())
    env["DIRECT_DOMAINS"] = "sogou,weixin,baidu,bilibili,hdslb,zhihu,csdn,juejin,xiaohongshu,cnki,dblp"
    env["USE_PROXY"] = "true"
    env["PROXY_URL"] = "http://127.0.0.1:7890"
    env["ALLOWED_SEARCH_ENGINES"] = "sogou,weixin"
    if cookies:
        env["FETCH_COOKIES"] = cookies
    else:
        env.pop("FETCH_COOKIES", None)

    print(f"=== {label} ===")
    for engine, query in (("sogou", "深度学习"), ("weixin", "大模型")):
        req = json.dumps({
            "jsonrpc": "2.0", "id": 1, "method": "tools/call",
            "params": {"name": "web",
                       "arguments": {"query": query, "engines": [engine], "limit": 3}},
        })
        p = subprocess.run([BIN], input=req, capture_output=True, text=True,
                           encoding="utf-8", errors="replace", env=env, timeout=180)
        d = json.loads(p.stdout.strip().splitlines()[0])
        r = json.loads(d["result"]["content"][0]["text"])
        fails = [x["message"][:58] for x in r.get("partialFailures", [])]
        print(f"  {engine:<8} n={r['totalResults']}  "
              + (f"{r['results'][0]['title'][:44]}" if r["totalResults"] else str(fails)))
        time.sleep(5)
    print()


def resolve_cookies():
    """Cookie source priority: argv[1] > $FETCH_COOKIES > mcp.json.

    Credentials are NEVER committed to this repo — set FETCH_COOKIES in your
    shell before running, e.g.

        $env:FETCH_COOKIES = "SNUID=...; SUV=...; SUID=...; ABTEST=...; IPLOC=..."
        python tests/manual/sogou_with_cookies.py
    """
    if len(sys.argv) > 1:
        return sys.argv[1], "argv[1]"
    if os.environ.get("FETCH_COOKIES"):
        return os.environ["FETCH_COOKIES"], "$FETCH_COOKIES"
    c = load_env().get("FETCH_COOKIES", "")
    return c, "mcp.json (local, uncommitted)"


cookies, source = resolve_cookies()
if not cookies:
    print("ERROR: no cookies found. Set the FETCH_COOKIES environment variable.\n")
    sys.exit(2)

print(f"cookie source : {source}")
print(f"has SNUID     : {'SNUID=' in cookies}")
print(f"has SUV       : {'SUV=' in cookies}\n")

run("A. 带 SNUID/SUV（应通过）", cookies)
run("B. 不带 cookie（对照，应被拦）", "")
