use ::predicates::prelude::*;

mod predicates;
mod reader;

pub use self::predicates::TrimmedLinesMatchPredicate;
pub use reader::ThreadedReader;

/// Produces a regex predicate using the given string
pub fn regex_pred(s: &str) -> ::predicates::str::RegexPredicate {
    predicate::str::is_match(s).unwrap()
}
