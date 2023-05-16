use std::path::{Component, Path, Prefix};

use once_cell::sync::Lazy;

// Returns true if running test in Github CI
pub static IS_CI: Lazy<bool> = Lazy::new(|| std::env::var("CI").as_deref() == Ok("true"));

pub fn ci_path_to_string(path: &Path) -> String {
    if cfg!(windows) && *IS_CI {
        convert_path_to_unix_string(path)
    } else {
        path.to_string_lossy().to_string()
    }
}

pub fn convert_path_to_unix_string(path: &Path) -> String {
    let mut s = String::new();
    for component in path.components() {
        s.push('/');

        match component {
            Component::Prefix(x) => match x.kind() {
                Prefix::Verbatim(x) => s.push_str(&x.to_string_lossy()),
                Prefix::VerbatimUNC(_, _) => unimplemented!(),
                Prefix::VerbatimDisk(x) => s.push(x as char),
                Prefix::DeviceNS(_) => unimplemented!(),
                Prefix::UNC(_, _) => unimplemented!(),
                Prefix::Disk(x) => s.push(x as char),
            },
            Component::RootDir => continue,
            Component::CurDir => s.push('.'),
            Component::ParentDir => s.push_str(".."),
            Component::Normal(x) => s.push_str(&x.to_string_lossy()),
        }
    }
    s
}
