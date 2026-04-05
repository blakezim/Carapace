//! Axum route handlers for the Gmail proxy HTTP API.
//!
//! Endpoints:
//!   GET  /search?q=<query>&max=<n>&page_token=<token>
//!   GET  /message/{id}
//!   GET  /thread/{id}
//!   POST /drafts          body: { to, subject, body, cc? }
//!   GET  /health

use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::Json;
use futures::stream::{self, StreamExt};
use serde::Deserialize;
use tokio::sync::RwLock;

use crate::auth::TokenManager;
use crate::gmail::client::GmailClient;
use crate::gmail::types::CreateDraftRequest;
use crate::scrub::content::ContentScrubber;
use crate::scrub::labels::LabelFilter;
use crate::scrub::query::{parse_query, validate_query};

pub struct AppState {
    pub gmail: Arc<GmailClient>,
    pub label_filter: Arc<LabelFilter>,
    pub scrubber: Arc<ContentScrubber>,
    pub allowed_operators: Vec<String>,
    pub blocked_label: String,
    pub max_query_depth: usize,
    pub search_concurrency: usize,
    pub token_manager: Arc<TokenManager>,
    pub start_time: std::time::Instant,
}

#[derive(Deserialize)]
pub struct SearchParams {
    pub q: Option<String>,
    pub max: Option<u32>,
    pub page_token: Option<String>,
}

pub fn build_router(state: Arc<AppState>) -> axum::Router {
    axum::Router::new()
        .route("/search", axum::routing::get(search_handler))
        .route("/message/{id}", axum::routing::get(get_message_handler))
        .route("/thread/{id}", axum::routing::get(get_thread_handler))
        .route("/drafts", axum::routing::post(create_draft_handler))
        .route("/health", axum::routing::get(health_handler))
        .with_state(state)
}

// ---------------------------------------------------------------------------
// GET /search
// ---------------------------------------------------------------------------

async fn search_handler(
    State(state): State<Arc<AppState>>,
    Query(params): Query<SearchParams>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let raw_query = match params.q {
        Some(q) if !q.trim().is_empty() => q,
        _ => {
            return Err((
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({
                    "error": "Missing required parameter 'q'",
                    "hint": "Provide a Gmail search query using the 'q' parameter"
                })),
            ));
        }
    };

    let max = params.max.unwrap_or(20).min(100);
    let page_token = params.page_token.as_deref();

    // Parse and validate the query through the AST-based security layer.
    let ast = match parse_query(&raw_query) {
        Ok(ast) => ast,
        Err(e) => {
            return Err((
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({
                    "error": e.message,
                    "hint": e.hint,
                    "query": e.query
                })),
            ));
        }
    };

    let allowed_ops: Vec<&str> = state.allowed_operators.iter().map(|s| s.as_str()).collect();
    if let Err(e) = validate_query(&ast, &allowed_ops, &state.blocked_label, state.max_query_depth) {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": e.message,
                "hint": e.hint,
                "query": raw_query
            })),
        ));
    }

    let secured_query = state.label_filter.secure_query_string(&ast);

    let search_result = state
        .gmail
        .search(&secured_query, max, page_token)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": format!("Gmail search failed: {e}")})),
            )
        })?;

    let refs = search_result.messages.unwrap_or_default();
    let ids: Vec<String> = refs.iter().map(|r| r.id.clone()).collect();

    // Fetch messages concurrently, filter and scrub.
    let fetched: Vec<_> = stream::iter(ids)
        .map(|id| {
            let gmail = state.gmail.clone();
            async move { gmail.get_message(&id).await }
        })
        .buffer_unordered(state.search_concurrency)
        .collect()
        .await;

    let mut messages = Vec::new();
    for result in fetched {
        let msg = match result {
            Ok(m) => m,
            Err(_) => continue,
        };
        let labels = msg.label_ids.clone().unwrap_or_default();
        if state.label_filter.is_message_blocked(&labels) {
            continue;
        }
        let from = msg.header("From").unwrap_or("");
        if state.scrubber.check_sender(from).is_blocked() {
            continue;
        }
        let body = msg.extract_text_body().unwrap_or_default();
        let scrubbed = state.scrubber.scrub_body(&body);
        messages.push(msg.to_sanitized(scrubbed));
    }

    Ok(Json(serde_json::json!({
        "messages": messages,
        "next_page_token": search_result.next_page_token,
        "result_size_estimate": search_result.result_size_estimate
    })))
}

