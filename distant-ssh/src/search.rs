//! Remote search implementation for SSH connections using best-effort tool detection.
//!
//! Discovers available search tools (`rg`, `grep`, `find`) on the remote host
//! and builds shell commands to execute searches. Supports both path-based and
//! content-based search queries.

use std::io;
use std::sync::Arc;

use distant_core::protocol::{
    RemotePath, SearchQuery, SearchQueryCondition, SearchQueryContentsMatch, SearchQueryMatch,
    SearchQueryMatchData, SearchQueryPathMatch, SearchQuerySubmatch, SearchQueryTarget,
};
use log::*;

use crate::pool;
use crate::utils;

/// Available search tools detected on the remote host.
#[derive(Debug, Clone, Default)]
pub struct SearchTools {
    /// Whether ripgrep is available.
    pub has_rg: bool,

    /// Whether GNU grep is available.
    pub has_grep: bool,

    /// Whether find is available.
    pub has_find: bool,
}

impl SearchTools {
    /// Returns true if at least one search tool is available.
    pub fn has_any(&self) -> bool {
        self.has_rg || self.has_grep || self.has_find
    }
}

/// Probe the remote host for available search tools via SSH exec.
///
/// Uses `which` to check for ripgrep, grep, and find on the remote system.
pub async fn probe_search_tools(pool: &Arc<pool::ChannelPool>) -> SearchTools {
    let mut tools = SearchTools::default();

    if let Ok(output) = probe_tool(pool, "rg").await {
        tools.has_rg = output.success;
    }

    if let Ok(output) = probe_tool(pool, "grep").await {
        tools.has_grep = output.success;
    }

    if let Ok(output) = probe_tool(pool, "find").await {
        tools.has_find = output.success;
    }

    debug!(
        "Search tools: rg={}, grep={}, find={}",
        tools.has_rg, tools.has_grep, tools.has_find
    );

    tools
}

/// Check whether a Unix-compatible search tool is available on the remote host.
///
/// Uses `--version` probes instead of `which` to avoid false positives from
/// incompatible Windows binaries (e.g., Windows `find.exe` is not Unix `find`).
/// For `find`, tests `-maxdepth` support since `--version` is not portable.
async fn probe_tool(pool: &Arc<pool::ChannelPool>, tool: &str) -> io::Result<utils::ExecOutput> {
    let cmd = match tool {
        // Unix find may not support --version (BSD find doesn't), so test a
        // Unix-specific flag. Windows find.exe will fail on this syntax.
        "find" => "find /dev/null -maxdepth 0 2>/dev/null".to_string(),
        _ => format!("{tool} --version 2>/dev/null"),
    };
    let (channel, _permit) = pool.open_exec().await?.take();
    utils::execute_output_on_channel(channel, &cmd, None).await
}

/// Indicates which search tool backs a command.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SearchTool {
    Rg,
    Grep,
    Find,
}

/// Result of building a search command, including the tool used.
pub struct SearchCommand {
    /// The shell command string to execute.
    pub command: String,

    /// The primary search tool used.
    pub tool: SearchTool,
}

impl SearchCommand {
    /// Returns true if the given exit code indicates a real error (not just "no matches").
    ///
    /// For grep/rg, exit code 1 means no matches (not an error), while >= 2 is an error.
    /// For find, any non-zero exit code is an error.
    pub fn is_error_exit(&self, code: u32) -> bool {
        match self.tool {
            SearchTool::Rg | SearchTool::Grep => code >= 2,
            SearchTool::Find => code != 0,
        }
    }
}

/// Escape backslashes in a regex pattern for safe embedding in awk `-v var=value`.
///
/// Awk processes `\` as an escape character in `-v` string assignments,
/// so literal backslashes must be doubled.
fn awk_escape_regex(s: &str) -> String {
    s.replace('\\', "\\\\")
}

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
pub fn build_search_command(query: &SearchQuery, tools: &SearchTools) -> io::Result<SearchCommand> {
    let path = query
        .paths
        .first()
        .map(|p| p.as_str().to_string())
        .unwrap_or_else(|| ".".to_string());

    let pattern = build_unix_pattern(&query.condition);

    // Build include/exclude filter patterns from query options
    let include_pattern = query.options.include.as_ref().map(build_unix_pattern);
    let exclude_pattern = query.options.exclude.as_ref().map(build_unix_pattern);
    let max_depth = query.options.max_depth;

    match query.target {
        SearchQueryTarget::Path => build_path_search_command(
            &path,
            &pattern,
            tools,
            include_pattern.as_deref(),
            exclude_pattern.as_deref(),
            max_depth,
        ),
        SearchQueryTarget::Contents => build_contents_search_command(
            &path,
            &pattern,
            tools,
            include_pattern.as_deref(),
            exclude_pattern.as_deref(),
            max_depth,
        ),
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

/// Shell-quote a string for safe embedding in remote shell commands.
///
/// Uses POSIX-compatible quoting via the `shell-words` crate.
fn shell_quote(s: &str) -> String {
    shell_words::quote(s).into_owned()
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
    path: &str,
    pattern: &str,
    tools: &SearchTools,
    include: Option<&str>,
    exclude: Option<&str>,
    max_depth: Option<u64>,
) -> io::Result<SearchCommand> {
    let quoted_path = shell_quote(path);
    let quoted_pattern = shell_quote(pattern);

    if tools.has_rg {
        let mut cmd = "rg --files".to_string();
        if let Some(depth) = max_depth {
            cmd.push_str(&format!(" --max-depth {depth}"));
        }
        cmd.push_str(&format!(" {quoted_path} | grep -E {quoted_pattern}"));
        // Apply include/exclude as additional grep filters on the path list
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
        let mut cmd = format!("find {quoted_path}");
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
    path: &str,
    pattern: &str,
    tools: &SearchTools,
    include: Option<&str>,
    exclude: Option<&str>,
    max_depth: Option<u64>,
) -> io::Result<SearchCommand> {
    let quoted_path = shell_quote(path);
    let quoted_pattern = shell_quote(pattern);

    // Include/exclude are regex patterns (from SearchQueryCondition), not globs.
    // We filter by piping output through awk, matching the path field (before
    // the first `:` in `filepath:linenum:content` format) against the regex.
    if tools.has_rg {
        let mut cmd = "rg -n".to_string();
        if let Some(depth) = max_depth {
            cmd.push_str(&format!(" --max-depth {depth}"));
        }
        cmd.push_str(&format!(" {quoted_pattern} {quoted_path}"));
        append_path_filters(&mut cmd, include, exclude);
        Ok(SearchCommand {
            command: cmd,
            tool: SearchTool::Rg,
        })
    } else if tools.has_grep {
        // BSD grep (macOS) does not support --max-depth. When max_depth is set,
        // use find with -maxdepth to enumerate files, then grep each one.
        // /dev/null is included so grep always prints file paths even for a
        // single match. Exit code follows find semantics (0 = ok, else error).
        if let Some(depth) = max_depth {
            let mut cmd = format!(
                "find {quoted_path} -maxdepth {depth} -type f \
                 -exec grep -n {quoted_pattern} {{}} /dev/null \\;"
            );
            append_path_filters(&mut cmd, include, exclude);
            Ok(SearchCommand {
                command: cmd,
                tool: SearchTool::Find,
            })
        } else {
            let mut cmd = format!("grep -rn {quoted_pattern} {quoted_path}");
            append_path_filters(&mut cmd, include, exclude);
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

/// Parse grep/rg output lines into content search matches.
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
