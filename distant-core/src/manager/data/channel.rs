use serde::{Deserialize, Serialize};

/// Type of channel
#[derive(Copy, Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "clap", derive(clap::ValueEnum))]
#[serde(rename_all = "snake_case", deny_unknown_fields, tag = "type")]
pub enum ChannelKind {
    /// No response is expected
    NoResponse,

    /// Singular response is expected
    SingleResponse,

    /// Multiple responses are expected
    MultiResponse,
}
