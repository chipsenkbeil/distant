use clap::{Args, ValueEnum};
use distant_core::data::FileType;
use distant_core::data::{SearchQueryOptions, SearchQueryTarget};
use std::collections::HashSet;

pub use distant_core::data::SearchQueryCondition as CliSearchQueryCondition;

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
