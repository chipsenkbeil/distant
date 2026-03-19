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

/// Escape backslashes in a regex pattern for safe embedding in awk `-v var=value`.
///
/// Awk processes `\` as an escape character in `-v` string assignments,
/// so literal backslashes must be doubled.
fn awk_escape_regex(s: &str) -> String {
    s.replace('\\', "\\\\")
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
    /// ripgrep (rg) with plain text output (used for path searches).
    Rg,

    /// ripgrep with JSON output for structured parsing (used for contents searches).
    RgJson,

    /// GNU grep with byte-offset output (`-b -n`).
    GrepByteOffset,

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
            SearchTool::Rg | SearchTool::RgJson | SearchTool::GrepByteOffset => code >= 2,
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
        let mut current = path.as_str();
        let mut remaining = query.options.max_depth;

        // If max_depth is Some(0), do not traverse any ancestors
        if remaining == Some(0) {
            continue;
        }

        loop {
            let parent = unix_parent(current);
            if parent == current {
                break;
            }

            if let Some(ref mut rem) = remaining {
                if *rem == 0 {
                    break;
                }
                *rem -= 1;
            }

            let quoted_parent = shell_quote(parent);
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
fn unix_parent(path: &str) -> &str {
    if path == "/" || path == "." {
        return path;
    }

    let trimmed = path.trim_end_matches('/');
    match trimmed.rfind('/') {
        Some(0) => "/",
        Some(pos) => &trimmed[..pos],
        None => ".",
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

/// Append awk-based path filters for include/exclude on contents search output.
///
/// Contents search output has the format `filepath:linenum:content`. The awk
/// filter splits on `:` and matches the first field (the file path) against
/// the regex pattern. This correctly handles regex patterns (including anchors
/// like `$` and `^`) unlike glob-based `--glob` or `--include` flags.
fn append_path_filters(cmd: &mut String, include: Option<&str>, exclude: Option<&str>) {
    if let Some(inc) = include {
        let escaped = awk_escape_regex(inc);
        cmd.push_str(&format!(
            " | awk -F: -v pat={} '$1 ~ pat'",
            shell_quote(&escaped)
        ));
    }
    if let Some(exc) = exclude {
        let escaped = awk_escape_regex(exc);
        cmd.push_str(&format!(
            " | awk -F: -v pat={} '$1 !~ pat'",
            shell_quote(&escaped)
        ));
    }
}

/// Build a path search command using rg or find.
fn build_path_search_command(
    paths: &str,
    pattern: &str,
    tools: &SearchTools,
    include: Option<&str>,
    exclude: Option<&str>,
    max_depth: Option<u64>,
) -> io::Result<SearchCommand> {
    let quoted_pattern = shell_quote(pattern);

    if tools.has_rg {
        let mut cmd = "rg --files".to_string();
        if let Some(depth) = max_depth {
            cmd.push_str(&format!(" --max-depth {depth}"));
        }
        cmd.push_str(&format!(" {paths} | grep -E {quoted_pattern}"));
        if let Some(inc) = include {
            cmd.push_str(&format!(" | grep -E {}", shell_quote(inc)));
        }
        if let Some(exc) = exclude {
            cmd.push_str(&format!(" | grep -vE {}", shell_quote(exc)));
        }
        Ok(SearchCommand {
            command: cmd,
            tool: SearchTool::Rg,
        })
    } else if tools.has_find {
        let mut cmd = format!("find {paths}");
        if let Some(depth) = max_depth {
            cmd.push_str(&format!(" -maxdepth {depth}"));
        }
        cmd.push_str(&format!(" -regex '.*{pattern}.*' -print"));
        if let Some(inc) = include {
            cmd.push_str(&format!(" | grep -E {}", shell_quote(inc)));
        }
        if let Some(exc) = exclude {
            cmd.push_str(&format!(" | grep -vE {}", shell_quote(exc)));
        }
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
fn build_contents_search_command(
    paths: &str,
    pattern: &str,
    tools: &SearchTools,
    include: Option<&str>,
    exclude: Option<&str>,
    max_depth: Option<u64>,
) -> io::Result<SearchCommand> {
    let quoted_pattern = shell_quote(pattern);

    if tools.has_rg {
        // Use rg --json for structured output with byte offsets and submatches.
        // Note: awk-based path filters are not applied here because rg JSON
        // output is not in `filepath:linenum:content` format. Include/exclude
        // filtering is handled during parsing in `parse_rg_json_contents`.
        let mut cmd = "rg --json".to_string();
        if let Some(depth) = max_depth {
            cmd.push_str(&format!(" --max-depth {depth}"));
        }
        cmd.push_str(&format!(" {quoted_pattern} {paths}"));
        Ok(SearchCommand {
            command: cmd,
            tool: SearchTool::RgJson,
        })
    } else if tools.has_grep {
        // BSD grep (macOS) does not support --max-depth. When max_depth is set,
        // use find with -maxdepth to enumerate files, then grep each one.
        // /dev/null is included so grep always prints file paths even for a
        // single match. Exit code follows find semantics (0 = ok, else error).
        if let Some(depth) = max_depth {
            let mut cmd = format!(
                "find {paths} -maxdepth {depth} -type f \
                 -exec grep -n {quoted_pattern} {{}} /dev/null \\;"
            );
            append_path_filters(&mut cmd, include, exclude);
            Ok(SearchCommand {
                command: cmd,
                tool: SearchTool::Find,
            })
        } else {
            let mut cmd = format!("grep -rbn {quoted_pattern} {paths}");
            append_path_filters(&mut cmd, include, exclude);
            Ok(SearchCommand {
                command: cmd,
                tool: SearchTool::GrepByteOffset,
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
/// Dispatches to the appropriate parser for rg JSON, grep byte-offset, or
/// legacy `filepath:linenum:content` formats. The `include` and `exclude`
/// patterns are only used for rg JSON output (other formats apply these
/// filters via shell pipes during command execution).
pub fn parse_contents_matches(
    output: &str,
    tool: SearchTool,
    include: Option<&str>,
    exclude: Option<&str>,
) -> Vec<SearchQueryMatch> {
    match tool {
        SearchTool::RgJson => parse_rg_json_contents(output, include, exclude),
        SearchTool::GrepByteOffset => parse_grep_byte_offset_contents(output),
        _ => parse_legacy_contents(output),
    }
}

/// Parse rg `--json` output into content search matches.
///
/// Each JSON line with `type: "match"` contains structured data including
/// file path, line number, absolute byte offset, and submatch positions.
/// Include/exclude regex patterns are applied as post-filters on the file
/// path since rg JSON output cannot be piped through awk.
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
/// The `-b` flag adds a byte offset field before the line number.
fn parse_grep_byte_offset_contents(output: &str) -> Vec<SearchQueryMatch> {
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

/// Parse legacy `filepath:linenum:content` format into content search matches.
///
/// Used for tools that don't provide byte offsets (plain `grep -rn` or `rg -n`).
fn parse_legacy_contents(output: &str) -> Vec<SearchQueryMatch> {
    let mut matches = Vec::new();

    for line in output.lines() {
        if line.is_empty() {
            continue;
        }

        let parts: Vec<&str> = line.splitn(3, ':').collect();
        if parts.len() < 3 {
            continue;
        }

        let filepath = parts[0];
        let line_num_str = parts[1];
        let content = parts[2];

        let line_num = match line_num_str.parse::<u64>() {
            Ok(n) => n,
            Err(_) => continue,
        };

        let content = content.to_string();
        matches.push(SearchQueryMatch::Contents(SearchQueryContentsMatch {
            path: RemotePath::new(filepath),
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
pub fn parse_path_matches(output: &str, condition: &SearchQueryCondition) -> Vec<SearchQueryMatch> {
    let pattern = build_unix_pattern(condition);
    let re = Regex::new(&pattern).ok();

    output
        .lines()
        .filter(|line| !line.is_empty())
        .map(|line| {
            let path = line.trim();
            let submatches = compute_path_submatches(path, re.as_ref());
            SearchQueryMatch::Path(SearchQueryPathMatch {
                path: RemotePath::new(path),
                submatches,
            })
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
