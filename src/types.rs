use serde::{Deserialize, Serialize};

/// Query identifier type.
pub type QueryId = u8;

#[derive(Debug, Serialize, Deserialize)]
pub struct BenchConfig {
    pub apitoken: String,
    #[serde(default)]
    pub name: String,
    pub test: bool,
    /// Query IDs this benchmark supports.
    // #[serde(default = "default_queries")]
    // pub queries: Vec<QueryId>,
    pub max_batches: Option<u32>,
}

pub fn default_queries() -> Vec<QueryId> {
    vec![0]
}
