//! Search implementation for Docker containers using best-effort tool detection.

use std::io;

use distant_core::protocol::{
    RemotePath, SearchQuery, SearchQueryCondition, SearchQueryContentsMatch, SearchQueryMatch,
    SearchQueryMatchData, SearchQueryPathMatch, SearchQuerySubmatch, SearchQueryTarget,
};

use crate::utils::{SearchTools, shell_quote};

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

/// Result of building a search command, including the tool used.
pub struct SearchCommand {
    /// The shell command string to execute.
    pub command: String,

    /// The primary search tool used.
    pub tool: SearchTool,
}

/// Indicates which search tool backs a command.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SearchTool {
    /// ripgrep (rg).
    Rg,
    /// GNU grep.
    Grep,
    /// find.
    Find,
}

impl SearchCommand {
    /// Returns true if the given exit code indicates a real error (not just "no matches").
    ///
    /// For grep/rg, exit code 1 means no matches (not an error), while >= 2 is an error.
    /// For find, any non-zero exit code is an error.
    pub fn is_error_exit(&self, code: i64) -> bool {
        match self.tool {
            SearchTool::Rg | SearchTool::Grep => code >= 2,
            SearchTool::Find => code != 0,
        }
    }
}

/// Build a shell command for a search query based on available tools.
pub fn build_search_command(query: &SearchQuery, tools: &SearchTools) -> io::Result<SearchCommand> {
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
fn build_path_search_command(
    path: &str,
    pattern: &str,
    tools: &SearchTools,
) -> io::Result<SearchCommand> {
    let quoted_path = shell_quote(path);
    let quoted_pattern = shell_quote(pattern);
    if tools.has_rg {
        Ok(SearchCommand {
            command: format!("rg --files {quoted_path} | grep -E {quoted_pattern}"),
            tool: SearchTool::Rg,
        })
    } else if tools.has_find {
        Ok(SearchCommand {
            command: format!("find {quoted_path} -regex '.*{pattern}.*' -print"),
            tool: SearchTool::Find,
        })
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
) -> io::Result<SearchCommand> {
    let quoted_path = shell_quote(path);
    let quoted_pattern = shell_quote(pattern);
    if tools.has_rg {
        // Use ripgrep with line numbers for structured output
        Ok(SearchCommand {
            command: format!("rg -n {quoted_pattern} {quoted_path}"),
            tool: SearchTool::Rg,
        })
    } else if tools.has_grep {
        Ok(SearchCommand {
            command: format!("grep -rn {quoted_pattern} {quoted_path}"),
            tool: SearchTool::Grep,
        })
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
