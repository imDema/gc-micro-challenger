use super::Context;
use axum::{
    body::Bytes,
    extract::{Json, Path, State},
    http::StatusCode,
    routing::post,
    Router,
};

pub fn router(ctx: Context) -> Router {
    Router::new()
        .route("/set-input/:q/:batch-id", post(set_input))
        .route("/set-output/:q/:batch-id", post(set_output))
        .with_state(ctx)
}

async fn set_input(
    State(ctx): State<Context>,
    Path((q, batch_id)): Path<(u8, usize)>,
    body: Bytes,
) -> Result<StatusCode, (StatusCode, String)> {
    ctx.set_input(q, batch_id, &body[..])
        .map_err(super::json::server_err)?;
    Ok(StatusCode::OK)
}

async fn set_output(
    State(ctx): State<Context>,
    Path((q, batch_id)): Path<(u8, usize)>,
    body: Bytes,
) -> Result<StatusCode, (StatusCode, String)> {
    ctx.set_output(q, batch_id, &body[..])
        .map_err(super::json::server_err)?;
    Ok(StatusCode::OK)
}
