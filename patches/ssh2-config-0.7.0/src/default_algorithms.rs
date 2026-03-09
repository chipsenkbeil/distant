mod openssh;
pub use openssh::defaults as default_algorithms;

/// Default algorithms for ssh.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DefaultAlgorithms {
    pub ca_signature_algorithms: Vec<String>,
    pub ciphers: Vec<String>,
    pub host_key_algorithms: Vec<String>,
    pub kex_algorithms: Vec<String>,
    pub mac: Vec<String>,
    pub pubkey_accepted_algorithms: Vec<String>,
}

impl Default for DefaultAlgorithms {
    fn default() -> Self {
        default_algorithms()
    }
}

impl DefaultAlgorithms {
    /// Create a new instance of [`DefaultAlgorithms`] with empty fields.
    pub fn empty() -> Self {
        Self {
            ca_signature_algorithms: vec![],
            ciphers: vec![],
            host_key_algorithms: vec![],
            kex_algorithms: vec![],
            mac: vec![],
            pubkey_accepted_algorithms: vec![],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_algorithms_default() {
        let default_algos = DefaultAlgorithms::default();
        // Default should have non-empty algorithms from openssh defaults
        assert!(!default_algos.ciphers.is_empty());
        assert!(!default_algos.kex_algorithms.is_empty());
        assert!(!default_algos.mac.is_empty());
        assert!(!default_algos.host_key_algorithms.is_empty());
    }

    #[test]
    fn test_default_algorithms_empty() {
        let empty = DefaultAlgorithms::empty();
        assert!(empty.ca_signature_algorithms.is_empty());
        assert!(empty.ciphers.is_empty());
        assert!(empty.host_key_algorithms.is_empty());
        assert!(empty.kex_algorithms.is_empty());
        assert!(empty.mac.is_empty());
        assert!(empty.pubkey_accepted_algorithms.is_empty());
    }

    #[test]
    fn test_default_algorithms_equality() {
        let algos1 = DefaultAlgorithms::default();
        let algos2 = DefaultAlgorithms::default();
        assert_eq!(algos1, algos2);

        let empty1 = DefaultAlgorithms::empty();
        let empty2 = DefaultAlgorithms::empty();
        assert_eq!(empty1, empty2);

        assert_ne!(algos1, empty1);
    }

    #[test]
    fn test_default_algorithms_clone() {
        let original = DefaultAlgorithms::default();
        let cloned = original.clone();
        assert_eq!(original, cloned);
    }

    #[test]
    fn test_default_algorithms_debug() {
        let algos = DefaultAlgorithms::empty();
        let debug_str = format!("{:?}", algos);
        assert!(debug_str.contains("DefaultAlgorithms"));
    }
}
