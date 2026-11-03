use axum::http::{header, StatusCode};
use axum::{
    response::{IntoResponse, Response},
    routing::get,
    Router,
};
use rust_embed::Embed;

#[derive(Embed)]
#[folder = "html/"]
struct Asset;
pub struct StaticFile<T>(pub T);

impl<T> IntoResponse for StaticFile<T>
where
    T: Into<String>,
{
    fn into_response(self) -> Response {
        let path = self.0.into();

        match Asset::get(path.as_str()) {
            Some(content) => ([(header::CONTENT_TYPE, "text/html")], content.data).into_response(),
            None => (StatusCode::NOT_FOUND, "404 Not Found").into_response(),
        }
    }
}

pub fn router() -> Router {
    Router::new().route("/dash", get(dash))
}

async fn dash() -> impl IntoResponse {
    StaticFile("dash.html")
}
