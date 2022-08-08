use once_cell::sync::Lazy;
use predicates::prelude::*;

mod reader;
pub use reader::ThreadedReader;

/// Predicate that checks for a single line that is a failure
pub static FAILURE_LINE: Lazy<predicates::str::RegexPredicate> =
    Lazy::new(|| regex_pred(r"^.*\n$"));

/// Produces a regex predicate using the given string
pub fn regex_pred(s: &str) -> predicates::str::RegexPredicate {
    predicate::str::is_match(s).unwrap()
}
