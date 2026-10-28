//! X / Twitter page rendering through the `twscrape` CLI.
//!
//! X's web app is JavaScript-only for logged-out clients: `get_page` on a
//! profile returns a thin SSR shell, and `/search` returns an empty login
//! wall. With a logged-in `auth_token` + `ct0` in the domain-scoped cookie
//! store, `twscrape` speaks X's GraphQL API directly and returns structured
//! JSON — profile, recent posts, a post with its replies.
//!
//! Design rules:
//! - **Nothing here runs unless the caller has both auth cookies.** Without
//!   them every function returns `None`, and `get_page` falls back to its
//!   normal HTTP extraction, so public pages keep working for guests.
//! - **One subprocess per call, no daemon.** `twscrape` exits after printing
//!   its JSON; nothing stays resident.
//! - **Serialized + jittered.** X rate-limits per endpoint and per account,
//!   so only one `twscrape` process may talk to x.com at a time, and
//!   consecutive calls are spaced by a random delay.
//! - **Bounded.** Every call gets a hard timeout; a locked account (twscrape
//!   waits for the reset window) is reported instead of hanging the tool.

use crate::config::Config;
use crate::fetch::FetchResult;
use serde_json::Value;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::OnceLock;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio::io::AsyncWriteExt;
use tokio::process::Command;
use tokio::sync::{Mutex, MutexGuard};

/// A URL shape this module knows how to render.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    /// `x.com/<handle>` — profile + recent posts.
    Profile(String),
    /// `x.com/<handle>/status/<id>` (or `/i/status/<id>`) — post + replies.
    Status(String),
}

/// Paths that are not handles even though they fit the handle pattern.
const RESERVED: &[&str] = &[
    "home", "explore", "search", "settings", "i", "messages", "notifications",
    "compose", "login", "logout", "signup", "tos", "privacy", "help", "about",
    "jobs", "intent", "share", "hashtag", "lists", "bookmarks", "premium",
];

fn valid_handle(s: &str) -> bool {
    let len = s.len();
    (1..=15).contains(&len)
        && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
        && !RESERVED.contains(&s.to_ascii_lowercase().as_str())
}

fn valid_status_id(s: &str) -> bool {
    !s.is_empty() && s.len() <= 25 && s.chars().all(|c| c.is_ascii_digit())
}

/// Classify an X/Twitter URL. `None` means "not a shape we handle" and the
/// caller should use the normal fetch path.
pub fn classify(raw: &str) -> Option<Target> {
    let url = url::Url::parse(raw.trim()).ok()?;
    let host = url.host_str()?.to_ascii_lowercase();
    let is_x = host == "x.com"
        || host == "twitter.com"
        || host == "mobile.twitter.com"
        || host.ends_with(".x.com")
        || host.ends_with(".twitter.com");
    if !is_x {
        return None;
    }
    let segs: Vec<&str> = url.path_segments()?.filter(|s| !s.is_empty()).collect();
    match segs.as_slice() {
        [handle] if valid_handle(handle) => Some(Target::Profile((*handle).to_string())),
        [handle, "status", id, ..] if valid_handle(handle) && valid_status_id(id) => {
            Some(Target::Status((*id).to_string()))
        }
        ["i", "status", id, ..] if valid_status_id(id) => Some(Target::Status((*id).to_string())),
        ["i", "web", "status", id, ..] if valid_status_id(id) => Some(Target::Status((*id).to_string())),
        _ => None,
    }
}

/// Pull one cookie's value out of a `Cookie:` header string.
pub fn cookie_value(header: &str, name: &str) -> Option<String> {
    header.split(';').find_map(|part| {
        let (k, v) = part.split_once('=')?;
        if k.trim() == name {
            Some(v.trim().to_string())
        } else {
            None
        }
    })
}

pub fn has_login(config: &Config) -> bool {
    let cookies = config.cookies_for("x.com");
    cookie_value(&cookies, "auth_token").is_some() && cookie_value(&cookies, "ct0").is_some()
}

fn db_path() -> PathBuf {
    crate::cookie_cache::cache_path().with_file_name("twscrape-accounts.db")
}

fn hash_path() -> PathBuf {
    crate::cookie_cache::cache_path().with_file_name("twscrape-cookies.sha256")
}

/// One gate for every `twscrape` invocation: mutual exclusion plus a random
/// minimum spacing. X's limits are per account, so two concurrent calls buy
/// nothing and only raise the ban risk.
static CLI_GATE: OnceLock<Mutex<Option<Instant>>> = OnceLock::new();

fn jitter_ms(max: u64) -> u64 {
    if max == 0 {
        return 0;
    }
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.subsec_nanos() as u64 % (max + 1))
        .unwrap_or(0)
}

