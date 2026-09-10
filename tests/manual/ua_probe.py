"""Test whether Anubis (dblp / startpage) lets any User-Agent through."""
import gzip
import ssl
import urllib.error
import urllib.request
import warnings

warnings.filterwarnings("ignore")
ctx = ssl._create_unverified_context()
PROXY = {"http": "http://127.0.0.1:7890", "https": "http://127.0.0.1:7890"}

UAS = {
    "chrome-win": ("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 "
                   "(KHTML, like Gecko) Chrome/136.0.0.0 Safari/537.36"),
    "firefox": "Mozilla/5.0 (X11; Linux x86_64; rv:128.0) Gecko/20100101 Firefox/128.0",
    "googlebot": "Mozilla/5.0 (compatible; Googlebot/2.1; +http://www.google.com/bot.html)",
    "bingbot": "Mozilla/5.0 (compatible; bingbot/2.0; +http://www.bing.com/bingbot.htm)",
    "curl": "curl/8.18.0",
    "wget": "Wget/1.21.4",
    "python-requests": "python-requests/2.32.3",
    "feedreader": "Feedparser/6.0 +https://github.com/kurtmckee/feedparser/",
}

TARGETS = [
    ("dblp.org", "https://dblp.org/search/publ/api?q=transformer&format=json&h=2&f=0"),
    ("startpage", "https://www.startpage.com/"),
]


def fetch(url, ua, proxies):
    op = urllib.request.build_opener(urllib.request.ProxyHandler(proxies),
                                     urllib.request.HTTPSHandler(context=ctx))
    r = op.open(urllib.request.Request(url, headers={
        "User-Agent": ua, "Accept": "application/json",
        "Accept-Encoding": "gzip"}), timeout=25)
    raw = r.read()
    if raw[:2] == b"\x1f\x8b":
        raw = gzip.decompress(raw)
    body = raw.decode("utf-8", "ignore")
    low = body.lower()
    if "anubis" in low or "not a bot" in low:
        return "ANUBIS"
    if "too many requests" in low or "429" in low:
        return "RATE-LIMIT"
    return "PASS" if body.lstrip().startswith(("{", "<!DOCTYPE", "<html")) else "??"


for name, url in TARGETS:
    print(f"=== {name} ===")
    for label, ua in UAS.items():
        for plabel, proxies in (("直连", {}), ("代理", PROXY)):
            try:
                print(f"  {label:<16} {plabel}  -> {fetch(url, ua, proxies)}")
            except urllib.error.HTTPError as e:
                print(f"  {label:<16} {plabel}  -> HTTP {e.code}")
            except Exception as e:  # noqa: BLE001
                print(f"  {label:<16} {plabel}  -> ERR {type(e).__name__}")
    print()
