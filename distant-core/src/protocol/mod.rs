mod common;
mod msg;
mod request;
mod response;
mod utils;

pub use common::*;
pub use msg::*;
pub use request::*;
pub use response::*;
pub use semver;

/// Protocol version of major/minor/patch.
///
/// This should match the version of this crate such that any significant change to the crate
/// version will also be reflected in this constant that can be used to verify compatibility across
/// the wire.
pub const PROTOCOL_VERSION: semver::Version = semver::Version::new(
    const_str::parse!(env!("CARGO_PKG_VERSION_MAJOR"), u64),
    const_str::parse!(env!("CARGO_PKG_VERSION_MINOR"), u64),
    const_str::parse!(env!("CARGO_PKG_VERSION_PATCH"), u64),
);

/// Comparators used to indicate the [lower, upper) bounds of supported protocol versions.
const PROTOCOL_VERSION_COMPAT: (semver::Comparator, semver::Comparator) = (
    semver::Comparator {
        op: semver::Op::GreaterEq,
        major: const_str::parse!(env!("CARGO_PKG_VERSION_MAJOR"), u64),
        minor: Some(const_str::parse!(env!("CARGO_PKG_VERSION_MINOR"), u64)),
        patch: Some(const_str::parse!(env!("CARGO_PKG_VERSION_PATCH"), u64)),
        pre: semver::Prerelease::EMPTY,
    },
    semver::Comparator {
        op: semver::Op::Less,
        major: {
            let major = const_str::parse!(env!("CARGO_PKG_VERSION_MAJOR"), u64);

            // If we have a version like 0.20, then the upper bound is 0.21,
            // otherwise if we have a version like 1.2, then the upper bound is 2.0
            //
            // So only increment the major if it is greater than 0
            if major > 0 {
                major + 1
            } else {
                major
            }
        },
        minor: {
            let major = const_str::parse!(env!("CARGO_PKG_VERSION_MAJOR"), u64);
            let minor = const_str::parse!(env!("CARGO_PKG_VERSION_MINOR"), u64);

            // If we have a version like 0.20, then the upper bound is 0.21,
            // otherwise if we have a version like 1.2, then the upper bound is 2.0
            //
            // So only increment the minor if major is 0
            if major > 0 {
                None
            } else {
                Some(minor + 1)
            }
        },
        patch: None,
        pre: semver::Prerelease::EMPTY,
    },
);

/// Returns true if the provided version is compatible with the protocol version.
///
/// ```
/// use distant_core::protocol::{is_compatible_with, PROTOCOL_VERSION};
/// use distant_core::protocol::semver::Version;
///
/// // The current protocol version tied to this crate is always compatible
/// assert!(is_compatible_with(&PROTOCOL_VERSION));
///
/// // Major bumps in distant's protocol version are always considered incompatible
/// assert!(!is_compatible_with(&Version::new(
///     PROTOCOL_VERSION.major + 1,
///     PROTOCOL_VERSION.minor,
///     PROTOCOL_VERSION.patch,
/// )));
///
/// // While distant's protocol is being stabilized, minor version bumps
/// // are also considered incompatible!
/// assert!(!is_compatible_with(&Version::new(
///     PROTOCOL_VERSION.major,
///     PROTOCOL_VERSION.minor + 1,
///     PROTOCOL_VERSION.patch,
/// )));
///
/// // Patch bumps in distant's protocol are always considered compatible
/// assert!(is_compatible_with(&Version::new(
///     PROTOCOL_VERSION.major,
///     PROTOCOL_VERSION.minor,
///     PROTOCOL_VERSION.patch + 1,
/// )));
/// ```
pub fn is_compatible_with(version: &semver::Version) -> bool {
    let (lower, upper) = PROTOCOL_VERSION_COMPAT;

    lower.matches(version) && upper.matches(version)
}
