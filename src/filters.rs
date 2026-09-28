//! Per-request search filters: freshness (time range), topic, and domain
//! include/exclude lists.
//!
//! Engines vary wildly in what they support natively, and this module is the
//! single place that knows how to express a filter in each engine's own
//! vocabulary. The response reports back which engines actually applied a
//! filter, so callers are never misled into thinking an unsupported engine was
//! filtered (see `FreshnessTier`).

/// Requested freshness window.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Freshness {
    Day,
    Week,
    Month,
    Year,
    /// Explicit inclusive date range, ISO `YYYY-MM-DD`.
    Range { start: String, end: String },
}

/// How well an engine can honour a freshness filter. Reported per engine in the
/// search response so callers can tell "filtered" from "best effort" from
/// "not supported at all".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FreshnessTier {
    /// Engine has a real server-side time filter and it was applied.
    Applied,
    /// A parameter was sent or a newest-first sort was used, but the engine was
    /// not verified to actually restrict results (or the market/egress ignores
    /// it). Treat as a hint, not a guarantee.
    BestEffort,
    /// Engine has no time filter at all; results are unfiltered.
    Unsupported,
}

impl FreshnessTier {
    pub fn as_str(self) -> &'static str {
        match self {
            FreshnessTier::Applied => "applied",
            FreshnessTier::BestEffort => "best_effort",
            FreshnessTier::Unsupported => "unsupported",
        }
    }
}

impl Freshness {
    /// Parse `day` / `week` / `month` / `year` or `YYYY-MM-DD..YYYY-MM-DD`.
    pub fn parse(raw: &str) -> Option<Freshness> {
        let s = raw.trim();
        if s.is_empty() {
            return None;
        }
        match s.to_ascii_lowercase().as_str() {
            "day" | "d" | "24h" => return Some(Freshness::Day),
            "week" | "w" | "7d" => return Some(Freshness::Week),
            "month" | "m" | "30d" => return Some(Freshness::Month),
            "year" | "y" | "365d" => return Some(Freshness::Year),
            _ => {}
        }
        // Explicit range: YYYY-MM-DD..YYYY-MM-DD
        let (a, b) = s.split_once("..")?;
        let start = normalize_date(a.trim())?;
        let end = normalize_date(b.trim())?;
        Some(Freshness::Range { start, end })
    }

    /// Window length in days, for the named presets.
    pub fn days(&self) -> Option<i64> {
        match self {
            Freshness::Day => Some(1),
            Freshness::Week => Some(7),
            Freshness::Month => Some(30),
            Freshness::Year => Some(365),
            Freshness::Range { .. } => None,
        }
    }

    /// Inclusive `(start, end)` as ISO `YYYY-MM-DD`, resolved against `today`.
    pub fn iso_range(&self, today: i64) -> (String, String) {
        match self {
            Freshness::Range { start, end } => (start.clone(), end.clone()),
            other => {
                let d = other.days().unwrap_or(0);
                let (sy, sm, sd) = civil_from_days(today - d);
                let (ey, em, ed) = civil_from_days(today);
                (iso_date(sy, sm, sd), iso_date(ey, em, ed))
            }
        }
    }

    /// Same window as compact `YYYYMMDD` values (IEEE's format).
    pub fn compact_range(&self, today: i64) -> (String, String) {
        let (s, e) = self.iso_range(today);
        (s.replace('-', ""), e.replace('-', ""))
    }

    /// A short human-readable label for the response metadata.
    pub fn label(&self) -> String {
        match self {
            Freshness::Day => "day".to_string(),
            Freshness::Week => "week".to_string(),
            Freshness::Month => "month".to_string(),
            Freshness::Year => "year".to_string(),
            Freshness::Range { start, end } => format!("{}..{}", start, end),
        }
    }

    /// Start year for engines that only filter by publication year (dblp).
    pub fn start_year(&self, today: i64) -> Option<i32> {
        match self {
            Freshness::Range { start, .. } => start
                .get(0..4)
                .and_then(|y| y.parse::<i32>().ok()),
            _ => {
                let d = self.days().unwrap_or(0);
                let (y, _, _) = civil_from_days(today - d);
                Some(y)
            }
        }
    }
}

/// Content category hint (Exa's `category`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Topic {
    News,
    General,
}

impl Topic {
    pub fn parse(raw: &str) -> Option<Topic> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "news" => Some(Topic::News),
            "general" | "" => Some(Topic::General),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Topic::News => "news",
            Topic::General => "general",
        }
    }
}

/// Everything a single search request can ask for, beyond query + limit.
#[derive(Debug, Clone, Default)]
pub struct SearchOptions {
    pub freshness: Option<Freshness>,
    pub topic: Option<Topic>,
    pub include_domains: Vec<String>,
    pub exclude_domains: Vec<String>,
}