async fn acquire_gate(config: &Config) -> MutexGuard<'static, Option<Instant>> {
    let mutex = CLI_GATE.get_or_init(|| Mutex::new(None));
    let mut guard = mutex.lock().await;
    if let Some(last) = *guard {
        let need = Duration::from_millis(config.x_min_interval_ms + jitter_ms(3000));
        let elapsed = last.elapsed();
        if elapsed < need {
            tokio::time::sleep(need - elapsed).await;
        }
    }
    *guard = Some(Instant::now());
    guard
}

fn spawn_cmd(config: &Config, args: &[String]) -> Command {
    let mut cmd = Command::new(&config.twscrape_path);
    cmd.arg("--db").arg(db_path());
    cmd.args(args);
    cmd.env("NO_COLOR", "1");
    cmd.stdin(Stdio::piped());
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());
    // twscrape reads TWS_PROXY for both its HTTP client and the transaction-id
    // bootstrap; x.com is not in DIRECT_DOMAINS, so this follows the same
    // proxy rule as every other engine.
    if !config.should_bypass_proxy("x.com") {
        if let Some(proxy) = config.proxy_url.as_deref() {
            cmd.env("TWS_PROXY", proxy);
        }
    }
    cmd
}

/// Run one `twscrape` command. `stdin` is used by `add_cookie`.
async fn run(config: &Config, args: &[String], stdin: Option<String>) -> Result<String, String> {
    let mut cmd = spawn_cmd(config, args);
    let mut child = cmd
        .spawn()
        .map_err(|e| format!("twscrape not runnable ({}): {}", config.twscrape_path, e))?;

    if let Some(data) = stdin {
        if let Some(mut pipe) = child.stdin.take() {
            pipe.write_all(data.as_bytes())
                .await
                .map_err(|e| format!("twscrape stdin: {}", e))?;
        }
    }

    let output = tokio::time::timeout(Duration::from_secs(90), child.wait_with_output())
        .await
        .map_err(|_| {
            "twscrape timed out after 90s (X rate limit lock, slow proxy, or the account is locked)"
                .to_string()
        })?
        .map_err(|e| format!("twscrape wait: {}", e))?;

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    if !output.status.success() {
        let tail: Vec<&str> = stderr.lines().filter(|l| !l.trim().is_empty()).collect();
        let msg = tail
            .last()
            .copied()
            .unwrap_or("twscrape exited with an error")
            .trim()
            .to_string();
        // The CLI prints nothing on stdout for failures; keep a short hint.
        return Err(msg);
    }
    Ok(stdout)
}

/// Keep twscrape's account row in sync with the cookie store. Re-running
/// `add_cookie` is cheap but not free, so it only happens when the stored
/// cookie string actually changed (login refresh, account switch).
async fn sync_account(config: &Config) -> Result<(), String> {
    let cookies = config.cookies_for("x.com");
    if cookie_value(&cookies, "auth_token").is_none() || cookie_value(&cookies, "ct0").is_none() {
        return Err("no x.com login cookies".to_string());
    }
    use sha2::{Digest, Sha256};
    let digest = format!("{:x}", Sha256::digest(cookies.as_bytes()));
    if std::fs::read_to_string(hash_path()).ok().as_deref() == Some(digest.as_str()) {
        return Ok(());
    }
    if let Some(dir) = db_path().parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let args = vec![
        "add_cookie".to_string(),
        "advent".to_string(),
    ];
    run(config, &args, Some(cookies)).await?;
    let _ = std::fs::write(hash_path(), digest);
    Ok(())
}

fn parse_tweet_lines(stdout: &str) -> Vec<Value> {
    stdout
        .lines()
        .map(str::trim)
        .filter(|l| l.starts_with('{'))
        .filter_map(|l| serde_json::from_str::<Value>(l).ok())
        .collect()
}

pub(crate) fn text_of(v: &Value, key: &str) -> String {
    v.get(key)
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_string()
}

pub(crate) fn num_of(v: &Value, key: &str) -> i64 {
    v.get(key).and_then(Value::as_i64).unwrap_or(0)
}

pub(crate) fn handle_of(tweet: &Value) -> String {
    tweet
        .get("user")
        .and_then(|u| u.get("username"))
        .and_then(Value::as_str)
        .unwrap_or("i")
        .to_string()
}

pub(crate) fn date_of(tweet: &Value) -> String {
    text_of(tweet, "date")
        .trim_end_matches("+00:00")
        .trim()
        .to_string()
}

