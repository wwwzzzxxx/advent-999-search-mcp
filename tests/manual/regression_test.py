"""Full engine regression test against the MCP server binary.

Usage: python regression_test.py [path-to-binary]
"""
import json
import os
import subprocess
import sys
import time

BIN = sys.argv[1] if len(sys.argv) > 1 else r".\target\debug\advent-999-search-mcp.exe"


def load_mcp_env():
    """Reuse the keys the user already configured — never hardcode credentials."""
    import pathlib
    p = pathlib.Path(os.environ.get("APPDATA", "")) / "Code" / "User" / "mcp.json"
    try:
        return dict(json.loads(p.read_text(encoding="utf-8"))["servers"]["advent"]["env"])
    except Exception as e:  # noqa: BLE001
        print(f"(warn: could not read {p}: {e})")
        return {}


ENV = load_mcp_env()
ENV.update({
    "DIRECT_DOMAINS": "sogou,weixin,baidu,bilibili,hdslb,zhihu,csdn,juejin,xiaohongshu,cnki,dblp",
    "DEFAULT_SEARCH_ENGINE": "exa",
    # Overridable so the same script runs on Linux/WSL, where the Windows-side
    # proxy is unreachable (test there with USE_PROXY=false).
    "PROXY_URL": os.environ.get("PROXY_URL", "http://127.0.0.1:7890"),
    "USE_PROXY": os.environ.get("USE_PROXY", "true"),
    "ALLOWED_SEARCH_ENGINES": "exa,bing,csdn,juejin,startpage,sogou,weixin,dblp,cnki,ieee",
})

CASES = [
    ("exa", "deep learning"),
    ("bing", "深度学习"),
    ("csdn", "深度学习"),
    ("juejin", "大模型"),
    ("startpage", "python asyncio"),
    ("sogou", "深度学习"),
    ("weixin", "大模型"),
    ("dblp", "transformer"),
    ("cnki", "知识图谱"),
    ("ieee", "deep learning"),
]


def call(engine, query):
    req = json.dumps({
        "jsonrpc": "2.0", "id": 1, "method": "tools/call",
        "params": {"name": "web", "arguments": {"query": query, "engines": [engine], "limit": 3}},
    })
    env = dict(os.environ)
    env.update(ENV)
    p = subprocess.run([BIN], input=req, capture_output=True, text=True,
                       encoding="utf-8", errors="replace", env=env, timeout=180)
    d = json.loads(p.stdout.strip().splitlines()[0])
    return json.loads(d["result"]["content"][0]["text"]), p.stderr


print(f"{'engine':<12} {'results':>7}  detail")
print("-" * 88)
ok = 0
for engine, query in CASES:
    try:
        r, _ = call(engine, query)
        n = r["totalResults"]
        fails = r.get("partialFailures", [])
        if n > 0:
            ok += 1
            print(f"{engine:<12} {n:>7}  OK   e.g. {r['results'][0]['title'][:46]}")
        else:
            msg = fails[0]["message"][:60] if fails else "(no error, 0 results)"
            print(f"{engine:<12} {n:>7}  FAIL {msg}")
    except Exception as e:  # noqa: BLE001
        print(f"{engine:<12} {'-':>7}  ERR  {type(e).__name__} {str(e)[:50]}")
    time.sleep(3)

print("-" * 88)
print(f"passing engines: {ok}/{len(CASES)}")
