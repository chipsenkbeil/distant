use std::fmt;
use std::str::FromStr;

use crate::SshParserError;

const ID_APPEND: char = '+';
const ID_HEAD: char = '^';
const ID_EXCLUDE: char = '-';

/// List of algorithms to be used.
/// The algorithms can be appended to the default set, placed at the head of the list,
/// excluded from the default set, or set as the default set.
///
/// # Configuring SSH Algorithms
///
/// In order to configure ssh you should use the `to_string()` method to get the string representation
/// with the correct format for ssh2.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Algorithms {
    /// Algorithms to be used.
    algos: Vec<String>,
    /// whether the default algorithms have been overridden
    overridden: bool,
    /// applied rule
    rule: Option<AlgorithmsRule>,
}

impl Algorithms {
    /// Create a new instance of [`Algorithms`] with the given default algorithms.
    ///
    /// ## Example
    ///
    /// ```rust
    /// use ssh2_config::Algorithms;
    ///
    /// let algos = Algorithms::new(&["aes128-ctr", "aes192-ctr"]);
    /// ```
    pub fn new<I, S>(default: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        Self {
            algos: default
                .into_iter()
                .map(|s| s.as_ref().to_string())
                .collect(),
            overridden: false,
            rule: None,
        }
    }
}

/// List of algorithms to be used.
/// The algorithms can be appended to the default set, placed at the head of the list,
/// excluded from the default set, or set as the default set.
///
/// # Configuring SSH Algorithms
///
/// In order to configure ssh you should use the `to_string()` method to get the string representation
/// with the correct format for ssh2.
///
/// # Algorithms vector
///
/// Otherwise you can access the inner [`Vec`] of algorithms with the [`Algorithms::algos`] method.
///
/// Beware though, that you must **TAKE CARE of the current variant**.
///
/// For instance in case the variant is [`Algorithms::Exclude`] the algos contained in the vec are the ones **to be excluded**.
///
/// While in case of [`Algorithms::Append`] the algos contained in the vec are the ones to be appended to the default ones.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AlgorithmsRule {
    /// Append the given algorithms to the default set.
    Append(Vec<String>),
    /// Place the given algorithms at the head of the list.
    Head(Vec<String>),
    /// Exclude the given algorithms from the default set.
    Exclude(Vec<String>),
    /// Set the given algorithms as the default set.
    Set(Vec<String>),
}

/// Rule applied; used to format algorithms
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AlgorithmsOp {
    Append,
    Head,
    Exclude,
    Set,
}

impl Algorithms {
    /// Returns whether the default algorithms are being used.
    pub fn is_default(&self) -> bool {
        !self.overridden
    }

    /// Returns algorithms to be used.
    pub fn algorithms(&self) -> &[String] {
        &self.algos
    }

    /// Apply an `AlgorithmsRule` to the [`Algorithms`] instance.
    ///
    /// If defaults haven't been overridden, apply changes from incoming rule;
    /// otherwise keep as-is.
    pub fn apply(&mut self, rule: AlgorithmsRule) {
        if self.overridden {
            // don't apply changes if defaults have been overridden
            return;
        }

        let mut current_algos = self.algos.clone();

        match rule.clone() {
            AlgorithmsRule::Append(algos) => {
                // append but exclude duplicates
                for algo in algos {
                    if !current_algos.iter().any(|s| s == &algo) {
                        current_algos.push(algo);
                    }
                }
            }
            AlgorithmsRule::Head(algos) => {
                current_algos = algos;
                current_algos.extend(self.algorithms().iter().map(|s| s.to_string()));
            }
            AlgorithmsRule::Exclude(exclude) => {
                current_algos = current_algos
                    .iter()
                    .filter(|algo| !exclude.contains(algo))
                    .map(|s| s.to_string())
                    .collect();
            }
            AlgorithmsRule::Set(algos) => {
                // override default with new set
                current_algos = algos;
            }
        }

        // apply changes
        self.rule = Some(rule);
        self.algos = current_algos;
        self.overridden = true;
    }
}

impl AlgorithmsRule {
    fn op(&self) -> AlgorithmsOp {
        match self {
            Self::Append(_) => AlgorithmsOp::Append,
            Self::Head(_) => AlgorithmsOp::Head,
            Self::Exclude(_) => AlgorithmsOp::Exclude,
            Self::Set(_) => AlgorithmsOp::Set,
        }
    }
}

impl FromStr for AlgorithmsRule {
    type Err = SshParserError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.is_empty() {
            return Err(SshParserError::ExpectedAlgorithms);
        }

        // get first char
        let (op, start) = match s.chars().next().expect("can't be empty") {
            ID_APPEND => (AlgorithmsOp::Append, 1),
            ID_HEAD => (AlgorithmsOp::Head, 1),
            ID_EXCLUDE => (AlgorithmsOp::Exclude, 1),
            _ => (AlgorithmsOp::Set, 0),
        };

