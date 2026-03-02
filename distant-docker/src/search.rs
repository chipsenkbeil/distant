//! Search implementation for Docker containers using best-effort tool detection.

use std::io;

use distant_core::protocol::{
    RemotePath, SearchQuery, SearchQueryCondition, SearchQueryContentsMatch, SearchQueryMatch,
    SearchQueryMatchData, SearchQueryPathMatch, SearchQuerySubmatch, SearchQueryTarget,
};

use crate::utils::SearchTools;

/// Escape regex metacharacters in a string for use in grep/rg patterns.
fn shell_escape_pattern(s: &str) -> String {
    let mut escaped = String::with_capacity(s.len() * 2);
    for c in s.chars() {
        match c {
            '\\' | '.' | '+' | '*' | '?' | '(' | ')' | '|' | '[' | ']' | '{' | '}' | '^' | '$' => {
                escaped.push('\\');
                escaped.push(c);
            }
            _ => escaped.push(c),
        }
    }
    escaped
}

/// Build a shell command for a search query based on available tools.
pub fn build_search_command(query: &SearchQuery, tools: &SearchTools) -> io::Result<String> {
    let path = query
        .paths
        .first()
        .map(|p| p.as_str().to_string())
        .unwrap_or_else(|| ".".to_string());

    let pattern = build_unix_pattern(&query.condition);

    match query.target {
        SearchQueryTarget::Path => build_path_search_command(&path, &pattern, tools),
        SearchQueryTarget::Contents => build_contents_search_command(&path, &pattern, tools),
    }
}

/// Build a Unix grep/rg-compatible regex pattern from a search condition.
fn build_unix_pattern(condition: &SearchQueryCondition) -> String {
    match condition {
        SearchQueryCondition::Regex { value } => value.clone(),
        SearchQueryCondition::Contains { value } => shell_escape_pattern(value),
        SearchQueryCondition::EndsWith { value } => format!("{}$", shell_escape_pattern(value)),
        SearchQueryCondition::StartsWith { value } => format!("^{}", shell_escape_pattern(value)),
        SearchQueryCondition::Equals { value } => {
            format!("^{}$", shell_escape_pattern(value))
        }
        SearchQueryCondition::Or { value } => {
            // Combine sub-conditions with | for alternation
            let parts: Vec<String> = value
                .iter()
                .map(|cond| match cond {
                    SearchQueryCondition::Regex { value } => value.clone(),
                    SearchQueryCondition::Contains { value } => shell_escape_pattern(value),
                    SearchQueryCondition::EndsWith { value } => {
                        format!("{}$", shell_escape_pattern(value))
                    }
                    SearchQueryCondition::StartsWith { value } => {
                        format!("^{}", shell_escape_pattern(value))
                    }
                    SearchQueryCondition::Equals { value } => {
                        format!("^{}$", shell_escape_pattern(value))
                    }
                    SearchQueryCondition::Or { .. } => {
                        // Nested Or -- flatten to simple alternation
                        ".*".to_string()
                    }
                })
                .collect();
            parts.join("|")
        }
    }
}

/// Build a Unix path search command using rg or find.
fn build_path_search_command(path: &str, pattern: &str, tools: &SearchTools) -> io::Result<String> {
    if tools.has_rg {
        Ok(format!("rg --files {} | grep -E '{}'", path, pattern))
    } else if tools.has_find {
        Ok(format!("find {} -regex '.*{}.*' -print", path, pattern))
    } else {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "No search tools available (need rg or find)",
        ))
    }
}

/// Build a Unix contents search command using rg or grep.
fn build_contents_search_command(
    path: &str,
    pattern: &str,
    tools: &SearchTools,
) -> io::Result<String> {
    if tools.has_rg {
        // Use ripgrep with line numbers for structured output
        Ok(format!("rg -n '{}' {}", pattern, path))
    } else if tools.has_grep {
        Ok(format!("grep -rn '{}' {}", pattern, path))
    } else {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "No search tools available (need rg or grep)",
        ))
    }
}

/// Parse grep/rg output lines into search matches.
///
/// Expected format: `filepath:linenum:matched_line`
pub fn parse_contents_matches(output: &str) -> Vec<SearchQueryMatch> {
    let mut matches = Vec::new();

    for line in output.lines() {
        if line.is_empty() {
            continue;
        }

        let parts: Vec<&str> = line.splitn(3, ':').collect();

        if parts.len() < 3 {
            continue;
        }

        let filepath = parts[0].to_string();
        let line_num_str = parts[1];
        let content = parts[2];

        if let Ok(line_num) = line_num_str.parse::<u64>() {
            let content = content.to_string();
            matches.push(SearchQueryMatch::Contents(SearchQueryContentsMatch {
                path: RemotePath::new(&filepath),
                lines: SearchQueryMatchData::Text(content.clone()),
                line_number: line_num,
                absolute_offset: 0,
                submatches: vec![SearchQuerySubmatch {
                    r#match: SearchQueryMatchData::Text(content),
                    start: 0,
                    end: 0,
                }],
            }));
        }
    }

    matches
}

/// Parse path search output into search matches.
pub fn parse_path_matches(output: &str) -> Vec<SearchQueryMatch> {
    output
        .lines()
        .filter(|line| !line.is_empty())
        .map(|line| {
            SearchQueryMatch::Path(SearchQueryPathMatch {
                path: RemotePath::new(line.trim()),
                submatches: vec![SearchQuerySubmatch {
                    r#match: SearchQueryMatchData::Text(line.trim().to_string()),
                    start: 0,
                    end: 0,
                }],
            })
        })
        .collect()
}