// ---------------------------------------------------------------------------
// GET /message/{id}
// ---------------------------------------------------------------------------

async fn get_message_handler(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let msg = state.gmail.get_message(&id).await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": format!("Failed to fetch message: {e}")})),
        )
    })?;

    let labels = msg.label_ids.clone().unwrap_or_default();
    if state.label_filter.is_message_blocked(&labels) {
        return Err((
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": "Message not found"})),
        ));
    }

    let from = msg.header("From").unwrap_or("").to_string();
    if state.scrubber.check_sender(&from).is_blocked() {
        return Err((
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": "Message not found"})),
        ));
    }

    let body = msg.extract_text_body().unwrap_or_default();
    let scrubbed = state.scrubber.scrub_body(&body);
    let sanitized = msg.to_sanitized(scrubbed);

    Ok(Json(serde_json::to_value(sanitized).unwrap()))
}

// ---------------------------------------------------------------------------
// GET /thread/{id}
// ---------------------------------------------------------------------------

async fn get_thread_handler(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let thread = state.gmail.get_thread(&id).await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": format!("Failed to fetch thread: {e}")})),
        )
    })?;

    let all_messages = thread.messages.unwrap_or_default();
    let mut sanitized_messages = Vec::new();

    for msg in &all_messages {
        let labels = msg.label_ids.clone().unwrap_or_default();
        if state.label_filter.is_message_blocked(&labels) {
            continue;
        }
        let from = msg.header("From").unwrap_or("");
        if state.scrubber.check_sender(from).is_blocked() {
            continue;
        }
        let body = msg.extract_text_body().unwrap_or_default();
        let scrubbed = state.scrubber.scrub_body(&body);
        sanitized_messages.push(msg.to_sanitized(scrubbed));
    }

    Ok(Json(serde_json::json!({
        "thread_id": id,
        "messages": sanitized_messages
    })))
}

// ---------------------------------------------------------------------------
// POST /drafts
// ---------------------------------------------------------------------------

async fn create_draft_handler(
    State(state): State<Arc<AppState>>,
    Json(req): Json<CreateDraftRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    if req.to.trim().is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "Missing required field: 'to'"})),
        ));
    }
    if req.subject.trim().is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "Missing required field: 'subject'"})),
        ));
    }

    let draft = state
        .gmail
        .create_draft(&req.to, &req.subject, &req.body, req.cc.as_deref())
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": format!("Failed to create draft: {e}")})),
            )
        })?;

    tracing::info!(draft_id = %draft.id, to = %req.to, subject = %req.subject, "draft created");

    Ok(Json(serde_json::json!({
        "draft_id": draft.id,
        "message_id": draft.message.id,
        "thread_id": draft.message.thread_id
    })))
}

// ---------------------------------------------------------------------------
// GET /health
// ---------------------------------------------------------------------------

async fn health_handler(State(state): State<Arc<AppState>>) -> Json<serde_json::Value> {
    let token_valid = state.token_manager.is_valid().await;
    let expires_in = state.token_manager.expires_in_secs().await;
    let uptime_secs = state.start_time.elapsed().as_secs();

    Json(serde_json::json!({
        "status": "ok",
        "uptime_secs": uptime_secs,
        "token": {
            "valid": token_valid,
            "expires_in_secs": expires_in
        }
    }))
}
