use std::path::Path;

pub fn ci_path_to_string(path: &Path) -> String {
    // Native Windows OpenSSH expects Windows-style paths, so no conversion needed.
    // Unix conversion was only needed for Cygwin/MSYS2 sshd.
    path.to_string_lossy().to_string()
}
