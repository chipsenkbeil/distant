use std::fmt;

use semver::{Comparator, Op, Prerelease, Version as SemVer};

/// Represents a version and compatibility rules.
#[derive(Clone, Debug)]
pub struct Version {
    inner: SemVer,
    lower: Comparator,
    upper: Comparator,
}

impl Version {
    /// Creates a new version in the form `major.minor.patch` with a ruleset that is used to check
    /// other versions such that `>=0.1.2, <0.2.0` or `>=1.2.3, <2` depending on whether or not the
    /// major version is `0`.
    ///
    /// ```
    /// use distant_core::net::common::Version;
    ///
    /// // Matching versions are compatible
    /// let a = Version::new(1, 2, 3);
    /// let b = Version::new(1, 2, 3);
    /// assert!(a.is_compatible_with(&b));
    ///
    /// // Version 1.2.3 is compatible with 1.2.4, but not the other way
    /// let a = Version::new(1, 2, 3);
    /// let b = Version::new(1, 2, 4);
    /// assert!(a.is_compatible_with(&b));
    /// assert!(!b.is_compatible_with(&a));
    ///
    /// // Version 1.2.3 is compatible with 1.3.0, but not 2
    /// let a = Version::new(1, 2, 3);
    /// assert!(a.is_compatible_with(&Version::new(1, 3, 0)));
    /// assert!(!a.is_compatible_with(&Version::new(2, 0, 0)));
    ///
    /// // Version 0.1.2 is compatible with 0.1.3, but not the other way
    /// let a = Version::new(0, 1, 2);
    /// let b = Version::new(0, 1, 3);
    /// assert!(a.is_compatible_with(&b));
    /// assert!(!b.is_compatible_with(&a));
    ///
    /// // Version 0.1.2 is not compatible with 0.2
    /// let a = Version::new(0, 1, 2);
    /// let b = Version::new(0, 2, 0);
    /// assert!(!a.is_compatible_with(&b));
    /// assert!(!b.is_compatible_with(&a));
    /// ```
    pub const fn new(major: u64, minor: u64, patch: u64) -> Self {
        Self {
            inner: SemVer::new(major, minor, patch),
            lower: Comparator {
                op: Op::GreaterEq,
                major,
                minor: Some(minor),
                patch: Some(patch),
                pre: Prerelease::EMPTY,
            },
            upper: Comparator {
                op: Op::Less,
                major: if major == 0 { 0 } else { major + 1 },
                minor: if major == 0 { Some(minor + 1) } else { None },
                patch: None,
                pre: Prerelease::EMPTY,
            },
        }
    }

    /// Returns true if this version is compatible with another version.
    pub fn is_compatible_with(&self, other: &Self) -> bool {
        self.lower.matches(&other.inner) && self.upper.matches(&other.inner)
    }

    /// Converts from a collection of bytes into a version using the byte form major/minor/patch
    /// using big endian.
    pub const fn from_be_bytes(bytes: [u8; 24]) -> Self {
        Self::new(
            u64::from_be_bytes([
                bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
            ]),
            u64::from_be_bytes([
                bytes[8], bytes[9], bytes[10], bytes[11], bytes[12], bytes[13], bytes[14],
                bytes[15],
            ]),
            u64::from_be_bytes([
                bytes[16], bytes[17], bytes[18], bytes[19], bytes[20], bytes[21], bytes[22],
                bytes[23],
            ]),
        )
    }

    /// Converts the version into a byte form of major/minor/patch using big endian.
    pub const fn to_be_bytes(&self) -> [u8; 24] {
        let major = self.inner.major.to_be_bytes();
        let minor = self.inner.minor.to_be_bytes();
        let patch = self.inner.patch.to_be_bytes();

        [
            major[0], major[1], major[2], major[3], major[4], major[5], major[6], major[7],
            minor[0], minor[1], minor[2], minor[3], minor[4], minor[5], minor[6], minor[7],
            patch[0], patch[1], patch[2], patch[3], patch[4], patch[5], patch[6], patch[7],
        ]
    }
}

