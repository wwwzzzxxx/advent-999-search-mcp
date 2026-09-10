"""Inspect the Sogou anti-bot page: which IP does it blame, and is it IP-level?"""
import re
import ssl
import urllib.request
import warnings

warnings.filterwarnings("ignore")
ctx = ssl._create_unverified_context()
UA = ("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 "
      "(KHTML, like Gecko) Chrome/136.0.0.0 Safari/537.36")
URL = "https://www.sogou.com/web?query=%E6%B7%B1%E5%BA%A6%E5%AD%A6%E4%B9%A0&ie=utf8"


def probe(proxies, label):
    op = urllib.request.build_opener(urllib.request.ProxyHandler(proxies),
                                     urllib.request.HTTPSHandler(context=ctx))
    try:
        r = op.open(urllib.request.Request(URL, headers={
            "User-Agent": UA, "Accept-Language": "zh-CN,zh;q=0.9"}), timeout=25)
        html = r.read().decode("utf-8", "ignore")
        blocked = "antispider" in r.url.lower() or "请依次点击" in html
        ip = re.search(r"IP[：:]\s*([\d.]+)", html)
        t = re.search(r"<title>(.*?)</title>", html, re.S)
        print(f"  {label:<24} HTTP {r.status} len={len(html):<7} blocked={blocked}")
        print(f"       title = {t.group(1).strip()[:50] if t else '?'}")
        if ip:
            print(f"       页面回显 IP = {ip.group(1)}")
    except Exception as e:  # noqa: BLE001
        print(f"  {label:<24} ERR {type(e).__name__}: {str(e)[:60]}")


print("=== 搜狗搜索页 ===")
probe({}, "直连(禁代理)")
probe({"http": "http://127.0.0.1:7890", "https": "http://127.0.0.1:7890"}, "走代理")

print("\n=== 搜狗首页（对照组）===")
op = urllib.request.build_opener(urllib.request.ProxyHandler({}),
                                 urllib.request.HTTPSHandler(context=ctx))
r = op.open(urllib.request.Request("https://www.sogou.com/",
                                   headers={"User-Agent": UA}), timeout=25)
h = r.read().decode("utf-8", "ignore")
print(f"  直连首页 HTTP {r.status} len={len(h)} antispider={'antispider' in r.url.lower()}")