/// Render one tweet as a markdown block (used for timelines, replies and
/// quoted posts).
fn tweet_block(v: &Value, heading: Option<&str>) -> String {
    let mut out = String::new();
    let user = handle_of(v);
    let name = v
        .get("user")
        .and_then(|u| u.get("displayname"))
        .and_then(Value::as_str)
        .unwrap_or("");
    let date = date_of(v);
    let url = text_of(v, "url");
    let body = text_of(v, "rawContent");

    match heading {
        Some(h) => out.push_str(&format!("### {}\n", h)),
        None => out.push_str(&format!("### @{} · {}\n", user, date)),
    }
    if !name.is_empty() {
        out.push_str(&format!("{} (@{})\n", name, user));
    }
    out.push_str(&format!(
        "♥ {} · 🔁 {} · 💬 {} · 👁 {}\n\n",
        num_of(v, "likeCount"),
        num_of(v, "retweetCount"),
        num_of(v, "replyCount"),
        num_of(v, "viewCount")
    ));
    out.push_str(&body);
    out.push('\n');
    if let Some(q) = v.get("quotedTweet").filter(|q| !q.is_null()) {
        out.push_str(&format!(
            "\n> 引用 @{}: {}\n",
            handle_of(q),
            text_of(q, "rawContent").replace('\n', " ")
        ));
    }
    if !url.is_empty() {
        out.push_str(&format!("\n{}\n", url));
    }
    out
}

fn render_profile(user: &Value, tweets: &[Value]) -> (String, String) {
    let username = text_of(user, "username");
    let name = text_of(user, "displayname");
    let title = format!("{} (@{}) / X", if name.is_empty() { &username } else { &name }, username);
    let mut out = format!("# {}\n\n", title);
    let bio = text_of(user, "rawDescription");
    if !bio.is_empty() {
        out.push_str(&bio);
        out.push_str("\n\n");
    }
    out.push_str(&format!(
        "Followers {} · Following {} · Posts {}{}\n",
        num_of(user, "followersCount"),
        num_of(user, "friendsCount"),
        num_of(user, "statusesCount"),
        if user.get("verified").and_then(Value::as_bool).unwrap_or(false) {
            " · ✓ verified"
        } else {
            ""
        }
    ));
    let joined = text_of(user, "created");
    if !joined.is_empty() {
        out.push_str(&format!("Joined {}\n", joined.trim_end_matches("+00:00").trim()));
    }
    out.push_str(&format!("https://x.com/{}\n", username));

    if tweets.is_empty() {
        out.push_str("\n## Recent posts\n\n（没有拿到推文；可能是限流、私密账号，或该账号没有公开推文）\n");
    } else {
        out.push_str(&format!("\n## Recent posts ({})\n\n", tweets.len()));
        for t in tweets {
            out.push_str(&tweet_block(t, None));
        }
    }
    (title, out)
}

fn render_status(tweet: &Value, replies: &[Value]) -> (String, String) {
    let user = handle_of(tweet);
    let date = date_of(tweet);
    let title = format!("@{} on X · {}", user, date);
    let mut out = format!("# @{} · {}\n\n", user, date);
    out.push_str(&text_of(tweet, "rawContent"));
    out.push_str(&format!(
        "\n\n♥ {} · 🔁 {} · 💬 {} · 👁 {}\n{}\n",
        num_of(tweet, "likeCount"),
        num_of(tweet, "retweetCount"),
        num_of(tweet, "replyCount"),
        num_of(tweet, "viewCount"),
        text_of(tweet, "url")
    ));
    if let Some(q) = tweet.get("quotedTweet").filter(|q| !q.is_null()) {
        out.push_str(&format!(
            "\n> 引用 @{}: {}\n",
            handle_of(q),
            text_of(q, "rawContent").replace('\n', " ")
        ));
    }
    if replies.is_empty() {
        out.push_str("\n## Replies\n\n（没有拿到回复；可能是限流或该推文没有公开回复）\n");
    } else {
        out.push_str(&format!("\n## Replies ({})\n\n", replies.len()));
        for r in replies {
            out.push_str(&tweet_block(r, None));
        }
    }
    (title, out)
}

/// Run one keyword search through twscrape. Used by the `x` engine.
/// The caller gets raw tweet objects; mapping to `SearchResult` lives in
/// `src/engines/x.rs`.
pub async fn search_tweets(
    config: &Config,
    query: &str,
    limit: usize,
) -> Result<Vec<Value>, String> {
    let _gate = acquire_gate(config).await;
    sync_account(config).await?;
    let args = vec![
        "search".to_string(),
        query.to_string(),
        "--limit".to_string(),
        limit.to_string(),
    ];
    let out = run(config, &args, None).await?;
    Ok(parse_tweet_lines(&out))
}

