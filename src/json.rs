use std::collections::HashMap;

use axum::{
    Router,
    extract::{Path, State},
    http::StatusCode,
    routing::{get, post},
};
use serde::Serialize;
use time::format_description::well_known::Iso8601;

use crate::{BenchConfig, Context, Report, TimingReport};

pub fn router(ctx: Context) -> Router {
    Router::new()
        .route("/create", post(create))
        .route("/start/:bench-id", post(start))
        .route("/end/:bench-id", post(end))
        .route("/next_batch/:bench-id", get(next_batch))
        .route("/result/:bench-id/:batch-id", post(result))
        .route("/history", get(history))
        .with_state(ctx)
}

#[derive(Serialize)]
struct ResultResponse {
    timestamp: Option<String>,
}

#[derive(Serialize)]
struct ReportResponse {
    count: usize,
    missing: usize,
    correct: usize,
    wrong: usize,
    throughput: f64,
    latency_mean: String,
    latency_min: String,
    latency_max: String,
    latency_p99: String,
}

#[derive(Serialize)]
struct EndResponse {
    overall: ReportResponse,
    tracks: HashMap<String, ReportResponse>,
}

#[derive(Serialize)]
struct SummaryResponse {
    apitoken: String,
    bench_name: String,
    bench_id: String,
    query_id: String,
    count: usize,
    missing: usize,
    throughput: f64,
    latency_mean: String,
    latency_p99: String,
    latency_max: String,
}

impl SummaryResponse {
    pub fn from_report(r: &Report) -> Vec<Self> {
        r.tracks
            .iter()
            .map(|(query_id, q)| SummaryResponse {
                bench_id: r.bench_id.clone(),
                apitoken: r.apitoken.clone(),
                bench_name: r.bench_name.clone(),
                query_id: query_id.clone(),
                count: q.count,
                missing: q.missing,
                throughput: q.throughput,
                latency_mean: q.latency_mean.to_string(),
                latency_p99: q.latency_p99.to_string(),
                latency_max: q.latency_max.to_string(),
            })
            .collect()
    }
}

impl From<TimingReport> for ReportResponse {
    fn from(value: TimingReport) -> Self {
        Self {
            count: value.count,
            missing: value.missing,
            correct: value.correct,
            wrong: value.wrong,
            throughput: value.throughput,
            latency_mean: value.latency_mean.to_string(),
            latency_min: value.latency_min.to_string(),
            latency_max: value.latency_max.to_string(),
            latency_p99: value.latency_p99.to_string(),
        }
    }
}

async fn create(
    State(ctx): State<Context>,
    axum::Json(body): axum::Json<BenchConfig>,
) -> Result<axum::Json<String>, (StatusCode, String)> {
    let r = ctx.create(body).map_err(server_err)?;
    tracing::info!("created bench {r}");
    Ok(axum::Json(r))
}

async fn start() -> Result<StatusCode, (StatusCode, String)> {
    Ok(StatusCode::OK)
}

async fn end(
    State(ctx): State<Context>,
    Path(id): Path<String>,
) -> Result<axum::Json<EndResponse>, (StatusCode, String)> {
    let r = ctx.complete(&id).map_err(server_err)?;
    tracing::info!("finished bench {id}");

    let resp = EndResponse {
        overall: r.timings.into(),
        tracks: r.tracks.into_iter().map(|(k, v)| (k, v.into())).collect(),
    };

    Ok(axum::Json(resp))
}

async fn next_batch(
    State(ctx): State<Context>,
    Path(id): Path<String>,
) -> Result<String, (StatusCode, String)> {
    let r = ctx.next_batch(&id).map_err(server_err)?;
    match r {
        Some(line) => {
            tracing::debug!("next_batch: {}", &line[..20.min(line.len())]);
            Ok(line)
        }
        None => Err((
            StatusCode::NOT_FOUND,
            "no more batches for this benchmark".into(),
        )),
    }
}

async fn result(
    State(ctx): State<Context>,
    Path((id, batch_id)): Path<(String, usize)>,
    body: axum::body::Bytes,
) -> Result<axum::Json<ResultResponse>, (StatusCode, String)> {
    let output_csv = String::from_utf8(body.to_vec()).map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            format!("invalid UTF-8 in result: {e}"),
        )
    })?;

    let r = ctx.result(&id, batch_id, output_csv).map_err(server_err)?;
    Ok(axum::Json(ResultResponse {
        timestamp: r.and_then(|ts| ts.format(&Iso8601::DEFAULT).ok()),
    }))
}

async fn history(
    State(ctx): State<Context>,
) -> Result<axum::Json<Vec<SummaryResponse>>, (StatusCode, String)> {
    let r = ctx.history().map_err(server_err)?;

    let mut summary: Vec<SummaryResponse> =
        r.values().flat_map(SummaryResponse::from_report).collect();
    summary.sort_by(|a, b| a.bench_id.cmp(&b.bench_id).reverse());

    Ok(axum::Json(summary))
}

pub fn server_err(e: impl Into<Box<dyn std::error::Error>>) -> (StatusCode, String) {
    (StatusCode::INTERNAL_SERVER_ERROR, e.into().to_string())
}
