//! Search implementation for Docker containers using best-effort tool detection.

use std::io;
use std::path::PathBuf;

use distant_core::protocol::{
    SearchQuery, SearchQueryCondition, SearchQueryContentsMatch, SearchQueryMatch,
    SearchQueryMatchData, SearchQueryPathMatch, SearchQuerySubmatch, SearchQueryTarget,
};

use crate::DockerFamily;
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

/// Escape findstr regex metacharacters.
///
/// Findstr supports only: `.` `*` `^` `$` `[class]` `\x` — much more limited than POSIX regex.
fn findstr_escape_pattern(s: &str) -> String {
    let mut escaped = String::with_capacity(s.len() * 2);
    for c in s.chars() {
        match c {
            '\\' | '.' | '*' | '^' | '$' | '[' | ']' => {
                escaped.push('\\');
                escaped.push(c);
            }
            _ => escaped.push(c),
        }
    }
    escaped
}

/// Build a findstr-compatible pattern from a search query condition.
///
/// Returns `(pattern, is_literal)` where `is_literal` indicates the pattern should be used
/// with `/C:` (literal) rather than `/R` (regex).
///
/// # Errors
///
/// Returns `Unsupported` for `Regex` and `Or` conditions — findstr's regex is too limited
/// for arbitrary patterns and does not support alternation.
fn build_findstr_pattern(condition: &SearchQueryCondition) -> io::Result<(String, bool)> {
    match condition {
        SearchQueryCondition::Contains { value } => Ok((value.clone(), true)),
        SearchQueryCondition::Equals { value } => {
            Ok((format!("^{}$", findstr_escape_pattern(value)), false))
        }
        SearchQueryCondition::StartsWith { value } => {
            Ok((format!("^{}", findstr_escape_pattern(value)), false))
        }
        SearchQueryCondition::EndsWith { value } => {
            Ok((format!("{}$", findstr_escape_pattern(value)), false))
        }
        SearchQueryCondition::Regex { .. } | SearchQueryCondition::Or { .. } => {
            Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "findstr does not support full regex or alternation patterns",
            ))
        }
    }
}

/// Build a shell command for a search query based on available tools.
pub fn build_search_command(
    query: &SearchQuery,
    tools: &SearchTools,
    family: DockerFamily,
) -> io::Result<String> {
    let path = query
        .paths
        .first()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|| ".".to_string());

    // When findstr is the only available tool (Windows), use the findstr pattern builder
    // which handles the limited regex subset and returns Unsupported for Or/Regex.
    let uses_findstr = tools.has_findstr
        && !tools.has_rg
        && !tools.has_grep
        && !tools.has_find
        && family == DockerFamily::Windows;

    if uses_findstr {
        let (pattern, is_literal) = build_findstr_pattern(&query.condition)?;
        return match query.target {
            SearchQueryTarget::Path => build_findstr_path_command(&path, &pattern, is_literal),
            SearchQueryTarget::Contents => {
                build_findstr_contents_command(&path, &pattern, is_literal)
            }
        };
    }

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

/// Build a findstr path search command using `dir /s /b` piped to `findstr`.
fn build_findstr_path_command(path: &str, pattern: &str, is_literal: bool) -> io::Result<String> {
    let flag = if is_literal {
        format!("/I /C:\"{}\"", pattern)
    } else {
        format!("/I /R \"{}\"", pattern)
    };
    Ok(format!("dir /s /b \"{}\" | findstr {}", path, flag))
}

/// Build a findstr contents search command.
///
/// Uses `/N` for line numbers and `/S` for recursive search.
/// Output format matches grep: `filepath:linenum:content`.
fn build_findstr_contents_command(
    path: &str,
    pattern: &str,
    is_literal: bool,
) -> io::Result<String> {
    let flag = if is_literal {
        format!("/N /S /I /C:\"{}\"", pattern)
    } else {
        format!("/N /S /I /R \"{}\"", pattern)
    };
    Ok(format!("findstr {} \"{}\\*\"", flag, path))
}

/// Parse grep/rg/findstr output lines into search matches.
///
/// Expected format: `filepath:linenum:matched_line`
///
/// Handles Windows drive-letter paths where output is `C:\path:10:content` — the drive
/// letter and backslash-prefixed path are reconstituted from the first two colon-split parts.
pub fn parse_contents_matches(output: &str) -> Vec<SearchQueryMatch> {
    let mut matches = Vec::new();

    for line in output.lines() {
        if line.is_empty() {
            continue;
        }

        // Split into up to 4 parts to handle Windows drive-letter paths (C:\path:linenum:content)
        let parts: Vec<&str> = line.splitn(4, ':').collect();

        // Try Windows drive-letter path: single ASCII letter + backslash-prefixed path
        let (filepath, line_num_str, content) = if parts.len() >= 4
            && parts[0].len() == 1
            && parts[0]
                .chars()
                .next()
                .is_some_and(|c| c.is_ascii_alphabetic())
            && parts[1].starts_with('\\')
        {
            // Recombine drive letter: "C" + ":" + "\path"
            let path = format!("{}:{}", parts[0], parts[1]);
            (path, parts[2], parts[3])
        } else if parts.len() >= 3 {
            (parts[0].to_string(), parts[1], parts[2])
        } else {
            continue;
        };

        if let Ok(line_num) = line_num_str.parse::<u64>() {
            let content = content.to_string();
            matches.push(SearchQueryMatch::Contents(SearchQueryContentsMatch {
                path: PathBuf::from(&filepath),
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

/// Check if a string matches a search query condition.
///
/// Used by the tar-based search fallback on Windows nanoserver, where exec-based
/// search tools (`findstr`, `dir`) cannot access paths created via the Docker tar API.
#[allow(dead_code)]
pub fn condition_matches(condition: &SearchQueryCondition, text: &str) -> bool {
    match condition {
        SearchQueryCondition::Contains { value } => {
            text.to_lowercase().contains(&value.to_lowercase())
        }
        SearchQueryCondition::Equals { value } => text.eq_ignore_ascii_case(value),
        SearchQueryCondition::StartsWith { value } => {
            text.to_lowercase().starts_with(&value.to_lowercase())
        }
        SearchQueryCondition::EndsWith { value } => {
            text.to_lowercase().ends_with(&value.to_lowercase())
        }
        SearchQueryCondition::Regex { .. } => {
            // Regex matching is not supported in the tar-based fallback.
            // On Windows nanoserver, Regex conditions are already rejected
            // by `build_findstr_pattern` before reaching this path.
            false
        }
        SearchQueryCondition::Or { value } => {
            value.iter().any(|cond| condition_matches(cond, text))
        }
    }
}

/// Parse path search output into search matches.
pub fn parse_path_matches(output: &str) -> Vec<SearchQueryMatch> {
    output
        .lines()
        .filter(|line| !line.is_empty())
        .map(|line| {
            SearchQueryMatch::Path(SearchQueryPathMatch {
                path: PathBuf::from(line.trim()),
                submatches: vec![SearchQuerySubmatch {
                    r#match: SearchQueryMatchData::Text(line.trim().to_string()),
                    start: 0,
                    end: 0,
                }],
            })
        })
        .collect()
}
