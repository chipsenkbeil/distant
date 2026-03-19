//! Remote search implementation for SSH connections using best-effort tool detection.
//!
//! Discovers available search tools (`rg`, `grep`, `find`) on the remote host
//! and builds shell commands to execute searches. Supports both path-based and
//! content-based search queries, including upward directory traversal.

use std::io;
use std::sync::Arc;

use distant_core::protocol::{
    RemotePath, SearchQuery, SearchQueryCondition, SearchQueryContentsMatch, SearchQueryMatch,
    SearchQueryMatchData, SearchQueryPathMatch, SearchQuerySubmatch, SearchQueryTarget,
};
use log::*;
use regex::Regex;
use serde_json::Value;

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

/// Whether rg JSON output mode is used for contents search.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputFormat {
    /// Standard `filepath:linenum:content` format from grep/rg.
    Standard,

    /// JSON lines format from `rg --json`.
    RgJson,

    /// Byte-offset format from `grep -b -n`: `byte_offset:linenum:content`.
    GrepByteOffset,
}

/// Result of building a search command, including the tool used and metadata
/// needed for parsing the output.
pub struct SearchCommand {
    /// The shell command string to execute.
    pub command: String,

    /// The primary search tool used.
    pub tool: SearchTool,

    /// The output format produced by the command.
    pub output_format: OutputFormat,

    /// The search target (path or contents).
    pub target: SearchQueryTarget,

    /// The regex pattern used for the search, needed by parsers to compute
    /// submatch positions.
    pub pattern: String,

    /// Include filter pattern (regex), for rg JSON output filtering.
    pub include: Option<String>,

    /// Exclude filter pattern (regex), for rg JSON output filtering.
    pub exclude: Option<String>,
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

