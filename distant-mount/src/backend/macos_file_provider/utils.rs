use objc2_foundation::NSBundle;
use std::path::PathBuf;

/// App Group identifier, prefixed with the Team ID as required by the
/// provisioning profile's `application-groups` wildcard (`39C6AGD73Z.*`).
pub const APP_GROUP_ID: &str = "39C6AGD73Z.group.dev.distant";

/// Detects whether this process is running inside a `.app` bundle.
///
/// Uses `NSBundle.mainBundle.bundleIdentifier` — non-nil means bundled.
/// This is Apple's recommended approach (more reliable than path string matching).
pub fn is_running_in_app_bundle() -> bool {
    NSBundle::mainBundle().bundleIdentifier().is_some()
}

/// Returns the path to the App Group shared container.
pub fn app_group_container_path() -> Option<PathBuf> {
    use objc2_foundation::{NSFileManager, NSString};

    let group_id = NSString::from_str(APP_GROUP_ID);
    let manager = NSFileManager::defaultManager();
    let url = manager.containerURLForSecurityApplicationGroupIdentifier(&group_id)?;
    let path_ns = url.path()?;
    Some(PathBuf::from(path_ns.to_string()))
}

/// Returns `true` if this process is running as a macOS `.appex` FileProvider extension.
pub fn is_file_provider_extension() -> bool {
    use objc2_foundation::NSBundle;
    NSBundle::mainBundle()
        .bundlePath()
        .to_string()
        .ends_with(".appex")
}
