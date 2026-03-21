//! Search implementation for Docker containers using best-effort tool detection.
//!
//! Discovers available search tools (`rg`, `grep`, `find`) in the container
//! and builds shell commands to execute searches. Supports both path-based and
//! content-based search queries with include/exclude filtering, max depth,
//! upward traversal, and accurate byte-offset/submatch computation.

use std::io;

use distant_core::protocol::{
    RemotePath, SearchQuery, SearchQueryCondition, SearchQueryContentsMatch, SearchQueryMatch,
    SearchQueryMatchData, SearchQueryPathMatch, SearchQuerySubmatch, SearchQueryTarget,
};
use regex::Regex;
use serde_json::Value;
use typed_path::Utf8UnixPath;

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
    Ripgrep,
    Grep,
    Find,
}

impl SearchCommand {
    /// Returns true if the given exit code indicates a real error (not just "no matches").
    ///
    /// For grep/rg, exit code 1 means no matches (not an error), while >= 2 is an error.
    /// For find, any non-zero exit code is an error.
    pub fn is_error_exit(&self, code: i64) -> bool {
        match self.tool {
            SearchTool::Ripgrep | SearchTool::Grep => code >= 2,
            SearchTool::Find => code != 0,
        }
    }
}

/// Build one or more shell commands for a search query based on available tools.
///
/// When the query has `upward` set to true, returns multiple commands — one per
/// ancestor directory — each searching with depth=1. Otherwise returns a single command.
pub fn build_search_commands(
    query: &SearchQuery,
    tools: &SearchTools,
) -> io::Result<Vec<SearchCommand>> {
    let pattern = build_unix_pattern(&query.condition);

    let include_pattern = query.options.include.as_ref().map(build_unix_pattern);
    let exclude_pattern = query.options.exclude.as_ref().map(build_unix_pattern);

    if query.options.upward {
        build_upward_search_commands(
            query,
            &pattern,
            tools,
            include_pattern.as_deref(),
            exclude_pattern.as_deref(),
        )
    } else {
        let paths = build_paths_string(&query.paths);
        let max_depth = query.options.max_depth;

        let cmd = match query.target {
            SearchQueryTarget::Path => build_path_search_command(
                &paths,
                &pattern,
                tools,
                include_pattern.as_deref(),
                exclude_pattern.as_deref(),
                max_depth,
            )?,
            SearchQueryTarget::Contents => build_contents_search_command(
                &paths,
                &pattern,
                tools,
                include_pattern.as_deref(),
                exclude_pattern.as_deref(),
                max_depth,
            )?,
        };
        Ok(vec![cmd])
    }
}

/// Build commands for upward (ancestor) traversal.
///
/// For each query path, walks parent directories upward, generating a search command
/// at each level with depth=1. The number of ancestor levels is bounded by `max_depth`.
fn build_upward_search_commands(
    query: &SearchQuery,
    pattern: &str,
    tools: &SearchTools,
    include: Option<&str>,
    exclude: Option<&str>,
) -> io::Result<Vec<SearchCommand>> {
    let mut commands = Vec::new();
    let paths: Vec<String> = if query.paths.is_empty() {
        vec![".".to_string()]
    } else {
        query.paths.iter().map(|p| p.as_str().to_string()).collect()
    };

    for path in &paths {
        // Always search the starting directory itself (at depth=1 so we see
        // its immediate entries).
        let quoted = shell_quote(path);
        let cmd = match query.target {
            SearchQueryTarget::Path => {
                build_path_search_command(&quoted, pattern, tools, include, exclude, Some(1))?
            }
            SearchQueryTarget::Contents => {
                build_contents_search_command(&quoted, pattern, tools, include, exclude, Some(1))?
            }
        };
        commands.push(cmd);

        // Walk upward through ancestors.
        let mut current = path.clone();
        let mut remaining = query.options.max_depth;

        // If max_depth is Some(0), do not traverse any ancestors
        if remaining == Some(0) {
            continue;
        }

        loop {
            let parent = unix_parent(&current);
            if parent == current {
                break;
            }

            if let Some(ref mut rem) = remaining {
                if *rem == 0 {
                    break;
                }
                *rem -= 1;
            }

            let quoted_parent = shell_quote(&parent);
            let cmd = match query.target {
                SearchQueryTarget::Path => build_path_search_command(
                    &quoted_parent,
                    pattern,
                    tools,
                    include,
                    exclude,
                    Some(1),
                )?,
                SearchQueryTarget::Contents => build_contents_search_command(
                    &quoted_parent,
                    pattern,
                    tools,
                    include,
                    exclude,
                    Some(1),
                )?,
            };
            commands.push(cmd);
            current = parent;
        }
    }

    Ok(commands)
}

