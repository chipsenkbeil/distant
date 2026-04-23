mod backend;
mod core;
pub mod plugin;

pub use backend::MountBackend;

// Re-export Windows Cloud Files utilities for the binary crate.
#[cfg(all(target_os = "windows", feature = "windows-cloud-files"))]
pub mod windows_cloud_files {
    pub use crate::backend::windows_cloud_files::{unmount, unmount_path};
}

// Re-export macOS utilities for the binary crate.
#[cfg(all(target_os = "macos", feature = "macos-file-provider"))]
pub mod macos {
    pub mod fp {
        pub mod appex {
            pub use crate::backend::macos_file_provider::utils::{
                app_group_container_path, is_file_provider_extension, is_running_in_app_bundle,
            };
            pub use crate::backend::macos_file_provider::{
                ChannelResolver, init_file_provider, register_file_provider_classes,
            };
        }
    }

    pub use crate::backend::macos_file_provider::{
        DomainInfo, list_file_provider_domains, remove_all_file_provider_domains,
        remove_file_provider_domain_for_destination,
    };

    pub use crate::backend::macos_file_provider::utils::is_running_in_app_bundle;
}
