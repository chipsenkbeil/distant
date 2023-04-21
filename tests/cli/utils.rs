use predicates::prelude::*;

mod reader;
pub use reader::ThreadedReader;

/// Produces a regex predicate using the given string
pub fn regex_pred(s: &str) -> predicates::str::RegexPredicate {
    predicate::str::is_match(s).unwrap()
}

/// Produces a contains predicate to assert a path missing error was reported
pub fn missing_path_pred() -> predicates::str::ContainsPredicate {
    if cfg!(unix) {
        predicate::str::contains("No such file or directory")
    } else if cfg!(windows) {
        predicate::str::contains("The system cannot find the path specified")
    } else {
        unreachable!("Only other option is wasm, which is not supported")
    }
}

/// Produces a contains predicate to assert a directory was not empty
pub fn directory_not_empty_pred() -> predicates::str::ContainsPredicate {
    if cfg!(unix) {
        predicate::str::contains("Directory not empty")
    } else if cfg!(windows) {
        predicate::str::contains("The directory is not empty")
    } else {
        unreachable!("Only other option is wasm, which is not supported")
    }
}
