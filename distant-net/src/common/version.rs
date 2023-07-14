use semver::{Comparator, Op, Prerelease, Version as SemVer};
use std::fmt;

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
    /// use distant_net::common::Version;
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