impl Default for Version {
    /// Default version is `0.0.0`.
    fn default() -> Self {
        Self::new(0, 0, 0)
    }
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.inner)
    }
}

impl From<semver::Version> for Version {
    /// Creates a new [`Version`] using the major, minor, and patch information from
    /// [`semver::Version`].
    fn from(version: semver::Version) -> Self {
        let mut this = Self::new(version.major, version.minor, version.patch);
        this.inner = version;
        this
    }
}

impl From<Version> for semver::Version {
    fn from(version: Version) -> Self {
        version.inner
    }
}

#[cfg(test)]
mod tests {
    //! Tests for Version: byte serialization round-trips, known byte patterns, Display,
    //! semver conversions, and exhaustive is_compatible_with scenarios (same version,
    //! forward compatibility, major=0 boundaries, major>0 boundaries).

    use test_log::test;

    use super::*;

    #[test]
    fn to_be_bytes_and_from_be_bytes_should_round_trip_for_zero_version() {
        let v = Version::new(0, 0, 0);
        let bytes = v.to_be_bytes();
        let v2 = Version::from_be_bytes(bytes);
        assert_eq!(v2.to_string(), "0.0.0");
    }

    #[test]
    fn to_be_bytes_and_from_be_bytes_should_round_trip_for_simple_version() {
        let v = Version::new(1, 2, 3);
        let bytes = v.to_be_bytes();
        let v2 = Version::from_be_bytes(bytes);
        assert_eq!(v2.to_string(), "1.2.3");
    }

    #[test]
    fn to_be_bytes_and_from_be_bytes_should_round_trip_for_large_version() {
        let v = Version::new(100, 200, 300);
        let bytes = v.to_be_bytes();
        let v2 = Version::from_be_bytes(bytes);
        assert_eq!(v2.to_string(), "100.200.300");
    }

    #[test]
    fn to_be_bytes_and_from_be_bytes_should_round_trip_for_large_components() {
        let v = Version::new(999_999, 888_888, 777_777);
        let bytes = v.to_be_bytes();
        let v2 = Version::from_be_bytes(bytes);
        assert_eq!(v2.to_string(), "999999.888888.777777");
    }

    #[test]
    fn from_be_bytes_should_decode_known_byte_pattern_for_version_1_0_0() {
        // major=1, minor=0, patch=0 in big-endian u64
        let mut bytes = [0u8; 24];
        bytes[7] = 1; // major = 1
                      // minor and patch remain 0
        let v = Version::from_be_bytes(bytes);
        assert_eq!(v.to_string(), "1.0.0");
    }

    #[test]
    fn from_be_bytes_should_decode_known_byte_pattern_for_version_0_1_0() {
        let mut bytes = [0u8; 24];
        bytes[15] = 1; // minor = 1
        let v = Version::from_be_bytes(bytes);
        assert_eq!(v.to_string(), "0.1.0");
    }

    #[test]
    fn from_be_bytes_should_decode_known_byte_pattern_for_version_0_0_1() {
        let mut bytes = [0u8; 24];
        bytes[23] = 1; // patch = 1
        let v = Version::from_be_bytes(bytes);
        assert_eq!(v.to_string(), "0.0.1");
    }

    #[test]
    fn to_be_bytes_should_produce_all_zeros_for_version_0_0_0() {
        let v = Version::new(0, 0, 0);
        assert_eq!(v.to_be_bytes(), [0u8; 24]);
    }

    #[test]
    fn default_should_be_0_0_0() {
        let v = Version::default();
        assert_eq!(v.to_string(), "0.0.0");
    }

    #[test]
    fn display_should_use_semver_format() {
        assert_eq!(Version::new(1, 2, 3).to_string(), "1.2.3");
        assert_eq!(Version::new(0, 0, 0).to_string(), "0.0.0");
        assert_eq!(Version::new(10, 20, 30).to_string(), "10.20.30");
    }

    #[test]
    fn from_semver_version_should_preserve_major_minor_patch() {
        let sv = semver::Version::new(3, 4, 5);
        let v = Version::from(sv);
        assert_eq!(v.to_string(), "3.4.5");
    }

