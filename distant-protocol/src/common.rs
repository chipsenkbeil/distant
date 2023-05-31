mod capabilities;
mod change;
mod cmd;
mod error;
mod filesystem;
mod metadata;
mod permissions;
mod pty;
mod search;
mod system;
mod version;

pub use capabilities::*;
pub use change::*;
pub use cmd::*;
pub use error::*;
pub use filesystem::*;
pub use metadata::*;
pub use permissions::*;
pub use pty::*;
pub use search::*;
pub use system::*;
pub use version::*;

/// Id for a remote process
pub type ProcessId = u32;

/// Version indicated by the tuple of (major, minor, patch).
pub type SemVer = (u8, u8, u8);
