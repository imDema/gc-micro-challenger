use super::Context;
use axum::{
    body::Bytes,
    extract::{Path, State},
    http::StatusCode,
    routing::post,
    Router,
};

pub fn router(ctx: Context) -> Router {
    Router::new()
        .route("/set-input/:batch-id", post(set_input))
        .route("/set-output/:batch-id", post(set_output))
        .with_state(ctx)
}

async fn set_input(
    State(ctx): State<Context>,
    Path(batch_id): Path<usize>,
    body: Bytes,
) -> Result<StatusCode, (StatusCode, String)> {
    ctx.set_input(batch_id as u32, &body)
        .map_err(super::json::server_err)?;
    Ok(StatusCode::OK)
}

async fn set_output(
    State(ctx): State<Context>,
    Path(batch_id): Path<usize>,
    body: Bytes,
) -> Result<StatusCode, (StatusCode, String)> {
    ctx.set_output(batch_id as u32, &body)
        .map_err(super::json::server_err)?;
    Ok(StatusCode::OK)
}
