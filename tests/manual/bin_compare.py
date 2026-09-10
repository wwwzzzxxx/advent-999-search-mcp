"""Compare sogou across the debug (fixed) and release (old) binaries.

The old binary's "direct" path still honoured reqwest's automatic system
proxy (Windows registry), so DIRECT_DOMAINS had no effect and sogou requests
went out through the proxy IP — which is permanently captcha-walled, unlike
the user's real IP (unblocked after they solved the captcha in a browser).
"""
import json
import os
import pathlib
import subprocess
import sys
import time

KEEP = sys.argv[1:] or ["sogou", "cnki"]


def load_env():
    p = pathlib.Path(os.environ.get("APPDATA", "")) / "Code" / "User" / "mcp.json"
    try:
        return dict(json.loads(p.read_text(encoding="utf-8"))["servers"]["advent"]["env"])
    except Exception:  # noqa: BLE001
        return {}


env = dict(os.environ)
env.update(load_env())
env["DIRECT_DOMAINS"] = "sogou,weixin,baidu,bilibili,hdslb,zhihu,csdn,juejin,xiaohongshu,cnki,dblp"
env["USE_PROXY"] = "true"
env["PROXY_URL"] = "http://127.0.0.1:7890"
env["ALLOWED_SEARCH_ENGINES"] = ",".join(KEEP)

QUERIES = {"sogou": "深度学习", "cnki": "知识图谱", "weixin": "大模型"}
BINARIES = [
    ("debug   (含 .no_proxy 修复)", r"target\debug\advent-999-search-mcp.exe"),
    ("release (当前 MCP 在用)", r"target\release\advent-999-search-mcp.exe"),
]

for engine in KEEP:
    print(f"=== {engine} ===")
    for label, binp in BINARIES:
        out = []
        for _ in range(2):
            req = json.dumps({
                "jsonrpc": "2.0", "id": 1, "method": "tools/call",
                "params": {"name": "web",
                           "arguments": {"query": QUERIES[engine], "engines": [engine], "limit": 3}},
            })
            try:
                p = subprocess.run([binp], input=req, capture_output=True, text=True,
                                   encoding="utf-8", errors="replace", env=env, timeout=150)
                d = json.loads(p.stdout.strip().splitlines()[0])
                r = json.loads(d["result"]["content"][0]["text"])
                f = [x["message"][:44] for x in r.get("partialFailures", [])]
                out.append(f"n={r['totalResults']}" + (f" {f}" if f else ""))
            except Exception as e:  # noqa: BLE001
                out.append(f"ERR {type(e).__name__}")
            time.sleep(4)
        print(f"  {label:<26} {'  |  '.join(out)}")
    print()
