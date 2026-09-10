"""Isolate which Sogou request attribute triggers the anti-bot page.

Variables: page=1 param, full browser headers, and direct vs proxy.
"""
import ssl
import time
import urllib.request
import warnings

warnings.filterwarnings("ignore")
ctx = ssl._create_unverified_context()
UA = ("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 "
      "(KHTML, like Gecko) Chrome/136.0.0.0 Safari/537.36")
Q = "%E6%B7%B1%E5%BA%A6%E5%AD%A6%E4%B9%A0"
PROXY = {"http": "http://127.0.0.1:7890", "https": "http://127.0.0.1:7890"}

SIMPLE = {"User-Agent": UA, "Accept-Language": "zh-CN,zh;q=0.9"}
FULL = {
    "User-Agent": UA,
    "Accept": ("text/html,application/xhtml+xml,application/xml;q=0.9,image/avif,"
               "image/webp,image/apng,*/*;q=0.8,application/signed-exchange;v=b3;q=0.7"),
    "Accept-Language": "zh-CN,zh;q=0.9,en;q=0.8",
    "Referer": "https://www.sogou.com/",
    "sec-ch-ua": '"Chromium";v="136", "Google Chrome";v="136", "Not?A_Brand";v="99"',
    "sec-ch-ua-mobile": "?0",
    "sec-ch-ua-platform": '"Windows"',
    "sec-fetch-site": "same-origin",
    "sec-fetch-mode": "navigate",
    "sec-fetch-user": "?1",
    "sec-fetch-dest": "document",
}

CASES = [
    ("无 page + 简头", "https://www.sogou.com/web?query=" + Q + "&ie=utf8", SIMPLE),
    ("无 page + 全头", "https://www.sogou.com/web?query=" + Q + "&ie=utf8", FULL),
    ("page=1 + 简头", "https://www.sogou.com/web?query=" + Q + "&page=1&ie=utf8", SIMPLE),
    ("page=1 + 全头", "https://www.sogou.com/web?query=" + Q + "&page=1&ie=utf8", FULL),
]

for plabel, proxies in (("走代理", PROXY), ("直连", {})):
    print(f"=== {plabel} ===")
    for name, url, hdrs in CASES:
        op = urllib.request.build_opener(urllib.request.ProxyHandler(proxies),
                                         urllib.request.HTTPSHandler(context=ctx))
        try:
            r = op.open(urllib.request.Request(url, headers=hdrs), timeout=25)
            h = r.read().decode("utf-8", "ignore")
            blocked = "antispider" in r.url.lower() or "请依次点击" in h
            print(f"  {name:<16} len={len(h):<8} blocked={blocked}")
        except Exception as e:  # noqa: BLE001
            print(f"  {name:<16} ERR {type(e).__name__}: {str(e)[:40]}")
        time.sleep(4)
    print()
