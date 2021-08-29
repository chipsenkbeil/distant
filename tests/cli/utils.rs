use predicates::prelude::*;

lazy_static::lazy_static! {
    /// Predicate that checks for a single line that is a failure
    pub static ref FAILURE_LINE: predicates::str::RegexPredicate =
        predicate::str::is_match(r"^Failed \(.*\): '.*'\.\n$").unwrap();
}

/// Creates a random tenant name
pub fn random_tenant() -> String {
    format!("test-tenant-{}", rand::random::<u16>())
}
