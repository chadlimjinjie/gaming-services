//! sudoku-service — real-time PvP Sudoku WebSocket game server.
//!
//! Routes: `GET /healthz` (JSON health) and `GET /sudoku/ws` (WebSocket).
//! Mirrors the shape of connect-four-service, ported to Rust + axum.

mod game;
mod protocol;
mod room;
mod ws;

use axum::{routing::get, Json, Router};
use serde_json::json;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() {
    dotenvy::dotenv().ok();
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")))
        .init();

    let mgr = room::Manager::new();

    let app = Router::new()
        .route("/healthz", get(health))
        .route("/sudoku/ws", get(ws::handler))
        .with_state(mgr);

    let port = std::env::var("PORT").unwrap_or_else(|_| "4111".to_string());
    let addr = format!("[::]:{port}");
    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .unwrap_or_else(|e| panic!("failed to bind {addr}: {e}"));
    tracing::info!("sudoku-service listening on {addr}");
    axum::serve(listener, app).await.unwrap();
}

async fn health() -> Json<serde_json::Value> {
    Json(json!({ "status": "ok", "service": "sudoku-service" }))
}
