mod predicates;
mod reader;

use ::predicates::prelude::*;

#[allow(unused_imports)] // Used in cli_tests but not stress_tests
pub use self::predicates::TrimmedLinesMatchPredicate;
#[allow(unused_imports)] // Used in cli_tests but not stress_tests
pub use self::reader::ThreadedReader;

/// Produces a regex predicate using the given string
pub fn regex_pred(s: &str) -> ::predicates::str::RegexPredicate {
    predicate::str::is_match(s).unwrap()
}
