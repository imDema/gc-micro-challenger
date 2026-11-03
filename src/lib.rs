use std::{
    collections::HashMap,
    path::Path,
    string::FromUtf8Error,
    sync::{
        atomic::{AtomicU32, Ordering},
        Arc,
    },
};

use dashmap::DashMap;
use fxhash::FxBuildHasher;
use log::ResultLog;
use serde::{Deserialize, Serialize};
use time::{Duration, OffsetDateTime};
use types::{BenchConfig, QueryId};

pub mod cli;
pub mod json;
pub mod log;
pub mod plot;
pub mod types;
pub mod html;

const CONFIG: &'static str = "CONFIG";
const BENCH: &'static str = "BENCH";
const RESULT: &'static str = "RESULT";
const INPUT: &'static str = "INPUT";
const OUTPUT: &'static str = "OUTPUT";
const META: &'static str = "META";

#[derive(Clone)]
pub struct Context {
    sled: sled::Db,
    sessions: Arc<DashMap<String, Session>>,
}

pub fn output_key(id: QueryId, index: u32) -> impl AsRef<[u8]> {
    let mut key = [0u8; 9];
    key[0] = id;
    key[1..9].swap_with_slice(&mut index.to_be_bytes());
    key
}

pub fn input_key(index: u32) -> impl AsRef<[u8]> {
    index.to_be_bytes()
}

pub fn result_key(bench_id: &str, q: QueryId, index: u32) -> impl AsRef<[u8]> {
    let mut key = bench_id.as_bytes().to_vec();
    key.extend(&q.to_be_bytes());
    key.extend(&index.to_be_bytes());
    key
}

impl Context {
    pub fn new(path: &Path) -> Result<Self, sled::Error> {
        Ok(Self {
            sled: sled::open(path)?,
            sessions: Default::default(),
        })
    }

    pub fn create(&self, config: BenchConfig) -> Result<String, Error> {
        let uuid = ulid::Ulid::new().to_string();

        let db = self.sled.open_tree(CONFIG)?;
        let conf = serde_json::to_vec(&config)?;
        db.insert(&uuid.as_bytes(), conf)?;

        let max = match config.max_batches {
            Some(max) => max,
            None => get_meta(&self.sled, "count")?.expect("missing count variable in db metadata"),
        };

        let logs = config
            .queries
            .into_iter()
            .map(|q| (q, ResultLog::new(max as usize, num_cpus::get() * 8)))
            .collect();

        let session = Session {
            index: 0.into(),
            end: max,
            logs,
        };

        self.sessions.insert(uuid.clone(), session);

        Ok(uuid)
    }

