"""Diagnose current network: TUN adapters, egress IPs, and cnki reachability."""
import json
import socket
import ssl
import subprocess
import urllib.request
import warnings

warnings.filterwarnings("ignore")

print("=== 1. 网络适配器 ===")
ps = subprocess.run(
    ["powershell", "-NoProfile", "-Command",
     "Get-NetAdapter | Select-Object Name,InterfaceDescription,Status | ConvertTo-Json -Compress"],
    capture_output=True, text=True)
try:
    for a in json.loads(ps.stdout):
        print(f"  {a['Name']:<35} {a['InterfaceDescription'][:42]:<44} {a['Status']}")
except Exception:
    print(ps.stdout[:400])

print("\n=== 2. 默认路由 ===")
ps = subprocess.run(
    ["powershell", "-NoProfile", "-Command",
     "Get-NetRoute -DestinationPrefix 0.0.0.0/0 | Select-Object ifIndex,NextHop,RouteMetric,InterfaceAlias | ConvertTo-Json -Compress"],
    capture_output=True, text=True)
try:
    r = json.loads(ps.stdout)
    if isinstance(r, dict):
        r = [r]
    for x in r:
        print(f"  ifIndex={x['ifIndex']} nextHop={x['NextHop']} metric={x['RouteMetric']} alias={x['InterfaceAlias']}")
except Exception:
    print(ps.stdout[:400])

print("\n=== 3. 环境变量代理 ===")
for k in ("HTTP_PROXY", "HTTPS_PROXY", "ALL_PROXY", "NO_PROXY"):
    v = __import__("os").environ.get(k)
    if v:
        print(f"  {k} = {v[:120]}")

ctx = ssl._create_unverified_context()


def probe_ip(label, proxies):
    op = urllib.request.build_opener(urllib.request.ProxyHandler(proxies),
                                     urllib.request.HTTPSHandler(context=ctx))
    for url in ("https://api.ip.sb/ip", "https://myip.ipip.net/"):
        try:
            r = op.open(urllib.request.Request(url, headers={"User-Agent": "Mozilla/5.0"}), timeout=15)
            print(f"  {label} [{url.split('/')[2]}] -> {r.read().decode('utf-8', 'ignore').strip()[:70]}")
            return
        except Exception as e:  # noqa: BLE001
            last = e
    print(f"  {label} -> ERR {type(last).__name__} {str(last)[:70]}")


print("\n=== 4. 出口 IP ===")
probe_ip("禁代理(DIRECT) ", {})
probe_ip("显式走代理     ", {"http": "http://127.0.0.1:7890", "https": "http://127.0.0.1:7890"})

print("\n=== 5. scholar.cnki.net 可达性 ===")
HOST = "scholar.cnki.net"
for label, proxies in (("直连", {}), ("代理", {"https": "http://127.0.0.1:7890"})):
    op = urllib.request.build_opener(urllib.request.ProxyHandler(proxies),
                                     urllib.request.HTTPSHandler(context=ctx))
    try:
        r = op.open(urllib.request.Request(f"https://{HOST}/",
                                           headers={"User-Agent": "Mozilla/5.0"}), timeout=20)
        body = r.read().decode("utf-8", "ignore")
        print(f"  {label}: HTTP {r.status} len={len(body)}")
    except urllib.error.HTTPError as e:
        print(f"  {label}: HTTPError {e.code}")
    except Exception as e:  # noqa: BLE001
        print(f"  {label}: ERR {type(e).__name__} {str(e)[:90]}")

print("\n=== 6. 裸 socket 直连 CNKI（DNS→TCP→TLS） ===")
try:
    print(f"  DNS: {sorted({ai[4][0] for ai in socket.getaddrinfo(HOST, 443, socket.AF_INET)})}")
except Exception as e:  # noqa: BLE001
    print(f"  DNS ERR: {e}")
try:
    s = socket.create_connection((HOST, 443), 15)
    print(f"  TCP 连接成功 -> {s.getpeername()}")
    try:
        ss = ctx.wrap_socket(s, server_hostname=HOST)
        print(f"  TLS OK, 对端证书 subject={ss.getpeercert().get('subject')}")
        ss.close()
    except Exception as e:  # noqa: BLE001
        print(f"  TLS ERR {type(e).__name__} {str(e)[:120]}")
except Exception as e:  # noqa: BLE001
    print(f"  TCP ERR {type(e).__name__} {str(e)[:120]}")
