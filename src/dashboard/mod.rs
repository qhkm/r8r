//! Web Dashboard for r8r
//!
//! Provides a web interface for monitoring workflows and executions.
//! Built with React + Tailwind CSS + React Flow, served as static files.

use axum::{
    http::StatusCode,
    response::{Html, IntoResponse, Response},
    routing::get,
    Router,
};
use std::path::Path;

/// Dashboard static files embedded at compile time
const INDEX_HTML: &str = include_str!("static/index.html");

/// Create dashboard routes
pub fn create_dashboard_routes() -> Router {
    Router::new()
        .route("/", get(serve_index))
        .route("/editor", get(serve_index))
        .route("/assets/{*file}", get(serve_assets))
        .fallback(serve_index)
}

/// Serve the main index.html (handles all SPA routes)
async fn serve_index() -> impl IntoResponse {
    Html(INDEX_HTML)
}

/// Serve static assets (JS, CSS files)
async fn serve_assets(axum::extract::Path(file): axum::extract::Path<String>) -> Response {
    let assets_dir = Path::new(file!())
        .parent()
        .unwrap()
        .join("static/assets");
    
    let file_path = assets_dir.join(&file);
    
    // Security: ensure the path doesn't escape the assets directory
    if !file_path.starts_with(&assets_dir) {
        return StatusCode::FORBIDDEN.into_response();
    }
    
    match tokio::fs::read(&file_path).await {
        Ok(contents) => {
            let mime_type = match file_path.extension().and_then(|e| e.to_str()) {
                Some("js") => "application/javascript",
                Some("css") => "text/css",
                Some("html") => "text/html",
                Some("json") => "application/json",
                Some("png") => "image/png",
                Some("svg") => "image/svg+xml",
                _ => "application/octet-stream",
            };
            
            ([("content-type", mime_type)], contents).into_response()
        }
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}

/// Get the dashboard static files directory path
pub fn static_dir() -> std::path::PathBuf {
    Path::new(file!())
        .parent()
        .unwrap()
        .join("static")
}
