use crate::options::common::Seconds;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ClientApiConfig {
    pub timeout: Option<Seconds>,
}
