"""Inspect what Sogou returns for a full-header request vs a simple one.

The two variants produced different response sizes (1165 vs 5420 bytes), so
they are different pages — worth knowing which is which.

Usage: python tests/manual/sogou_block_inspect.py
"""
import os
import re
import ssl
import urllib.request
import warnings

warnings.filterwarnings("ignore")
ctx = ssl._create_unverified_context()
COOKIES = os.environ.get("FETCH_COOKIES", "")
UA = ("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 "
      "(KHTML, like Gecko) Chrome/136.0.0.0 Safari/537.36")
URL = "https://www.sogou.com/web?query=test&ie=utf8"

VARIANTS = {
    "全头 + Client Hints": {
        "User-Agent": UA,
        "Accept": ("text/html,application/xhtml+xml,application/xml;q=0.9,"
                   "image/avif,image/webp,*/*;q=0.8"),
        "Accept-Language": "zh-CN,zh;q=0.9,en;q=0.8",
        "Referer": "https://www.sogou.com/",
        "sec-ch-ua": '"Chromium";v="136", "Google Chrome";v="136"',
        "sec-fetch-mode": "navigate",
        "sec-fetch-dest": "document",
    },
    "简头": {
        "User-Agent": UA,
        "Accept-Language": "zh-CN,zh;q=0.9",
    },
}

for label, headers in VARIANTS.items():
    h = dict(headers)
    if COOKIES:
        h["Cookie"] = COOKIES
    op = urllib.request.build_opener(urllib.request.ProxyHandler({}),
                                     urllib.request.HTTPSHandler(context=ctx))
    try:
        r = op.open(urllib.request.Request(URL, headers=h), timeout=25)
        html = r.read().decode("utf-8", "ignore")
    except Exception as e:  # noqa: BLE001
        print(f"=== {label} === ERR {type(e).__name__}: {e}\n")
        continue

    text = re.sub(r"\s+", " ", re.sub(r"<[^>]+>", " ", html)).strip()
    print(f"=== {label} ===")
    print(f"  final url : {r.url[:95]}")
    print(f"  length    : {len(html)}")
    print(f"  server    : {r.headers.get('Server')}")
    print(f"  title     : {(re.search(r'<title>(.*?)</title>', html, re.S) or [None, '?'])[1][:60]}")
    print(f"  visible   : {text[:260]}")
    print()