        let algos = s[start..]
            .split(',')
            .map(|s| s.trim().to_string())
            .collect::<Vec<String>>();

        match op {
            AlgorithmsOp::Append => Ok(Self::Append(algos)),
            AlgorithmsOp::Head => Ok(Self::Head(algos)),
            AlgorithmsOp::Exclude => Ok(Self::Exclude(algos)),
            AlgorithmsOp::Set => Ok(Self::Set(algos)),
        }
    }
}

impl fmt::Display for AlgorithmsRule {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let op = self.op();
        write!(f, "{op}")
    }
}

impl fmt::Display for AlgorithmsOp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self {
            Self::Append => write!(f, "{ID_APPEND}"),
            Self::Head => write!(f, "{ID_HEAD}"),
            Self::Exclude => write!(f, "{ID_EXCLUDE}"),
            Self::Set => write!(f, ""),
        }
    }
}

impl fmt::Display for Algorithms {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(rule) = self.rule.as_ref() {
            write!(f, "{rule}",)
        } else {
            write!(f, "{}", self.algos.join(","))
        }
    }
}

#[cfg(test)]
mod tests {

    use pretty_assertions::assert_eq;

    use super::*;

    #[test]
    fn test_should_parse_algos_set() {
        let algo =
            AlgorithmsRule::from_str("aes128-ctr,aes192-ctr,aes256-ctr").expect("failed to parse");
        assert_eq!(
            algo,
            AlgorithmsRule::Set(vec![
                "aes128-ctr".to_string(),
                "aes192-ctr".to_string(),
                "aes256-ctr".to_string()
            ])
        );
    }

    #[test]
    fn test_should_parse_algos_append() {
        let algo =
            AlgorithmsRule::from_str("+aes128-ctr,aes192-ctr,aes256-ctr").expect("failed to parse");
        assert_eq!(
            algo,
            AlgorithmsRule::Append(vec![
                "aes128-ctr".to_string(),
                "aes192-ctr".to_string(),
                "aes256-ctr".to_string()
            ])
        );
    }

    #[test]
    fn test_should_parse_algos_head() {
        let algo =
            AlgorithmsRule::from_str("^aes128-ctr,aes192-ctr,aes256-ctr").expect("failed to parse");
        assert_eq!(
            algo,
            AlgorithmsRule::Head(vec![
                "aes128-ctr".to_string(),
                "aes192-ctr".to_string(),
                "aes256-ctr".to_string()
            ])
        );
    }

    #[test]
    fn test_should_parse_algos_exclude() {
        let algo =
            AlgorithmsRule::from_str("-aes128-ctr,aes192-ctr,aes256-ctr").expect("failed to parse");
        assert_eq!(
            algo,
            AlgorithmsRule::Exclude(vec![
                "aes128-ctr".to_string(),
                "aes192-ctr".to_string(),
                "aes256-ctr".to_string()
            ])
        );
    }

    #[test]
    fn test_should_apply_append() {
        let mut algo1 = Algorithms::new(&["aes128-ctr", "aes192-ctr"]);
        let algo2 = AlgorithmsRule::from_str("+aes256-ctr").expect("failed to parse");
        algo1.apply(algo2);
        assert_eq!(
            algo1.algorithms(),
            vec![
                "aes128-ctr".to_string(),
                "aes192-ctr".to_string(),
                "aes256-ctr".to_string()
            ]
        );
    }

    #[test]
    fn test_should_merge_append_if_undefined() {
        let algos: Vec<String> = vec![];
        let mut algo1 = Algorithms::new(algos);
        let algo2 = AlgorithmsRule::from_str("+aes256-ctr").expect("failed to parse");
        algo1.apply(algo2);
        assert_eq!(algo1.algorithms(), vec!["aes256-ctr".to_string()]);
    }

    #[test]
    fn test_should_merge_head() {
        let mut algo1 = Algorithms::new(&["aes128-ctr", "aes192-ctr"]);
        let algo2 = AlgorithmsRule::from_str("^aes256-ctr").expect("failed to parse");
        algo1.apply(algo2);
        assert_eq!(
            algo1.algorithms(),
            vec![
                "aes256-ctr".to_string(),
                "aes128-ctr".to_string(),
                "aes192-ctr".to_string()
            ]
        );
    }

    #[test]
    fn test_should_apply_head() {
        let mut algo1 = Algorithms::new(&["aes128-ctr", "aes192-ctr"]);
        let algo2 = AlgorithmsRule::from_str("^aes256-ctr").expect("failed to parse");
        algo1.apply(algo2);
        assert_eq!(
            algo1.algorithms(),
            vec![
                "aes256-ctr".to_string(),
                "aes128-ctr".to_string(),
                "aes192-ctr".to_string()
            ]
        );
    }

    #[test]
    fn test_should_merge_exclude() {
        let mut algo1 = Algorithms::new(&["aes128-ctr", "aes192-ctr", "aes256-ctr"]);
        let algo2 = AlgorithmsRule::from_str("-aes192-ctr").expect("failed to parse");
        algo1.apply(algo2);
        assert_eq!(
            algo1.algorithms(),
            vec!["aes128-ctr".to_string(), "aes256-ctr".to_string()]
        );
    }