    /// Parse the command's stdout output into search matches, dispatching to the
    /// appropriate parser based on the output format and search target.
    pub fn parse_output(&self, stdout: &str) -> Vec<SearchQueryMatch> {
        match self.target {
            SearchQueryTarget::Path => parse_path_matches(stdout, &self.pattern),
            SearchQueryTarget::Contents => match self.output_format {
                OutputFormat::RgJson => parse_rg_json_contents_matches(
                    stdout,
                    self.include.as_deref(),
                    self.exclude.as_deref(),
                ),
                OutputFormat::GrepByteOffset => parse_grep_contents_matches(stdout, &self.pattern),
                OutputFormat::Standard => parse_contents_matches(stdout, &self.pattern),
            },
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

/// Build one or more shell commands for a search query based on available tools.
///
/// When `options.upward` is true, returns multiple commands (one per ancestor directory).
/// Otherwise returns a single command. All paths from the query are included.
pub fn build_search_commands(
    query: &SearchQuery,
    tools: &SearchTools,
) -> io::Result<Vec<SearchCommand>> {
    let pattern = build_unix_pattern(&query.condition);

    // Build include/exclude filter patterns from query options
    let include_pattern = query.options.include.as_ref().map(build_unix_pattern);
    let exclude_pattern = query.options.exclude.as_ref().map(build_unix_pattern);

    let mut commands = if query.options.upward {
        build_upward_search_commands(
            query,
            &pattern,
            tools,
            include_pattern.as_deref(),
            exclude_pattern.as_deref(),
        )?
    } else {
        // Collect all paths, defaulting to "." if none specified
        let paths: Vec<String> = if query.paths.is_empty() {
            vec![".".to_string()]
        } else {
            query.paths.iter().map(|p| p.as_str().to_string()).collect()
        };

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

        vec![cmd]
    };

    // Stamp metadata needed by parsers onto each command
    for cmd in &mut commands {
        cmd.target = query.target;
        cmd.pattern = pattern.clone();
        cmd.include = include_pattern.clone();
        cmd.exclude = exclude_pattern.clone();
    }

    Ok(commands)
}

/// Build search commands for upward directory traversal.
///
/// Walks parent directories from each search path upward toward root. At each
/// ancestor directory, searches with depth=1 (only that directory level).
/// `max_depth` limits how many ancestor directories to walk.
fn build_upward_search_commands(
    query: &SearchQuery,
    pattern: &str,
    tools: &SearchTools,
    include: Option<&str>,
    exclude: Option<&str>,
) -> io::Result<Vec<SearchCommand>> {
    let paths: Vec<String> = if query.paths.is_empty() {
        vec![".".to_string()]
    } else {
        query.paths.iter().map(|p| p.as_str().to_string()).collect()
    };

    let mut commands = Vec::new();

    // Collect all directories to search (the starting dir and its ancestors)
    let mut all_dirs: Vec<String> = Vec::new();
    for path in &paths {
        // Always include the starting directory itself with depth=1.
        // This matches the host implementation: upward search always searches
        // the contents of the starting directory.
        all_dirs.push(path.clone());

        // Walk parent directories, limited by max_depth
        let mut remaining = query.options.max_depth;
        if query.options.max_depth.is_none() || query.options.max_depth > Some(0) {
            let mut current = path.as_str();
            while let Some(parent) = parent_path(current) {
                if remaining == Some(0) {
                    break;
                }

                all_dirs.push(parent.to_string());
                current = parent;

                if let Some(ref mut r) = remaining {
                    *r -= 1;
                }
            }
        }
    }

    // Deduplicate directories
    all_dirs.sort();
    all_dirs.dedup();

    // Build one command per directory with depth=1
    for dir in &all_dirs {
        let cmd = match query.target {
            SearchQueryTarget::Path => build_path_search_command(
                std::slice::from_ref(dir),
                pattern,
                tools,
                include,
                exclude,
                Some(1),
            )?,
            SearchQueryTarget::Contents => build_contents_search_command(
                std::slice::from_ref(dir),
                pattern,
                tools,
                include,
                exclude,
                Some(1),
            )?,
        };
        commands.push(cmd);
    }

    Ok(commands)
}

/// Extract the parent path from a Unix path string.
///
/// Returns `None` for root (`/`) or paths with no parent component.
fn parent_path(path: &str) -> Option<&str> {
    let trimmed = path.trim_end_matches('/');
    if trimmed.is_empty() {
        return None;
    }

    match trimmed.rfind('/') {
        Some(0) => {
            // Parent is root "/"
            if trimmed == "/" { None } else { Some("/") }
        }
        Some(idx) => Some(&trimmed[..idx]),
        None => None,
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

/// Join multiple paths into a space-separated shell-quoted string.
fn quote_paths(paths: &[String]) -> String {
    paths
        .iter()
        .map(|p| shell_quote(p))
        .collect::<Vec<_>>()
        .join(" ")
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
    paths: &[String],
    pattern: &str,
    tools: &SearchTools,
    include: Option<&str>,
    exclude: Option<&str>,
    max_depth: Option<u64>,
) -> io::Result<SearchCommand> {
    let quoted_paths = quote_paths(paths);
    let quoted_pattern = shell_quote(pattern);

    if tools.has_rg {
        let mut cmd = "rg --files".to_string();
        if let Some(depth) = max_depth {
            cmd.push_str(&format!(" --max-depth {depth}"));
        }
        cmd.push_str(&format!(" {quoted_paths} | grep -E {quoted_pattern}"));

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
            output_format: OutputFormat::Standard,
            target: SearchQueryTarget::Path,
            pattern: String::new(),
            include: None,
            exclude: None,
        })
    } else if tools.has_find {
        let mut cmd = format!("find {quoted_paths}");
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
            output_format: OutputFormat::Standard,
            target: SearchQueryTarget::Path,
            pattern: String::new(),
            include: None,
            exclude: None,
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
    paths: &[String],
    pattern: &str,
    tools: &SearchTools,
    include: Option<&str>,
    exclude: Option<&str>,
    max_depth: Option<u64>,
) -> io::Result<SearchCommand> {
    let quoted_paths = quote_paths(paths);
    let quoted_pattern = shell_quote(pattern);

    // Include/exclude are regex patterns (from SearchQueryCondition), not globs.
    // We filter by piping output through awk, matching the path field (before
    // the first `:` in `filepath:linenum:content` format) against the regex.
    if tools.has_rg {
        // Use --json for structured output with byte offsets and submatches
        let mut cmd = "rg --json".to_string();
        if let Some(depth) = max_depth {
            cmd.push_str(&format!(" --max-depth {depth}"));
        }
        cmd.push_str(&format!(" {quoted_pattern} {quoted_paths}"));

        // Note: awk-based path filters are not compatible with JSON output.
        // Include/exclude filtering is applied in parse_rg_json_contents_matches
        // by checking the path field of each match against the filter patterns.
        Ok(SearchCommand {
            command: cmd,
            tool: SearchTool::Rg,
            output_format: OutputFormat::RgJson,
            target: SearchQueryTarget::Contents,
            pattern: String::new(),
            include: include.map(String::from),
            exclude: exclude.map(String::from),
        })
    } else if tools.has_grep {
        // BSD grep (macOS) does not support --max-depth. When max_depth is set,
        // use find with -maxdepth to enumerate files, then grep each one.
        // /dev/null is included so grep always prints file paths even for a
        // single match. Exit code follows find semantics (0 = ok, else error).
        if let Some(depth) = max_depth {
            let mut cmd = format!(
                "find {quoted_paths} -maxdepth {depth} -type f \
                 -exec grep -b -n {quoted_pattern} {{}} /dev/null \\;"
            );
            append_path_filters(&mut cmd, include, exclude);
            Ok(SearchCommand {
                command: cmd,
                tool: SearchTool::Find,
                output_format: OutputFormat::GrepByteOffset,
                target: SearchQueryTarget::Contents,
                pattern: String::new(),
                include: None,
                exclude: None,
            })
        } else {
            let mut cmd = format!("grep -rbn {quoted_pattern} {quoted_paths}");
            append_path_filters(&mut cmd, include, exclude);
            Ok(SearchCommand {
                command: cmd,
                tool: SearchTool::Grep,
                output_format: OutputFormat::GrepByteOffset,
                target: SearchQueryTarget::Contents,
                pattern: String::new(),
                include: None,
                exclude: None,
            })
        }
    } else {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "No search tools available (need rg or grep)",
        ))
    }
}

/// Parse `rg --json` output lines into content search matches.
///
/// The JSON format has lines with `"type":"match"` containing structured match
/// data including byte offsets and submatch positions. Other line types (begin,
/// end, summary) are ignored.
///
/// When `include` or `exclude` patterns are provided, paths are filtered against
/// them since awk-based pipe filters are incompatible with JSON output.
pub fn parse_rg_json_contents_matches(
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

        if parsed.get("type").and_then(|t| t.as_str()) != Some("match") {
            continue;
        }

        let Some(data) = parsed.get("data") else {
            continue;
        };

        // Extract the file path
        let filepath = data
            .get("path")
            .and_then(|p| p.get("text"))
            .and_then(|t| t.as_str())
            .unwrap_or("");

        // Apply include/exclude filters on the path
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

        let line_number = data
            .get("line_number")
            .and_then(|n| n.as_u64())
            .unwrap_or(0);

        let absolute_offset = data
            .get("absolute_offset")
            .and_then(|n| n.as_u64())
            .unwrap_or(0);

        let line_text = data
            .get("lines")
            .and_then(|l| l.get("text"))
            .and_then(|t| t.as_str())
            .unwrap_or("");

        // Strip trailing newline from the line text, as rg includes it
        let line_text = line_text.strip_suffix('\n').unwrap_or(line_text);

        // Extract submatches from the structured data
        let submatches = if let Some(subs) = data.get("submatches").and_then(|s| s.as_array()) {
            subs.iter()
                .filter_map(|sub| {
                    let match_text = sub
                        .get("match")
                        .and_then(|m| m.get("text"))
                        .and_then(|t| t.as_str())?;
                    let start = sub.get("start").and_then(|s| s.as_u64()).unwrap_or(0);
                    let end = sub.get("end").and_then(|e| e.as_u64()).unwrap_or(0);
                    Some(SearchQuerySubmatch {
                        r#match: SearchQueryMatchData::Text(match_text.to_string()),
                        start,
                        end,
                    })
                })
                .collect()
        } else {
            vec![SearchQuerySubmatch {
                r#match: SearchQueryMatchData::Text(line_text.to_string()),
                start: 0,
                end: 0,
            }]
        };

        matches.push(SearchQueryMatch::Contents(SearchQueryContentsMatch {
            path: RemotePath::new(filepath),
            lines: SearchQueryMatchData::Text(line_text.to_string()),
            line_number,
            absolute_offset,
            submatches,
        }));
    }

    matches
}

/// Parse grep `-b -n` output lines into content search matches.
///
/// Expected format: `filepath:byte_offset:linenum:matched_line`
///
/// The `pattern` parameter is used to compute submatch positions within each
/// matching line by running the regex against the line text.
pub fn parse_grep_contents_matches(output: &str, pattern: &str) -> Vec<SearchQueryMatch> {
    let re = Regex::new(pattern).ok();
    let mut matches = Vec::new();

    for line in output.lines() {
        if line.is_empty() {
            continue;
        }

        // Format: filepath:byte_offset:linenum:content
        let parts: Vec<&str> = line.splitn(4, ':').collect();

        if parts.len() < 4 {
            continue;
        }

        let filepath = parts[0];
        let byte_offset_str = parts[1];
        let line_num_str = parts[2];
        let content = parts[3];

        let byte_offset = byte_offset_str.parse::<u64>().unwrap_or(0);
        let Ok(line_num) = line_num_str.parse::<u64>() else {
            continue;
        };

        let submatches = compute_submatches_for_line(content, re.as_ref());
        let content = content.to_string();

        matches.push(SearchQueryMatch::Contents(SearchQueryContentsMatch {
            path: RemotePath::new(filepath),
            lines: SearchQueryMatchData::Text(content),
            line_number: line_num,
            absolute_offset: byte_offset,
            submatches,
        }));
    }

    matches
}

/// Parse standard `filepath:linenum:content` output into content search matches.
///
/// Used as a fallback when neither rg JSON nor grep byte-offset output is available.
/// The `pattern` parameter computes submatch positions within each matching line.
pub fn parse_contents_matches(output: &str, pattern: &str) -> Vec<SearchQueryMatch> {
    let re = Regex::new(pattern).ok();
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
            let submatches = compute_submatches_for_line(content, re.as_ref());
            let content = content.to_string();
            matches.push(SearchQueryMatch::Contents(SearchQueryContentsMatch {
                path: RemotePath::new(&filepath),
                lines: SearchQueryMatchData::Text(content),
                line_number: line_num,
                absolute_offset: 0,
                submatches,
            }));
        }
    }

    matches
}