    pub fn next_batch<'a>(&'a self, bench_id: &str) -> Result<Option<Vec<u8>>, Error> {
        let session = self
            .sessions
            .get(bench_id)
            .ok_or_else(|| Error::InvalidId(bench_id.to_owned()))?;

        let index = session.index.fetch_add(1, Ordering::Relaxed);

        if index >= session.end {
            return Ok(None);
        }

        let db = self.sled.open_tree(INPUT)?;
        let output = db.get(input_key(index))?.map(|v| v.to_vec());

        session.logs.values().for_each(|l| l.start(index as usize));

        Ok(output)
    }

    pub fn result(
        &self,
        bench_id: &str,
        query: QueryId,
        batch_id: usize,
        payload: Vec<u8>,
    ) -> Result<Option<OffsetDateTime>, Error> {
        let session = self
            .sessions
            .get(bench_id)
            .ok_or_else(|| Error::InvalidId(bench_id.to_owned()))?;

        let log = session
            .logs
            .get(&query)
            .ok_or_else(|| Error::InvalidQuery(query))?;

        Ok(log.put(batch_id, payload))
    }

    pub fn get_result(
        &self,
        bench_id: &str,
        query: QueryId,
        batch_id: usize,
    ) -> Result<Option<Vec<u8>>, Error> {
        let tree = self.sled.open_tree(RESULT)?;
        let res = tree.get(result_key(bench_id, query, batch_id as u32))?;
        Ok(res.map(|x| x.to_vec()))
    }

    pub fn complete(&self, bench_id: &str) -> Result<Report, Error> {
        let session = self
            .sessions
            .remove(bench_id)
            .ok_or_else(|| Error::InvalidId(bench_id.to_owned()))?
            .1;

        let mut queries = HashMap::new();
        for (q, log) in session.logs.into_iter() {
            let timings = log.to_vec();

            let tree = self.sled.open_tree(RESULT)?;
            for (i, result) in timings
                .iter()
                .enumerate()
                .filter_map(|(i, x)| x.clone().map(|x| (i, x)))
            {
                tree.insert(result_key(bench_id, q, i as u32), result.data.as_ref())?;
            }

            let count = timings.iter().rev().skip_while(|x| x.is_none()).count();
            let missing = timings.iter().take(count).filter(|x| x.is_none()).count();
            let mut latencies = timings
                .iter()
                .flatten()
                .map(|x| (x.end - x.start))
                .collect::<Vec<_>>();
            latencies.sort_unstable();

            let latency_mean = latencies.iter().sum::<time::Duration>() / count as f64;
            let latency_min = latencies.first().cloned().unwrap_or_default();
            let latency_max = latencies.last().cloned().unwrap_or_default();
            let latency_p99 = latencies
                .get((count as f64 * 0.99).ceil() as usize)
                .cloned()
                .unwrap_or_default();

            let min_t = timings.iter().flatten().map(|r| r.start).min();
            let max_t = timings.iter().flatten().map(|r| r.end).max();
            let dt = min_t.zip(max_t).map(|(min, max)| max - min);
            let throughput = count as f64 / dt.unwrap_or_default().as_seconds_f64();

            queries.insert(
                q as u8,
                QueryReport {
                    count,
                    missing,
                    timings: timings
                        .into_iter()
                        .map(|o| o.map(|x| x.end - x.start))
                        .collect(),
                    throughput,
                    latency_mean,
                    latency_min,
                    latency_max,
                    latency_p99,
                },
            );
        }

        let conf = self.sled.open_tree(CONFIG)?.get(bench_id.as_bytes())?.expect("missing config for completed bench");
        let conf: BenchConfig = serde_json::from_slice(&conf)?;

        let r = Report {
            bench_id: bench_id.into(),
            queries,
            apitoken: conf.apitoken,
            bench_name: conf.name,
            test: conf.test,
        };

        let tree = self.sled.open_tree(BENCH)?;
        tree.insert(bench_id.as_bytes(), serde_json::to_vec(&r)?)?;

        Ok(r)
    }

    pub fn delete_result(&self, bench_id: &str) -> Result<bool, Error> {
        let tree = self.sled.open_tree(BENCH)?;
        let removed = tree.remove(bench_id.as_bytes())?.is_some();

        let tree = self.sled.open_tree(RESULT)?;
        let keys: Vec<_> = tree.scan_prefix(bench_id.as_bytes()).map(|e| e.unwrap().0).collect();
        for k in keys {
            tree.remove(k)?;
        }
        Ok(removed)
    }

    pub fn history(&self) -> Result<HashMap<String, Report>, Error> {
        let tree = self.sled.open_tree(BENCH)?;
        tree.into_iter()
            .flatten()
            .map(|(k, v)| (String::from_utf8(k.to_vec()), serde_json::from_slice(&v)))
            .map(|(k, v)| Ok((k?, v?)))
            .collect::<Result<HashMap<_, _>, Error>>()
    }

    pub fn set_input(&self, query: QueryId, batch_id: u32, data: &[u8]) -> Result<(), Error> {
        let tree = self.sled.open_tree(INPUT)?;
        let key = output_key(query, batch_id);

        tree.insert(&key, data)?;

        Ok(())
    }

    pub fn set_output(&self, query: QueryId, batch_id: u32, data: &[u8]) -> Result<(), Error> {
        let tree = self.sled.open_tree(OUTPUT)?;
        let key = output_key(query, batch_id);

        tree.insert(&key, data)?;

        Ok(())
    }
}

pub(crate) fn set_meta<T: Serialize>(sled: &sled::Db, key: &str, value: T) -> Result<(), Error> {
    let db = sled.open_tree(META)?;
    let ser = rmp_serde::to_vec(&value).unwrap();
    db.insert(key.as_bytes(), ser)?;
    Ok(())
}

pub(crate) fn get_meta<T: for<'de> Deserialize<'de>>(
    sled: &sled::Db,
    key: &str,
) -> Result<Option<T>, Error> {
    let db = sled.open_tree(META)?;
    let ser = db.get(key.as_bytes())?;
    let value = ser.map(|data| rmp_serde::from_slice(&data).unwrap());
    Ok(value)
}

struct Session {
    index: AtomicU32,
    end: u32,
    logs: HashMap<QueryId, ResultLog, FxBuildHasher>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Report {
    pub apitoken: String,
    pub bench_id: String,
    pub bench_name: String,
    pub test: bool,
    pub queries: HashMap<QueryId, QueryReport>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct QueryReport {
    pub count: usize,
    pub missing: usize,
    pub timings: Vec<Option<Duration>>,
    pub throughput: f64,
    pub latency_mean: Duration,
    pub latency_min: Duration,
    pub latency_max: Duration,
    pub latency_p99: Duration,
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Embedded database error {0}")]
    SledError(#[from] sled::Error),

    #[error("Input/Output error {0}")]
    IO(#[from] std::io::Error),

    #[error("Serialization error {0}")]
    Serialization(#[from] serde_json::Error),

    #[error("Invalid bench id `{0}`")]
    InvalidId(String),

    #[error("Invalid query `{0}`")]
    InvalidQuery(u8),

    #[error("Invalid chacracters in string {0}")]
    InvalidUtf8(#[from] FromUtf8Error),
}
