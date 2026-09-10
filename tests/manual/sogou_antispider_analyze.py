"""Offline analysis of the saved Sogou antispider page (no network requests).

Usage: python tests/manual/sogou_antispider_analyze.py
"""
import io
import os
import re

path = os.path.join(os.environ.get("TEMP", "."), "sogou_antispider.html")
html = io.open(path, encoding="utf-8").read()
print(f"file: {path} ({len(html)} chars)\n")

scripts = re.findall(r"<script(?![^>]*src)[^>]*>(.*?)</script>", html, re.S)
main = max(scripts, key=len)
print(f"main inline script: {len(main)} chars\n")

for marker in ("seccodeRight", "location", "reload", "imgCode", "verify", "submit"):
    for m in list(re.finditer(re.escape(marker), main))[:2]:
        seg = re.sub(r"\s+", " ", main[max(0, m.start() - 120):m.start() + 300])
        print(f"[{marker}]\n  …{seg}…\n")
