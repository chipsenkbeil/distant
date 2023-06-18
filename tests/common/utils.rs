use ::predicates::prelude::*;

mod predicates;
mod reader;

pub use reader::ThreadedReader;

pub use self::predicates::TrimmedLinesMatchPredicate;

/// Produces a regex predicate using the given string
pub fn regex_pred(s: &str) -> ::predicates::str::RegexPredicate {
    predicate::str::is_match(s).unwrap()
}
