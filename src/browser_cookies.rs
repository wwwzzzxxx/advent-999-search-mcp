//! Opt-in cookie import from a **dedicated** Edge profile, over the DevTools
//! protocol.
//!
//! Why not read the browser's cookie database directly? On current Windows
//! builds it does not work:
//!
//! - Edge encrypts cookies with App-Bound Encryption (v20). The key in
//!   `Local State` (`app_bound_encrypted_key`) can only be unwrapped by Edge's
//!   own elevation service, which validates the calling executable. Doing that
//!   from a third-party process is the infostealer technique.
//! - Even the live SQLite file is refused to normal processes (`EACCES`), and
//!   Chromium keeps it open while running.
//! - Chromium 136+ ignores `--remote-debugging-port` on the *default* profile,
//!   so the user's everyday Edge cannot be driven either.
//!
//! So: launch `msedge.exe` with our own `--user-data-dir` + debugging port and
//! let the browser decrypt and hand over the cookies for the domains the
//! caller asked for. The user logs in once in the window we open; the profile
//! (and therefore the session) persists for next time.
//!
//! Nothing here runs unless `ADVENT_ALLOW_COOKIE_IMPORT=1`.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// One import at a time: they share a profile and would close each other's
/// browser.
static IMPORT_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

pub struct ImportRequest {
    /// Domains the caller wants, e.g. `["bilibili.com"]`. Never a wildcard.
    pub domains: Vec<String>,
    /// Page to open for the login (defaults to `https://<domain>/` each).
    pub login_url: Option<String>,
    /// How long to wait for the user to log in, in seconds.
    pub wait_secs: u64,
    /// Leave the browser open afterwards (default: close it).
    pub keep_open: bool,
}

#[derive(Debug, Clone)]
pub struct BrowserCookie {
    pub name: String,
    pub value: String,
    pub domain: String,
    pub expires: f64,
}

pub struct ImportOutcome {
    pub cookies: Vec<BrowserCookie>,
    pub profile: PathBuf,
}

/// Run the whole import on a blocking thread (launch → poll → collect → stop).
pub fn import(req: &ImportRequest) -> Result<ImportOutcome, String> {
    let profile = crate::cookie_cache::browser_profile_dir();
    std::fs::create_dir_all(&profile)
        .map_err(|e| format!("cannot create profile dir {}: {}", profile.display(), e))?;

    // Serialize imports: two concurrent runs would fight over one profile (and
    // one of them would close the other's login window).
    let _guard = IMPORT_LOCK.lock().unwrap_or_else(|p| p.into_inner());

    // Phase 1 — invisible probe. A window must never appear just to read
    // cookies that are already there.
    eprintln!("🔓 import_cookies: probing the profile headlessly (no window)");
    let existing = match probe_headless(&profile) {
        Ok(cookies) => cookies,
        Err(e) => {
            eprintln!("⚠️ import_cookies: headless probe failed: {}", e);
            Vec::new()
        }
    };
    let matched = filter_cookies(existing, &req.domains);
    let covered = covered_domains(&matched, &req.domains);
    if covered == req.domains.len() {
        return Ok(ImportOutcome {
            cookies: matched,
            profile,
        });
    }

    // With no waiting time there is nothing a visible window could achieve.
    if req.wait_secs == 0 {
        return Ok(ImportOutcome {
            cookies: matched,
            profile,
        });
    }

    // Phase 2 — the user has to log in, so show the window. Make sure no
    // headless/leftover browser still owns the profile: reusing it would leave
    // the user staring at nothing for the whole timeout.
    eprintln!(
        "🔓 import_cookies: {}/{} domain(s) still need a login — opening Edge",
        covered,
        req.domains.len()
    );
    close_running(&profile);
    let (port, mut child) = ensure_browser(&profile, req, false, false)?;
    let ws_url = browser_ws_url(port)?;
    let ws_path = ws_url
        .split("://")
        .nth(1)
        .and_then(|rest| rest.find('/').map(|i| &rest[i..]))
        .ok_or_else(|| format!("unexpected DevTools endpoint: {}", ws_url))?
        .to_string();

    let result = collect(port, &ws_path, req);
    if !req.keep_open {
        close_browser(port, &ws_path, child.take());
    }
    result
}

