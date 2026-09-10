"""A/B Sogou: does dropping `page=1` + sec-ch-ua headers help?
Alternates the two request shapes with a gap so both see the same IP state.
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

# A = what the MCP engine currently sends
A_URL = "https://www.sogou.com/web?query=" + Q + "&page=1&ie=utf8"
A_HDR = {
    "User-Agent": UA,
    "Accept": ("text/html,application/xhtml+xml,application/xml;q=0.9,image/avif,image/webp,"
               "image/apng,*/*;q=0.8,application/signed-exchange;v=b3;q=0.7"),
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

# B = minimal shape
B_URL = "https://www.sogou.com/web?query=" + Q + "&ie=utf8"
B_HDR = {"User-Agent": UA, "Accept-Language": "zh-CN,zh;q=0.9"}


def hit(label, url, hdrs):
    op = urllib.request.build_opener(urllib.request.ProxyHandler(PROXY),
                                     urllib.request.HTTPSHandler(context=ctx))
    try:
        r = op.open(urllib.request.Request(url, headers=hdrs), timeout=25)
        h = r.read().decode("utf-8", "ignore")
        blocked = "antispider" in r.url.lower() or "请依次点击" in h
        print(f"  {label:<28} len={len(h):<8} {'BLOCKED' if blocked else 'OK'}")
        return not blocked
    except Exception as e:  # noqa: BLE001
        print(f"  {label:<28} ERR {type(e).__name__}")
        return False


a_ok = b_ok = 0
for rnd in range(3):
    print(f"--- round {rnd + 1} ---")
    a_ok += hit("A: 完整头 + page=1 (现状)", A_URL, A_HDR)
    time.sleep(12)
    b_ok += hit("B: 简头 + 无 page (候选)", B_URL, B_HDR)
    time.sleep(12)

print(f"\nA (current MCP shape) passed {a_ok}/3")
print(f"B (candidate shape)   passed {b_ok}/3")
