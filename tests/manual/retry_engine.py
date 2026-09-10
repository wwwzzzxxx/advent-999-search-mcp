"""Retry a single engine N times to detect flakiness.

Usage: python retry_engine.py <engine> <query> [n]
"""
import json
import os
import subprocess
import sys
import time

engine = sys.argv[1] if len(sys.argv) > 1 else "csdn"
query = sys.argv[2] if len(sys.argv) > 2 else "深度学习"
n = int(sys.argv[3]) if len(sys.argv) > 3 else 3
BIN = r".\target\debug\advent-999-search-mcp.exe"

env = dict(os.environ)
env.update({
    "DIRECT_DOMAINS": "sogou,weixin,baidu,bilibili,hdslb,zhihu,csdn,juejin,xiaohongshu,cnki,dblp",
    "PROXY_URL": "http://127.0.0.1:7890",
    "USE_PROXY": "true",
    "ALLOWED_SEARCH_ENGINES": engine,
})
req = json.dumps({
    "jsonrpc": "2.0", "id": 1, "method": "tools/call",
    "params": {"name": "web", "arguments": {"query": query, "engines": [engine], "limit": 3}},
})

for i in range(n):
    p = subprocess.run([BIN], input=req, capture_output=True, text=True,
                       encoding="utf-8", errors="replace", env=env, timeout=180)
    d = json.loads(p.stdout.strip().splitlines()[0])
    r = json.loads(d["result"]["content"][0]["text"])
    fails = [f["message"][:70] for f in r.get("partialFailures", [])]
    print(f"try{i + 1}: total={r['totalResults']} fails={fails}")
    time.sleep(3)