    #[test]
    fn test_should_merge_set() {
        let mut algo1 = Algorithms::new(&["aes128-ctr", "aes192-ctr"]);
        let algo2 = AlgorithmsRule::from_str("aes256-ctr").expect("failed to parse");
        algo1.apply(algo2);
        assert_eq!(algo1.algorithms(), vec!["aes256-ctr".to_string()]);
    }

    #[test]
    fn test_should_not_apply_twice() {
        let mut algo1 = Algorithms::new(&["aes128-ctr", "aes192-ctr"]);
        let algo2 = AlgorithmsRule::from_str("aes256-ctr").expect("failed to parse");
        algo1.apply(algo2);
        assert_eq!(algo1.algorithms(), vec!["aes256-ctr".to_string(),]);

        let algo3 = AlgorithmsRule::from_str("aes128-ctr").expect("failed to parse");
        algo1.apply(algo3);
        assert_eq!(algo1.algorithms(), vec!["aes256-ctr".to_string()]);
        assert_eq!(algo1.overridden, true);
    }

    #[test]
    fn test_algorithms_display_with_rule() {
        let mut algos = Algorithms::new(&["aes128-ctr"]);

        // Apply append rule
        let rule = AlgorithmsRule::from_str("+aes256-ctr").expect("failed to parse");
        algos.apply(rule);

        // Display should show the rule prefix
        let display = algos.to_string();
        assert_eq!(display, "+");
    }

    #[test]
    fn test_algorithms_display_without_rule() {
        let algos = Algorithms::new(&["aes128-ctr", "aes256-ctr"]);
        let display = algos.to_string();
        assert_eq!(display, "aes128-ctr,aes256-ctr");
    }

    #[test]
    fn test_algorithms_rule_display() {
        let append = AlgorithmsRule::from_str("+algo").expect("failed to parse");
        assert_eq!(append.to_string(), "+");

        let head = AlgorithmsRule::from_str("^algo").expect("failed to parse");
        assert_eq!(head.to_string(), "^");

        let exclude = AlgorithmsRule::from_str("-algo").expect("failed to parse");
        assert_eq!(exclude.to_string(), "-");

        let set = AlgorithmsRule::from_str("algo").expect("failed to parse");
        assert_eq!(set.to_string(), "");
    }

    #[test]
    fn test_algorithms_is_default() {
        let algos = Algorithms::new(&["aes128-ctr"]);
        assert!(algos.is_default());

        let mut algos2 = Algorithms::new(&["aes128-ctr"]);
        algos2.apply(AlgorithmsRule::from_str("aes256-ctr").expect("failed to parse"));
        assert!(!algos2.is_default());
    }

    #[test]
    fn test_parse_empty_algos_returns_error() {
        let result = AlgorithmsRule::from_str("");
        assert!(result.is_err());
    }

    #[test]
    fn test_append_with_duplicate_algorithms() {
        let mut algos = Algorithms::new(&["aes128-ctr", "aes256-ctr"]);
        let rule = AlgorithmsRule::from_str("+aes128-ctr,aes512-ctr").expect("failed to parse");
        algos.apply(rule);
        // aes128-ctr should not be duplicated
        assert_eq!(
            algos.algorithms(),
            vec![
                "aes128-ctr".to_string(),
                "aes256-ctr".to_string(),
                "aes512-ctr".to_string()
            ]
        );
    }

    #[test]
    fn test_exclude_all_algorithms() {
        let mut algos = Algorithms::new(&["aes128-ctr", "aes256-ctr"]);
        let rule = AlgorithmsRule::from_str("-aes128-ctr,aes256-ctr").expect("failed to parse");
        algos.apply(rule);
        assert!(algos.algorithms().is_empty());
    }

    #[test]
    fn test_head_with_empty_defaults() {
        let empty: Vec<String> = vec![];
        let mut algos = Algorithms::new(empty);
        let rule = AlgorithmsRule::from_str("^aes256-ctr").expect("failed to parse");
        algos.apply(rule);
        assert_eq!(algos.algorithms(), vec!["aes256-ctr".to_string()]);
    }

    #[test]
    fn test_algorithms_rule_op() {
        let append = AlgorithmsRule::Append(vec!["algo".to_string()]);
        assert_eq!(append.op(), AlgorithmsOp::Append);

        let head = AlgorithmsRule::Head(vec!["algo".to_string()]);
        assert_eq!(head.op(), AlgorithmsOp::Head);

        let exclude = AlgorithmsRule::Exclude(vec!["algo".to_string()]);
        assert_eq!(exclude.op(), AlgorithmsOp::Exclude);

        let set = AlgorithmsRule::Set(vec!["algo".to_string()]);
        assert_eq!(set.op(), AlgorithmsOp::Set);
    }
}
