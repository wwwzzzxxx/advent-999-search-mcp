use async_trait::async_trait;
use aes::Aes256;
use cipher::{BlockEncrypt, KeyInit, generic_array::GenericArray};
use crate::config::Config;
use crate::models::{SearchError, SearchResult};
use super::SearchEngine;

/// CNKI (知网) — Chinese academic papers, mainly useful for citation counts.
///
/// Uses the scholar.cnki.net REST API which requires NO cookie / login —
/// the `vv` token is an AES-256-ECB encryption of a timestamp with a
/// hardcoded (public) secret, verified only for form-validity.
///
/// Endpoint: `POST https://scholar.cnki.net/restapi/kns8s-api/v2/criteria/query`
/// (reference: olo-dot-io/Uni-CLI src/adapters/cnki/search.ts, 2026-06)
pub struct CnkiEngine;

const CNKI_QUERY_API: &str = "https://scholar.cnki.net/restapi/kns8s-api/v2/criteria/query";
const CNKI_TOKEN_SECRET: &str = "cf4e8f25360248f89248af06a55d21ea";
const CNKI_CLIENT_ID: &str = "c5fd4ef0-d314-4888-b0a7-f6190eaefaf0";
const CNKI_ALL_DATABASE_CLASS_ID: &str = "WD0FTY92";

#[async_trait]
impl SearchEngine for CnkiEngine {
    fn name(&self) -> &'static str { "cnki" }

