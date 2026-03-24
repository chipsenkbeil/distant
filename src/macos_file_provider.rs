//! macOS FileProvider extension entry point.
//!
//! When the binary is launched inside a `.appex` bundle, `is_file_provider_extension()`
//! returns `true` and `run_extension()` takes over before the CLI parser runs.
//! The extension connects to the distant manager via the App Group shared container
//! and blocks to serve FileProvider requests.

/// Returns `true` if this process is running as a `.appex` FileProvider extension.
///
/// Delegates to `distant_mount::is_file_provider_extension()` which uses
/// `NSBundle.mainBundle.bundlePath` to check for a `.appex` suffix.
pub fn is_file_provider_extension() -> bool {
    distant_mount::is_file_provider_extension()
}

/// Runs the FileProvider extension process.
///
/// Blocks forever. macOS `fileproviderd` will instantiate the
/// `DistantFileProvider` class (defined in `distant-mount`) via
/// `initWithDomain:` on a background XPC queue, which triggers the
/// bootstrap flow.
pub fn run_extension() -> ! {
    // The ObjC classes (DistantFileProvider, etc.) are auto-registered by
    // the `define_class!` macros in distant-mount when the library is linked.
    // macOS fileproviderd calls `initWithDomain:` on our class when a domain
    // is accessed.

    // Block the main thread forever. The FileProvider framework communicates
    // via XPC on background dispatch queues, so the main thread just needs
    // to stay alive.
    loop {
        std::thread::park();
    }
}
