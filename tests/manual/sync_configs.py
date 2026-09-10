"""Align the advent MCP settings across all local agent configs.

Updates DIRECT_DOMAINS / ALLOWED_SEARCH_ENGINES and trims FETCH_COOKIES down to
the cookies the code actually reads (dead anti-bot cookies now live in the
cookie cache instead of env vars).

Run with --dry-run first to preview.

Usage: python tests/manual/sync_configs.py [--dry-run]
"""
import json
import pathlib
import re
import sys

DRY = "--dry-run" in sys.argv

# Canonical values ------------------------------------------------------------
# cnki/dblp must bypass the proxy (EdgeOne 418 / Anubis), hdslb is Bilibili's
# subtitle CDN, and youtube must NOT be here (it needs egress abroad).
DIRECT_DOMAINS = ("sogou,weixin,baidu,bilibili,hdslb,zhihu,csdn,juejin,"
                  "xiaohongshu,cnki,dblp")

# Only `d_c0` (Zhihu) and `SESSDATA`/`buvid3`/`buvid4` (Bilibili) are read from
# FETCH_COOKIES — see fetch.rs `extract_cookie_value`. Everything else was
# accumulated from browser dumps and is now dead weight (and a larger surface
# for accidental leaks).
KEEP_COOKIES = ["d_c0", "SESSDATA", "buvid3", "buvid4"]

HOME = pathlib.Path.home()
FILES = [
    (HOME / "AppData/Roaming/Code/User/mcp.json", "vscode"),
    (HOME / ".config/opencode/opencode.json", "opencode"),
    (HOME / "AppData/Roaming/reasonix/config.toml", "reasonix"),
]


def trim_cookies(raw: str, pool: dict[str, str]) -> str:
    """Rebuild FETCH_COOKIES from the shared pool, keeping only KEEP_COOKIES.

    Values come from `pool`, which is the union of every config's cookies, so a
    config missing (say) the Bilibili cookies gets them too — every tool then
    has the same capabilities.
    """
    kept = [f"{name}={pool[name]}" for name in KEEP_COOKIES if name in pool]
    return "; ".join(kept)


def collect_pool(text: str, pool: dict[str, str]) -> None:
    """Merge this file's FETCH_COOKIES into the shared pool."""
    m = re.search(r'(FETCH_COOKIES"?\s*[=:]\s*")([^"]*)(")', text)
    if not m:
        return
    for part in m.group(2).split(";"):
        part = part.strip()
        if "=" in part:
            k, v = part.split("=", 1)
            pool.setdefault(k.strip(), v.strip())


def patch(text: str, label: str, pool: dict[str, str]) -> tuple[str, list[str]]:
    changes = []

    # --- DIRECT_DOMAINS ---
    # Keys may be quoted (JSON: "DIRECT_DOMAINS":) or bare (TOML: DIRECT_DOMAINS =).
    m = re.search(r'(DIRECT_DOMAINS"?\s*[=:]\s*")([^"]*)(")', text)
    if m:
        if m.group(2) != DIRECT_DOMAINS:
            text = text[:m.start(2)] + DIRECT_DOMAINS + text[m.end(2):]
            changes.append(f"DIRECT_DOMAINS\n      - {m.group(2)}\n      + {DIRECT_DOMAINS}")
    else:
        changes.append("DIRECT_DOMAINS  NOT FOUND")

    # --- ALLOWED_SEARCH_ENGINES: add ieee if missing, keep everything else ---
    m = re.search(r'(ALLOWED_SEARCH_ENGINES"?\s*[=:]\s*")([^"]*)(")', text)
    if m:
        engines = [e.strip() for e in m.group(2).split(",") if e.strip()]
        if "ieee" not in engines:
            engines.append("ieee")
            newval = ",".join(engines)
            text = text[:m.start(2)] + newval + text[m.end(2):]
            changes.append(f"ALLOWED_SEARCH_ENGINES\n      - {m.group(2)}\n      + {newval}")
        else:
            changes.append(f"ALLOWED_SEARCH_ENGINES  ok ({len(engines)} engines, ieee present)")
    else:
        changes.append("ALLOWED_SEARCH_ENGINES  NOT FOUND")

    # --- FETCH_COOKIES: rebuild from the shared pool, dropping dead cookies ---
    m = re.search(r'(FETCH_COOKIES"?\s*[=:]\s*")([^"]*)(")', text)
    if m:
        old = m.group(2)
        new = trim_cookies(old, pool)
        old_names = [p.split("=")[0].strip() for p in old.split(";") if "=" in p]
        new_names = [p.split("=")[0].strip() for p in new.split(";") if "=" in p]
        added = [n for n in new_names if n not in old_names]
        removed = [n for n in old_names if n not in new_names]
        if new != old:
            text = text[:m.start(2)] + new + text[m.end(2):]
            detail = [f"FETCH_COOKIES  {len(old)} -> {len(new)} bytes",
                      f"      keep    : {', '.join(new_names) or '(none)'}"]
            if added:
                detail.append(f"      added   : {', '.join(added)}")
            if removed:
                detail.append(f"      removed : {len(removed)} unused "
                              f"({', '.join(removed[:6])}" + ("…" if len(removed) > 6 else "") + ")")
            changes.append("\n".join(detail))
        else:
            changes.append(f"FETCH_COOKIES  already minimal ({len(new_names)} cookies)")
    else:
        changes.append("FETCH_COOKIES  not present (fine — engines fall back to the cache)")

    return text, changes


# Pass 1: merge every config's cookies into one pool, so each config can be
# filled in with anything it is missing (idempotent).
POOL: dict[str, str] = {}
for path, _ in FILES:
    if path.exists():
        collect_pool(path.read_text(encoding="utf-8"), POOL)

print(f"cookie pool: {', '.join(k for k in KEEP_COOKIES if k in POOL) or '(empty)'}")
missing = [k for k in KEEP_COOKIES if k not in POOL]
if missing:
    print(f"  (not found anywhere, cannot sync: {', '.join(missing)})")
print(f"{'DRY RUN — no files written' if DRY else 'APPLYING CHANGES'}\n")

for path, label in FILES:
    print(f"=== {label}: {path} ===")
    if not path.exists():
        print("    FILE NOT FOUND — skipped\n")
        continue

    original = path.read_text(encoding="utf-8")
    try:
        updated, changes = patch(original, label, POOL)
    except Exception as e:  # noqa: BLE001
        print(f"    ERROR: {e}\n")
        continue

    for c in changes:
        print(f"    {c}")

    if updated == original:
        print("    (no change needed)\n")
        continue

    # Validate JSON files before writing, so a bad edit can never be saved.
    # opencode.json is JSONC (it contains `//` comments), so fall back to a
    # comment-stripped parse when the strict one fails.
    if path.suffix == ".json":
        try:
            json.loads(updated)
        except Exception:
            try:
                stripped = re.sub(r"^\s*//.*$", "", updated, flags=re.M)
                json.loads(stripped)
            except Exception as e:  # noqa: BLE001
                print(f"    ❌ ABORT — result is not valid JSON: {e}\n")
                continue

    if DRY:
        print("    (dry run — not written)\n")
        continue

    backup = path.with_suffix(path.suffix + ".bak")
    backup.write_text(original, encoding="utf-8")
    path.write_text(updated, encoding="utf-8")
    print(f"    ✅ written (backup: {backup.name})\n")
