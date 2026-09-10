"""Analyse the solved Startpage result page so the Rust parser can target the
current markup (the old .w-gl__result-wrapper container is gone).

Usage: python tests/manual/startpage_parse_probe.py
"""
import io
import re
from collections import Counter

PATH = r"C:\Users\wzx\AppData\Local\Temp\sp_final.html"
html = io.open(PATH, encoding="utf-8").read()

print(f"file         : {PATH}")
print(f"length       : {len(html):,}")
print(f"<title>      : {(re.search(r'<title>(.*?)</title>', html, re.S) or [None, '?'])[1][:60]}")

print("\n--- selector frequency ---")
for c in ("w-gl__result", "result-title", "wgl-title", "result-link",
          "gl-description", "w-gl", "gl-result", "result-description",
          "description", "result-wrapper"):
    print(f"  {c:<22} {html.count(c)}")

print("\n--- data-testid values ---")
for t in sorted(set(re.findall(r'data-testid="([^"]+)"', html))):
    print(f"  {t}")

print("\n--- first result-title anchor block ---")
i = html.find("result-title")
print(html[i - 60:i + 100].replace("\n", " "))

print("\n--- element right AFTER the title anchor (description?) ---")
m = re.search(r"</a>(.{0,900})", html[i:], re.S)
if m:
    print(re.sub(r"<style[^>]*>.*?</style>", "", m.group(1), flags=re.S)[:600].replace("\n", " "))

print("\n--- classes used on <p> elements near results ---")
for c, n in Counter(re.findall(r'<p class="([^"]+)"', html[i:i + 20000])).most_common(8):
    print(f"  {n:>3}x  {c}")

print("\n--- hrefs of all result-title anchors ---")
for m in list(re.finditer(r'<a class="result-title[^"]*"[^>]*href="([^"]+)"', html))[:6]:
    print(f"  {m.group(1)[:88]}")
