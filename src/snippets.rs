//! Query-relevant snippet extraction for `get_page` (the "highlights" mode).
//!
//! Given a fetched page and a query, return the most relevant passages instead
//! of the whole document. Purely local and deterministic — no LLM call, so it
//! stays cheap and works offline.
//!
//! Query terms are matched as follows: ASCII words (length >= 2, case
//! insensitive) and CJK bigrams (plus single chars for one-char runs). CJK has
//! no spaces, so bigrams stand in for a segmenter without pulling in a
//! dictionary. Windows are scored by **distinct-term coverage first** (a
//! passage containing several different query terms beats one repeating a
//! single term), then by raw hit count.

use std::collections::HashSet;

/// A ranked passage. Offsets are character-based, half-open `[start, end)`,
/// matching the `startChar`/`endChar` contract of `get_page`.
#[derive(Debug, Clone)]
pub struct Snippet {
    pub start_char: usize,
    pub end_char: usize,
    pub score: usize,
    pub terms: Vec<String>,
    pub context: String,
}

fn is_cjk(c: char) -> bool {
    matches!(c as u32,
        0x3400..=0x4DBF | 0x4E00..=0x9FFF | 0xF900..=0xFAFF | 0x3040..=0x30FF)
}

/// Split a query into matchable terms (ASCII words + CJK n-grams).
pub fn tokenize(query: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut ascii = String::new();
    let mut cjk: Vec<char> = Vec::new();

    fn flush_ascii(a: &mut String, out: &mut Vec<String>) {
        if a.chars().count() >= 2 {
            out.push(a.to_ascii_lowercase());
        }
        a.clear();
    }
    fn flush_cjk(c: &mut Vec<char>, out: &mut Vec<String>) {
        if c.len() >= 2 {
            for w in c.windows(2) {
                out.push(w.iter().collect::<String>());
            }
        } else if c.len() == 1 {
            out.push(c[0].to_string());
        }
        c.clear();
    }

    for ch in query.chars() {
        if ch.is_ascii_alphanumeric() || ch == '_' || ch == '+' || ch == '#' {
            flush_cjk(&mut cjk, &mut out);
            ascii.push(ch);
        } else if is_cjk(ch) {
            flush_ascii(&mut ascii, &mut out);
            cjk.push(ch);
        } else {
            flush_ascii(&mut ascii, &mut out);
            flush_cjk(&mut cjk, &mut out);
        }
    }
    flush_ascii(&mut ascii, &mut out);
    flush_cjk(&mut cjk, &mut out);

    // De-duplicate, preserving order.
    let mut seen = HashSet::new();
    out.retain(|t| seen.insert(t.clone()));
    out
}

/// All occurrences of `needle` in `hay` (character offsets, ASCII-insensitive).
fn find_all(hay: &[char], needle: &[char]) -> Vec<(usize, usize)> {
    let mut hits = Vec::new();
    if needle.is_empty() || needle.len() > hay.len() {
        return hits;
    }
    let ascii = needle.iter().all(|c| c.is_ascii());
    'outer: for i in 0..=hay.len() - needle.len() {
        for j in 0..needle.len() {
            let (h, n) = (hay[i + j], needle[j]);
            let same = if ascii {
                h.eq_ignore_ascii_case(&n)
            } else {
                h == n
            };
            if !same {
                continue 'outer;
            }
        }
        hits.push((i, i + needle.len()));
    }
    hits
}

/// Rank passages of `text` against `query`.
pub fn highlight(text: &str, query: &str, context_chars: usize, max_snippets: usize) -> Vec<Snippet> {
    let terms = tokenize(query);
    if terms.is_empty() {
        return Vec::new();
    }
    let hay: Vec<char> = text.chars().collect();
    let total = hay.len();

    // (start, end, term, term_index) for every occurrence.
    struct Hit {
        start: usize,
        end: usize,
        term_idx: usize,
    }
    let mut hits: Vec<Hit> = Vec::new();
    for (ti, term) in terms.iter().enumerate() {
        let needle: Vec<char> = term.chars().collect();
        for (s, e) in find_all(&hay, &needle) {
            hits.push(Hit {
                start: s,
                end: e,
                term_idx: ti,
            });
        }
    }
    if hits.is_empty() {
        return Vec::new();
    }
    hits.sort_by_key(|h| (h.start, h.end));

    // Merge hits into windows of ±context_chars.
    struct Window {
        start: usize,
        end: usize,
        hit_count: usize,
        terms: HashSet<usize>,
    }
    let mut windows: Vec<Window> = Vec::new();
    for h in &hits {
        let ws = h.start.saturating_sub(context_chars);
        let we = std::cmp::min(h.end + context_chars, total);
        if let Some(last) = windows.last_mut() {
            if ws <= last.end {
                last.end = last.end.max(we);
                last.hit_count += 1;
                last.terms.insert(h.term_idx);
                continue;
            }
        }
        let mut terms_set = HashSet::new();
        terms_set.insert(h.term_idx);
        windows.push(Window {
            start: ws,
            end: we,
            hit_count: 1,
            terms: terms_set,
        });
    }

    // Score: coverage dominates, hit count breaks ties.
    let mut ranked: Vec<(usize, usize, &Window)> = windows
        .iter()
        .map(|w| (w.terms.len() * 1000 + w.hit_count, w.hit_count, w))
        .collect();
    ranked.sort_by(|a, b| b.0.cmp(&a.0).then(b.1.cmp(&a.1)));

    let max_snippets = max_snippets.max(1);
    let mut chosen: Vec<&Window> = ranked
        .into_iter()
        .take(max_snippets)
        .map(|(_, _, w)| w)
        .collect();

    // Present in document order.
    chosen.sort_by_key(|w| w.start);

    chosen
        .into_iter()
        .map(|w| Snippet {
            start_char: w.start,
            end_char: w.end,
            score: w.terms.len() * 1000 + w.hit_count,
            terms: {
                let mut t: Vec<String> = w.terms.iter().map(|&i| terms[i].clone()).collect();
                t.sort();
                t
            },
            context: hay[w.start..w.end].iter().collect(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokenizes_ascii_and_cjk() {
        assert_eq!(tokenize("Python asyncio"), vec!["python", "asyncio"]);
        // single ascii chars are dropped
        assert_eq!(tokenize("a bc"), vec!["bc"]);
        // CJK becomes bigrams
        let t = tokenize("大模型");
        assert_eq!(t, vec!["大模", "模型"]);
    }

    #[test]
    fn ranks_by_distinct_term_coverage() {
        let text = "开头无关内容。\n\n大模型训练需要大量数据。\n\n中间无关。\n\n关于大模型推理与部署的讨论，涉及模型压缩。\n\n结尾。";
        let snips = highlight(text, "大模型 推理", 12, 3);
        assert!(!snips.is_empty());
        // The passage covering both "大模/模型" and "推理" should rank first.
        let first = &snips[0];
        assert!(
            first.context.contains("推理"),
            "expected the coverage-best snippet first, got: {}",
            first.context
        );
    }

    #[test]
    fn returns_empty_when_nothing_matches() {
        assert!(highlight("hello world", "zzzz", 20, 5).is_empty());
    }

    #[test]
    fn offsets_are_char_based() {
        let text = "中文abc中文";
        let snips = highlight(text, "abc", 2, 1);
        assert_eq!(snips.len(), 1);
        let s = &snips[0];
        let chars: Vec<char> = text.chars().collect();
        let slice: String = chars[s.start_char..s.end_char].iter().collect();
        assert_eq!(slice, s.context);
    }
}
