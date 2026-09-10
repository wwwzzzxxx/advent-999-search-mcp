"""Replicate SearXNG's Startpage request shape and compare with our current one.

SearXNG (searx/engines/startpage.py) notes:
  "To avoid CAPTCHAs we need to send a well formed HTTP POST request with a
   cookie."  — the `preferences` cookie + a fresh `sc` code.

This script A/B tests:
  A. our current request   (query/cat/t/sc/abp/abd/abe, no cookie)
  B. SearXNG's request     (adds qsr/qadf/language/lui, drops abp, + preferences cookie)
"""
import gzip
import re
import ssl
import urllib.error
import urllib.parse
import urllib.request
import warnings

warnings.filterwarnings("ignore")
ctx = ssl._create_unverified_context()
UA = ("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 "
      "(KHTML, like Gecko) Chrome/136.0.0.0 Safari/537.36")
PROXY = {"http": "http://127.0.0.1:7890", "https": "http://127.0.0.1:7890"}
BASE = "https://www.startpage.com"

opener = urllib.request.build_opener(
    urllib.request.ProxyHandler(PROXY),
    urllib.request.HTTPSHandler(context=ctx),
)


def fetch(url, data=None, headers=None):
    h = {"User-Agent": UA, "Accept-Language": "en-US,en;q=0.9"}
    if headers:
        h.update(headers)
    if data is not None:
        data = urllib.parse.urlencode(data).encode()
    r = opener.open(urllib.request.Request(url, data=data, headers=h), timeout=30)
    raw = r.read()
    if raw[:2] == b"\x1f\x8b":
        raw = gzip.decompress(raw)
    return r, raw.decode("utf-8", "ignore")


def get_sc():
    _, html = fetch(BASE + "/")
    m = re.search(r'name="sc"\s+value="([^"]+)"', html) or \
        re.search(r'value="([^"]+)"\s+name="sc"', html)
    return m.group(1) if m else None


def verdict(r, html):
    low = html.lower()
    if "anubis_challenge" in low or "anubis_version" in low:
        d = re.search(r'"difficulty":\s*(\d+)', html)
        return f"ANUBIS(difficulty={d.group(1) if d else '?'})"
    if "captcha" in r.url.lower():
        return "CAPTCHA-REDIRECT"
    if "w-gl__result" in html or "result-title" in html:
        n = len(re.findall(r"w-gl__result", html))
        return f"RESULTS({n})"
    return f"OTHER(len={len(html)})"


# --- A: our current shape -------------------------------------------------
print("=== A. 当前 MCP 的请求形态 ===")
sc = get_sc()
print(f"    sc={sc[:24] if sc else None}...")
try:
    r, html = fetch(BASE + "/sp/search", {
        "query": "python asyncio", "cat": "web", "t": "device", "sc": sc,
        "abp": "1", "abd": "1", "abe": "1",
    }, {"Content-Type": "application/x-www-form-urlencoded",
        "Origin": BASE, "Referer": BASE + "/"})
    print(f"    -> HTTP {r.status}  {verdict(r, html)}")
except urllib.error.HTTPError as e:
    print(f"    -> HTTP {e.code}")

# --- B: SearXNG's shape ---------------------------------------------------
print("\n=== B. SearXNG 的请求形态（含 preferences cookie）===")
sc = get_sc()
print(f"    sc={sc[:24] if sc else None}...")

prefs = [
    ("date_time", "world"),
    ("disable_family_filter", "none"),
    ("disable_open_in_new_window", "0"),
    ("enable_post_method", "1"),
    ("enable_proxy_safety_suggest", "1"),
    ("enable_stay_control", "1"),
    ("instant_answers", "1"),
    ("lang_homepage", "s/device/en/"),
    ("num_of_results", "10"),
    ("suggestions", "1"),
    ("wt_unit", "celsius"),
    ("language", "en"),
    ("language_ui", "en"),
    ("search_results_region", "en-US"),
]
pref_cookie = "N1N".join(f"{k}EEE{v}" for k, v in prefs)
print(f"    preferences cookie: {len(pref_cookie)} bytes")

try:
    r, html = fetch(BASE + "/sp/search", {
        "query": "python asyncio", "cat": "web", "t": "device", "sc": sc,
        "with_date": "", "abd": "1", "abe": "1", "qsr": "all", "qadf": "none",
        "language": "en", "lui": "en",
    }, {"Content-Type": "application/x-www-form-urlencoded",
        "Origin": BASE, "Referer": BASE + "/",
        "Cookie": "preferences=" + pref_cookie})
    print(f"    -> HTTP {r.status}  {verdict(r, html)}")
    if "RESULTS" not in verdict(r, html):
        open(r"C:\Users\wzx\AppData\Local\Temp\sp_b.html", "w", encoding="utf-8").write(html)
        print("       (saved to %TEMP%\\sp_b.html)")
except urllib.error.HTTPError as e:
    print(f"    -> HTTP {e.code}")
