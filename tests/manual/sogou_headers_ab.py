"""Does claiming to be Chrome (via sec-ch-ua headers) HURT Sogou requests?

Anti-bot systems cross-check the User-Agent / Client-Hints against the TLS
fingerprint. A request that says "I am Chrome 136" while presenting an
OpenSSL/rustls fingerprint is *inconsistent* — a stronger bot signal than a
plain request that claims nothing.

Runs ONE request per variant to avoid burning more quota.

Usage: python tests/manual/sogou_headers_ab.py
"""
import os
import ssl
import urllib.error
import urllib.request
import warnings

warnings.filterwarnings("ignore")
ctx = ssl._create_unverified_context()
COOKIES = os.environ.get("FETCH_COOKIES", "")
URL = "https://www.sogou.com/web?query=%E6%B7%B1%E5%BA%A6%E5%AD%A6%E4%B9%A0&ie=utf8"

UA = ("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 "
      "(KHTML, like Gecko) Chrome/136.0.0.0 Safari/537.36")

# A: what the engine currently sends (claims Chrome via Client Hints)
FULL = {
    "User-Agent": UA,
    "Accept": ("text/html,application/xhtml+xml,application/xml;q=0.9,image/avif,image/webp,"
               "image/apng,*/*;q=0.8,application/signed-exchange;v=b3;q=0.7"),
    "Accept-Language": "zh-CN,zh;q=0.9,en;q=0.8",
    "Accept-Encoding": "gzip, deflate, br",
    "Referer": "https://www.sogou.com/",
    "sec-ch-ua": '"Chromium";v="136", "Google Chrome";v="136", "Not?A_Brand";v="99"',
    "sec-ch-ua-mobile": "?0",
    "sec-ch-ua-platform": '"Windows"',
    "sec-fetch-site": "same-origin",
    "sec-fetch-mode": "navigate",
    "sec-fetch-user": "?1",
    "sec-fetch-dest": "document",
    "Upgrade-Insecure-Requests": "1",
}

# B: minimal — no claim about being Chrome
SIMPLE = {
    "User-Agent": UA,
    "Accept": "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8",
    "Accept-Language": "zh-CN,zh;q=0.9",
}

# C: full headers minus the Client Hints (sec-ch-ua family)
NO_CH = {k: v for k, v in FULL.items() if not k.startswith("sec-ch-ua")}

if not COOKIES:
    print("ERROR: set FETCH_COOKIES first")
    raise SystemExit(2)


def probe(label, headers):
    h = dict(headers)
    h["Cookie"] = COOKIES
    op = urllib.request.build_opener(urllib.request.ProxyHandler({}),
                                     urllib.request.HTTPSHandler(context=ctx))
    try:
        r = op.open(urllib.request.Request(URL, headers=h), timeout=25)
        html = r.read().decode("utf-8", "ignore")
        blocked = "antispider" in r.url.lower() or "请依次点击" in html
        print(f"  {label:<34} HTTP {r.status} len={len(html):<8} "
              f"{'BLOCKED' if blocked else 'OK ✅'}")
        return not blocked
    except urllib.error.HTTPError as e:
        print(f"  {label:<34} HTTP {e.code} {'(403 hard block)' if e.code == 403 else ''}")
        return False
    except Exception as e:  # noqa: BLE001
        print(f"  {label:<34} ERR {type(e).__name__}")
        return False


print(f"cookies: {len(COOKIES)} bytes, SNUID={'SNUID=' in COOKIES}\n")
print("=== 单次请求对照（同一 cookie，同一 IP）===")
probe("A. 全头 + sec-ch-ua（引擎现状）", FULL)
probe("B. 简头（不声称 Chrome）", SIMPLE)
probe("C. 全头但去掉 sec-ch-ua", NO_CH)
print("\n提示：若 A 被拦而 B/C 通过，说明 Client Hints 与 TLS 指纹不一致反而暴露了脚本身份。")
