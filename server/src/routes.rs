//! The HTTP API. Route names follow `core/src/ffi.rs` one to one, so the
//! Swift app stays the reference for what each call means.

use axum::extract::State;
use axum::http::{header, StatusCode};
use axum::response::{Html, IntoResponse};
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::{json, Value};

use anicat_core::ffi::{FfiCatalog, SearchFilters, StreamRequest};

use crate::error::{ApiError, ApiJson, ApiPath, ApiQuery, ApiResult};
use crate::player::PlayRequest;
use crate::state::AppState;

const INDEX_HTML: &str = include_str!("../static/index.html");

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/", get(index))
        .route("/api/search", post(search))
        .route("/api/trending", get(trending))
        .route("/api/discover", get(discover))
        .route("/api/cinema/rows", get(cinema_rows))
        .route("/api/cinema/row/{kind}", get(cinema_row))
        .route("/api/cinema/list", get(cinema_list).post(set_cinema_list_status))
        .route("/api/cinema/{id}", get(cinema_detail))
        .route("/api/user-list", get(user_list))
        .route("/api/media/{id}", get(media_detail))
        .route("/api/progress/{id}", get(progress))
        .route("/api/activity", get(activity))
        .route("/api/play", post(play))
        .route("/api/stop", post(stop))
        .route("/api/resolve-progress/{catalog}/{id}/{episode}", get(resolve_progress))
        .route("/api/releases", get(releases))
        .route("/api/track-preference", post(track_preference))
        .route("/api/watched", post(watched))
        .route("/api/list/{entry}", delete(remove_from_list))
        .route("/api/cache", get(cache_bytes).delete(purge_cache))
        .route("/api/token", post(set_token))
        .route("/api/viewer", get(viewer))
        .route("/api/health", get(health))
        .route("/api/version", get(version))
        .route("/api/debug-report", get(debug_report))
        .with_state(state)
}

async fn index() -> Html<&'static str> {
    Html(INDEX_HTML)
}

fn default_limit() -> i32 {
    20
}

fn default_page() -> i32 {
    1
}

// ---- catalogs -------------------------------------------------------------

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum SearchKind {
    Anime,
    Cinema,
}

#[derive(Deserialize)]
struct SearchBody {
    query: String,
    kind: SearchKind,
    filters: Option<SearchFilters>,
    #[serde(default = "default_page")]
    page: i32,
    #[serde(default = "default_limit")]
    limit: i32,
}

async fn search(State(s): State<AppState>, ApiJson(b): ApiJson<SearchBody>) -> ApiResult<Json<Value>> {
    let rows = match b.kind {
        SearchKind::Anime => {
            let q = Some(b.query).filter(|q| !q.trim().is_empty());
            s.engine
                .search_catalog(q, Some("ANIME".into()), b.filters, Some(b.page))
                .await?
        }
        SearchKind::Cinema => s.engine.search_cinema(b.query, b.limit, b.page).await?,
    };
    Ok(Json(json!(rows)))
}

#[derive(Deserialize)]
struct TrendingQuery {
    format: Option<String>,
    #[serde(default = "default_limit")]
    limit: i32,
}

async fn trending(State(s): State<AppState>, ApiQuery(q): ApiQuery<TrendingQuery>) -> ApiResult<Json<Value>> {
    let rows = s.engine.trending("ANIME".into(), q.format, q.limit).await?;
    Ok(Json(json!(rows)))
}

#[derive(Deserialize)]
struct DiscoverQuery {
    status: Option<String>,
    season: Option<String>,
    season_year: Option<i32>,
    #[serde(default = "default_limit")]
    limit: i32,
}

async fn discover(State(s): State<AppState>, ApiQuery(q): ApiQuery<DiscoverQuery>) -> ApiResult<Json<Value>> {
    let rows = s
        .engine
        .discover("ANIME".into(), q.status, q.season, q.season_year, q.limit)
        .await?;
    Ok(Json(json!(rows)))
}

async fn cinema_rows(State(s): State<AppState>) -> Json<Value> {
    Json(json!(s.engine.cinema_row_kinds()))
}

#[derive(Deserialize)]
struct PageQuery {
    #[serde(default = "default_page")]
    page: i32,
}

async fn cinema_row(
    State(s): State<AppState>,
    ApiPath(kind): ApiPath<String>,
    ApiQuery(q): ApiQuery<PageQuery>,
) -> ApiResult<Json<Value>> {
    Ok(Json(json!(s.engine.cinema_row(kind, q.page).await?)))
}