/// Run the browser headless, read its cookies and close it: no window at all.
fn probe_headless(profile: &std::path::Path) -> Result<Vec<BrowserCookie>, String> {
    let req = ImportRequest {
        domains: Vec::new(),
        login_url: None,
        wait_secs: 0,
        keep_open: false,
    };
    let (port, child) = ensure_browser(profile, &req, true, true)?;
    let ws_url = browser_ws_url(port)?;
    let ws_path = ws_url
        .split("://")
        .nth(1)
        .and_then(|rest| rest.find('/').map(|i| &rest[i..]))
        .ok_or_else(|| format!("unexpected DevTools endpoint: {}", ws_url))?
        .to_string();
    let cookies = all_cookies(port, &ws_path);
    // If we attached to a window that was already open (keepOpen), leave it
    // alone — the probe must never close something the user is looking at.
    if child.is_some() {
        close_browser(port, &ws_path, child);
    }
    cookies
}

/// The cookie that makes a session actually usable, where we know one.
///
/// "Any cookie for the domain" is not enough: zhihu keeps d_c0 on every visit
/// but its API now answers `need_login` without z_c0, and bilibili serves no
/// subtitle list without SESSDATA.
pub fn key_cookie(domain: &str) -> Option<&'static str> {
    if crate::cookie_cache::domain_matches(domain, "zhihu.com") {
        Some("z_c0")
    } else if crate::cookie_cache::domain_matches(domain, "bilibili.com") {
        Some("SESSDATA")
    } else if crate::cookie_cache::domain_matches(domain, "sogou.com") {
        Some("SNUID")
    } else {
        None
    }
}

fn domain_usable(cookies: &[BrowserCookie], domain: &str) -> bool {
    match key_cookie(domain) {
        Some(key) => cookies.iter().any(|c| {
            crate::cookie_cache::domain_matches(&c.domain, domain)
                && c.name.eq_ignore_ascii_case(key)
        }),
        None => cookies
            .iter()
            .any(|c| crate::cookie_cache::domain_matches(&c.domain, domain)),
    }
}

fn covered_domains(cookies: &[BrowserCookie], domains: &[String]) -> usize {
    domains
        .iter()
        .filter(|d| domain_usable(cookies, d))
        .count()
}

/// One round of collection plus the login wait loop.
fn collect(port: u16, ws_path: &str, req: &ImportRequest) -> Result<ImportOutcome, String> {
    let deadline = Instant::now() + Duration::from_secs(req.wait_secs);

    loop {
        let all = all_cookies(port, ws_path)?;
        let matched = filter_cookies(all, &req.domains);
        let covered = covered_domains(&matched, &req.domains);
        if covered == req.domains.len() || Instant::now() >= deadline {
            return Ok(ImportOutcome {
                cookies: matched,
                profile: crate::cookie_cache::browser_profile_dir(),
            });
        }
        let left = deadline.saturating_duration_since(Instant::now()).as_secs();
        eprintln!(
            "🔓 import_cookies: {}/{} domain(s) have cookies, {}s left — log in in the Edge window",
            covered,
            req.domains.len(),
            left
        );
        std::thread::sleep(Duration::from_secs(2));
    }
}

fn filter_cookies(cookies: Vec<BrowserCookie>, domains: &[String]) -> Vec<BrowserCookie> {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0);
    cookies
        .into_iter()
        .filter(|c| !c.name.is_empty() && !c.value.is_empty())
        .filter(|c| c.expires <= 0.0 || c.expires > now)
        .filter(|c| {
            let host = c.domain.trim_start_matches('.');
            domains
                .iter()
                .any(|d| crate::cookie_cache::domain_matches(host, d))
        })
        .collect()
}

// ─────────────────────────────────────────────────────────────────────────────
// Browser lifecycle
// ─────────────────────────────────────────────────────────────────────────────

fn edge_exe() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("ADVENT_EDGE_EXE") {
        let p = PathBuf::from(p);
        if p.is_file() {
            return Some(p);
        }
    }
    for candidate in [
        r"C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe",
        r"C:\Program Files\Microsoft\Edge\Application\msedge.exe",
    ] {
        let p = PathBuf::from(candidate);
        if p.is_file() {
            return Some(p);
        }
    }
    None
}

fn devtools_file(profile: &std::path::Path) -> PathBuf {
    profile.join("DevToolsActivePort")
}

