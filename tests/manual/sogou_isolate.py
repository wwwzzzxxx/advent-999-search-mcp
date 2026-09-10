"""Find the minimal request shape Sogou accepts (via proxy), and confirm stability."""
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
BASE_URL = "https://www.sogou.com/web?query=" + Q + "&ie=utf8"

SIMPLE = {"User-Agent": UA, "Accept-Language": "zh-CN,zh;q=0.9"}


def with_extra(extra):
    h = dict(SIMPLE)
    h.update(extra)
    return h


CH = {'sec-ch-ua': '"Chromium";v="136", "Google Chrome";v="136", "Not?A_Brand";v="99"',
      "sec-ch-ua-mobile": "?0", "sec-ch-ua-platform": '"Windows"'}
SF = {"sec-fetch-site": "same-origin", "sec-fetch-mode": "navigate",
      "sec-fetch-user": "?1", "sec-fetch-dest": "document"}

CASES = [
    ("简头 baseline", BASE_URL, SIMPLE),
    ("+Referer", BASE_URL, with_extra({"Referer": "https://www.sogou.com/"})),
    ("+sec-ch-ua", BASE_URL, with_extra(CH)),
    ("+sec-fetch", BASE_URL, with_extra(SF)),
    ("+Accept(全)", BASE_URL, with_extra({"Accept": "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8"})),
    ("page=2", BASE_URL + "&page=2", SIMPLE),
]

for name, url, hdrs in CASES:
    results = []
    for _ in range(2):
        op = urllib.request.build_opener(urllib.request.ProxyHandler(PROXY),
                                         urllib.request.HTTPSHandler(context=ctx))
        try:
            r = op.open(urllib.request.Request(url, headers=hdrs), timeout=25)
            h = r.read().decode("utf-8", "ignore")
            blocked = "antispider" in r.url.lower() or "请依次点击" in h
            results.append(f"len={len(h)}{'(BLOCKED)' if blocked else '(OK)'}")
        except Exception as e:  # noqa: BLE001
            results.append(f"ERR {type(e).__name__}")
        time.sleep(4)
    print(f"  {name:<16} {'  |  '.join(results)}")
