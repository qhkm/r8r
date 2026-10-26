/*
 * Copyright: Kitakod Ventures 2026
 * This file and its contents are licensed under the AGPLv3 License.
 * Please see the included NOTICE for copyright information and
 * LICENSE-AGPL for a copy of the license.
 */
//! Web Dashboard for r8r
//!
//! Provides a web interface for monitoring workflows and executions.
//! Built with React + Tailwind CSS + React Flow, served as static files.

use axum::{
    Router,
    http::StatusCode,
    response::{Html, IntoResponse, Response},
    routing::get,
};
use include_dir::{Dir, include_dir};
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

/// Dashboard assets (JS, CSS, fonts) embedded at compile time, so the
/// dashboard works from an installed binary with no network access.
static ASSETS: Dir<'static> = include_dir!("$CARGO_MANIFEST_DIR/src/dashboard/static/assets");

/// Serve embedded static assets (JS, CSS, fonts)
async fn serve_assets(axum::extract::Path(file): axum::extract::Path<String>) -> Response {
    // Lookup is by exact embedded path, so `..` segments can never escape.
    let Some(asset) = ASSETS.get_file(&file) else {
        return StatusCode::NOT_FOUND.into_response();
    };

    let mime_type = match asset.path().extension().and_then(|e| e.to_str()) {
        Some("js") => "application/javascript",
        Some("css") => "text/css",
        Some("html") => "text/html",
        Some("json") => "application/json",
        Some("png") => "image/png",
        Some("svg") => "image/svg+xml",
        Some("woff2") => "font/woff2",
        Some("woff") => "font/woff",
        _ => "application/octet-stream",
    };

    (
        [
            ("content-type", mime_type),
            // Vite file names carry a content hash.
            ("cache-control", "public, max-age=31536000, immutable"),
        ],
        asset.contents(),
    )
        .into_response()
}

/// Get the dashboard static files directory path
pub fn static_dir() -> std::path::PathBuf {
    Path::new(file!()).parent().unwrap().join("static")
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::extract::Path as AxumPath;

    #[tokio::test]
    async fn serves_embedded_assets_with_mime_type() {
        let font = ASSETS
            .files()
            .find(|f| f.path().extension().is_some_and(|e| e == "woff2"))
            .expect("bundled fonts are embedded");
        let name = font.path().to_str().unwrap().to_string();

        let resp = serve_assets(AxumPath(name)).await;
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(resp.headers()["content-type"], "font/woff2");
    }

    #[tokio::test]
    async fn rejects_unknown_and_traversal_paths() {
        for path in ["missing.js", "../index.html", "../../mod.rs", "/etc/passwd"] {
            let resp = serve_assets(AxumPath(path.to_string())).await;
            assert_eq!(resp.status(), StatusCode::NOT_FOUND, "{path}");
        }
    }
}
