use std::collections::HashSet;

use clap::{Args, ValueEnum};
pub use distant_core::protocol::SearchQueryCondition as CliSearchQueryCondition;
use distant_core::protocol::{FileType, SearchQueryOptions, SearchQueryTarget};

/// Options to customize the search results.
#[derive(Args, Clone, Debug, Default, PartialEq, Eq)]
pub struct CliSearchQueryOptions {
    /// Restrict search to only these file types (otherwise all are allowed)
    #[clap(skip)]
    pub allowed_file_types: HashSet<FileType>,

    /// Regex to use to filter paths being searched to only those that match the include condition
    #[clap(long)]
    pub include: Option<CliSearchQueryCondition>,

    /// Regex to use to filter paths being searched to only those that do not match the exclude
    /// condition
    #[clap(long)]
    pub exclude: Option<CliSearchQueryCondition>,

    /// Search upward through parent directories rather than the traditional downward search that
    /// recurses through all children directories.
    ///
    /// Note that this will use maximum depth to apply to the reverse direction, and will only look
    /// through each ancestor directory's immediate entries. In other words, this will not result
    /// in recursing through sibling directories.
    ///
    /// An upward search will ALWAYS search the contents of a directory, so this means providing a
    /// path to a directory will search its entries EVEN if the max_depth is 0.
    #[clap(long)]
    pub upward: bool,

    /// Search should follow symbolic links
    #[clap(long)]
    pub follow_symbolic_links: bool,

    /// Maximum results to return before stopping the query
    #[clap(long)]
    pub limit: Option<u64>,

    /// Maximum depth (directories) to search
    ///
    /// The smallest depth is 0 and always corresponds to the path given to the new function on
    /// this type. Its direct descendents have depth 1, and their descendents have depth 2, and so
    /// on.
    ///
    /// Note that this will not simply filter the entries of the iterator, but it will actually
    /// avoid descending into directories when the depth is exceeded.
    #[clap(long)]
    pub max_depth: Option<u64>,

    /// Amount of results to batch before sending back excluding final submission that will always
    /// include the remaining results even if less than pagination request
    #[clap(long)]
    pub pagination: Option<u64>,

    /// If true, will skip searching hidden files.
    #[clap(long)]
    pub ignore_hidden: bool,

    /// If true, will read `.ignore` files that are used by `ripgrep` and `The Silver Searcher`
    /// to determine which files and directories to not search.
    #[clap(long)]
    pub use_ignore_files: bool,

    /// If true, will read `.ignore` files from parent directories that are used by `ripgrep` and
    /// `The Silver Searcher` to determine which files and directories to not search.
    #[clap(long)]
    pub use_parent_ignore_files: bool,

    /// If true, will read `.gitignore` files to determine which files and directories to not
    /// search.
    #[clap(long)]
    pub use_git_ignore_files: bool,

    /// If true, will read global `.gitignore` files to determine which files and directories to
    /// not search.
    #[clap(long)]
    pub use_global_git_ignore_files: bool,

    /// If true, will read `.git/info/exclude` files to determine which files and directories to
    /// not search.
    #[clap(long)]
    pub use_git_exclude_files: bool,
}

impl From<CliSearchQueryOptions> for SearchQueryOptions {
    fn from(x: CliSearchQueryOptions) -> Self {
        Self {
            allowed_file_types: x.allowed_file_types,
            include: x.include,
            exclude: x.exclude,
            upward: x.upward,
            follow_symbolic_links: x.follow_symbolic_links,
            limit: x.limit,
            max_depth: x.max_depth,
            pagination: x.pagination,
            ignore_hidden: x.ignore_hidden,
            use_ignore_files: x.use_ignore_files,
            use_parent_ignore_files: x.use_parent_ignore_files,
            use_git_ignore_files: x.use_git_ignore_files,
            use_global_git_ignore_files: x.use_global_git_ignore_files,
            use_git_exclude_files: x.use_git_exclude_files,
        }
    }
}

/// Kind of data to examine using conditions
#[derive(Copy, Clone, Debug, PartialEq, Eq, ValueEnum)]
#[clap(rename_all = "snake_case")]
pub enum CliSearchQueryTarget {
    /// Checks path of file, directory, or symlink
    Path,

    /// Checks contents of files
    Contents,
}

impl From<CliSearchQueryTarget> for SearchQueryTarget {
    fn from(x: CliSearchQueryTarget) -> Self {
        match x {
            CliSearchQueryTarget::Contents => Self::Contents,
            CliSearchQueryTarget::Path => Self::Path,
        }
    }
}

#[cfg(test)]
mod tests {
    use test_log::test;

    use super::*;

    // -------------------------------------------------------
    // CliSearchQueryTarget -> SearchQueryTarget
    // -------------------------------------------------------
    #[test]
    fn cli_search_query_target_contents_converts() {
        let target: SearchQueryTarget = CliSearchQueryTarget::Contents.into();
        assert_eq!(target, SearchQueryTarget::Contents);
    }

    #[test]
    fn cli_search_query_target_path_converts() {
        let target: SearchQueryTarget = CliSearchQueryTarget::Path.into();
        assert_eq!(target, SearchQueryTarget::Path);
    }

    // -------------------------------------------------------
    // CliSearchQueryOptions -> SearchQueryOptions
    // -------------------------------------------------------
    #[test]
    fn cli_search_query_options_default_converts() {
        let opts = CliSearchQueryOptions::default();
        let converted: SearchQueryOptions = opts.into();
        assert!(converted.allowed_file_types.is_empty());
        assert!(converted.include.is_none());
        assert!(converted.exclude.is_none());
        assert!(!converted.upward);
        assert!(!converted.follow_symbolic_links);
        assert!(converted.limit.is_none());
        assert!(converted.max_depth.is_none());
        assert!(converted.pagination.is_none());
        assert!(!converted.ignore_hidden);
        assert!(!converted.use_ignore_files);
        assert!(!converted.use_parent_ignore_files);
        assert!(!converted.use_git_ignore_files);
        assert!(!converted.use_global_git_ignore_files);
        assert!(!converted.use_git_exclude_files);
    }

    #[test]
    fn cli_search_query_options_with_values_converts() {
        let opts = CliSearchQueryOptions {
            allowed_file_types: {
                let mut s = HashSet::new();
                s.insert(FileType::File);
                s
            },
            include: Some(CliSearchQueryCondition::regex("*.rs")),
            exclude: Some(CliSearchQueryCondition::regex("target")),
            upward: true,
            follow_symbolic_links: true,
            limit: Some(100),
            max_depth: Some(5),
            pagination: Some(10),
            ignore_hidden: true,
            use_ignore_files: true,
            use_parent_ignore_files: true,
            use_git_ignore_files: true,
            use_global_git_ignore_files: true,
            use_git_exclude_files: true,
        };
        let converted: SearchQueryOptions = opts.into();
        assert_eq!(converted.allowed_file_types.len(), 1);
        assert!(converted.include.is_some());
        assert!(converted.exclude.is_some());
        assert!(converted.upward);
        assert!(converted.follow_symbolic_links);
        assert_eq!(converted.limit, Some(100));
        assert_eq!(converted.max_depth, Some(5));
        assert_eq!(converted.pagination, Some(10));
        assert!(converted.ignore_hidden);
        assert!(converted.use_ignore_files);
        assert!(converted.use_parent_ignore_files);
        assert!(converted.use_git_ignore_files);
        assert!(converted.use_global_git_ignore_files);
        assert!(converted.use_git_exclude_files);
    }
}
