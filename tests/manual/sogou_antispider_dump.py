"""Dump Sogou's antispider page script to understand its SNUID check.

The captcha page contains a `checkSNUID()` helper, which tells us how Sogou
expects a solved session to behave (and what it does when SNUID is present).

Usage: python tests/manual/sogou_antispider_dump.py
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

h = {"User-Agent": UA, "Accept-Language": "zh-CN,zh;q=0.9"}
if COOKIES:
    h["Cookie"] = COOKIES

op = urllib.request.build_opener(urllib.request.ProxyHandler({}),
                                 urllib.request.HTTPSHandler(context=ctx))
r = op.open(urllib.request.Request("https://www.sogou.com/web?query=test&ie=utf8",
                                   headers=h), timeout=25)
html = r.read().decode("utf-8", "ignore")

print(f"final url: {r.url[:110]}")
print(f"length   : {len(html)}\n")

# Inline scripts
out = os.path.join(os.environ.get("TEMP", "."), "sogou_antispider.html")
with open(out, "w", encoding="utf-8") as f:
    f.write(html)
print(f"saved -> {out}\n")

for i, m in enumerate(re.finditer(r"<script(?![^>]*src)[^>]*>(.*?)</script>", html, re.S)):
    body = m.group(1).strip()
    if not body:
        continue
    print(f"--- inline script #{i + 1} ({len(body)} chars) ---")
    print(body)
    print()

for m in re.finditer(r'<script[^>]+src="([^"]+)"', html):
    print(f"--- external: {m.group(1)}")
