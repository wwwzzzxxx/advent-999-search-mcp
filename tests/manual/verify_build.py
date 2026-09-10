"""Verify a built binary reports the expected version and lists the expected tools.

Usage: python tests/manual/verify_build.py <binary> [expected-version]
"""
import json
import subprocess
import sys

BIN = sys.argv[1]
EXPECTED = sys.argv[2] if len(sys.argv) > 2 else "0.7.0"

init = json.dumps({"jsonrpc": "2.0", "id": 1, "method": "initialize",
                   "params": {"protocolVersion": "2024-11-05",
                              "capabilities": {},
                              "clientInfo": {"name": "verify", "version": "1"}}})
lst = json.dumps({"jsonrpc": "2.0", "id": 2, "method": "tools/list"})

p = subprocess.run([BIN], input=init + "\n" + lst + "\n", capture_output=True,
                   text=True, encoding="utf-8", errors="replace", timeout=60)

lines = [l for l in p.stdout.strip().splitlines() if l.strip()]
ok = True

try:
    info = json.loads(lines[0])["result"]["serverInfo"]
    ver = info["version"]
    match = ver == EXPECTED
    ok &= match
    print(f"  serverInfo : {info['name']} v{ver}  {'✓' if match else f'✗ (expected {EXPECTED})'}")
except Exception as e:  # noqa: BLE001
    ok = False
    print(f"  serverInfo : ✗ failed to parse ({e})")

try:
    tools = [t["name"] for t in json.loads(lines[1])["result"]["tools"]]
    print(f"  tools      : {tools}")
    for want in ("web", "get_page", "set_cookies"):
        hit = want in tools
        ok &= hit
        print(f"    {want:<12} {'✓' if hit else '✗ MISSING'}")
except Exception as e:  # noqa: BLE001
    ok = False
    print(f"  tools      : ✗ failed to parse ({e})")

print(f"\n  {'PASS' if ok else 'FAIL'}")
sys.exit(0 if ok else 1)
