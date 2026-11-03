use serde::{Serialize, Deserialize};

pub type QueryId = u8;

#[derive(Debug, Serialize, Deserialize)]
pub struct BenchConfig {
    pub apitoken: String,
    #[serde(default)]
    pub name: String,
    pub test: bool,
    #[serde(skip, default = "default_queries")]
    pub queries: Vec<QueryId>,
    pub max_batches: Option<u32>,
}

#[derive(Deserialize)]
pub struct Batch<'a> {
    pub batch_id: u32,
    pub print_id: &'a str,
    pub tile_id: u32,
    pub layer: u32,
    #[serde(with = "serde_bytes")]
    pub tif: &'a [u8],
}

#[derive(Serialize, Deserialize)]
pub struct Centroid {
    pub x: f64,
    pub y: f64,
    pub count: u32,
}

#[derive(Serialize, Deserialize)]
pub struct QueryResult {
    pub batch_id: u32,
    #[serde(skip, default)]
    pub query: QueryId,
    pub print_id: String,
    pub tile_id: u32,
    pub saturated: u32,
    pub centroids: Vec<Centroid>,
}

pub fn default_queries() -> Vec<QueryId> {
    vec![0]
}
