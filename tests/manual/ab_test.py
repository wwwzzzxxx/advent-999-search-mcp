"""A/B test: does DIRECT_DOMAINS actually bypass the proxy?

Runs the MCP server binary with different DIRECT_DOMAINS / env-proxy combos
and reports whether the `cnki` engine succeeds.

Usage: python ab_test.py [path-to-binary]
"""
import json
import os
import subprocess
import sys

BIN = sys.argv[1] if len(sys.argv) > 1 else r".\target\debug\advent-999-search-mcp.exe"
REQ = json.dumps({
    "jsonrpc": "2.0",
    "id": 1,
    "method": "tools/call",
    "params": {"name": "web", "arguments": {"query": "知识图谱", "engines": ["cnki"], "limit": 2}},
})


def run(label, direct_domains, env_proxy):
    env = dict(os.environ)
    env["PROXY_URL"] = "http://127.0.0.1:7890"
    env["USE_PROXY"] = "true"
    env["ALLOWED_SEARCH_ENGINES"] = "cnki"
    env["DIRECT_DOMAINS"] = direct_domains
    for k in ("HTTP_PROXY", "HTTPS_PROXY", "http_proxy", "https_proxy", "ALL_PROXY", "all_proxy"):
        env.pop(k, None)
    if env_proxy:
        for k in ("HTTP_PROXY", "HTTPS_PROXY", "http_proxy", "https_proxy"):
            env[k] = env_proxy

    p = subprocess.run([BIN], input=REQ, capture_output=True, text=True,
                       encoding="utf-8", errors="replace", env=env, timeout=120)
    print(f"--- {label} ---")
    print(f"    DIRECT_DOMAINS={direct_domains!r}  env_HTTPS_PROXY={env_proxy!r}")
    try:
        d = json.loads(p.stdout.strip().splitlines()[0])
        r = json.loads(d["result"]["content"][0]["text"])
        fails = [f["message"][:75] for f in r.get("partialFailures", [])]
        print(f"    => total={r['totalResults']}  fails={fails}")
        for x in r["results"][:2]:
            print(f"       * {x['title'][:58]}")
    except Exception as e:  # noqa: BLE001
        print(f"    => parse error: {e}\n       stdout={p.stdout[:200]!r}")
    print()


if __name__ == "__main__":
    run("A. 声明走代理（none）        ", "none", None)
    run("B. 声明直连 cnki            ", "cnki", None)
    run("C. 声明直连 + 环境变量代理(可用)", "cnki", "http://127.0.0.1:7890")
    run("D. 声明直连 + 环境变量代理(死) ", "cnki", "http://127.0.0.1:1")
