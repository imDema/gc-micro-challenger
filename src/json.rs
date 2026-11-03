use std::collections::HashMap;

use axum::{
    body::Bytes,
    extract::{Json, Path, State},
    http::{self, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
    Router,
};
use serde::Serialize;
use time::format_description::well_known::Iso8601;

use crate::{plot, types::QueryResult, BenchConfig, Context, QueryId, QueryReport, Report};

pub fn router(ctx: Context) -> Router {
    Router::new()
        .route("/create", post(create))
        .route("/start/:bench-id", post(start))
        .route("/end/:bench-id", post(end))
        .route("/next_batch/:bench-id", get(next_batch))
        .route("/result/:q/:bench-id/:batch-id", post(result))
        .route("/get_result/:q/:bench-id/:batch-id", get(get_result))
        .route("/plot/:q/:bench-id/:batch-id", get(plot_result))
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
    throughput: f64,
    latency_mean: String,
    latency_min: String,
    latency_max: String,
    latency_p99: String,
}

#[derive(Serialize)]
struct SummaryResponse {
    apitoken: String,
    bench_name: String,
    bench_id: String,
    query_id: QueryId,
    count: usize,
    missing: usize,
    throughput: f64,
    latency_mean: String,
    latency_p99: String,
    latency_max: String,
}

impl SummaryResponse {
    pub fn from_report(r: &Report) -> Vec<Self> {
        r.queries
            .iter()
            .map(|(&query_id, q)| SummaryResponse {
                bench_id: r.bench_id.clone(),
                apitoken: r.apitoken.clone(),
                bench_name: r.bench_name.clone(),
                query_id,
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

impl From<QueryReport> for ReportResponse {
    fn from(value: QueryReport) -> Self {
        Self {
            count: value.count,
            missing: value.missing,
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
    Json(body): Json<BenchConfig>,
) -> Result<Json<String>, (StatusCode, String)> {
    let r = ctx.create(body).map_err(server_err)?;
    tracing::info!("created bench {r}");
    Ok(Json(r))
}

async fn start(
    State(_ctx): State<Context>,
    Path(_id): Path<String>,
) -> Result<StatusCode, (StatusCode, String)> {
    // let r = ctx.sta(body).map_err(server_err)?;
    Ok(StatusCode::OK)
}

async fn end(
    State(ctx): State<Context>,
    Path(id): Path<String>,
) -> Result<Json<HashMap<QueryId, ReportResponse>>, (StatusCode, String)> {
    let r = ctx.complete(&id).map_err(server_err)?;
    tracing::info!("finished bench {id}");

    let resp = r.queries.into_iter().map(|(k, v)| (k, v.into())).collect();

    Ok(Json(resp))
}

async fn next_batch(
    State(ctx): State<Context>,
    Path(id): Path<String>,
) -> Result<Vec<u8>, (StatusCode, String)> {
    let r = ctx.next_batch(&id).map_err(server_err)?;
    r.ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            format!("no more batches for this benchmark"),
        )
    })
}

async fn result(
    State(ctx): State<Context>,
    Path((q, id, batch_id)): Path<(u8, String, usize)>,
    body: Bytes,
) -> Result<Json<ResultResponse>, (StatusCode, String)> {
    let r = ctx
        .result(&id, q, batch_id, body.to_vec())
        .map_err(server_err)?;
    Ok(Json(ResultResponse {
        timestamp: r.and_then(|ts| ts.format(&Iso8601::DEFAULT).ok())
    }))
}

async fn get_result(
    State(ctx): State<Context>,
    Path((q, id, batch_id)): Path<(u8, String, usize)>,
) -> Result<Bytes, (StatusCode, String)> {
    let r = ctx.get_result(&id, q, batch_id).map_err(server_err)?;
    r.map(|x| x.into())
        .ok_or_else(|| (StatusCode::NOT_FOUND, "Not found".into()))
}

async fn plot_result(
    State(ctx): State<Context>,
    Path((q, id, batch_id)): Path<(u8, String, usize)>,
) -> Result<Response, (StatusCode, String)> {
    let r = ctx.get_result(&id, q, batch_id).map_err(server_err)?;

    let Some(r) = r else {
        return Err((StatusCode::NOT_FOUND, "Not found".into()));
    };

    let r = match rmp_serde::from_slice::<QueryResult>(&r) {
        Ok(r) => r,
        Err(e) => {
            return Err((
                StatusCode::BAD_REQUEST,
                format!("deserialization error {e}"),
            ))
        }
    };

    let plot = plot::plot_centroids(r).map_err(server_err)?;

    Ok(Response::builder()
        .status(StatusCode::OK)
        .header(http::header::CONTENT_TYPE, "image/svg+xml; charset=utf-8")
        .header(
            http::header::CONTENT_DISPOSITION,
            format!("inline; filename={batch_id}_{q}_{id}.svg"),
        )
        .body(plot)
        .map_err(server_err)?
        .into_response())
}

async fn history(
    State(ctx): State<Context>,
) -> Result<Json<Vec<SummaryResponse>>, (StatusCode, String)> {
    let r = ctx.history().map_err(server_err)?;

    let mut summary: Vec<SummaryResponse> = r
        .values()
        .map(SummaryResponse::from_report)
        .flatten()
        .collect();
    summary.sort_by(|a, b| a.bench_id.cmp(&b.bench_id).reverse());

    Ok(Json(summary))
}

pub fn server_err(e: impl Into<Box<dyn std::error::Error>>) -> (StatusCode, String) {
    (StatusCode::INTERNAL_SERVER_ERROR, e.into().to_string())
}
