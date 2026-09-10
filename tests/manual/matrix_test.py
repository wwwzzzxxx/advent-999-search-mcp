"""Matrix test: which engines work under global/system proxy ON vs OFF.

The system proxy is read by reqwest from the Windows registry, so this script
does NOT clear HTTP_PROXY/HTTPS_PROXY — it reproduces the real user condition
(VPN "global proxy" = registry proxy enabled).

Usage: python tests/manual/matrix_test.py <binary> <proxy|noproxy> [engines...]
"""
import json
import os
import pathlib
import subprocess
import sys
import time

BIN = sys.argv[1] if len(sys.argv) > 1 else r"target\debug\advent-999-search-mcp.exe"
MODE = sys.argv[2] if len(sys.argv) > 2 else "proxy"

QUERIES = {
    "exa": "deep learning",
    "bing": "深度学习",
    "csdn": "深度学习",
    "juejin": "大模型",
    "startpage": "python asyncio",
    "sogou": "深度学习",
    "weixin": "大模型",
    "dblp": "transformer",
    "cnki": "知识图谱",
    "ieee": "deep learning",
}
ENGINES = sys.argv[3:] or list(QUERIES)


def load_mcp_env():
    """Reuse the keys the user already configured — never hardcode credentials."""
    p = pathlib.Path(os.environ.get("APPDATA", "")) / "Code" / "User" / "mcp.json"
    try:
        cfg = json.loads(p.read_text(encoding="utf-8"))
        return dict(cfg["servers"]["advent"]["env"])
    except Exception as e:  # noqa: BLE001
        print(f"  (warn: could not read {p}: {e})")
        return {}


env = dict(os.environ)
env.update(load_mcp_env())
env["DIRECT_DOMAINS"] = "sogou,weixin,baidu,bilibili,hdslb,zhihu,csdn,juejin,xiaohongshu,cnki,dblp"
env["ALLOWED_SEARCH_ENGINES"] = ",".join(QUERIES)
env["DEFAULT_SEARCH_ENGINE"] = "exa"
if MODE == "proxy":
    env["USE_PROXY"] = "true"
    env["PROXY_URL"] = "http://127.0.0.1:7890"
else:
    env["USE_PROXY"] = "false"
    env.pop("PROXY_URL", None)

print(f"binary : {BIN}")
print(f"mode   : USE_PROXY={env['USE_PROXY']}  (registry/system proxy untouched)")
print(f"\n{'engine':<11} {'n':>3}  detail")
print("-" * 84)

passed = 0
for e in ENGINES:
    req = json.dumps({
        "jsonrpc": "2.0", "id": 1, "method": "tools/call",
        "params": {"name": "web", "arguments": {"query": QUERIES[e], "engines": [e], "limit": 3}},
    })
    try:
        p = subprocess.run([BIN], input=req, capture_output=True, text=True,
                           encoding="utf-8", errors="replace", env=env, timeout=180)
        line = p.stdout.strip().splitlines()[0]
        r = json.loads(json.loads(line)["result"]["content"][0]["text"])
        fails = [f["message"][:56] for f in r.get("partialFailures", [])]
        if r["totalResults"] > 0:
            passed += 1
            print(f"{e:<11} {r['totalResults']:>3}  OK   {r['results'][0]['title'][:46]}")
        else:
            print(f"{e:<11} {r['totalResults']:>3}  FAIL {fails[0] if fails else '(silent 0)'}")
    except Exception as ex:  # noqa: BLE001
        print(f"{e:<11} {'-':>3}  ERR  {type(ex).__name__} {str(ex)[:44]}")
    time.sleep(2)

print("-" * 84)
print(f"passing = {passed}/{len(ENGINES)}")