    async fn search(&self, query: &str, limit: usize, config: &Config) -> Result<Vec<SearchResult>, SearchError> {
        let client = config.build_reqwest_client_for("cnki.net")
            .map_err(|e| SearchError::Http(e.to_string()))?;

        let limit = std::cmp::min(limit, 50).max(1);

        let vv = cnki_vv_token_hex();
        let url = format!("{}?vv={}&clientId={}", CNKI_QUERY_API, vv, CNKI_CLIENT_ID);

        let payload = serde_json::json!({
            "Resource": "",
            "Classid": CNKI_ALL_DATABASE_CLASS_ID,
            "Products": "",
            "KuaKuCode": "",
            "QNode": {
                "QGroup": [{
                    "Key": "",
                    "Title": "",
                    "Logic": 0,
                    "Items": [],
                    "ChildItems": [{
                        "Key": "subject",
                        "Title": "",
                        "Logic": 0,
                        "Items": [{
                            "Key": "",
                            "Title": "题名",
                            "Logic": 0,
                            "Field": "TI",
                            "Operator": "FUZZY",
                            "Value": query,
                        }],
                        "ChildItems": [],
                    }],
                }],
            },
            "ExScope": "1",
            "SearchType": 2,
            "SearchFrom": 1,
            "Rlang": "",
            "sort": "PT",
            "sortType": "DESC",
            "pageNum": 1,
            "pageSize": limit,
        });

        let resp = client.post(&url)
            .json(&payload)
            .header("Accept", "application/json, text/plain, */*")
            .header("Origin", "https://scholar.cnki.net")
            .header("Referer", "https://scholar.cnki.net/")
            .header("Version", "")
            .send()
            .await
            .map_err(|e| SearchError::Http(format!("CNKI request failed: {}", e)))?;

        let status = resp.status();
        if status == reqwest::StatusCode::UNAUTHORIZED {
            return Err(SearchError::Blocked(
                "CNKI rejected the vv token (401); the API may have changed".to_string()
            ));
        }
        if !status.is_success() {
            return Err(SearchError::Http(format!("CNKI returned status {}", status)));
        }

        let data: serde_json::Value = resp.json().await
            .map_err(|e| SearchError::Parse(format!("CNKI JSON parse failed: {}", e)))?;

        let code = data["code"].as_i64().unwrap_or(-1);
        if code != 0 {
            let msg = data["message"].as_str().unwrap_or("unknown");
            return Err(SearchError::Engine(format!("CNKI API error {}: {}", code, msg)));
        }

        let rows = match data["data"]["data"].as_array() {
            Some(arr) => arr,
            None => return Ok(Vec::new()),
        };

        let mut results = Vec::new();
        for row in rows {
            let meta = row["metadata"].as_array().map(|arr| {
                let mut m = std::collections::HashMap::new();
                for e in arr {
                    if let (Some(name), Some(value)) = (e["name"].as_str(), e["value"].as_str()) {
                        m.insert(name.to_string(), value.to_string());
                    }
                }
                m
            }).unwrap_or_default();

            let title = meta.get("TI").or_else(|| meta.get("ENTI"))
                .map(|s| strip_html(s))
                .unwrap_or_default();
            if title.is_empty() { continue; }

            let doi = meta.get("DOI").map(|s| s.trim_start_matches("doi:").to_string()).unwrap_or_default();
            let date = meta.get("PT").cloned().unwrap_or_default();
            let source = meta.get("LY").cloned().unwrap_or_else(|| {
                row["source"]["title"].as_str().map(|s| strip_html(s)).unwrap_or_default()
            });

            let authors: Vec<String> = row["authors"].as_array()
                .map(|arr| arr.iter()
                    .filter_map(|a| a["title"].as_str().map(strip_html))
                    .filter(|s| !s.is_empty())
                    .collect())
                .unwrap_or_default();

            // URL: prefer the ABSTRACT/PUBLICATION relation
            let url = row["relations"].as_array()
                .and_then(|rels| rels.iter().find(|r| {
                    r["scope"].as_str().map(|s| s.contains("ABSTRACT") || s.contains("PUBLICATION")).unwrap_or(false)
                }))
                .and_then(|r| r["url"].as_str())
                .map(|s| s.to_string())
                .unwrap_or_default();

            let mut description = String::new();
            if !authors.is_empty() {
                description.push_str(&format!("{} | ", authors.join(", ")));
            }
            if !source.is_empty() {
                description.push_str(&format!("{} | ", source));
            }
            if !date.is_empty() {
                description.push_str(&format!("{} | ", date));
            }
            if !doi.is_empty() {
                description.push_str(&format!("DOI: {}", doi));
            }

            results.push(SearchResult {
                title: title.chars().take(200).collect(),
                url: url.chars().take(500).collect(),
                description: description.chars().take(400).collect(),
                source: source.chars().take(200).collect(),
                engine: "cnki".to_string(),
                summary: None,
            });

            if results.len() >= limit { break; }
        }

        Ok(results)
    }
}

/// Build the CNKI vv token: AES-256-ECB(timestamp) as hex.
///
/// Matches the reference implementation:
/// `createCipheriv("aes-256-ecb", secret, null)` over `JSON.stringify({ timestamp })`
/// with PKCS7 padding, output hex.
fn cnki_vv_token_hex() -> String {
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);

    let plain = format!("{{\"timestamp\":{}}}", ts).into_bytes();

    // PKCS7 padding to block size 16
    let pad_len = 16 - (plain.len() % 16);
    let mut data = plain;
    data.extend(std::iter::repeat(pad_len as u8).take(pad_len));

    let cipher = Aes256::new_from_slice(CNKI_TOKEN_SECRET.as_bytes())
        .expect("valid 32-byte key");
    let mut out = Vec::with_capacity(data.len());
    for chunk in data.chunks(16) {
        let mut block = GenericArray::clone_from_slice(chunk);
        cipher.encrypt_block(&mut block);
        out.extend_from_slice(&block);
    }
    out.iter().map(|b| format!("{:02x}", b)).collect()
}

/// Strip HTML tags and decode common entities (CNKI highlights match terms
/// with `<font color='red'>...</font>`).
fn strip_html(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut in_tag = false;
    for c in s.chars() {
        match c {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => out.push(c),
            _ => {}
        }
    }
    out.replace("&nbsp;", " ")
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .trim()
        .to_string()
}
