"""Generic Anubis PoW client test — handles both GET and POST protected endpoints.

Proves the approach before porting it to Rust:
  1. request the protected resource
  2. if an Anubis challenge comes back, brute-force the nonce
  3. GET /api/pass-challenge to obtain the auth cookie
  4. replay the original request (as a real browser would after the JS solved it)
"""
import gzip
import hashlib
import http.cookiejar
import json
import re
import ssl
import sys
import time
import urllib.parse
import urllib.request
import warnings

warnings.filterwarnings("ignore")
ctx = ssl._create_unverified_context()
UA = ("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 "
      "(KHTML, like Gecko) Chrome/136.0.0.0 Safari/537.36")
PROXY = {"http": "http://127.0.0.1:7890", "https": "http://127.0.0.1:7890"}


def decompress(raw, enc):
    enc = (enc or "").lower()
    if raw[:2] == b"\x1f\x8b" or "gzip" in enc:
        return gzip.decompress(raw)
    return raw


def solve_challenge(challenge, difficulty):
    """Find nonce so sha256hex(randomData + nonce) has `difficulty` leading zero hex digits."""
    prefix = "0" * difficulty
    rd = challenge["randomData"]
    t0 = time.time()
    nonce = 0
    while True:
        h = hashlib.sha256(f"{rd}{nonce}".encode()).hexdigest()
        if h.startswith(prefix):
            return nonce, h, int((time.time() - t0) * 1000)
        nonce += 1


def run(label, url, data=None, headers=None, extra_cookie=None):
    jar = http.cookiejar.CookieJar()
    op = urllib.request.build_opener(
        urllib.request.ProxyHandler(PROXY),
        urllib.request.HTTPSHandler(context=ctx),
        urllib.request.HTTPCookieProcessor(jar),
    )
    origin = re.match(r"https?://[^/]+", url).group(0)
    h = {"User-Agent": UA, "Accept-Language": "en-US,en;q=0.9",
         "Accept": "text/html,application/xhtml+xml,*/*;q=0.8"}
    if headers:
        h.update(headers)
    if extra_cookie:
        h["Cookie"] = extra_cookie
    body = urllib.parse.urlencode(data).encode() if data else None

    print(f"\n{'=' * 70}\n{label}\n{'=' * 70}")
    print(f"  URL: {url[:80]}")

    # 1) first attempt
    r = op.open(urllib.request.Request(url, data=body, headers=h), timeout=40)
    html = decompress(r.read(), r.headers.get("Content-Encoding")).decode("utf-8", "ignore")
    print(f"  [1] attempt 1      -> HTTP {r.status} len={len(html)}")

    if "anubis_challenge" not in html:
        print(f"      no challenge! body: {html[:120]}")
        return html

    m = re.search(r'<script id="anubis_challenge"[^>]*>(.*?)</script>', html, re.S)
    chal = json.loads(m.group(1))
    ch, rules = chal["challenge"], chal["rules"]
    diff = rules.get("difficulty", ch.get("difficulty", 4))
    print(f"  [2] challenge      -> {rules.get('algorithm')} difficulty={diff} id={ch['id']}")

    nonce, digest, ms = solve_challenge(ch, diff)
    print(f"  [3] solved         -> nonce={nonce} in {ms} ms ({nonce + 1} hashes)")

    # 3) pass the challenge (GET)
    pass_url = origin + "/.within.website/x/cmd/anubis/api/pass-challenge?" + \
        urllib.parse.urlencode({"id": ch["id"], "response": digest, "nonce": str(nonce),
                                "redir": url, "elapsedTime": str(max(ms, 1))})
    try:
        r2 = op.open(urllib.request.Request(pass_url, headers={
            "User-Agent": UA, "Accept-Language": "en-US,en;q=0.9"}), timeout=40)
        print(f"  [4] pass-challenge -> HTTP {r2.status}")
    except urllib.error.HTTPError as e:
        print(f"  [4] pass-challenge -> HTTP {e.code}")

    names = [c.name for c in jar]
    print(f"  [5] cookies        -> {names}")

    # 4) replay original request with the auth cookie
    h2 = dict(h)
    r3 = op.open(urllib.request.Request(url, data=body, headers=h2), timeout=40)
    html3 = decompress(r3.read(), r3.headers.get("Content-Encoding")).decode("utf-8", "ignore")
    still = "anubis_challenge" in html3
    print(f"  [6] replay         -> HTTP {r3.status} len={len(html3)} "
          f"{'STILL CHALLENGED' if still else 'PASSED ANUBIS'}")
    if not still:
        print(f"      body: {html3[:150]}")
    return html3


# --- dblp (GET, JSON API) ---
run("DBLP (GET json api)", "https://dblp.org/search/publ/api?q=transformer&format=json&h=2&f=0")

# --- startpage (POST search) ---
sc = None
op0 = urllib.request.build_opener(urllib.request.ProxyHandler(PROXY),
                                  urllib.request.HTTPSHandler(context=ctx))
rh = op0.open(urllib.request.Request("https://www.startpage.com/",
              headers={"User-Agent": UA, "Accept-Language": "en-US,en;q=0.9"}), timeout=30)
hh = decompress(rh.read(), rh.headers.get("Content-Encoding")).decode("utf-8", "ignore")
mm = re.search(r'name="sc"\s+value="([^"]+)"', hh) or re.search(r'value="([^"]+)"\s+name="sc"', hh)
if mm:
    sc = mm.group(1)
    run("Startpage (POST search)",
        "https://www.startpage.com/sp/search",
        data={"query": "python asyncio", "cat": "web", "t": "device", "sc": sc,
              "abp": "1", "abd": "1", "abe": "1"},
        headers={"Content-Type": "application/x-www-form-urlencoded",
                 "Origin": "https://www.startpage.com",
                 "Referer": "https://www.startpage.com/"})
else:
    print("\n(no sc code found for startpage)")