/// Get the parent of a Unix path string.
///
/// Returns `"/"` for root paths and `"."` for relative paths with no parent.
fn unix_parent(path: &str) -> String {
    let unix = Utf8UnixPath::new(path);
    match unix.parent() {
        Some(p) if p.as_str().is_empty() => ".".to_string(),
        Some(p) => p.as_str().to_string(),
        None => path.to_string(),
    }
}

/// Build the combined quoted paths string from a list of remote paths.
///
/// Falls back to `"."` if the list is empty.
fn build_paths_string(paths: &[RemotePath]) -> String {
    if paths.is_empty() {
        shell_quote(".")
    } else {
        paths
            .iter()
            .map(|p| shell_quote(p.as_str()))
            .collect::<Vec<_>>()
            .join(" ")
    }
}

/// Build a Unix grep/rg-compatible regex pattern from a search condition.
pub fn build_unix_pattern(condition: &SearchQueryCondition) -> String {
    match condition {
        SearchQueryCondition::Regex { value } => value.clone(),
        SearchQueryCondition::Contains { value } => shell_escape_pattern(value),
        SearchQueryCondition::EndsWith { value } => format!("{}$", shell_escape_pattern(value)),
        SearchQueryCondition::StartsWith { value } => format!("^{}", shell_escape_pattern(value)),
        SearchQueryCondition::Equals { value } => {
            format!("^{}$", shell_escape_pattern(value))
        }
        SearchQueryCondition::Or { value } => {
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

/// Build a path search command using rg or find.
///
/// Pattern matching, include/exclude filtering are all applied in Rust during
/// result parsing (see [`parse_path_matches`]), so the shell command just lists files.
fn build_path_search_command(
    paths: &str,
    _pattern: &str,
    tools: &SearchTools,
    _include: Option<&str>,
    _exclude: Option<&str>,
    max_depth: Option<u64>,
) -> io::Result<SearchCommand> {
    if tools.has_rg {
        let mut cmd = "rg --files".to_string();
        if let Some(depth) = max_depth {
            cmd.push_str(&format!(" --max-depth {depth}"));
        }
        cmd.push_str(&format!(" {paths}"));
        Ok(SearchCommand {
            command: cmd,
            tool: SearchTool::Ripgrep,
        })
    } else if tools.has_find {
        let mut cmd = format!("find {paths}");
        if let Some(depth) = max_depth {
            cmd.push_str(&format!(" -maxdepth {depth}"));
        }
        cmd.push_str(" -print");
        Ok(SearchCommand {
            command: cmd,
            tool: SearchTool::Find,
        })
    } else {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "No search tools available (need rg or find)",
        ))
    }
}

/// Build a contents search command using rg or grep.
///
/// When `max_depth` is set and only grep is available, requires `find` to
/// enumerate files with depth limiting. Always requests byte offsets (`-b`)
/// from grep for consistent output parsing.
fn build_contents_search_command(
    paths: &str,
    pattern: &str,
    tools: &SearchTools,
    _include: Option<&str>,
    _exclude: Option<&str>,
    max_depth: Option<u64>,
) -> io::Result<SearchCommand> {
    let quoted_pattern = shell_quote(pattern);

    if tools.has_rg {
        // Use rg --json for structured output with byte offsets and submatches.
        // Include/exclude filtering is handled during parsing in Rust.
        let mut cmd = "rg --json".to_string();
        if let Some(depth) = max_depth {
            cmd.push_str(&format!(" --max-depth {depth}"));
        }
        cmd.push_str(&format!(" {quoted_pattern} {paths}"));
        Ok(SearchCommand {
            command: cmd,
            tool: SearchTool::Ripgrep,
        })
    } else if tools.has_grep {
        if let Some(depth) = max_depth {
            // BSD grep does not support --max-depth. When max_depth is set,
            // use find with -maxdepth to enumerate files, then grep each one.
            // /dev/null is included so grep always prints file paths even for a
            // single match. Exit code follows find semantics (0 = ok, else error).
            if !tools.has_find {
                return Err(io::Error::new(
                    io::ErrorKind::Unsupported,
                    "max_depth search requires find, but it is not available",
                ));
            }
            let cmd = format!(
                "find {paths} -maxdepth {depth} -type f \
                 -exec grep -bn {quoted_pattern} {{}} /dev/null \\;"
            );
            Ok(SearchCommand {
                command: cmd,
                tool: SearchTool::Find,
            })
        } else {
            let cmd = format!("grep -rbn {quoted_pattern} {paths}");
            Ok(SearchCommand {
                command: cmd,
                tool: SearchTool::Grep,
            })
        }
    } else {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "No search tools available (need rg or grep)",
        ))
    }
}

