use std::path::PathBuf;

use objc2_foundation::{NSBundle, NSString};

/// Default App Group identifier used when running outside any bundle.
///
/// Prefixed with the Team ID as required by the provisioning profile's
/// `application-groups` wildcard (`39C6AGD73Z.*`).
const DEFAULT_APP_GROUP_ID: &str = "39C6AGD73Z.group.dev.distant";

/// The Info.plist key that stores the App Group identifier for the
/// FileProvider extension. This key is already present in the .appex's
/// Extension-Info.plist.
const APP_GROUP_PLIST_KEY: &str = "NSExtensionFileProviderDocumentGroup";

/// Reads the App Group identifier from the bundle's plist configuration.
///
/// Resolution order:
/// 1. Own bundle's Info.plist (works when running as `.appex`)
/// 2. Embedded `.appex`'s Info.plist (works when running as host `.app`)
/// 3. Hardcoded default (works outside any bundle, e.g. `cargo run`)
pub fn app_group_id() -> String {
    if let Some(id) = read_info_key_from_main_bundle(APP_GROUP_PLIST_KEY) {
        return id;
    }

    if let Some(id) = read_info_key_from_embedded_appex(APP_GROUP_PLIST_KEY) {
        return id;
    }

    DEFAULT_APP_GROUP_ID.to_string()
}

/// Detects whether this process is running inside a `.app` bundle.
///
/// Uses `NSBundle.mainBundle.bundleIdentifier` — non-nil means bundled.
pub fn is_running_in_app_bundle() -> bool {
    NSBundle::mainBundle().bundleIdentifier().is_some()
}

/// Returns the path to the App Group shared container.
///
/// The group identifier is read from the bundle's plist at runtime,
/// allowing test bundles to use a different group ID without code changes.
pub fn app_group_container_path() -> Option<PathBuf> {
    use objc2_foundation::NSFileManager;

    let group_id = app_group_id();
    let group_ns = NSString::from_str(&group_id);
    let manager = NSFileManager::defaultManager();
    let url = manager.containerURLForSecurityApplicationGroupIdentifier(&group_ns)?;
    let path_ns = url.path()?;
    Some(PathBuf::from(path_ns.to_string()))
}

/// Returns `true` if this process is running as a macOS `.appex` FileProvider extension.
pub fn is_file_provider_extension() -> bool {
    NSBundle::mainBundle()
        .bundlePath()
        .to_string()
        .ends_with(".appex")
}

/// Reads a string value from the main bundle's Info.plist.
fn read_info_key_from_main_bundle(key: &str) -> Option<String> {
    let key_ns = NSString::from_str(key);
    let value = NSBundle::mainBundle().objectForInfoDictionaryKey(&key_ns)?;

    // SAFETY: objectForInfoDictionaryKey returns id which is NSString for
    // plist string values. The downcast is safe for string-typed keys.
    let ns_str: &NSString = unsafe { &*((&*value) as *const _ as *const NSString) };
    let s = ns_str.to_string();
    if s.is_empty() { None } else { Some(s) }
}

/// Reads a string value from the embedded `.appex`'s Info.plist.
///
/// The `.appex` is at `Contents/PlugIns/DistantFileProvider.appex/` inside
/// the host `.app` bundle. Returns `None` if not running in a bundle or
/// the `.appex` is not found.
fn read_info_key_from_embedded_appex(key: &str) -> Option<String> {
    let plugins_path = NSBundle::mainBundle().builtInPlugInsPath()?;
    let appex_path = PathBuf::from(plugins_path.to_string()).join("DistantFileProvider.appex");

    if !appex_path.exists() {
        return None;
    }

    let appex_path_ns = NSString::from_str(&appex_path.to_string_lossy());

    let appex_bundle = NSBundle::bundleWithPath(&appex_path_ns)?;

    let key_ns = NSString::from_str(key);
    let value = appex_bundle.objectForInfoDictionaryKey(&key_ns)?;

    let ns_str: &NSString = unsafe { &*((&*value) as *const _ as *const NSString) };
    let s = ns_str.to_string();
    if s.is_empty() { None } else { Some(s) }
}
