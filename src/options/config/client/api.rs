use serde::{Deserialize, Serialize};

use crate::options::common::Seconds;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ClientApiConfig {
    pub timeout: Option<Seconds>,
}
