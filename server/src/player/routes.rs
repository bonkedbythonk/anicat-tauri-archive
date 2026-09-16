//! The player's own routes, merged into the main router by `main.rs`.

use axum::extract::State;
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};

use super::prefs::{Prefs, PrefsPatch};
use super::Snapshot;
use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/player", get(player))
        .route("/api/player/stop", post(stop))
        .route("/api/prefs", get(get_prefs).post(set_prefs))
}

async fn player(State(s): State<AppState>) -> Json<Snapshot> {
    Json(s.player.snapshot())
}

async fn stop(State(s): State<AppState>) -> StatusCode {
    s.player.stop().await;
    StatusCode::NO_CONTENT
}

async fn get_prefs(State(s): State<AppState>) -> Json<Prefs> {
    Json(s.player.prefs())
}

/// Takes any subset of the fields and answers the whole, saved result. A
/// running mpv applies the change at once.
async fn set_prefs(State(s): State<AppState>, Json(patch): Json<PrefsPatch>) -> Json<Prefs> {
    Json(s.player.set_prefs(patch).await)
}
