use predicates::prelude::*;

lazy_static::lazy_static! {
    /// Predicate that checks for a single line that is a failure
    pub static ref FAILURE_LINE: predicates::str::RegexPredicate =
        regex_pred(r"^Failed \(.*\): '.*'\.\n$");
}

/// Produces a regex predicate using the given string
pub fn regex_pred(s: &str) -> predicates::str::RegexPredicate {
    predicate::str::is_match(s).unwrap()
}

/// Creates a random tenant name
pub fn random_tenant() -> String {
    format!("test-tenant-{}", rand::random::<u16>())
}
