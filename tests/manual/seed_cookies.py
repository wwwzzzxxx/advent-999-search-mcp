"""Seed the local cookie cache with fresh browser cookies.

Sogou rate-limits by egress IP: once it decides your IP is suspicious it serves
a "请依次点击" click-captcha. A human solves it once, and the resulting SNUID
cookie is replayed by the `sogou` / `weixin` engines.

The cookies live in a small JSON file (outside this repo) rather than an env var,
so refreshing them takes effect on the NEXT SEARCH — no MCP restart needed.

Usage
-----
1. Open a Sogou search page in a browser and solve the captcha:

       https://www.sogou.com/web?query=test

2. In DevTools → Console, run:

       copy(document.cookie)

3. Seed the cache:

       python tests/manual/seed_cookies.py sogou "<paste>"

   Or pipe it in:

       python tests/manual/seed_cookies.py sogou --stdin

   Or read it from a local file (never commit that file!):

       python tests/manual/seed_cookies.py sogou --file my-cookies.txt

Other commands
--------------
    python tests/manual/seed_cookies.py --show      # show cache state
    python tests/manual/seed_cookies.py --path      # print cache file path
    python tests/manual/seed_cookies.py --clear     # delete the cache
"""
import json
import os
import pathlib
import sys
import time

# Cookies the engines actually need (Sogou trust + session identity).
WANTED = ["SNUID", "SUV", "SUID", "ABTEST", "IPLOC", "cuid", "PHPSESSID"]
TTL_SECS = 1200  # ~20 min, matching Sogou's observed SNUID lifetime


def cache_path() -> pathlib.Path:
    if os.name == "nt":
        base = pathlib.Path(os.environ.get("LOCALAPPDATA", "."))
    else:
        base = pathlib.Path(os.environ.get("XDG_CACHE_HOME")
                            or pathlib.Path.home() / ".cache")
    return base / "advent-mcp" / "cookie-cache.json"


def fingerprint(cookies: str) -> str:
    """Show which SNUID is loaded WITHOUT revealing a usable value."""
    for part in cookies.split(";"):
        part = part.strip()
        if part.upper().startswith("SNUID="):
            v = part.split("=", 1)[1]
            if len(v) > 8:
                return f"SNUID({v[:4]}…{v[-4:]}, len {len(v)})"
            return f"SNUID(len {len(v)})"
    return "no SNUID"


def filter_cookies(raw: str) -> str:
    """Keep only the cookies the engines need, preserving order."""
    kept = []
    for part in raw.split(";"):
        part = part.strip()
        if not part or "=" not in part:
            continue
        key = part.split("=", 1)[0].strip()
        if any(key.lower() == w.lower() for w in WANTED):
            kept.append(part)
    return "; ".join(kept)


def show() -> int:
    p = cache_path()
    print(f"path: {p}")
    if not p.exists():
        print("  (no cache file — sogou/weixin will hit the captcha wall)")
        return 1
    try:
        data = json.loads(p.read_text(encoding="utf-8"))
    except Exception as e:  # noqa: BLE001
        print(f"  ERROR: cannot parse: {e}")
        return 1
    if not data:
        print("  (empty)")
        return 1
    for site, entry in data.items():
        cookies = entry.get("cookies", "")
        saved = entry.get("saved_at", 0)
        age = max(0, int(time.time()) - saved) if saved else -1
        state = "FRESH" if 0 <= age <= TTL_SECS else f"STALE ({age // 60} min old)"
        print(f"  {site:<8} {state:<20} {fingerprint(cookies)}")
        print(f"           cookies: {', '.join(c.split('=')[0] for c in cookies.split('; '))}")
    return 0


def main() -> int:
    args = sys.argv[1:]

    if not args or args[0] in ("-h", "--help"):
        print(__doc__)
        return 0
    if args[0] == "--path":
        print(cache_path())
        return 0
    if args[0] == "--show":
        return show()
    if args[0] == "--clear":
        p = cache_path()
        if p.exists():
            p.unlink()
            print(f"deleted {p}")
        else:
            print(f"nothing to delete ({p} does not exist)")
        return 0

    site = args[0]
    rest = args[1:]

    # Cookie value from --stdin / --file / literal argument.
    if not rest:
        print("ERROR: no cookies given. See --help.")
        return 2
    if rest[0] == "--stdin":
        raw = sys.stdin.read()
    elif rest[0] == "--file":
        if len(rest) < 2:
            print("ERROR: --file needs a path")
            return 2
        raw = pathlib.Path(rest[1]).read_text(encoding="utf-8")
    else:
        raw = rest[0]

    raw = raw.strip().strip('"').strip("'")
    if not raw:
        print("ERROR: empty cookie string")
        return 2

    filtered = filter_cookies(raw)
    if "SNUID" not in filtered.upper():
        print("ERROR: no SNUID found in the supplied cookies.")
        print(f"       got: {', '.join(c.split('=')[0] for c in filtered.split('; ') if c)}")
        print("       SNUID is only issued AFTER solving the captcha in a browser.")
        return 3

    p = cache_path()
    p.parent.mkdir(parents=True, exist_ok=True)
    data = {}
    if p.exists():
        try:
            data = json.loads(p.read_text(encoding="utf-8"))
        except Exception:  # noqa: BLE001
            data = {}
    data[site] = {"cookies": filtered, "saved_at": int(time.time())}

    p.write_text(json.dumps(data, indent=2), encoding="utf-8")
    try:
        p.chmod(0o600)  # best-effort; Windows may ignore this
    except Exception:  # noqa: BLE001
        pass

    print(f"seeded '{site}' -> {p}")
    print(f"  fingerprint: {fingerprint(filtered)}")
    print(f"  cookies    : {', '.join(c.split('=')[0] for c in filtered.split('; '))}")
    print("\nTakes effect on the next search — no MCP restart needed.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
