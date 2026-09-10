"""Solve the Anubis proof-of-work challenge and fetch the protected resource.

Anubis (https://anubis.techaro.lol) gates traffic behind a SHA-256 PoW:
  find nonce such that sha256hex(randomData + nonce) starts with
  `difficulty` zero hex digits, then GET the pass-challenge endpoint which
  sets the `within.website-x-cmd-anubis-auth` JWT cookie.

Usage: python tests/manual/anubis_solve.py [url]
"""
import gzip
import hashlib
import http.cookiejar
import json
import re
import ssl
import sys
import time
import urllib.error
import urllib.parse
import urllib.request
import warnings

warnings.filterwarnings("ignore")

TARGET = sys.argv[1] if len(sys.argv) > 1 else \
    "https://dblp.org/search/publ/api?q=transformer&format=json&h=2&f=0"
USE_PROXY = "--noproxy" not in sys.argv
PROXY = ({"http": "http://127.0.0.1:7890", "https": "http://127.0.0.1:7890"}
         if USE_PROXY else {})
UA = ("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 "
      "(KHTML, like Gecko) Chrome/136.0.0.0 Safari/537.36")
ORIGIN = re.match(r"https?://[^/]+", TARGET).group(0)

ctx = ssl._create_unverified_context()
jar = http.cookiejar.CookieJar()
opener = urllib.request.build_opener(
    urllib.request.ProxyHandler(PROXY),
    urllib.request.HTTPSHandler(context=ctx),
    urllib.request.HTTPCookieProcessor(jar),
)


def fetch(url, accept="*/*"):
    r = opener.open(urllib.request.Request(url, headers={
        "User-Agent": UA, "Accept": accept,
        "Accept-Language": "en-US,en;q=0.9"}), timeout=30)
    raw = r.read()
    enc = (r.headers.get("Content-Encoding") or "").lower()
    if raw[:2] == b"\x1f\x8b" or "gzip" in enc:
        raw = gzip.decompress(raw)
    return r, raw.decode("utf-8", "ignore")


print(f"target : {TARGET}")
print(f"proxy  : {USE_PROXY}\n")

# --- Step 1: fetch the challenge -------------------------------------------
r, html = fetch(TARGET, "text/html,application/xhtml+xml,*/*;q=0.8")
print(f"[1] GET target      -> HTTP {r.status}, {len(html)} bytes")
if "anubis_challenge" not in html:
    print("    No challenge — already allowed!")
    print(f"    body: {html[:160]}")
    sys.exit(0)

m = re.search(r'<script id="anubis_challenge"[^>]*>(.*?)</script>', html, re.S)
chal = json.loads(m.group(1))
rules = chal["rules"]
c = chal["challenge"]
difficulty = rules.get("difficulty", c.get("difficulty", 4))
print(f"[2] challenge       -> id={c['id']} method={rules.get('algorithm')} "
      f"difficulty={difficulty} (hex digits)")

# --- Step 2: brute-force the nonce -----------------------------------------
random_data = c["randomData"]
prefix = "0" * difficulty
t0 = time.time()
nonce = 0
digest = ""
while True:
    digest = hashlib.sha256(f"{random_data}{nonce}".encode()).hexdigest()
    if digest.startswith(prefix):
        break
    nonce += 1
elapsed_ms = int((time.time() - t0) * 1000)
print(f"[3] solved          -> nonce={nonce} hash={digest[:24]}... "
      f"({nonce + 1} tries, {elapsed_ms} ms, {digest.startswith(prefix)=})")

# --- Step 3: pass the challenge --------------------------------------------
pass_url = ORIGIN + "/.within.website/x/cmd/anubis/api/pass-challenge?" + \
    urllib.parse.urlencode({
        "id": c["id"], "response": digest, "nonce": str(nonce),
        "redir": TARGET, "elapsedTime": str(elapsed_ms),
    })
print(f"[4] GET pass-challenge ...")
try:
    r2, _ = fetch(pass_url)
    print(f"    -> HTTP {r2.status}  final_url={r2.url[:80]}")
except urllib.error.HTTPError as e:
    print(f"    -> HTTPError {e.code}  {e.headers.get('Set-Cookie', '')[:100]}")

cookies = {ck.name: ck.value for ck in jar}
print(f"    cookies: {[f'{k} ({len(v)}B)' for k, v in cookies.items()]}")

# Anubis names the JWT cookie from the site's configured prefix, e.g.
# `within.website-x-cmd-anubis-auth` by default, but dblp uses
# `dblp_org-auth-<hash>`. Match on "auth" rather than a fixed name.
auth = [k for k in cookies if "auth" in k.lower()]
if not auth:
    print("\n*** No auth cookie — challenge not accepted ***")
    sys.exit(1)
print(f"\n[5] got auth cookie: {auth[0]} ({len(cookies[auth[0]])} bytes)")

# --- Step 4: retry the protected resource ----------------------------------
r3, body = fetch(TARGET, "application/json")
print(f"[6] GET target again -> HTTP {r3.status}, {len(body)} bytes")
print(f"    body: {body[:200]}")
ok = body.lstrip().startswith("{")
print("\n*** SUCCESS ***" if ok else "\n*** still blocked ***")
sys.exit(0 if ok else 1)