/// Days since 1970-01-01 for "now" (UTC).
pub fn today_days() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| (d.as_secs() / 86_400) as i64)
        .unwrap_or(0)
}

fn iso_date(y: i32, m: u32, d: u32) -> String {
    format!("{:04}-{:02}-{:02}", y, m, d)
}

/// Validate and normalise a date string to `YYYY-MM-DD`.
fn normalize_date(s: &str) -> Option<String> {
    let s = s.trim();
    let compact = s.replace('/', "-");
    let parts: Vec<&str> = compact.split('-').collect();
    if parts.len() != 3 {
        return None;
    }
    let y: i32 = parts[0].parse().ok()?;
    let m: u32 = parts[1].parse().ok()?;
    let d: u32 = parts[2].parse().ok()?;
    if !(1..=12).contains(&m) || !(1..=31).contains(&d) || !(1900..=2999).contains(&y) {
        return None;
    }
    Some(iso_date(y, m, d))
}

/// Days since 1970-01-01 (Howard Hinnant's `days_from_civil`).
#[allow(dead_code)]
pub fn days_from_civil(y: i32, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y } as i64;
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = ((m as i64) + 9) % 12;
    let doy = (153 * mp + 2) / 5 + (d as i64) - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// Civil date from days since 1970-01-01 (`civil_from_days`).
pub fn civil_from_days(z: i64) -> (i32, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    ((if m <= 2 { y + 1 } else { y }) as i32, m, d)
}

/// Host of a URL, lowercased, `None` when unparseable.
pub fn host_of(url: &str) -> Option<String> {
    url::Url::parse(url.trim())
        .ok()
        .and_then(|u| u.host_str().map(|h| h.to_ascii_lowercase()))
}

/// True when `host` is `domain` or a subdomain of it.
pub fn host_matches(host: &str, domain: &str) -> bool {
    let d = domain
        .trim()
        .trim_start_matches('.')
        .trim_start_matches("www.")
        .to_ascii_lowercase();
    if d.is_empty() {
        return false;
    }
    let h = host.trim_start_matches("www.").to_ascii_lowercase();
    h == d || h.ends_with(&format!(".{}", d))
}

/// Apply include/exclude domain lists to a URL. Empty include list = no
/// restriction.
pub fn domain_allowed(url: &str, include: &[String], exclude: &[String]) -> bool {
    let host = match host_of(url) {
        Some(h) => h,
        // Unparseable URLs are kept unless an include list would have to match.
        None => return include.is_empty(),
    };
    if include.iter().any(|d| host_matches(&host, d)) {
        return true;
    }
    if !include.is_empty() {
        return false;
    }
    !exclude.iter().any(|d| host_matches(&host, d))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_presets_and_ranges() {
        assert_eq!(Freshness::parse("day"), Some(Freshness::Day));
        assert_eq!(Freshness::parse("WEEK"), Some(Freshness::Week));
        assert_eq!(Freshness::parse("month"), Some(Freshness::Month));
        assert_eq!(Freshness::parse("year"), Some(Freshness::Year));
        assert_eq!(
            Freshness::parse("2026-01-02..2026-02-03"),
            Some(Freshness::Range {
                start: "2026-01-02".into(),
                end: "2026-02-03".into()
            })
        );
        assert_eq!(Freshness::parse("nonsense"), None);
        assert_eq!(Freshness::parse(""), None);
    }

    #[test]
    fn civil_roundtrip() {
        for z in [-100_000_i64, -1, 0, 1, 719_468, 20_000, 100_000] {
            let (y, m, d) = civil_from_days(z);
            assert_eq!(days_from_civil(y, m, d), z);
        }
        // 1970-01-01 is day 0; 2024-01-01 is day 19723 (Bing's epoch).
        assert_eq!(days_from_civil(1970, 1, 1), 0);
        assert_eq!(days_from_civil(2024, 1, 1), 19723);
    }

    #[test]
    fn iso_range_for_week() {
        let today = days_from_civil(2026, 9, 29);
        let (s, e) = Freshness::Week.iso_range(today);
        assert_eq!(e, "2026-09-29");
        assert_eq!(s, "2026-09-22");
    }

    #[test]
    fn domain_matching() {
        assert!(host_matches("docs.python.org", "python.org"));
        assert!(host_matches("python.org", "python.org"));
        assert!(host_matches("www.python.org", "python.org"));
        assert!(!host_matches("notpython.org", "python.org"));
        assert!(domain_allowed(
            "https://docs.python.org/3/",
            &["python.org".into()],
            &[]
        ));
        assert!(!domain_allowed(
            "https://example.com/",
            &["python.org".into()],
            &[]
        ));
        assert!(!domain_allowed(
            "https://example.com/",
            &[],
            &["example.com".into()]
        ));
        assert!(domain_allowed("https://example.com/", &[], &[]));
    }
}