/// Compute submatch positions by running a regex against line text.
///
/// Returns all non-overlapping matches with their byte offsets relative to the
/// line start. If the regex is `None` or produces no matches, returns a single
/// submatch covering the entire line text.
fn compute_submatches_for_line(line_text: &str, re: Option<&Regex>) -> Vec<SearchQuerySubmatch> {
    if let Some(re) = re {
        let subs: Vec<SearchQuerySubmatch> = re
            .find_iter(line_text)
            .map(|m| SearchQuerySubmatch {
                r#match: SearchQueryMatchData::Text(m.as_str().to_string()),
                start: m.start() as u64,
                end: m.end() as u64,
            })
            .collect();

        if !subs.is_empty() {
            return subs;
        }
    }

    // Fallback: single submatch for the whole line
    vec![SearchQuerySubmatch {
        r#match: SearchQueryMatchData::Text(line_text.to_string()),
        start: 0,
        end: line_text.len() as u64,
    }]
}

/// Parse path search output into search matches.
///
/// The `pattern` parameter is used to compute submatch positions by finding where
/// the search pattern matches within each path string.
pub fn parse_path_matches(output: &str, pattern: &str) -> Vec<SearchQueryMatch> {
    let re = Regex::new(pattern).ok();

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

/// Compute submatch positions within a path string.
///
/// Finds all non-overlapping regex matches within the path and returns their
/// byte offsets. Falls back to a zero-length submatch if no regex or no matches.
fn compute_path_submatches(path: &str, re: Option<&Regex>) -> Vec<SearchQuerySubmatch> {
    if let Some(re) = re {
        let subs: Vec<SearchQuerySubmatch> = re
            .find_iter(path)
            .map(|m| SearchQuerySubmatch {
                r#match: SearchQueryMatchData::Text(m.as_str().to_string()),
                start: m.start() as u64,
                end: m.end() as u64,
            })
            .collect();

        if !subs.is_empty() {
            return subs;
        }
    }

    // Fallback when no regex or no matches found
    vec![SearchQuerySubmatch {
        r#match: SearchQueryMatchData::Text(path.to_string()),
        start: 0,
        end: 0,
    }]
}