    #[test]
    fn into_semver_version_should_preserve_major_minor_patch() {
        let v = Version::new(7, 8, 9);
        let sv: semver::Version = v.into();
        assert_eq!(sv.major, 7);
        assert_eq!(sv.minor, 8);
        assert_eq!(sv.patch, 9);
    }

    #[test]
    fn from_semver_version_round_trip_should_preserve_values() {
        let original = semver::Version::new(5, 6, 7);
        let v = Version::from(original.clone());
        let recovered: semver::Version = v.into();
        assert_eq!(recovered, original);
    }

    #[test]
    fn is_compatible_with_same_version_should_be_true() {
        let a = Version::new(1, 2, 3);
        let b = Version::new(1, 2, 3);
        assert!(a.is_compatible_with(&b));
    }

    #[test]
    fn is_compatible_with_0_0_0_and_0_0_1_should_be_true() {
        let a = Version::new(0, 0, 0);
        let b = Version::new(0, 0, 1);
        assert!(
            a.is_compatible_with(&b),
            "0.0.0 should be compatible with 0.0.1 (same 0.0.x range)"
        );
    }

    #[test]
    fn is_compatible_with_0_0_1_and_0_0_0_should_be_false() {
        let a = Version::new(0, 0, 1);
        let b = Version::new(0, 0, 0);
        assert!(
            !a.is_compatible_with(&b),
            "0.0.1 should not be compatible with 0.0.0 (older patch)"
        );
    }

    #[test]
    fn is_compatible_with_major_0_boundary_should_not_cross_minor() {
        let a = Version::new(0, 1, 0);
        let b = Version::new(0, 2, 0);
        assert!(
            !a.is_compatible_with(&b),
            "0.1.0 should not be compatible with 0.2.0 (different minor in 0.x)"
        );
    }

    #[test]
    fn is_compatible_with_major_0_same_minor_higher_patch_should_be_true() {
        let a = Version::new(0, 3, 0);
        let b = Version::new(0, 3, 99);
        assert!(
            a.is_compatible_with(&b),
            "0.3.0 should be compatible with 0.3.99 (same 0.3.x range)"
        );
    }

    #[test]
    fn is_compatible_with_major_nonzero_should_allow_higher_minor() {
        let a = Version::new(2, 0, 0);
        let b = Version::new(2, 5, 10);
        assert!(
            a.is_compatible_with(&b),
            "2.0.0 should be compatible with 2.5.10 (same major)"
        );
    }

    #[test]
    fn is_compatible_with_major_nonzero_should_not_cross_major() {
        let a = Version::new(2, 0, 0);
        let b = Version::new(3, 0, 0);
        assert!(
            !a.is_compatible_with(&b),
            "2.0.0 should not be compatible with 3.0.0 (different major)"
        );
    }

    #[test]
    fn is_compatible_with_higher_version_should_not_match_lower() {
        let a = Version::new(1, 5, 0);
        let b = Version::new(1, 4, 0);
        assert!(
            !a.is_compatible_with(&b),
            "1.5.0 should not be compatible with 1.4.0 (lower minor)"
        );
    }

    #[test]
    fn is_compatible_with_0_0_0_and_0_1_0_should_be_false() {
        let a = Version::new(0, 0, 0);
        let b = Version::new(0, 1, 0);
        assert!(
            !a.is_compatible_with(&b),
            "0.0.0 should not be compatible with 0.1.0 (different minor in 0.x)"
        );
    }

    #[test]
    fn is_compatible_with_default_and_default_should_be_true() {
        let a = Version::default();
        let b = Version::default();
        assert!(a.is_compatible_with(&b));
    }

    #[test]
    fn from_semver_version_should_preserve_compatibility_rules() {
        let sv = semver::Version::new(1, 2, 3);
        let v = Version::from(sv);
        assert!(v.is_compatible_with(&Version::new(1, 2, 4)));
        assert!(!v.is_compatible_with(&Version::new(2, 0, 0)));
    }
}
