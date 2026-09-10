"""Time each step of the dblp Anubis flow, to find where the latency is."""
import gzip
import hashlib
import http.cookiejar
import json
import re
import ssl
import time
import urllib.parse
import urllib.request
import warnings

warnings.filterwarnings("ignore")
ctx = ssl._create_unverified_context()
UA = ("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 "
      "(KHTML, like Gecko) Chrome/136.0.0.0 Safari/537.36")
PROXY = {"http": "http://127.0.0.1:7890", "https": "http://127.0.0.1:7890"}
HOST = "dblp.uni-trier.de"
URL = f"https://{HOST}/search/publ/api?q=transformer&format=json&h=3&f=0"


def t():
    return time.time()


def decomp(raw, enc):
    if raw[:2] == b"\x1f\x8b" or "gzip" in (enc or "").lower():
        return gzip.decompress(raw)
    return raw


def fetch(op, url, name):
    t0 = t()
    r = op.open(urllib.request.Request(url, headers={"User-Agent": UA}), timeout=60)
    body = decomp(r.read(), r.headers.get("Content-Encoding")).decode("utf-8", "ignore")
    print(f"  {name:<34} {t() - t0:6.2f}s  HTTP {r.status}  len={len(body):<7} "
          f"anubis={'anubis_challenge' in body}")
    return body


for label, proxies in (("直连", {}), ("代理", PROXY)):
    print(f"=== {label} ===")
    jar = http.cookiejar.CookieJar()
    op = urllib.request.build_opener(urllib.request.ProxyHandler(proxies),
                                     urllib.request.HTTPSHandler(context=ctx),
                                     urllib.request.HTTPCookieProcessor(jar))
    html = fetch(op, URL, "1. 首次请求")

    if "anubis_challenge" not in html:
        print("     (no challenge)\n")
        continue

    m = re.search(r'<script id="anubis_challenge"[^>]*>(.*?)</script>', html, re.S)
    ch = json.loads(m.group(1))
    c, diff = ch["challenge"], ch["rules"]["difficulty"]

    t0 = t()
    n, prefix = 0, "0" * diff
    while True:
        dg = hashlib.sha256((c["randomData"] + str(n)).encode()).hexdigest()
        if dg.startswith(prefix):
            break
        n += 1
    print(f"  {'2. PoW 求解':<34} {t() - t0:6.2f}s  nonce={n}")

    pu = f"https://{HOST}/.within.website/x/cmd/anubis/api/pass-challenge?" + \
        urllib.parse.urlencode({"id": c["id"], "response": dg, "nonce": str(n),
                                "redir": f"https://{HOST}/", "elapsedTime": "100"})
    fetch(op, pu, "3. pass-challenge（含重定向）")

    fetch(op, URL, "4. 重放请求")

    # Warm repeat, reusing the cookie.
    fetch(op, URL, "5. 再次请求（复用 cookie）")
    print()