/// Parse search output into content matches based on the tool that produced it.
///
/// Dispatches to the appropriate parser for rg JSON or grep byte-offset formats.
/// Include/exclude regex patterns are applied in Rust during parsing to avoid
/// shell pipeline dependencies on awk.
pub fn parse_contents_matches(
    output: &str,
    tool: SearchTool,
    include: Option<&str>,
    exclude: Option<&str>,
) -> Vec<SearchQueryMatch> {
    match tool {
        SearchTool::Ripgrep => parse_rg_json_contents(output, include, exclude),
        SearchTool::Grep | SearchTool::Find => parse_grep_contents(output, include, exclude),
    }
}

/// Parse rg `--json` output into content search matches.
///
/// Each JSON line with `type: "match"` contains structured data including
/// file path, line number, absolute byte offset, and submatch positions.
/// Include/exclude regex patterns are applied as post-filters on the file path.
fn parse_rg_json_contents(
    output: &str,
    include: Option<&str>,
    exclude: Option<&str>,
) -> Vec<SearchQueryMatch> {
    let include_re = include.and_then(|p| Regex::new(p).ok());
    let exclude_re = exclude.and_then(|p| Regex::new(p).ok());

    let mut matches = Vec::new();

    for line in output.lines() {
        if line.is_empty() {
            continue;
        }

        let parsed: Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(_) => continue,
        };

        if parsed.get("type").and_then(Value::as_str) != Some("match") {
            continue;
        }

        let data = match parsed.get("data") {
            Some(d) => d,
            None => continue,
        };

        let path_str = data
            .get("path")
            .and_then(|p| p.get("text"))
            .and_then(Value::as_str)
            .unwrap_or("");

        // Apply include/exclude filters on the file path
        if let Some(ref re) = include_re
            && !re.is_match(path_str)
        {
            continue;
        }
        if let Some(ref re) = exclude_re
            && re.is_match(path_str)
        {
            continue;
        }

        let line_number = data.get("line_number").and_then(Value::as_u64).unwrap_or(0);

        let absolute_offset = data
            .get("absolute_offset")
            .and_then(Value::as_u64)
            .unwrap_or(0);

        let lines_text = data
            .get("lines")
            .and_then(|l| l.get("text"))
            .and_then(Value::as_str)
            .unwrap_or("");

        let mut submatches = Vec::new();
        if let Some(subs) = data.get("submatches").and_then(Value::as_array) {
            for sub in subs {
                let match_text = sub
                    .get("match")
                    .and_then(|m| m.get("text"))
                    .and_then(Value::as_str)
                    .unwrap_or("");
                let start = sub.get("start").and_then(Value::as_u64).unwrap_or(0);
                let end = sub.get("end").and_then(Value::as_u64).unwrap_or(0);

                submatches.push(SearchQuerySubmatch {
                    r#match: SearchQueryMatchData::Text(match_text.to_string()),
                    start,
                    end,
                });
            }
        }

        // If rg reported no submatches, fall back to the full line
        if submatches.is_empty() {
            submatches.push(SearchQuerySubmatch {
                r#match: SearchQueryMatchData::Text(lines_text.to_string()),
                start: 0,
                end: 0,
            });
        }

        matches.push(SearchQueryMatch::Contents(SearchQueryContentsMatch {
            path: RemotePath::new(path_str),
            lines: SearchQueryMatchData::Text(lines_text.to_string()),
            line_number,
            absolute_offset,
            submatches,
        }));
    }

    matches
}