#[derive(Deserialize)]
struct CatalogQuery {
    catalog: FfiCatalog,
}

/// `?catalog=tmdb_movie` or `tmdb_tv`: a TMDB id alone does not say which,
/// and the same number is a different title in each.
async fn cinema_detail(
    State(s): State<AppState>,
    ApiPath(id): ApiPath<i64>,
    ApiQuery(q): ApiQuery<CatalogQuery>,
) -> ApiResult<Json<Value>> {
    if q.catalog == FfiCatalog::Anilist || q.catalog == FfiCatalog::MangaDex {
        return Err(ApiError::bad_request("catalog must be tmdb_movie or tmdb_tv"));
    }
    Ok(Json(json!(s.engine.cinema_detail(q.catalog, id).await?)))
}

#[derive(Deserialize)]
struct StatusQuery {
    status: Option<String>,
}

async fn cinema_list(State(s): State<AppState>, ApiQuery(q): ApiQuery<StatusQuery>) -> ApiResult<Json<Value>> {
    let engine = s.engine.clone();
    let rows = tokio::task::spawn_blocking(move || engine.cinema_list(q.status))
        .await
        .map_err(|e| ApiError::internal(e.to_string()))??;
    Ok(Json(json!(rows)))
}

#[derive(Deserialize)]
struct CinemaStatusBody {
    catalog: FfiCatalog,
    catalog_id: i64,
    /// `null` removes the title from the local list.
    status: Option<String>,
}