/// Fetch an X URL. Returns:
/// - `None` — not an X URL we handle, or the user isn't logged in / twscrape
///   is missing: the caller should use the normal fetch path.
/// - `Some(Ok(..))` — rendered markdown.
/// - `Some(Err(..))` — we tried and failed (logged by the caller, which then
///   falls back to the normal fetch path as well).
pub async fn fetch_page(raw: &str, config: &Config) -> Option<Result<FetchResult, String>> {
    let target = classify(raw)?;
    if !has_login(config) {
        return None;
    }

    let _gate = acquire_gate(config).await;
    if let Err(e) = sync_account(config).await {
        return Some(Err(e));
    }

    match target {
        Target::Profile(handle) => {
            let user_out = match run(config, &["user_by_login".into(), handle.clone()], None).await {
                Ok(s) => s,
                Err(e) => return Some(Err(e)),
            };
            let user: Value = match serde_json::from_str(user_out.trim()) {
                Ok(v) => v,
                Err(e) => return Some(Err(format!("user_by_login: bad JSON: {}", e))),
            };
            let uid = user
                .get("id")
                .and_then(|v| v.as_i64().or_else(|| v.as_str().and_then(|s| s.parse().ok())))
                .unwrap_or(0);
            let mut tweets = Vec::new();
            if uid > 0 {
                let args = vec![
                    "user_tweets".to_string(),
                    uid.to_string(),
                    "--limit".to_string(),
                    "10".to_string(),
                ];
                match run(config, &args, None).await {
                    Ok(s) => tweets = parse_tweet_lines(&s),
                    Err(e) => eprintln!("⚠️ x: user_tweets failed for @{}: {}", handle, e),
                }
            }
            // twscrape returns a whole GraphQL page even when --limit is
            // smaller; cap locally so a profile fetch stays cheap.
            tweets.truncate(10);
            let (title, content) = render_profile(&user, &tweets);
            Some(Ok(FetchResult {
                url: raw.to_string(),
                title,
                content_type: "text/markdown".to_string(),
                site_name: "x.com".to_string(),
                content,
                status_code: 200,
                via: "twscrape".to_string(),
            }))
        }
        Target::Status(id) => {
            let tweet_out = match run(config, &["tweet_details".into(), id.clone()], None).await {
                Ok(s) => s,
                Err(e) => return Some(Err(e)),
            };
            let tweet: Value = match serde_json::from_str(tweet_out.trim()) {
                Ok(v) => v,
                Err(e) => return Some(Err(format!("tweet_details: bad JSON: {}", e))),
            };
            let mut replies = Vec::new();
            let args = vec![
                "tweet_replies".to_string(),
                id.clone(),
                "--limit".to_string(),
                "10".to_string(),
            ];
            match run(config, &args, None).await {
                Ok(s) => replies = parse_tweet_lines(&s),
                Err(e) => eprintln!("⚠️ x: tweet_replies failed for {}: {}", id, e),
            }
            replies.truncate(10);
            let (title, content) = render_status(&tweet, &replies);
            Some(Ok(FetchResult {
                url: raw.to_string(),
                title,
                content_type: "text/markdown".to_string(),
                site_name: "x.com".to_string(),
                content,
                status_code: 200,
                via: "twscrape".to_string(),
            }))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_profiles_and_statuses() {
        assert_eq!(
            classify("https://x.com/OpenAI"),
            Some(Target::Profile("OpenAI".into()))
        );
        assert_eq!(
            classify("https://twitter.com/OpenAI/"),
            Some(Target::Profile("OpenAI".into()))
        );
        assert_eq!(
            classify("https://x.com/jack/status/20"),
            Some(Target::Status("20".into()))
        );
        assert_eq!(
            classify("https://x.com/jack/status/20/photo/1"),
            Some(Target::Status("20".into()))
        );
        assert_eq!(
            classify("https://x.com/i/status/20"),
            Some(Target::Status("20".into()))
        );
        assert_eq!(classify("https://x.com/home"), None);
        assert_eq!(classify("https://x.com/search?q=openai"), None);
        assert_eq!(classify("https://x.com/OpenAI/followers"), None);
        assert_eq!(classify("https://example.com/OpenAI"), None);
    }

    #[test]
    fn reads_cookie_values() {
        let header = "auth_token=abc; ct0=def; twid=u%3D1";
        assert_eq!(cookie_value(header, "auth_token").as_deref(), Some("abc"));
        assert_eq!(cookie_value(header, "ct0").as_deref(), Some("def"));
        assert_eq!(cookie_value(header, "nope"), None);
    }
}