/// Parse grep `-b -n` output into content search matches.
///
/// Expected format: `filepath:byte_offset:linenum:matched_line`
///
/// The `-b` flag provides byte offsets for each matching line. Include/exclude
/// regex patterns are applied as post-filters on the file path in Rust.
fn parse_grep_contents(
    output: &str,
    include: Option<&str>,
    exclude: Option<&str>,
) -> Vec<SearchQueryMatch> {
    let include_re = include.and_then(|p| Regex::new(p).ok());
    let exclude_re = exclude.and_then(|p| Regex::new(p).ok());

    let mut matches = Vec::new();

    for line in output.lines() {
        if line.is_empty() {
            continue;
        }

        let parts: Vec<&str> = line.splitn(4, ':').collect();
        if parts.len() < 4 {
            continue;
        }

        let filepath = parts[0];
        let byte_offset_str = parts[1];
        let line_num_str = parts[2];
        let content = parts[3];

        // Apply include/exclude filters on the file path
        if let Some(ref re) = include_re
            && !re.is_match(filepath)
        {
            continue;
        }
        if let Some(ref re) = exclude_re
            && re.is_match(filepath)
        {
            continue;
        }

        let line_num = match line_num_str.parse::<u64>() {
            Ok(n) => n,
            Err(_) => continue,
        };
        let byte_offset = byte_offset_str.parse::<u64>().unwrap_or(0);

        let content = content.to_string();
        matches.push(SearchQueryMatch::Contents(SearchQueryContentsMatch {
            path: RemotePath::new(filepath),
            lines: SearchQueryMatchData::Text(content.clone()),
            line_number: line_num,
            absolute_offset: byte_offset,
            submatches: compute_text_submatches(&content),
        }));
    }

    matches
}

/// Compute submatches by attempting to infer match text from the line content.
///
/// Without a known pattern, we return the full content as a single submatch.
/// This is the fallback used by grep byte-offset output where we have position
/// information but no structured submatch data from the tool.
fn compute_text_submatches(content: &str) -> Vec<SearchQuerySubmatch> {
    vec![SearchQuerySubmatch {
        r#match: SearchQueryMatchData::Text(content.to_string()),
        start: 0,
        end: content.len() as u64,
    }]
}

/// Parse path search output into search matches, computing submatches from
/// the search condition's regex pattern.
///
/// When the condition can be compiled as a regex, each path is matched to find
/// the exact substring that triggered the match. Otherwise falls back to the
/// full path as the submatch.
pub fn parse_path_matches(
    output: &str,
    condition: &SearchQueryCondition,
    include: Option<&str>,
    exclude: Option<&str>,
) -> Vec<SearchQueryMatch> {
    let pattern = build_unix_pattern(condition);
    let re = Regex::new(&pattern).ok();
    let include_re = include.and_then(|p| Regex::new(p).ok());
    let exclude_re = exclude.and_then(|p| Regex::new(p).ok());

    output
        .lines()
        .filter(|line| !line.is_empty())
        .filter_map(|line| {
            let path = line.trim();

            // Apply the main search pattern
            if let Some(ref re) = re
                && !re.is_match(path)
            {
                return None;
            }

            // Apply include/exclude filters
            if let Some(ref re) = include_re
                && !re.is_match(path)
            {
                return None;
            }
            if let Some(ref re) = exclude_re
                && re.is_match(path)
            {
                return None;
            }

            let submatches = compute_path_submatches(path, re.as_ref());
            Some(SearchQueryMatch::Path(SearchQueryPathMatch {
                path: RemotePath::new(path),
                submatches,
            }))
        })
        .collect()
}

/// Compute submatches for a path by running the search regex against it.
///
/// If the regex finds a match, the submatch reports the matched substring
/// and its byte offset range. Otherwise, the full path is returned with
/// zero offsets as a fallback.
fn compute_path_submatches(path: &str, re: Option<&Regex>) -> Vec<SearchQuerySubmatch> {
    if let Some(re) = re
        && let Some(m) = re.find(path)
    {
        return vec![SearchQuerySubmatch {
            r#match: SearchQueryMatchData::Text(m.as_str().to_string()),
            start: m.start() as u64,
            end: m.end() as u64,
        }];
    }

    vec![SearchQuerySubmatch {
        r#match: SearchQueryMatchData::Text(path.to_string()),
        start: 0,
        end: 0,
    }]
}
