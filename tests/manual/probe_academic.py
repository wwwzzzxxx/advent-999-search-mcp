"""Probe dblp / startpage over direct vs proxy, showing the real block reason."""
import gzip
import ssl
import urllib.error
import urllib.request
import warnings
import zlib

warnings.filterwarnings("ignore")
ctx = ssl._create_unverified_context()
UA = ("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 "
      "(KHTML, like Gecko) Chrome/136.0.0.0 Safari/537.36")
PROXY = {"http": "http://127.0.0.1:7890", "https": "http://127.0.0.1:7890"}


def decode(raw, encoding):
    """reqwest auto-decompresses; mimic that so we see the real body."""
    if raw[:2] == b"\x1f\x8b" or "gzip" in (encoding or ""):
        try:
            raw = gzip.decompress(raw)
        except Exception:  # noqa: BLE001
            pass
    elif "deflate" in (encoding or ""):
        try:
            raw = zlib.decompress(raw, -zlib.MAX_WBITS)
        except Exception:  # noqa: BLE001
            pass
    return raw.decode("utf-8", "ignore")


def probe(url, proxies, label):
    op = urllib.request.build_opener(urllib.request.ProxyHandler(proxies),
                                     urllib.request.HTTPSHandler(context=ctx))
    try:
        r = op.open(urllib.request.Request(url, headers={
            "User-Agent": UA, "Accept": "application/json",
            "Accept-Encoding": "gzip"}), timeout=25)
        body = decode(r.read(), r.headers.get("Content-Encoding"))
        low = body.lower()
        marker = ("anubis" if "anubis" in low else
                  "429" if "too many requests" in low else
                  "json" if body.lstrip().startswith("{") else "html")
        print(f"  {label:<22} HTTP {r.status} len={len(body):<7} -> {marker}")
        if marker == "html":
            print(f"       {body[:90]!r}")
    except urllib.error.HTTPError as e:
        print(f"  {label:<22} HTTP {e.code} {e.reason}")
    except Exception as e:  # noqa: BLE001
        print(f"  {label:<22} ERR {type(e).__name__}: {str(e)[:60]}")


print("=== DBLP API ===")
for host in ("dblp.org", "dblp.uni-trier.de"):
    print(f"-- {host} --")
    probe(f"https://{host}/search/publ/api?q=transformer&format=json&h=2&f=0", {}, "直连")
    probe(f"https://{host}/search/publ/api?q=transformer&format=json&h=2&f=0", PROXY, "代理")

print("\n=== Startpage ===")
probe("https://www.startpage.com/", {}, "直连")
probe("https://www.startpage.com/", PROXY, "代理")
