use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ClientApiConfig {
    pub timeout: Option<f32>,
}
