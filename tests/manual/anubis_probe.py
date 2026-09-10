"""Inspect an Anubis challenge page: challenge payload, script sources, and the
JS logic that solves it. Used to work out how to pass the PoW programmatically.

Usage: python tests/manual/anubis_probe.py [url]
"""
import gzip
import json
import re
import ssl
import sys
import urllib.request
import zlib

URL = sys.argv[1] if len(sys.argv) > 1 else \
    "https://dblp.org/search/publ/api?q=transformer&format=json&h=2&f=0"
PROXY = {"http": "http://127.0.0.1:7890", "https": "http://127.0.0.1:7890"}
UA = ("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 "
      "(KHTML, like Gecko) Chrome/136.0.0.0 Safari/537.36")
ctx = ssl._create_unverified_context()


def get(url, headers=None):
    h = {"User-Agent": UA, "Accept": "text/html,application/xhtml+xml,*/*;q=0.8",
         "Accept-Language": "en-US,en;q=0.9"}
    if headers:
        h.update(headers)
    op = urllib.request.build_opener(urllib.request.ProxyHandler(PROXY),
                                     urllib.request.HTTPSHandler(context=ctx))
    r = op.open(urllib.request.Request(url, headers=h), timeout=25)
    raw = r.read()
    enc = (r.headers.get("Content-Encoding") or "").lower()
    if raw[:2] == b"\x1f\x8b" or "gzip" in enc:
        raw = gzip.decompress(raw)
    elif "deflate" in enc:
        raw = zlib.decompress(raw, -zlib.MAX_WBITS)
    return r, raw.decode("utf-8", "ignore")


print(f"URL: {URL}")
print(f"base: {re.match(r'https?://[^/]+', URL).group(0)}\n")

r, html = get(URL)
base = re.match(r"https?://[^/]+", URL).group(0)
print(f"HTTP {r.status}  len={len(html)}  content-type={r.headers.get('Content-Type')}")
print(f"Server: {r.headers.get('Server')}")

# --- challenge payload ---
for cid in ("anubis_version", "anubis_challenge", "anubis_base_prefix"):
    m = re.search(r'<script id="' + cid + r'"[^>]*>(.*?)</script>', html, re.S)
    if m:
        val = m.group(1).strip()
        if cid == "anubis_challenge":
            try:
                print(f"\n[{cid}]")
                print(json.dumps(json.loads(val), indent=2, ensure_ascii=False)[:900])
            except Exception:  # noqa: BLE001
                print(f"\n[{cid}] {val[:300]}")
        else:
            print(f"\n[{cid}] {val[:120]}")

# --- scripts ---
print("\n--- script tags ---")
for m in re.finditer(r"<script([^>]*)>(.*?)</script>", html, re.S):
    attrs, body = m.group(1), m.group(2).strip()
    src = re.search(r'src="([^"]+)"', attrs)
    if src:
        print(f"  SRC  {src.group(1)}")
    elif body:
        print(f"  INLINE ({len(body)} chars) {body[:110]!r}")

# --- look for the challenge/verify endpoint references ---
print("\n--- endpoint hints in HTML ---")
for pat in ("pass-challenge", "api/challenge", "/.within.website/", "challenge_url",
            "verify", "redir"):
    for m in re.finditer(re.escape(pat), html):
        line = html[max(0, m.start() - 90):m.start() + 90].replace("\n", " ")
        print(f"  [{pat}] ...{line}...")
        break

# --- download and grep the JS bundle ---
srcs = re.findall(r'<script[^>]+src="([^"]+)"', html)
print(f"\n--- downloading {len(srcs)} script(s) ---")
for s in srcs:
    full = s if s.startswith("http") else base + ("" if s.startswith("/") else "/") + s
    try:
        _, js = get(full)
    except Exception as e:  # noqa: BLE001
        print(f"  {s} -> ERR {type(e).__name__}")
        continue
    print(f"  {s} -> {len(js)} chars")
    for pat in ("pass-challenge", "nonce", "randomData", "difficulty", "sha256",
                "startsWith", "repeat(", "elapsedTime", "challenge"):
        hits = [m.start() for m in re.finditer(re.escape(pat), js)][:2]
        for h in hits:
            ctxs = js[max(0, h - 100):h + 110].replace("\n", " ")
            print(f"      [{pat}] ...{ctxs}...")
