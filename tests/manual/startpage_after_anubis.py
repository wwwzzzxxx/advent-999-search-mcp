"""Solve Anubis for Startpage and inspect the result page structure,
so the Rust parser selectors can be checked/fixed.
"""
import gzip
import hashlib
import http.cookiejar
import json
import re
import ssl
import sys
import urllib.parse
import urllib.request
import warnings

warnings.filterwarnings("ignore")
ctx = ssl._create_unverified_context()
UA = ("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 "
      "(KHTML, like Gecko) Chrome/136.0.0.0 Safari/537.36")
PROXY = {"http": "http://127.0.0.1:7890", "https": "http://127.0.0.1:7890"}
ORIGIN = "https://www.startpage.com"

jar = http.cookiejar.CookieJar()
op = urllib.request.build_opener(urllib.request.ProxyHandler(PROXY),
                                 urllib.request.HTTPSHandler(context=ctx),
                                 urllib.request.HTTPCookieProcessor(jar))


def dec(raw, enc):
    if raw[:2] == b"\x1f\x8b" or "gzip" in (enc or "").lower():
        return gzip.decompress(raw)
    return raw


def get(url, headers=None):
    h = {"User-Agent": UA, "Accept-Language": "en-US,en;q=0.9"}
    if headers:
        h.update(headers)
    r = op.open(urllib.request.Request(url, headers=h), timeout=40)
    return dec(r.read(), r.headers.get("Content-Encoding")).decode("utf-8", "ignore")


html = get(ORIGIN + "/")
m = re.search(r'name="sc"\s+value="([^"]+)"', html) or \
    re.search(r'value="([^"]+)"\s+name="sc"', html)
sc = m.group(1)
print(f"sc code: {sc[:30]}...")

form = {"query": "python asyncio", "cat": "web", "t": "device", "sc": sc,
        "abp": "1", "abd": "1", "abe": "1"}
hdr = {"User-Agent": UA, "Accept-Language": "en-US,en;q=0.9",
       "Content-Type": "application/x-www-form-urlencoded",
       "Origin": ORIGIN, "Referer": ORIGIN + "/"}
body = urllib.parse.urlencode(form).encode()

r = op.open(urllib.request.Request(ORIGIN + "/sp/search", data=body, headers=hdr), timeout=40)
page = dec(r.read(), r.headers.get("Content-Encoding")).decode("utf-8", "ignore")
print(f"attempt 1: len={len(page)} challenged={'anubis_challenge' in page}")

if "anubis_challenge" in page:
    m = re.search(r'<script id="anubis_challenge"[^>]*>(.*?)</script>', page, re.S)
    chal = json.loads(m.group(1))
    c, diff = chal["challenge"], chal["rules"]["difficulty"]
    prefix = "0" * diff
    n = 0
    while True:
        digest = hashlib.sha256((c["randomData"] + str(n)).encode()).hexdigest()
        if digest.startswith(prefix):
            break
        n += 1
    print(f"solved: difficulty={diff} nonce={n}")
    pu = ORIGIN + "/.within.website/x/cmd/anubis/api/pass-challenge?" + urllib.parse.urlencode(
        {"id": c["id"], "response": digest, "nonce": str(n),
         "redir": ORIGIN + "/sp/search", "elapsedTime": "500"})
    op.open(urllib.request.Request(pu, headers={"User-Agent": UA}), timeout=40)
    print(f"cookies: {[ck.name for ck in jar]}")
    r2 = op.open(urllib.request.Request(ORIGIN + "/sp/search", data=body, headers=hdr),
                 timeout=40)
    page = dec(r2.read(), r2.headers.get("Content-Encoding")).decode("utf-8", "ignore")

print(f"final: len={len(page)} challenged={'anubis_challenge' in page}")
print(f"title: {(re.search(r'<title>(.*?)</title>', page, re.S) or [None, '?'])[1][:60]}")

print("\n--- selector counts (what our Rust parser looks for) ---")
for sel in ("w-gl__result", "w-gl__result-wrapper", "result-title", "result-wrapper",
            "result-description", "description", 'class="result'):
    print(f"  {sel:<24} {page.count(sel)}")

print("\n--- sample result anchors ---")
for m in list(re.finditer(r'<a[^>]+class="[^"]*result-title[^"]*"[^>]*>', page))[:3]:
    print("  " + m.group(0)[:150])
print("  ...")
for m in list(re.finditer(r'<a[^>]+href="(https?://[^"]+)"[^>]*class="[^"]*"[^>]*>', page))[:5]:
    print(f"  href={m.group(1)[:80]}")

path = r"C:\Users\wzx\AppData\Local\Temp\sp_final.html"
open(path, "w", encoding="utf-8").write(page)
print(f"\nsaved -> {path}")