async fn set_cinema_list_status(
    State(s): State<AppState>,
    ApiJson(b): ApiJson<CinemaStatusBody>,
) -> ApiResult<StatusCode> {
    s.writer.set_cinema_list_status(b.catalog, b.catalog_id, b.status).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
struct UserListQuery {
    #[serde(default = "current")]
    status: String,
}

fn current() -> String {
    "CURRENT".into()
}

async fn user_list(State(s): State<AppState>, ApiQuery(q): ApiQuery<UserListQuery>) -> ApiResult<Json<Value>> {
    if !s.signed_in() {
        return Err(ApiError::new(StatusCode::UNAUTHORIZED, "not signed in to AniList"));
    }
    Ok(Json(json!(s.engine.user_list(q.status, "ANIME".into()).await?)))
}

async fn media_detail(State(s): State<AppState>, ApiPath(id): ApiPath<i64>) -> ApiResult<Json<Value>> {
    Ok(Json(json!(s.engine.media_detail(id, false).await?)))
}

// ---- local history ----------------------------------------------------------

fn default_catalog() -> FfiCatalog {
    FfiCatalog::Anilist
}

#[derive(Deserialize)]
struct ProgressQuery {
    episode: i64,
    #[serde(default = "default_catalog")]
    catalog: FfiCatalog,
}

async fn progress(
    State(s): State<AppState>,
    ApiPath(id): ApiPath<i64>,
    ApiQuery(q): ApiQuery<ProgressQuery>,
) -> ApiResult<Json<Value>> {
    let engine = s.engine.clone();
    let hit = tokio::task::spawn_blocking(move || engine.get_progress(q.catalog, id, q.episode))
        .await
        .map_err(|e| ApiError::internal(e.to_string()))??;
    Ok(Json(json!(hit)))
}

#[derive(Deserialize)]
struct LimitQuery {
    #[serde(default = "default_limit")]
    limit: i32,
}

async fn activity(State(s): State<AppState>, ApiQuery(q): ApiQuery<LimitQuery>) -> ApiResult<Json<Value>> {
    let engine = s.engine.clone();
    let rows = tokio::task::spawn_blocking(move || engine.watch_activity(q.limit))
        .await
        .map_err(|e| ApiError::internal(e.to_string()))??;
    Ok(Json(json!(rows)))
}

// ---- playback ---------------------------------------------------------------

#[derive(Deserialize)]
struct PlayBody {
    #[serde(default = "default_catalog")]
    catalog: FfiCatalog,
    catalog_id: i64,
    episode: i64,
    title: Option<String>,
    #[serde(default)]
    prefer_dub: bool,
    /// A release picked from `/api/releases`; tried first.
    chosen_name: Option<String>,
    /// "Start over": ignore the recorded position.
    #[serde(default)]
    from_start: bool,
}


async fn play(State(s): State<AppState>, ApiJson(b): ApiJson<PlayBody>) -> ApiResult<Json<Value>> {
    let (mut start, mut duration) = (0.0_f64, None);
    if !b.from_start {
        let engine = s.engine.clone();
        let (catalog, id, ep) = (b.catalog, b.catalog_id, b.episode);
        let recorded = tokio::task::spawn_blocking(move || engine.get_progress(catalog, id, ep))
            .await
            .map_err(|e| ApiError::internal(e.to_string()))??;
        (start, duration) = crate::player::resume_point(recorded.as_ref());
    }
    let resume_fraction = crate::player::resume_fraction(start, duration);
    let req = StreamRequest {
        catalog: b.catalog,
        catalog_id: b.catalog_id,
        episode: b.episode,
        title: b.title.clone(),
        prefer_dub: b.prefer_dub,
        chosen_name: b.chosen_name,
        resume_fraction,
        preload: false,
    };
    let handle = s.engine.resolve_stream(req).await?;
    let request = PlayRequest {
        catalog: b.catalog,
        catalog_id: b.catalog_id,
        episode: b.episode,
        title: b.title,
        prefer_dub: b.prefer_dub,
        start_seconds: start,
        duration_seconds: duration,
    };
    s.player.play(&handle, &request).await.map_err(ApiError::internal)?;
    Ok(Json(json!({
        "stream": handle,
        "start_seconds": start,
    })))
}

async fn stop(State(s): State<AppState>) -> StatusCode {
    s.player.stop().await;
    StatusCode::NO_CONTENT
}

async fn resolve_progress(
    State(s): State<AppState>,
    ApiPath((catalog, id, episode)): ApiPath<(String, i64, i64)>,
) -> ApiResult<Json<Value>> {
    let catalog: FfiCatalog = serde_json::from_value(Value::String(catalog))
        .map_err(|e| ApiError::bad_request(format!("catalog: {e}")))?;
    Ok(Json(json!(s.engine.resolve_progress(catalog, id, episode))))
}

#[derive(Deserialize)]
struct ReleasesQuery {
    #[serde(default = "default_catalog")]
    catalog: FfiCatalog,
    catalog_id: i64,
    episode: i64,
    title: Option<String>,
    #[serde(default)]
    prefer_dub: bool,
}

async fn releases(State(s): State<AppState>, ApiQuery(q): ApiQuery<ReleasesQuery>) -> ApiResult<Json<Value>> {
    let rows = s
        .engine
        .list_release_candidates(q.catalog, q.catalog_id, q.episode, q.title)
        .await?;
    let remembered = s
        .engine
        .remembered_release_name(q.catalog, q.catalog_id, q.episode, q.prefer_dub);
    Ok(Json(json!({ "releases": rows, "remembered": remembered })))
}

#[derive(Deserialize)]
struct TrackPreferenceBody {
    #[serde(default = "default_catalog")]
    catalog: FfiCatalog,
    catalog_id: i64,
    audio_lang: Option<String>,
    subtitle_lang: Option<String>,
    subtitle_title: Option<String>,
}

async fn track_preference(
    State(s): State<AppState>,
    ApiJson(b): ApiJson<TrackPreferenceBody>,
) -> ApiResult<StatusCode> {
    s.writer
        .record_title_track_preference(b.catalog, b.catalog_id, b.audio_lang, b.subtitle_lang, b.subtitle_title)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
struct WatchedBody {
    #[serde(default = "default_catalog")]
    catalog: FfiCatalog,
    catalog_id: i64,
    episode: i64,
}

/// Marks the episode finished locally, then advances AniList's progress to
/// it when signed in. Both halves: the local one alone leaves AniList
/// behind, the AniList one alone leaves Up Next offering the episode again.
async fn watched(State(s): State<AppState>, ApiJson(b): ApiJson<WatchedBody>) -> ApiResult<Json<Value>> {
    s.writer
        .mark_episode_completed(b.catalog, b.catalog_id, b.episode)
        .await?;
    let synced = b.catalog == FfiCatalog::Anilist && s.signed_in();
    if synced {
        s.writer
            .update_list_entry(b.catalog_id, None, None, Some(b.episode))
            .await?;
    }
    Ok(Json(json!({ "anilist_synced": synced })))
}

async fn remove_from_list(State(s): State<AppState>, ApiPath(entry): ApiPath<i64>) -> ApiResult<StatusCode> {
    s.writer.remove_from_list(entry).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn cache_bytes(State(s): State<AppState>) -> Json<Value> {
    Json(json!({ "bytes": s.engine.stream_cache_bytes().await }))
}

async fn purge_cache(State(s): State<AppState>) -> ApiResult<Json<Value>> {
    s.engine.purge_stream_cache().await?;
    Ok(Json(json!({ "bytes": s.engine.stream_cache_bytes().await })))
}

// ---- account ----------------------------------------------------------------

#[derive(Deserialize)]
struct TokenBody {
    /// `null` or empty signs out.
    token: Option<String>,
}

/// Validates before persisting: a pasted token that AniList rejects must
/// not be written to config.json, or every later launch starts signed in
/// to nothing and every list call fails.
async fn set_token(State(s): State<AppState>, ApiJson(b): ApiJson<TokenBody>) -> ApiResult<Json<Value>> {
    let token = b.token.map(|t| t.trim().to_string()).filter(|t| !t.is_empty());
    let Some(token) = token else {
        s.engine.set_anilist_token(None);
        *s.token.lock().expect("token lock") = None;
        crate::config::save_token(&s.data_dir, None).map_err(|e| ApiError::internal(e.to_string()))?;
        return Ok(Json(json!({ "signed_in": false })));
    };
    let previous = s.token.lock().expect("token lock").clone();
    s.engine.set_anilist_token(Some(token.clone()));
    match s.engine.viewer_profile().await {
        Ok(profile) => {
            *s.token.lock().expect("token lock") = Some(token.clone());
            crate::config::save_token(&s.data_dir, Some(&token))
                .map_err(|e| ApiError::internal(e.to_string()))?;
            Ok(Json(json!({ "signed_in": true, "profile": profile })))
        }
        Err(e) => {
            s.engine.set_anilist_token(previous);
            Err(e.into())
        }
    }
}

async fn viewer(State(s): State<AppState>) -> ApiResult<Json<Value>> {
    if !s.signed_in() {
        return Ok(Json(json!({ "signed_in": false })));
    }
    let profile = s.engine.viewer_profile().await?;
    Ok(Json(json!({ "signed_in": true, "profile": profile })))
}

// ---- support ----------------------------------------------------------------

/// What the single-instance probe asks. `/api/version` can make the daily
/// GitHub request inline, so probing it let a second launch wait out that
/// request, time out, and start a second engine on the same registry.
async fn health() -> Json<Value> {
    Json(json!({ "app": "anicat", "version": crate::version::current() }))
}

async fn version(State(s): State<AppState>) -> Json<Value> {
    Json(json!(crate::version::check(&s.http, &s.data_dir).await))
}

async fn debug_report(State(s): State<AppState>) -> impl IntoResponse {
    let engine = s.engine.clone();
    let counts = tokio::task::spawn_blocking(move || {
        let count = |r: Result<usize, anicat_core::ffi::AnicatError>| match r {
            Ok(n) => n.to_string(),
            Err(e) => format!("error: {e}"),
        };
        [
            ("watch history rows", count(engine.watch_activity(i32::MAX).map(|v| v.len()))),
            ("remembered releases", count(engine.export_resolved_releases().map(|v| v.len()))),
            ("cinema list entries", count(engine.cinema_list(None).map(|v| v.len()))),
        ]
    })
    .await
    .map(|rows| rows.map(|(k, v)| format!("{k}: {v}")).join("\n"))
    .unwrap_or_else(|e| format!("registry counts failed: {e}"));
    let stream_port = match s.engine.stream_port().await {
        Ok(p) => p.to_string(),
        Err(e) => format!("error: {e}"),
    };
    let report = format!(
        "Anicat server {version}\nOS: {os} {arch}\nData dir: {dir}\nAPI port: {port}\nStream port: {stream_port}\n\
         Signed in to AniList: {signed_in}\nTMDB proxy compiled or set: {proxy}\nStream cache bytes: {cache}\n\
         Log redirected to file: {redirected}\n\n{counts}\n\n--- last 200 log lines ---\n{tail}\n",
        version = crate::version::current(),
        os = std::env::consts::OS,
        arch = std::env::consts::ARCH,
        dir = s.data_dir.display(),
        port = s.port,
        signed_in = s.signed_in(),
        proxy = crate::config::tmdb_proxy().is_some(),
        cache = s.engine.stream_cache_bytes().await,
        redirected = s.log_redirected,
        tail = crate::logging::tail(&s.data_dir, 200),
    );
    ([(header::CONTENT_TYPE, "text/plain; charset=utf-8")], report)
}
