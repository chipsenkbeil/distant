use std::io;

use crate::config::Config;

mod process;
pub use process::*;

mod search;
pub use search::*;

mod watcher;
pub use watcher::*;

/// Holds global state state managed by the server
pub struct GlobalState {
    /// State that holds information about processes running on the server
    pub process: ProcessState,

    /// State that holds information about searches running on the server
    pub search: SearchState,

    /// Watcher used for filesystem events
    pub watcher: WatcherState,
}

impl GlobalState {
    pub fn initialize(config: Config) -> io::Result<Self> {
        Ok(Self {
            process: ProcessState::new(),
            search: SearchState::new(),
            watcher: WatcherBuilder::new()
                .with_config(config.watch)
                .initialize()?,
        })
    }
}

#[cfg(test)]
mod tests {
    //! Tests for `GlobalState` initialization with various config combinations and
    //! field accessibility (process spawn, watcher abort, search abort).

    use super::*;
    use crate::config::WatchConfig;
    use std::time::Duration;

    #[test_log::test(tokio::test)]
    async fn initialize_with_default_config_succeeds() {
        let state = GlobalState::initialize(Config::default());
        assert!(state.is_ok());
    }

    #[test_log::test(tokio::test)]
    async fn initialize_with_poll_watcher_config_succeeds() {
        let config = Config {
            watch: WatchConfig {
                native: false,
                poll_interval: Some(Duration::from_secs(1)),
                ..WatchConfig::default()
            },
        };
        let state = GlobalState::initialize(config);
        assert!(state.is_ok());
    }

    #[test_log::test(tokio::test)]
    async fn initialize_with_custom_watch_config_succeeds() {
        let config = Config {
            watch: WatchConfig {
                native: true,
                poll_interval: Some(Duration::from_secs(5)),
                compare_contents: true,
                debounce_timeout: Duration::from_millis(100),
                debounce_tick_rate: Some(Duration::from_millis(50)),
            },
        };
        let state = GlobalState::initialize(config);
        assert!(state.is_ok());
    }

    #[test_log::test(tokio::test)]
    async fn fields_are_accessible() {
        let state = GlobalState::initialize(Config::default()).unwrap();

        // Verify process state is usable via its channel
        let (reply, _rx) = tokio::sync::mpsc::unbounded_channel();
        let cmd = if cfg!(windows) {
            "cmd /C echo test".to_string()
        } else {
            "echo test".to_string()
        };
        let result = state
            .process
            .spawn(
                cmd,
                distant_core::protocol::Environment::new(),
                None,
                None,
                Box::new(reply),
            )
            .await;
        assert!(result.is_ok());

        // Verify watcher state is usable by checking it can be aborted
        state.watcher.abort();

        // Verify search state is usable by checking it can be aborted
        state.search.abort();
    }
}
