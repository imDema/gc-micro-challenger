use std::{
    collections::{HashMap, HashSet},
    path::Path,
    string::FromUtf8Error,
    sync::{
        Arc, OnceLock,
        atomic::{AtomicU32, Ordering},
    },
};

use bstr::BStr;
use dashmap::DashMap;
use log::ResultLog;
use serde::{Deserialize, Serialize};
use time::{Duration, OffsetDateTime};
use types::BenchConfig;

pub mod cli;
pub mod html;
pub mod json;
pub mod log;
pub mod types;

const CONFIG: &str = "CONFIG";
const BENCH: &str = "BENCH";
const RESULT: &str = "RESULT";
const INPUT: &str = "INPUT";
const OUTPUT: &str = "OUTPUT";
const META: &str = "META";

const COL_EVENT_ID: usize = 1;
const COL_EVENT_TYPE: usize = 2;

const REQ_MAPPING_KEY: &str = "meta:request_mapping";

#[derive(Clone)]
pub struct Context {
    sled: sled::Db,
    sessions: Arc<DashMap<String, Session>>,
}

pub fn input_key(index: u32) -> impl AsRef<[u8]> {
    index.to_be_bytes()
}

pub fn result_key(bench_id: &str, index: u32) -> impl AsRef<[u8]> {
    let mut key = bench_id.as_bytes().to_vec();
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
        db.insert(uuid.as_bytes(), conf)?;

        let max = match config.max_batches {
            Some(max) => max,
            None => get_meta(&self.sled, "count")?.expect("missing count variable in db metadata"),
        };

        let logs = ResultLog::new(max as usize, num_cpus::get() * 8);

        let session = Session {
            index: 0.into(),
            end: max,
            logs,
            query_indices: Default::default(),
            start_time: Default::default(),
        };

        self.sessions.insert(uuid.clone(), session);

        Ok(uuid)
    }

    /// Returns the next input CSV line and its event type (0=transaction, 1=query).
    pub fn next_batch(&self, bench_id: &str) -> Result<Option<String>, Error> {
        let mut session = self
            .sessions
            .get_mut(bench_id)
            .ok_or_else(|| Error::InvalidId(bench_id.to_owned()))?;

        let index = session.index.fetch_add(1, Ordering::Relaxed);

        if index >= session.end {
            return Ok(None);
        }

        session.start_time.get_or_init(OffsetDateTime::now_utc);

        let db = self.sled.open_tree(INPUT)?;
        let raw = db.get(input_key(index))?;

        let line = match raw {
            Some(v) => String::from_utf8(v.to_vec()).map_err(Error::from)?,
            None => return Ok(None),
        };

        // Parse event type from the first CSV field (0 or 1).
        if let Some(request_id) = line.split(',').nth(1).and_then(|f| f.parse::<usize>().ok()) {
            // Record the event type for all query logs.
            session.logs.start(request_id);
            session.query_indices.insert(index as usize);
        }

        Ok(Some(line))
    }

    pub fn result(
        &self,
        bench_id: &str,
        index: usize,
        output_csv: String,
    ) -> Result<Option<OffsetDateTime>, Error> {
        let session = self
            .sessions
            .get(bench_id)
            .ok_or_else(|| Error::InvalidId(bench_id.to_owned()))?;

        let log = &session.logs;

        // Store result in the log only — persisted to SLED at completion.
        Ok(log.put(index, output_csv.into_bytes()))
    }

    pub fn complete(&self, bench_id: &str) -> Result<Report, Error> {
        let session = self
            .sessions
            .remove(bench_id)
            .ok_or_else(|| Error::InvalidId(bench_id.to_owned()))?
            .1;

        let mappings =
            get_meta::<HashMap<String, String>>(&self.sled, REQ_MAPPING_KEY)?.unwrap_or_default();
        let output_timings = session.logs.to_completed_vec();

        let result_tree = self.sled.open_tree(RESULT)?;
        for (i, result) in output_timings.iter().enumerate() {
            if let Some(r) = result {
                let key = result_key(bench_id, i as u32);
                result_tree.insert(key, r.data.as_ref())?;
            }
        }

        let conf = self
            .sled
            .open_tree(CONFIG)?
            .get(bench_id.as_bytes())?
            .ok_or_else(|| Error::InvalidId(bench_id.into()))?;
        let conf: BenchConfig = serde_json::from_slice(&conf)?;

        // Every request in the input (not every event), so unanswered ones are included.
        // Also grouped by track (mapping: request type -> track). Rows are batches of
        // newline-separated lines.
        let mut all = Vec::new();
        let mut by_track: HashMap<&String, Vec<_>> = HashMap::new();
        let range = 0u32.to_be_bytes()..session.end.to_be_bytes();
        for batch in self.sled.open_tree(INPUT)?.range(range).values() {
            for line in batch?.split(|&b| b == b'\n') {
                let Some(id) = csv_col(line, COL_EVENT_ID).and_then(|c| c.parse::<usize>().ok())
                else {
                    continue; // transaction
                };
                let req = (id, output_timings.get(id).and_then(Option::as_ref));
                all.push(req);
                if let Some(track) = csv_col(line, COL_EVENT_TYPE).and_then(|t| mappings.get(t)) {
                    by_track.entry(track).or_default().push(req);
                }
            }
        }

        // Throughput window: first input event sent until the last result received.
        let last_end = output_timings.iter().flatten().map(|r| r.end).max();
        let run_duration = session
            .start_time
            .get()
            .zip(last_end)
            .map(|(a, b)| b - *a)
            .unwrap_or_default();

        let timings = compute_timings(&self.sled, bench_id, all, run_duration)?;
        // Only report tracks with at least one answered request.
        let tracks = by_track
            .into_iter()
            .filter(|(_, reqs)| reqs.iter().any(|(_, a)| a.is_some()))
            .map(|(track, reqs)| {
                let t = compute_timings(&self.sled, bench_id, reqs, run_duration)?;
                Ok((track.clone(), t))
            })
            .collect::<Result<HashMap<_, _>, Error>>()?;

        let r = Report {
            bench_id: bench_id.into(),
            timings,
            tracks,
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
        let keys: Vec<_> = tree
            .scan_prefix(bench_id.as_bytes())
            .map(|e| e.unwrap().0)
            .collect();
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

    pub fn set_input(&self, batch_id: u32, data: &[u8]) -> Result<(), Error> {
        let tree = self.sled.open_tree(INPUT)?;
        tree.insert(batch_id.to_be_bytes(), data)?;

        Ok(())
    }

    pub fn set_output(&self, batch_id: u32, data: &[u8]) -> Result<(), Error> {
        let tree = self.sled.open_tree(OUTPUT)?;
        tree.insert(batch_id.to_be_bytes(), data)?;

        Ok(())
    }
}

fn compute_timings(
    db: &sled::Db,
    bench_id: &str,
    output_timings: Vec<(usize, Option<&log::CompleteResult>)>,
    run_duration: Duration,
) -> Result<TimingReport, Error> {
    let output_tree = db.open_tree(OUTPUT)?;

    let count = output_timings.len();
    let (missing, correct, wrong) = output_timings
        .iter()
        .map(|&(i, result)| {
            tracing::debug!("analyzing query output {i}");

            let Some(result) = result else {
                return (1, 0, 0);
            };

            let Ok(Some(expected)) = output_tree.get(input_key(i as u32)) else {
                tracing::error!("Missing entry in OUTPUT tree for {bench_id}, {i}");
                return (0, 0, 0);
            };

            let correct = result.data.as_ref() == expected.as_ref();

            if !correct {
                tracing::warn!(
                    "mismatch in result {i}. Expected: `{}` got `{}`",
                    BStr::new(expected.as_ref()),
                    BStr::new(result.data.as_ref())
                );
            }

            (0, correct as usize, !correct as usize)
        })
        .fold((0, 0, 0), |a, x| (a.0 + x.0, a.1 + x.1, a.2 + x.2));
    let mut latencies: Vec<_> = output_timings
        .iter()
        .filter_map(|x| x.1)
        .map(|r| r.end - r.start)
        .collect();
    latencies.sort_unstable();
    let answered = latencies.len();
    let latency_mean = if answered > 0 {
        latencies.iter().sum::<time::Duration>() / answered as f64
    } else {
        time::Duration::ZERO
    };
    let latency_min = latencies.first().cloned().unwrap_or_default();
    let latency_max = latencies.last().cloned().unwrap_or_default();
    let latency_p99 = latencies
        .get(((answered as f64 * 0.99).ceil() as usize).saturating_sub(1))
        .cloned()
        .unwrap_or_default();
    let throughput = if !run_duration.is_zero() {
        answered as f64 / run_duration.as_seconds_f64()
    } else {
        0.0
    };
    let full_timings: Vec<_> = output_timings
        .iter()
        .map(|x| x.1.map(|r| r.end - r.start))
        .collect();
    tracing::warn!("TODO: add input counts");

    Ok(TimingReport {
        count,
        missing,
        correct,
        wrong,
        timings: full_timings,
        throughput,
        latency_mean,
        latency_min,
        latency_max,
        latency_p99,
    })
}

fn csv_col(line: &[u8], col: usize) -> Option<&str> {
    str::from_utf8(line)
        .ok()?
        .split(',')
        .nth(col)
        .map(str::trim)
}

pub(crate) fn set_meta<T: Serialize>(sled: &sled::Db, key: &str, value: T) -> Result<(), Error> {
    let db = sled.open_tree(META)?;
    let ser = serde_json::to_vec(&value)?;
    db.insert(key.as_bytes(), ser)?;
    Ok(())
}

pub(crate) fn get_meta<T: for<'de> Deserialize<'de>>(
    sled: &sled::Db,
    key: &str,
) -> Result<Option<T>, Error> {
    let db = sled.open_tree(META)?;
    let ser = db.get(key.as_bytes())?;
    let value = ser
        .map(|data| serde_json::from_slice(&data).map_err(Error::from))
        .transpose()?;
    Ok(value)
}

struct Session {
    index: AtomicU32,
    end: u32,
    logs: ResultLog,
    /// Which indices are query events (need a response).
    query_indices: HashSet<usize>,
    /// When the first input event was handed out; start of the measured run.
    start_time: OnceLock<OffsetDateTime>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Report {
    pub apitoken: String,
    pub bench_id: String,
    pub bench_name: String,
    pub test: bool,
    pub timings: TimingReport,
    pub tracks: HashMap<String, TimingReport>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct TimingReport {
    pub count: usize,
    pub missing: usize,
    pub correct: usize,
    pub wrong: usize,
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

    #[error("Invalid characters in string {0}")]
    InvalidUtf8(#[from] FromUtf8Error),
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_ctx() -> (tempfile::TempDir, Context) {
        let dir = tempfile::tempdir().unwrap();
        let ctx = Context::new(&dir.path().join("data")).unwrap();
        (dir, ctx)
    }

    fn new_bench(ctx: &Context, max_batches: u32) -> String {
        ctx.create(BenchConfig {
            apitoken: "test".into(),
            name: "test".into(),
            test: true,
            max_batches: Some(max_batches),
            batch_size: None,
        })
        .unwrap()
    }

    /// Maps BALANCE requests to track "t" and stores expected outputs for `ids`.
    fn setup_track(ctx: &Context, ids: &[u32]) {
        set_meta(
            &ctx.sled,
            REQ_MAPPING_KEY,
            HashMap::from([("BALANCE", "t")]),
        )
        .unwrap();
        for &id in ids {
            ctx.set_output(
                id,
                format!("2025-01-01T00:00:00Z,{id},BALANCE,x").as_bytes(),
            )
            .unwrap();
        }
    }

    // --- ResultLog tests ---

    #[test]
    fn result_log_only_started_entries_are_reported() {
        let log = ResultLog::new(10, 4);
        log.start(0);
        log.start(1);
        log.put(1, b"data1".to_vec());
        log.put(2, b"not started".to_vec()); // ignored

        let results = log.to_completed_vec();
        assert_eq!(results.len(), 2);
        assert!(results[0].is_none());
        assert_eq!(results[1].as_ref().unwrap().data.as_ref(), b"data1");
    }

    #[test]
    fn result_log_uses_correct_timestamps() {
        let log = ResultLog::new(10, 4);
        log.start(0);
        let start_ts = OffsetDateTime::now_utc();
        std::thread::sleep(std::time::Duration::from_millis(2));
        log.put(0, b"data".to_vec());
        let end_ts = OffsetDateTime::now_utc();

        let results = log.to_completed_vec();
        let r = results[0].as_ref().unwrap();
        assert!(r.start <= start_ts, "start should be before mid-point");
        assert!(r.end >= start_ts, "end should be after mid-point");
        assert!(r.end <= end_ts, "end should be before or at end_ts");
        assert!(r.end > r.start, "end must be after start");
    }

    #[test]
    fn result_log_missing_result() {
        let log = ResultLog::new(10, 4);
        log.start(0);
        // Don't put a result for index 0
        let results = log.to_completed_vec();
        assert_eq!(results.len(), 1);
        assert!(results[0].is_none());
    }

    // --- Full flow tests ---

    #[test]
    fn next_batch_registers_only_query_events() {
        let (_dir, ctx) = temp_ctx();
        let bench_id = new_bench(&ctx, 4);

        // 2 transactions, 2 queries
        ctx.set_input(0, b"2025-01-01T00:00:01Z,,,,tx1,,").unwrap();
        ctx.set_input(1, b"2025-01-01T00:00:05Z,0,BALANCE,address=A,,,")
            .unwrap();
        ctx.set_input(2, b"2025-01-01T00:00:03Z,,,,tx2,,").unwrap();
        ctx.set_input(3, b"2025-01-01T00:00:06Z,1,BALANCE,address=A,,,")
            .unwrap();

        setup_track(&ctx, &[0, 1]);
        for _ in 0..4 {
            ctx.next_batch(&bench_id).unwrap();
        }
        // Answer only request 0
        ctx.result(&bench_id, 0, "2025-01-01T00:00:03Z,0,DUMMY,{}".into())
            .unwrap();

        let report = ctx.complete(&bench_id).unwrap();
        let q = report.tracks.get("t").unwrap();
        // Transactions are not logged, so they are not "missing"; only request 1 is
        assert_eq!(q.missing, 1);
        // The dummy answer differs from the expected output.
        assert_eq!((q.correct, q.wrong), (0, 1));
        assert_eq!(q.timings.iter().filter(|t| t.is_some()).count(), 1);
        assert_eq!(q.count, 2);
        // Mean and p99 are over the single answered request, not all requests.
        assert_ne!(q.latency_p99, Duration::ZERO);
        assert_eq!(q.latency_mean, q.latency_max);
    }

    #[test]
    fn unfetched_requests_count_as_missing() {
        let (_dir, ctx) = temp_ctx();
        let bench_id = new_bench(&ctx, 3);
        ctx.set_input(0, b"2025-01-01T00:00:01Z,,,,tx1,,").unwrap();
        ctx.set_input(1, b"2025-01-01T00:00:05Z,0,BALANCE,address=A,,,")
            .unwrap();
        ctx.set_input(2, b"2025-01-01T00:00:06Z,1,BALANCE,address=A,,,")
            .unwrap();
        setup_track(&ctx, &[0, 1]);

        // The client stops after request 0: request 1 is never handed out.
        ctx.next_batch(&bench_id).unwrap();
        ctx.next_batch(&bench_id).unwrap();
        ctx.result(&bench_id, 0, "2025-01-01T00:00:00Z,0,BALANCE,x".into())
            .unwrap();

        let report = ctx.complete(&bench_id).unwrap();
        // Counts are requests, not events (the transaction is not counted).
        assert_eq!((report.timings.count, report.timings.missing), (2, 1));
        let q = &report.tracks["t"];
        assert_eq!((q.count, q.missing), (2, 1));
        assert!(q.throughput > 0.0);
    }

    #[test]
    fn full_benchmark_flow() {
        let (_dir, ctx) = temp_ctx();
        let bench_id = new_bench(&ctx, 5);

        let input_lines = vec![
            "2025-01-01T00:00:01Z,,,,tx1,,",
            "2025-01-01T00:00:05Z,0,BALANCE,address=A,,,",
            "2025-01-01T00:00:03Z,,,,tx2,,",
            "2025-01-01T00:00:06Z,1,BALANCE,address=A,,,",
            "2025-01-01T00:00:05Z,,,,tx3,,",
        ];
        for (i, line) in input_lines.iter().enumerate() {
            ctx.set_input(i as u32, line.as_bytes()).unwrap();
        }

        setup_track(&ctx, &[0, 1]);
        let mut query_responses = 0;
        for _ in 0..5 {
            let line = ctx.next_batch(&bench_id).unwrap().unwrap();
            if !line.split(',').nth(1).unwrap().is_empty() {
                let request_id: usize = line.split(',').nth(1).unwrap().parse().unwrap();
                let output = "2025-01-01T00:00:05Z,0,DUMMY,{\"result\":42}".to_string();
                ctx.result(&bench_id, request_id, output).unwrap();
                query_responses += 1;
            }
        }
        assert_eq!(query_responses, 2);

        let report = ctx.complete(&bench_id).unwrap();
        let q = report.tracks.get("t").unwrap();
        assert_eq!(q.missing, 0);
        assert_eq!(q.timings.iter().filter(|t| t.is_some()).count(), 2);
        assert_eq!(q.count, 2);
    }

    #[test]
    fn query_only_metrics_filtered() {
        let (_dir, ctx) = temp_ctx();
        let bench_id = new_bench(&ctx, 4);

        // 2 transactions, 2 queries
        let inputs = vec![
            "2025-01-01T00:00:01Z,,,,tx1,,",
            "2025-01-01T00:00:05Z,0,BALANCE,address=A,,,",
            "2025-01-01T00:00:03Z,,,,tx2,,",
            "2025-01-01T00:00:06Z,1,BALANCE,address=A,,,",
        ];
        for (i, line) in inputs.iter().enumerate() {
            ctx.set_input(i as u32, line.as_bytes()).unwrap();
        }

        // Process all 4 events, respond only to query at index 1
        setup_track(&ctx, &[0, 1]);
        for _ in 0..4 {
            ctx.next_batch(&bench_id).unwrap();
        }
        ctx.result(
            &bench_id,
            0,
            "2025-01-01T00:00:03Z,0,DUMMY,{\"x\":1}".into(),
        )
        .unwrap();

        let report = ctx.complete(&bench_id).unwrap();
        let q = report.tracks.get("t").unwrap();
        // 1 completed query (index 1), 1 query without response (index 3)
        assert_eq!(q.missing, 1);
        assert_eq!(q.timings.iter().filter(|t| t.is_some()).count(), 1);
    }

    #[test]
    fn correct_counts_matching_expected_output() {
        let (_dir, ctx) = temp_ctx();
        let bench_id = new_bench(&ctx, 2);

        ctx.set_input(0, b"2025-01-01T00:00:01Z,,,,tx1,,").unwrap();
        ctx.set_input(1, b"2025-01-01T00:00:05Z,0,BALANCE,address=A,,,")
            .unwrap();
        setup_track(&ctx, &[0]);

        for _ in 0..2 {
            ctx.next_batch(&bench_id).unwrap();
        }
        ctx.result(&bench_id, 0, "2025-01-01T00:00:00Z,0,BALANCE,x".into())
            .unwrap();

        let report = ctx.complete(&bench_id).unwrap();
        let q = report.tracks.get("t").unwrap();
        assert_eq!((q.correct, q.wrong), (1, 0));
        assert_eq!(q.missing, 0);
    }

    #[test]
    fn set_input_uses_batch_id_as_key() {
        let (_dir, ctx) = temp_ctx();

        ctx.set_input(42, b"test-data").unwrap();
        let tree = ctx.sled.open_tree(INPUT).unwrap();
        assert_eq!(
            tree.get(42u32.to_be_bytes()).unwrap().map(|v| v.to_vec()),
            Some(b"test-data".to_vec())
        );

        // Verify set_input overwrites
        ctx.set_input(42, b"overwritten").unwrap();
        assert_eq!(
            tree.get(42u32.to_be_bytes()).unwrap().map(|v| v.to_vec()),
            Some(b"overwritten".to_vec())
        );
    }

    #[test]
    fn next_batch_returns_none_at_end() {
        let (_dir, ctx) = temp_ctx();
        let bench_id = new_bench(&ctx, 1);

        // No input loaded — first batch returns None
        assert!(ctx.next_batch(&bench_id).unwrap().is_none());
    }
}