/// Return a live debugging port, launching Edge if none is running.
///
/// `Child` is `Some` only when we started the browser ourselves.
fn ensure_browser(
    profile: &std::path::Path,
    req: &ImportRequest,
    headless: bool,
    allow_reuse: bool,
) -> Result<(u16, Option<Child>), String> {
    // An earlier import may still have the browser open (keep_open, or a
    // concurrent call). Reuse it when the recorded port answers.
    if let Some((port, _)) = read_devtools_file(profile) {
        if allow_reuse && http_get(port, "/json/version").is_ok() {
            if !headless {
                open_login_tabs(port, req);
            }
            return Ok((port, None));
        }
        let _ = std::fs::remove_file(devtools_file(profile));
    }

    let exe = edge_exe().ok_or_else(|| {
        "Microsoft Edge not found. Set ADVENT_EDGE_EXE to msedge.exe if it is installed elsewhere."
            .to_string()
    })?;

    let mut cmd = Command::new(&exe);
    cmd.arg(format!("--user-data-dir={}", profile.display()))
        .arg("--remote-debugging-port=0")
        .arg("--no-first-run")
        .arg("--no-default-browser-check")
        .arg("--disable-sync")
        .arg("--disable-background-networking")
        .arg("--disable-component-update")
        .arg("--restore-last-session=false")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());

    if headless {
        cmd.arg("--headless=new");
        cmd.arg("about:blank");
    } else {
        let urls: Vec<String> = match &req.login_url {
            Some(u) => vec![u.clone()],
            None => req
                .domains
                .iter()
                .map(|d| format!("https://{}/", d.trim_start_matches('.')))
                .collect(),
        };
        for u in &urls {
            cmd.arg(u);
        }
    }

    let child = cmd
        .spawn()
        .map_err(|e| format!("cannot start Edge ({}): {}", exe.display(), e))?;

    // Wait for the browser to publish its port (cold start: a couple of
    // seconds; a first-ever profile can take longer).
    let deadline = Instant::now() + Duration::from_secs(25);
    while Instant::now() < deadline {
        if let Some((port, _)) = read_devtools_file(profile) {
            if http_get(port, "/json/version").is_ok() {
                return Ok((port, Some(child)));
            }
        }
        std::thread::sleep(Duration::from_millis(300));
    }
    Err(
        "Edge started but never exposed a DevTools port. Close any Edge window using the \
         advent profile and try again."
            .to_string(),
    )
}

/// Read `<profile>/DevToolsActivePort` → (port, browser ws path).
fn read_devtools_file(profile: &std::path::Path) -> Option<(u16, String)> {
    let text = std::fs::read_to_string(devtools_file(profile)).ok()?;
    let mut lines = text.lines();
    let port: u16 = lines.next()?.trim().parse().ok()?;
    let path = lines.next().unwrap_or("").trim().to_string();
    Some((port, path))
}

/// If the browser was already running, make sure a login tab exists.
fn open_login_tabs(port: u16, req: &ImportRequest) {
    let urls: Vec<String> = match &req.login_url {
        Some(u) => vec![u.clone()],
        None => req
            .domains
            .iter()
            .map(|d| format!("https://{}/", d.trim_start_matches('.')))
            .collect(),
    };
    for u in urls {
        // Best effort: PUT /json/new?<url> opens a tab.
        let path = format!("/json/new?{}", url_encode(&u));
        let _ = http_put(port, &path);
    }
}

/// Close the browser we opened: ask it over CDP first (clean, reaps its own
/// children), then fall back to killing every process that carries our
/// profile path.
///
/// A plain `child.kill()` is not enough: Edge's launcher hands off to the real
/// browser process and exits, leaving the tree alive.
fn close_browser(port: u16, ws_path: &str, child: Option<Child>) {
    if let Ok(mut ws) = Ws::connect(port, ws_path) {
        eprintln!("🔒 import_cookies: asking the browser to close");
        let _ = ws.call("Browser.close", serde_json::json!({}), 4242);
    }

    let deadline = Instant::now() + Duration::from_secs(3);
    let mut gone = false;
    while Instant::now() < deadline {
        if http_get(port, "/json/version").is_err() {
            gone = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(200));
    }

    if let Some(mut child) = child {
        let _ = child.kill();
        let _ = child.wait();
    }
    if !gone {
        eprintln!("🔒 import_cookies: port still open, killing the import profile's processes");
        kill_profile_processes();
    }
}

