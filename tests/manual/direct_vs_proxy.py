"""Compare engine results with the site listed in DIRECT_DOMAINS vs proxied.

Usage: python tests/manual/direct_vs_proxy.py <engine> <query> [dirlist]
"""
import json
import os
import pathlib
import subprocess
import sys

BIN = r"target\debug\advent-999-search-mcp.exe"


def load_env():
    p = pathlib.Path(os.environ.get("APPDATA", "")) / "Code" / "User" / "mcp.json"
    try:
        return dict(json.loads(p.read_text(encoding="utf-8"))["servers"]["advent"]["env"])
    except Exception:  # noqa: BLE001
        return {}


def call(engine, query, direct_domains):
    env = dict(os.environ)
    env.update(load_env())
    env["DIRECT_DOMAINS"] = direct_domains
    env["ALLOWED_SEARCH_ENGINES"] = engine
    env["DEFAULT_SEARCH_ENGINE"] = engine
    req = json.dumps({
        "jsonrpc": "2.0", "id": 1, "method": "tools/call",
        "params": {"name": "web", "arguments": {"query": query, "engines": [engine], "limit": 3}},
    })
    p = subprocess.run([BIN], input=req, capture_output=True, text=True,
                       encoding="utf-8", errors="replace", env=env, timeout=180)
    r = json.loads(json.loads(p.stdout.strip().splitlines()[0])["result"]["content"][0]["text"])
    return r


BASE = "baidu,bilibili,hdslb,zhihu,csdn,juejin,xiaohongshu,cnki,dblp"

CASES = [
    ("sogou", "深度学习", "sogou", f"sogou,{BASE}"),
    ("weixin", "大模型", "weixin", f"weixin,{BASE}"),
]

for engine, query, dname, dirs in CASES:
    print(f"=== {engine} ===")
    for label, dl in ((f"直连（{dname} 在 DIRECT_DOMAINS）", dirs),
                      (f"走代理（{dname} 不在列表）      ", BASE)):
        r = call(engine, query, dl)
        n = r["totalResults"]
        detail = (r["results"][0]["title"][:44] if n else
                  (r.get("partialFailures") or [{}])[0].get("message", "(silent 0)")[:52])
        print(f"  {label}  -> n={n}  {detail}")
    print()
