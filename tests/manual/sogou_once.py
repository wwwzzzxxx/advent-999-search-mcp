"""Single-shot Sogou check: is the block IP-level, and whose IP is blamed?

Kept to ONE request per egress path so it does not burn more quota.
"""
import re
import ssl
import urllib.request
import warnings

warnings.filterwarnings("ignore")
ctx = ssl._create_unverified_context()
UA = ("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 "
      "(KHTML, like Gecko) Chrome/136.0.0.0 Safari/537.36")
URL = "https://www.sogou.com/web?query=%E6%B7%B1%E5%BA%A6%E5%AD%A6%E4%B9%A0&ie=utf8"


def check(proxies, label):
    op = urllib.request.build_opener(urllib.request.ProxyHandler(proxies),
                                     urllib.request.HTTPSHandler(context=ctx))
    try:
        r = op.open(urllib.request.Request(URL, headers={
            "User-Agent": UA, "Accept-Language": "zh-CN,zh;q=0.9"}), timeout=25)
        html = r.read().decode("utf-8", "ignore")
        blocked = "antispider" in r.url.lower() or "请依次点击" in html
        m = re.search(r"IP[：:]\s*([\d.]+)", html)
        print(f"  {label:<22} HTTP {r.status} len={len(html):<8} "
              f"{'BLOCKED' if blocked else 'OK'}"
              + (f"  (blames IP {m.group(1)})" if m else ""))
        return not blocked
    except Exception as e:  # noqa: BLE001
        print(f"  {label:<22} ERR {type(e).__name__}: {str(e)[:50]}")
        return False


print("=== 搜狗搜索页（每路仅 1 次请求）===")
check({}, "直连(你的真实 IP)")
check({"http": "http://127.0.0.1:7890", "https": "http://127.0.0.1:7890"}, "走代理")