/// Close any browser that still owns the import profile (e.g. a previous
/// `keepOpen` run), so a fresh headed window can start.
fn close_running(profile: &std::path::Path) {
    if let Some((port, ws_path)) = read_devtools_file(profile) {
        if http_get(port, "/json/version").is_ok() {
            if let Ok(mut ws) = Ws::connect(port, &ws_path) {
                let _ = ws.call("Browser.close", serde_json::json!({}), 7777);
            }
            for _ in 0..30 {
                if http_get(port, "/json/version").is_err() {
                    break;
                }
                std::thread::sleep(Duration::from_millis(200));
            }
            kill_profile_processes();
        }
    }
    let _ = std::fs::remove_file(devtools_file(profile));
}

/// Last-resort cleanup: kill `msedge.exe` processes started with our profile.
#[cfg(target_os = "windows")]
fn kill_profile_processes() {
    let profile = crate::cookie_cache::browser_profile_dir();
    // The pattern is a PowerShell wildcard; quoting it keeps spaces intact.
    let script = format!(
        "Get-CimInstance Win32_Process | Where-Object {{ $_.Name -eq \"msedge.exe\" -and $_.CommandLine -like \"*{}*\" }} | \
         ForEach-Object {{ Stop-Process -Id $_.ProcessId -Force -ErrorAction SilentlyContinue }}",
        profile.display()
    );
    let _ = Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", &script])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

#[cfg(not(target_os = "windows"))]
fn kill_profile_processes() {}

// ─────────────────────────────────────────────────────────────────────────────
// CDP over a hand-rolled WebSocket (no extra dependencies)
// ─────────────────────────────────────────────────────────────────────────────

fn browser_ws_url(port: u16) -> Result<String, String> {
    let body = http_get(port, "/json/version")?;
    let v: serde_json::Value =
        serde_json::from_str(&body).map_err(|e| format!("bad /json/version: {}", e))?;
    v.get("webSocketDebuggerUrl")
        .and_then(|u| u.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| "DevTools did not report a webSocketDebuggerUrl".to_string())
}

fn all_cookies(port: u16, ws_path: &str) -> Result<Vec<BrowserCookie>, String> {
    let mut ws = Ws::connect(port, ws_path)?;
    // Storage.getCookies is the modern method (browser target). Fall back to
    // Network.getAllCookies on older builds.
    let resp = match ws.call("Storage.getCookies", serde_json::json!({}), 1) {
        Ok(v) => v,
        Err(_) => ws.call("Network.getAllCookies", serde_json::json!({}), 2)?,
    };
    let list = resp
        .get("cookies")
        .and_then(|c| c.as_array())
        .ok_or_else(|| "CDP response carried no cookie list".to_string())?;

    let mut out = Vec::with_capacity(list.len());
    for c in list {
        let name = c.get("name").and_then(|v| v.as_str()).unwrap_or("").to_string();
        let value = c.get("value").and_then(|v| v.as_str()).unwrap_or("").to_string();
        let domain = c.get("domain").and_then(|v| v.as_str()).unwrap_or("").to_string();
        let expires = c.get("expires").and_then(|v| v.as_f64()).unwrap_or(0.0);
        // Control characters would make an invalid HTTP header later.
        if name.chars().any(|ch| ch.is_control()) || value.chars().any(|ch| ch.is_control()) {
            continue;
        }
        out.push(BrowserCookie { name, value, domain, expires });
    }
    Ok(out)
}

struct Ws {
    stream: TcpStream,
}

impl Ws {
    fn connect(port: u16, path: &str) -> Result<Self, String> {
        let addr = format!("127.0.0.1:{}", port);
        let mut stream = TcpStream::connect(&addr)
            .map_err(|e| format!("cannot connect to DevTools at {}: {}", addr, e))?;
        stream
            .set_read_timeout(Some(Duration::from_secs(10)))
            .and_then(|_| stream.set_write_timeout(Some(Duration::from_secs(10))))
            .map_err(|e| e.to_string())?;

        let key = base64(&nonce16());
        let request = format!(
            "GET {} HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\n\
             Sec-WebSocket-Key: {}\r\nSec-WebSocket-Version: 13\r\n\r\n",
            path, port, key
        );
        stream
            .write_all(request.as_bytes())
            .map_err(|e| format!("websocket handshake write failed: {}", e))?;

        // Read the handshake response header block.
        let mut buf = Vec::new();
        let mut byte = [0u8; 1];
        while !buf.ends_with(b"\r\n\r\n") {
            stream
                .read_exact(&mut byte)
                .map_err(|e| format!("websocket handshake read failed: {}", e))?;
            buf.push(byte[0]);
            if buf.len() > 16 * 1024 {
                return Err("websocket handshake response too large".to_string());
            }
        }
        let head = String::from_utf8_lossy(&buf).to_lowercase();
        if !head.starts_with("http/1.1 101") {
            return Err(format!(
                "DevTools refused the websocket upgrade: {}",
                head.lines().next().unwrap_or("")
            ));
        }
        Ok(Self { stream })
    }

    fn call(
        &mut self,
        method: &str,
        params: serde_json::Value,
        id: u64,
    ) -> Result<serde_json::Value, String> {
        let msg = serde_json::json!({ "id": id, "method": method, "params": params });
        self.send_text(&msg.to_string())?;

        // Events may arrive before the response; skip until our id.
        for _ in 0..64 {
            let text = self.recv_text()?;
            let v: serde_json::Value = match serde_json::from_str(&text) {
                Ok(v) => v,
                Err(_) => continue,
            };
            if v.get("id").and_then(|i| i.as_u64()) != Some(id) {
                continue;
            }
            if let Some(err) = v.get("error") {
                return Err(format!("CDP {} failed: {}", method, err));
            }
            return Ok(v.get("result").cloned().unwrap_or(serde_json::Value::Null));
        }
        Err(format!("CDP {} returned no response", method))
    }

    fn send_text(&mut self, text: &str) -> Result<(), String> {
        let payload = text.as_bytes();
        let mut frame = Vec::with_capacity(payload.len() + 14);
        frame.push(0x81); // FIN + text
        let len = payload.len();
        if len < 126 {
            frame.push(0x80 | len as u8);
        } else if len <= u16::MAX as usize {
            frame.push(0x80 | 126);
            frame.extend_from_slice(&(len as u16).to_be_bytes());
        } else {
            frame.push(0x80 | 127);
            frame.extend_from_slice(&(len as u64).to_be_bytes());
        }
        let mask = nonce4();
        frame.extend_from_slice(&mask);
        frame.extend(payload.iter().enumerate().map(|(i, b)| b ^ mask[i % 4]));
        self.stream
            .write_all(&frame)
            .map_err(|e| format!("websocket write failed: {}", e))
    }

    fn recv_text(&mut self) -> Result<String, String> {
        let mut assembled: Vec<u8> = Vec::new();
        loop {
            let mut h = [0u8; 2];
            self.stream
                .read_exact(&mut h)
                .map_err(|e| format!("websocket read failed: {}", e))?;
            let fin = h[0] & 0x80 != 0;
            let opcode = h[0] & 0x0f;
            let masked = h[1] & 0x80 != 0;
            let mut len = (h[1] & 0x7f) as u64;
            if len == 126 {
                let mut b = [0u8; 2];
                self.stream.read_exact(&mut b).map_err(|e| e.to_string())?;
                len = u16::from_be_bytes(b) as u64;
            } else if len == 127 {
                let mut b = [0u8; 8];
                self.stream.read_exact(&mut b).map_err(|e| e.to_string())?;
                len = u64::from_be_bytes(b);
            }
            if len > 64 * 1024 * 1024 {
                return Err("websocket frame too large".to_string());
            }
            let mask = if masked {
                let mut m = [0u8; 4];
                self.stream.read_exact(&mut m).map_err(|e| e.to_string())?;
                Some(m)
            } else {
                None
            };
            let mut payload = vec![0u8; len as usize];
            self.stream
                .read_exact(&mut payload)
                .map_err(|e| e.to_string())?;
            if let Some(m) = mask {
                for (i, b) in payload.iter_mut().enumerate() {
                    *b ^= m[i % 4];
                }
            }

            match opcode {
                0x9 => {
                    // ping → masked empty pong
                    let _ = self.stream.write_all(&[0x8a, 0x80, 0, 0, 0, 0]);
                    continue;
                }
                0x8 => return Err("DevTools closed the websocket".to_string()),
                0x1 | 0x0 => {
                    assembled.extend_from_slice(&payload);
                    if fin {
                        return String::from_utf8(assembled)
                            .map_err(|e| format!("DevTools sent invalid UTF-8: {}", e));
                    }
                }
                _ => continue, // ignore binary / pong
            }
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Tiny HTTP client for the DevTools HTTP endpoints
// ─────────────────────────────────────────────────────────────────────────────

fn http_get(port: u16, path: &str) -> Result<String, String> {
    http_request(port, "GET", path)
}

fn http_put(port: u16, path: &str) -> Result<String, String> {
    http_request(port, "PUT", path)
}

fn http_request(port: u16, method: &str, path: &str) -> Result<String, String> {
    let mut stream = TcpStream::connect(("127.0.0.1", port))
        .map_err(|e| format!("cannot reach DevTools: {}", e))?;
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .map_err(|e| e.to_string())?;
    let req = format!(
        "{} {} HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nConnection: close\r\n\r\n",
        method, path, port
    );
    stream.write_all(req.as_bytes()).map_err(|e| e.to_string())?;

    // Read the header block.
    let mut buf = Vec::new();
    let mut byte = [0u8; 1];
    loop {
        stream.read_exact(&mut byte).map_err(|e| e.to_string())?;
        buf.push(byte[0]);
        if buf.len() > 64 * 1024 {
            return Err("DevTools HTTP response header too large".to_string());
        }
        if buf.ends_with(b"\r\n\r\n") {
            break;
        }
    }
    let split = buf.len() - 4;
    let head = String::from_utf8_lossy(&buf[..split]).to_lowercase();
    if !head.starts_with("http/1.1 200") && !head.starts_with("http/1.0 200") {
        return Err(format!(
            "DevTools HTTP {} {} → {}",
            method,
            path,
            head.lines().next().unwrap_or("")
        ));
    }

    let content_length = head
        .lines()
        .find_map(|l| l.strip_prefix("content-length:"))
        .and_then(|v| v.trim().parse::<usize>().ok())
        .unwrap_or(0);
    let mut body = Vec::with_capacity(content_length);
    if content_length > 0 {
        body.resize(content_length, 0);
        stream.read_exact(&mut body).map_err(|e| e.to_string())?;
    } else {
        // No length (shouldn't happen for these endpoints): read to EOF.
        let _ = stream.read_to_end(&mut body);
    }
    Ok(String::from_utf8_lossy(&body).to_string())
}

// ─────────────────────────────────────────────────────────────────────────────
// Small helpers
// ─────────────────────────────────────────────────────────────────────────────

fn nonce16() -> [u8; 16] {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
    let pid = std::process::id() as u64;
    let stack = &nanos as *const u64 as u64;
    let mut x = nanos ^ (pid << 32) ^ stack.rotate_left(17);
    let mut out = [0u8; 16];
    for chunk in out.chunks_mut(8) {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        chunk.copy_from_slice(&x.to_le_bytes()[..chunk.len()]);
    }
    out
}

fn nonce4() -> [u8; 4] {
    let n = nonce16();
    [n[0], n[1], n[2], n[3]]
}

/// Minimal standard base64 encoder (no padding-free variants needed).
fn base64(bytes: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity((bytes.len() + 2) / 3 * 4);
    for chunk in bytes.chunks(3) {
        let b = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        out.push(T[(b[0] >> 2) as usize] as char);
        out.push(T[(((b[0] & 0x03) << 4) | (b[1] >> 4)) as usize] as char);
        out.push(if chunk.len() > 1 {
            T[(((b[1] & 0x0f) << 2) | (b[2] >> 6)) as usize] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            T[(b[2] & 0x3f) as usize] as char
        } else {
            '='
        });
    }
    out
}

fn url_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 3);
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{:02X}", b)),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_matches_known_vectors() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(
            base64(b"The quick brown fox jumps over the lazy dog"),
            "VGhlIHF1aWNrIGJyb3duIGZveCBqdW1wcyBvdmVyIHRoZSBsYXp5IGRvZw=="
        );
    }

    #[test]
    fn expired_and_foreign_cookies_are_dropped() {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs_f64();
        let mk = |name: &str, domain: &str, expires: f64| BrowserCookie {
            name: name.into(),
            value: "v".into(),
            domain: domain.into(),
            expires,
        };
        let got = filter_cookies(
            vec![
                mk("SESSDATA", ".bilibili.com", now + 3600.0),
                mk("old", ".bilibili.com", now - 10.0),
                mk("evil", ".example.com", now + 3600.0),
                mk("session", ".bilibili.com", -1.0),
            ],
            &["bilibili.com".to_string()],
        );
        let names: Vec<&str> = got.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, vec!["SESSDATA", "session"]);
    }
}
