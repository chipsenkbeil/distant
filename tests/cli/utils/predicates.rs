use predicates::reflection::PredicateReflection;
use predicates::Predicate;
use std::fmt;

/// Checks if lines of text match the provided, trimming each line
/// of both before comparing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TrimmedLinesMatchPredicate {
    pattern: String,
}

impl TrimmedLinesMatchPredicate {
    pub fn new(pattern: impl Into<String>) -> Self {
        Self {
            pattern: pattern.into(),
        }
    }
}

impl fmt::Display for TrimmedLinesMatchPredicate {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "trimmed_lines expects {}", self.pattern)
    }
}

impl Predicate<str> for TrimmedLinesMatchPredicate {
    fn eval(&self, variable: &str) -> bool {
        let mut expected = self.pattern.lines();
        let mut actual = variable.lines();

        // Fail if we don't have the same number of lines
        // or of the trimmed result of lines don't match
        //
        // Otherwise if we finish processing all lines,
        // we are a success
        loop {
            match (expected.next(), actual.next()) {
                (Some(expected), Some(actual)) => {
                    if expected.trim() != actual.trim() {
                        return false;
                    }
                }
                (None, None) => return true,
                _ => return false,
            }
        }
    }
}

impl PredicateReflection for TrimmedLinesMatchPredicate {}
