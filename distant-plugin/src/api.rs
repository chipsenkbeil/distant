/// Full API that represents a distant-compatible server.
pub trait Api {
    type FileSystem: FileSystemApi;
    type Process: ProcessApi;
    type Search: SearchApi;
    type SystemInfo: SystemInfoApi;
    type Version: VersionApi;
}

/// API supporting filesystem operations.
pub trait FileSystemApi {}

/// API supporting process creation and manipulation.
pub trait ProcessApi {}

/// API supporting searching through the remote system.
pub trait SearchApi {}

/// API supporting retrieval of information about the remote system.
pub trait SystemInfoApi {}

/// API supporting retrieval of the server's version.
pub trait VersionApi {}

/// Generic struct that implements all APIs as unsupported.
pub struct Unsupported;

impl FileSystemApi for Unsupported {
}
