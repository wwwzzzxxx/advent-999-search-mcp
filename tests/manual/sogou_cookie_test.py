"""Verify that browser cookies (SNUID/SUV from the user's real session) let a
programmatic Sogou request through.

This is the "cookie persistence" bypass: SNUID is issued after a human solves
the captcha and is NOT HttpOnly, so it can be lifted from the browser and
replayed.  Kept to a single request to avoid burning quota.
"""
import os
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

# Cookies are read from the FETCH_COOKIES environment variable — never
# hardcoded, so nothing leaks if this repo is public.
COOKIES = os.environ.get("FETCH_COOKIES", "")
URL = "https://www.sogou.com/web?" + urllib.parse.urlencode(
    {"query": "深度学习", "ie": "utf8"})

if not COOKIES:
    print("ERROR: set the FETCH_COOKIES env var first, e.g.\n"
          '  $env:FETCH_COOKIES = "SNUID=...; SUV=...; SUID=...; ABTEST=...; IPLOC=..."\n')
    sys.exit(2)

print(f"cookie bytes: {len(COOKIES)}")
print(f"has SNUID: {'SNUID' in COOKIES}   has SUV: {'SUV' in COOKIES}\n")

op = urllib.request.build_opener(urllib.request.ProxyHandler({}),
                                 urllib.request.HTTPSHandler(context=ctx))
h = {"User-Agent": UA, "Accept-Language": "zh-CN,zh;q=0.9"}
if COOKIES:
    h["Cookie"] = COOKIES
r = op.open(urllib.request.Request(URL, headers=h), timeout=25)
html = r.read().decode("utf-8", "ignore")

blocked = "antispider" in r.url.lower() or "请依次点击" in html
print(f"HTTP {r.status}  len={len(html)}  blocked={blocked}")
print(f"title: {(re.search(r'<title>(.*?)</title>', html, re.S) or [None, '?'])[1][:60]}")

if not blocked:
    hits = re.findall(r'<h3[^>]*>\s*<a[^>]*href="([^"]+)"[^>]*>(.*?)</a>', html, re.S)
    print(f"\n结果条目: {len(hits)}")
    for url, title in hits[:5]:
        t = re.sub(r"<[^>]+>", "", title).strip()
        print(f"  - {t[:58]}")
