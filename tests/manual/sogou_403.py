"""Inspect the Sogou 403 body and confirm where each egress path actually lands.

One request per path, kept minimal to avoid burning more quota.
"""
import ssl
import urllib.error
import urllib.request
import warnings

warnings.filterwarnings("ignore")
ctx = ssl._create_unverified_context()
UA = ("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 "
      "(KHTML, like Gecko) Chrome/136.0.0.0 Safari/537.36")
URL = "https://www.sogou.com/web?query=%E6%B7%B1%E5%BA%A6%E5%AD%A6%E4%B9%A0&ie=utf8"
PROXY = {"http": "http://127.0.0.1:7890", "https": "http://127.0.0.1:7890"}

print("=== 各出口的真实 IP（显式请求 ipify）===")
for label, p in (("直连", {}), ("走 127.0.0.1:7890", PROXY)):
    op = urllib.request.build_opener(urllib.request.ProxyHandler(p),
                                     urllib.request.HTTPSHandler(context=ctx))
    for u in ("https://api.ip.sb/ip", "https://myip.ipip.net/"):
        try:
            r = op.open(urllib.request.Request(u, headers={"User-Agent": "Mozilla/5.0"}),
                        timeout=15)
            print(f"  {label:<20} -> {r.read().decode('utf-8', 'ignore').strip()[:60]}")
            break
        except Exception as e:  # noqa: BLE001
            last = e
    else:
        print(f"  {label:<20} -> ERR {type(last).__name__}")

print("\n=== 搜狗页面返回内容 ===")
for label, p in (("直连", {}), ("走代理", PROXY)):
    op = urllib.request.build_opener(urllib.request.ProxyHandler(p),
                                     urllib.request.HTTPSHandler(context=ctx))
    try:
        r = op.open(urllib.request.Request(URL, headers={"User-Agent": UA}),
                    timeout=25)
        print(f"  {label}: HTTP {r.status}")
    except urllib.error.HTTPError as e:
        body = e.read().decode("utf-8", "ignore")
        print(f"  {label}: HTTP {e.code}  len={len(body)}")
        print(f"    Server: {e.headers.get('Server')}")
        print(f"    body: {body[:260]}")
