use std::path::PathBuf;
use std::process::Command;
use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct LocalSearchResult {
    pub name: String,
    pub path: String,
    pub size: u64,
    pub date: String,
}

#[derive(Debug, Serialize)]
pub struct LocalSearchResponse {
    pub results: Vec<LocalSearchResult>,
    pub count: usize,
    pub query: String,
}

pub struct LocalSearchOptions {
    pub query: String,
    pub max_results: Option<u32>,
    pub sort: Option<String>,
    pub match_case: Option<bool>,
    pub match_whole_word: Option<bool>,
    pub match_regex: Option<bool>,
    pub match_path: Option<bool>,
    pub offset: Option<u32>,
    pub path: Option<String>,
    pub parent_path: Option<String>,
    pub files_only: Option<bool>,
    pub folders_only: Option<bool>,
    pub content_search: Option<bool>,
    pub size_format: Option<String>,
    pub date_format: Option<String>,
}

pub fn find_es_exe() -> Option<PathBuf> {
    // Check environment variable first
    if let Ok(env_path) = std::env::var("EVERYTHING_ES_PATH") {
        let p = PathBuf::from(&env_path);
        if p.exists() { return Some(p); }
    }

    // Check bundled vendor/ directory (relative to the binary)
    if let Ok(exe_path) = std::env::current_exe() {
        let vendor_path = exe_path.parent()
            .and_then(|p| p.parent())
            .and_then(|p| p.parent())
            .map(|p| p.join("vendor").join("es.exe"));
        if let Some(p) = vendor_path {
            if p.exists() { return Some(p); }
        }
    }

    let candidates = vec![
        format!("{}/.local/bin/es.exe", std::env::var("USERPROFILE").unwrap_or_default()),
        "C:\\Program Files\\Everything\\es.exe".to_string(),
        "C:\\Program Files (x86)\\Everything\\es.exe".to_string(),
    ];

    for c in candidates {
        let p = PathBuf::from(&c);
        if p.exists() { return Some(p); }
    }

    None
}

pub fn search_local(options: &LocalSearchOptions) -> Result<LocalSearchResponse, String> {
    let es_path = find_es_exe().ok_or_else(|| {
        "es.exe not found. Install Everything (voidtools) or set EVERYTING_ES_PATH env var.".to_string()
    })?;

    let mut args = vec!["-csv".to_string(), "-size".to_string(), "-date-modified".to_string()];

    if let Some(ref mr) = options.max_results {
        args.push("-n".to_string());
        args.push(mr.to_string());
    }
    if let Some(ref offset) = options.offset {
        args.push("-o".to_string());
        args.push(offset.to_string());
    }
    if let Some(ref sort) = options.sort {
        args.push("-sort".to_string());
        args.push(sort.to_string());
    }
    if options.match_case.unwrap_or(false) {
        args.push("-case".to_string());
    }
    if options.match_whole_word.unwrap_or(false) {
        args.push("-w".to_string());
    }
    if options.match_regex.unwrap_or(false) {
        args.push("-r".to_string());
    }
    if options.match_path.unwrap_or(false) {
        args.push("-p".to_string());
    }
    if let Some(ref path) = options.path {
        args.push("-path".to_string());
        args.push(path.to_string());
    }
    if let Some(ref parent) = options.parent_path {
        args.push("-parent-path".to_string());
        args.push(parent.to_string());
    }
    if options.files_only.unwrap_or(false) {
        args.push("/a-d".to_string());
    }
    if options.folders_only.unwrap_or(false) {
        args.push("/ad".to_string());
    }
    if options.content_search.unwrap_or(false) {
        args.push(format!("content:{}", options.query));
    } else {
        args.push(options.query.clone());
    }
    match options.size_format.as_deref() {
        Some("bytes") | None => { args.push("-size-format".to_string()); args.push("1".to_string()); }
        Some("kb") => { args.push("-size-format".to_string()); args.push("2".to_string()); }
        Some("mb") => { args.push("-size-format".to_string()); args.push("3".to_string()); }
        _ => {}
    }
    match options.date_format.as_deref() {
        Some("iso-8601") => { args.push("-date-format".to_string()); args.push("1".to_string()); }
        Some("iso-8601-utc") => { args.push("-date-format".to_string()); args.push("3".to_string()); }
        _ => {}
    }
    let mut cmd = Command::new(&es_path);
    cmd.args(&args);
    let output = cmd.output()
        .map_err(|e| format!("Failed to execute es.exe: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("es.exe failed: {}", stderr));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let lines: Vec<&str> = stdout.trim().split('\n')
        .filter(|l| !l.is_empty())
        .collect();

    if lines.len() < 2 {
        return Ok(LocalSearchResponse {
            results: vec![],
            count: 0,
            query: options.query.clone(),
        });
    }

    let _headers = parse_csv_line(lines[0]);
    let results: Vec<LocalSearchResult> = lines[1..].iter()
        .map(|line| {
            let row = parse_csv_line(line);
            let file_path = row.iter().last().cloned().unwrap_or_default();
            let name = file_path.split(|c: char| c == '/' || c == '\\')
                .last().unwrap_or("").to_string();
            let size = row.get(0).and_then(|s| s.replace(',', "").parse().ok()).unwrap_or(0);
            let date = row.get(1).cloned().unwrap_or_default();
            LocalSearchResult { name, path: file_path, size, date }
        })
        .collect();

    Ok(LocalSearchResponse {
        count: results.len(),
        query: options.query.clone(),
        results,
    })
}

fn parse_csv_line(line: &str) -> Vec<String> {
    let mut fields = Vec::new();
    let mut current = String::new();
    let mut in_quotes = false;
    let chars: Vec<char> = line.chars().collect();
    let mut i = 0;

    while i < chars.len() {
        match chars[i] {
            '"' if !in_quotes => in_quotes = true,
            '"' if in_quotes => {
                if i + 1 < chars.len() && chars[i + 1] == '"' {
                    current.push('"');
                    i += 1;
                } else {
                    in_quotes = false;
                }
            }
            ',' if !in_quotes => {
                fields.push(current.clone());
                current.clear();
            }
            c => current.push(c),
        }
        i += 1;
    }
    fields.push(current);
    fields
}

