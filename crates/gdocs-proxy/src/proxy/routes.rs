//! Axum route handlers for the Google Docs proxy HTTP API.
//!
//! Endpoints:
//!   GET  /search?q=<query>&max=<n>&page_token=<token>  — Search Drive for files
//!   GET  /doc/{id}                                      — Read a Google Doc (structured)
//!   GET  /file/{id}                                     — Get file metadata
//!   POST /docs                                          — Create a new Google Doc
//!   POST /docs/copy/{id}                                — Copy a file
//!   PUT  /doc/{id}                                      — Append text to a doc
//!   GET  /health                                        — Token health check
//!
//! NOT exposed: delete, share, permission changes, move to trash.

use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::Json;
use serde::Deserialize;

use crate::auth::TokenManager;
use crate::docs::client::DocsClient;
use crate::docs::types::{CreateDocRequest, UpdateDocRequest};

pub struct AppState {
    pub docs: Arc<DocsClient>,
    pub token_manager: Arc<TokenManager>,
    pub scrub_patterns: Vec<regex::Regex>,
    pub strip_links: bool,
    pub start_time: std::time::Instant,
}

#[derive(Deserialize)]
pub struct SearchParams {
    pub q: Option<String>,
    pub max: Option<u32>,
    pub page_token: Option<String>,
    /// Restrict to Google Docs only. Defaults to false (all file types).
    pub docs_only: Option<bool>,
}

#[derive(Deserialize)]
pub struct CopyParams {
    pub title: Option<String>,
}

pub fn build_router(state: Arc<AppState>) -> axum::Router {
    axum::Router::new()
        .route("/search", axum::routing::get(search_handler))
        .route("/doc/{id}", axum::routing::get(get_doc_handler))
        .route("/doc/{id}", axum::routing::put(update_doc_handler))
        .route("/file/{id}", axum::routing::get(get_file_handler))
        .route("/docs", axum::routing::post(create_doc_handler))
        .route("/docs/copy/{id}", axum::routing::post(copy_handler))
        .route("/folders", axum::routing::post(create_folder_handler))
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
    let raw_query = params.q.unwrap_or_default();
    let max = params.max.unwrap_or(20).min(100);
    let page_token = params.page_token.as_deref();

    // Build Drive query. If docs_only, restrict to Google Docs MIME type.
    let query = if params.docs_only.unwrap_or(false) {
        if raw_query.is_empty() {
            "mimeType = 'application/vnd.google-apps.document'".to_string()
        } else {
            format!("({raw_query}) and mimeType = 'application/vnd.google-apps.document'")
        }
    } else {
        raw_query
    };

    let result = state
        .docs
        .search(&query, max, page_token)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": format!("Drive search failed: {e}")})),
            )
        })?;

    let files: Vec<_> = result
        .files
        .unwrap_or_default()
        .iter()
        .filter(|f| !f.trashed.unwrap_or(false))
        .map(|f| f.to_result())
        .collect();

    Ok(Json(serde_json::json!({
        "files": files,
        "next_page_token": result.next_page_token
    })))
}

// ---------------------------------------------------------------------------
// GET /doc/{id}
// ---------------------------------------------------------------------------

async fn get_doc_handler(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    // First, get the file metadata to determine the type.
    let file = state.docs.get_file(&id).await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": format!("Failed to fetch file metadata: {e}")})),
        )
    })?;

    let mime_type = &file.mime_type;

    match mime_type.as_str() {
        "application/vnd.google-apps.document" => {
            // Google Doc — use Docs API for structured content.
            let doc = state.docs.get_document(&id).await.map_err(|e| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({"error": format!("Failed to fetch document: {e}")})),
                )
            })?;

            let mut structured = doc.to_structured();
            scrub_content(&mut structured, &state.scrub_patterns, state.strip_links);
            Ok(Json(serde_json::to_value(structured).unwrap()))
        }

        "application/vnd.google-apps.spreadsheet" => {
            // Google Sheet — use Sheets API.
            let raw = state.docs.get_spreadsheet(&id).await.map_err(|e| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({"error": format!("Failed to fetch spreadsheet: {e}")})),
                )
            })?;

            let structured = convert_spreadsheet(&raw);
            Ok(Json(structured))
        }

        "application/vnd.google-apps.presentation" => {
            // Google Slides — use Slides API.
            let raw = state.docs.get_presentation(&id).await.map_err(|e| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({"error": format!("Failed to fetch presentation: {e}")})),
                )
            })?;

            let structured = convert_presentation(&raw);
            Ok(Json(structured))
        }

        "application/vnd.google-apps.form" => {
            // Google Form — use Forms API.
            let form = state.docs.get_form(&id).await.map_err(|e| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({"error": format!("Failed to fetch form: {e}")})),
                )
            })?;

            // Also fetch responses.
            let responses = state.docs.get_form_responses(&id).await.ok();

            let structured = convert_form(&form, responses.as_ref());
            Ok(Json(structured))
        }

        _ => {
            Err((
                StatusCode::UNPROCESSABLE_ENTITY,
                Json(serde_json::json!({
                    "error": format!(
                        "Cannot read file of type '{}'. Supported types: Google Docs, Sheets, Slides, and Forms. \
                         PDFs, images, and other binary files are not supported.",
                        mime_type
                    ),
                    "file_name": file.name,
                    "mime_type": mime_type,
                })),
            ))
        }
    }
}

// ---------------------------------------------------------------------------
// GET /file/{id}
// ---------------------------------------------------------------------------

async fn get_file_handler(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let file = state.docs.get_file(&id).await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": format!("Failed to fetch file: {e}")})),
        )
    })?;

    if file.trashed.unwrap_or(false) {
        return Err((
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": "File not found"})),
        ));
    }

    Ok(Json(serde_json::to_value(file.to_result()).unwrap()))
}

// ---------------------------------------------------------------------------
// POST /folders
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
pub struct CreateFolderRequest {
    pub name: String,
    pub parent_id: Option<String>,
}

async fn create_folder_handler(
    State(state): State<Arc<AppState>>,
    Json(req): Json<CreateFolderRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    if req.name.trim().is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "Missing required field: 'name'"})),
        ));
    }

    let folder = state
        .docs
        .create_folder(&req.name, req.parent_id.as_deref())
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": format!("Failed to create folder: {e}")})),
            )
        })?;

    tracing::info!(folder_id = %folder.id, name = %folder.name, "folder created");

    Ok(Json(serde_json::json!({
        "id": folder.id,
        "name": folder.name,
        "mime_type": folder.mime_type,
        "web_view_link": folder.web_view_link
    })))
}

// ---------------------------------------------------------------------------
// POST /docs
// ---------------------------------------------------------------------------

async fn create_doc_handler(
    State(state): State<Arc<AppState>>,
    Json(req): Json<CreateDocRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    if req.title.trim().is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "Missing required field: 'title'"})),
        ));
    }

    let doc = state.docs.create_document(&req.title, req.folder_id.as_deref()).await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": format!("Failed to create document: {e}")})),
        )
    })?;

    // If initial content was provided, append it.
    if let Some(ref content) = req.content {
        if !content.is_empty() {
            state
                .docs
                .append_text(&doc.document_id, content)
                .await
                .map_err(|e| {
                    (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(serde_json::json!({"error": format!("Document created but failed to add content: {e}")})),
                    )
                })?;
        }
    }

    tracing::info!(doc_id = %doc.document_id, title = %doc.title, "document created");

    Ok(Json(serde_json::json!({
        "document_id": doc.document_id,
        "title": doc.title,
        "web_view_link": format!("https://docs.google.com/document/d/{}/edit", doc.document_id)
    })))
}

// ---------------------------------------------------------------------------
// POST /docs/copy/{id}
// ---------------------------------------------------------------------------

async fn copy_handler(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Query(params): Query<CopyParams>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let new_file = state
        .docs
        .copy_file(&id, params.title.as_deref())
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": format!("Failed to copy file: {e}")})),
            )
        })?;

    tracing::info!(
        source_id = %id,
        new_id = %new_file.id,
        new_name = %new_file.name,
        "file copied"
    );

    Ok(Json(serde_json::json!({
        "id": new_file.id,
        "name": new_file.name,
        "mime_type": new_file.mime_type,
        "web_view_link": new_file.web_view_link
    })))
}

// ---------------------------------------------------------------------------
// PUT /doc/{id}
// ---------------------------------------------------------------------------

async fn update_doc_handler(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(req): Json<UpdateDocRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    if let Some(ref text) = req.append_text {
        if !text.is_empty() {
            state.docs.append_text(&id, text).await.map_err(|e| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({"error": format!("Failed to update document: {e}")})),
                )
            })?;

            tracing::info!(doc_id = %id, bytes = text.len(), "text appended");
        }
    }

    Ok(Json(serde_json::json!({
        "status": "ok",
        "document_id": id
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

// ---------------------------------------------------------------------------
// Content scrubbing
// ---------------------------------------------------------------------------

use crate::docs::types::{ContentBlock, StructuredDoc};

fn scrub_content(doc: &mut StructuredDoc, patterns: &[regex::Regex], strip_links: bool) {
    for block in &mut doc.content {
        match block {
            ContentBlock::Heading { text, .. } => {
                *text = scrub_text(text, patterns);
            }
            ContentBlock::Paragraph { text, links } => {
                *text = scrub_text(text, patterns);
                if strip_links {
                    links.clear();
                }
            }
            ContentBlock::Table { rows } => {
                for row in rows {
                    for cell in row {
                        *cell = scrub_text(cell, patterns);
                    }
                }
            }
        }
    }
}

fn scrub_text(text: &str, patterns: &[regex::Regex]) -> String {
    let mut result = text.to_string();
    for pat in patterns {
        result = pat.replace_all(&result, "[REDACTED]").to_string();
    }
    result
}

// ---------------------------------------------------------------------------
// Spreadsheet → structured JSON
// ---------------------------------------------------------------------------

/// Convert a raw Google Sheets API response into a readable structured format.
fn convert_spreadsheet(raw: &serde_json::Value) -> serde_json::Value {
    let title = raw.get("properties")
        .and_then(|p| p.get("title"))
        .and_then(|t| t.as_str())
        .unwrap_or("Untitled Spreadsheet");

    let spreadsheet_id = raw.get("spreadsheetId")
        .and_then(|v| v.as_str())
        .unwrap_or("");

    let mut sheets = Vec::new();

    if let Some(raw_sheets) = raw.get("sheets").and_then(|s| s.as_array()) {
        for sheet in raw_sheets {
            let sheet_title = sheet
                .pointer("/properties/title")
                .and_then(|t| t.as_str())
                .unwrap_or("Sheet");

            let mut rows: Vec<Vec<String>> = Vec::new();

            if let Some(grid_data) = sheet.get("data").and_then(|d| d.as_array()) {
                for grid in grid_data {
                    if let Some(row_data) = grid.get("rowData").and_then(|r| r.as_array()) {
                        for row in row_data {
                            let mut cells = Vec::new();
                            if let Some(values) = row.get("values").and_then(|v| v.as_array()) {
                                for cell in values {
                                    let text = cell
                                        .pointer("/formattedValue")
                                        .and_then(|v| v.as_str())
                                        .unwrap_or("")
                                        .to_string();
                                    cells.push(text);
                                }
                            }
                            // Skip completely empty rows.
                            if cells.iter().any(|c| !c.is_empty()) {
                                rows.push(cells);
                            }
                        }
                    }
                }
            }

            sheets.push(serde_json::json!({
                "name": sheet_title,
                "rows": rows
            }));
        }
    }

    serde_json::json!({
        "type": "spreadsheet",
        "spreadsheet_id": spreadsheet_id,
        "title": title,
        "sheets": sheets
    })
}

// ---------------------------------------------------------------------------
// Presentation → structured JSON
// ---------------------------------------------------------------------------

/// Convert a raw Google Forms API response into a readable structured format.
fn convert_form(form: &serde_json::Value, responses: Option<&serde_json::Value>) -> serde_json::Value {
    let title = form.get("info")
        .and_then(|i| i.get("title"))
        .and_then(|t| t.as_str())
        .unwrap_or("Untitled Form");

    let form_id = form.get("formId")
        .and_then(|v| v.as_str())
        .unwrap_or("");

    let description = form.get("info")
        .and_then(|i| i.get("description"))
        .and_then(|d| d.as_str())
        .unwrap_or("");

    let mut questions = Vec::new();

    if let Some(items) = form.get("items").and_then(|i| i.as_array()) {
        for item in items {
            let q_title = item.get("title")
                .and_then(|t| t.as_str())
                .unwrap_or("");
            let q_description = item.get("description")
                .and_then(|d| d.as_str())
                .unwrap_or("");

            let mut q = serde_json::json!({
                "title": q_title,
            });
            if !q_description.is_empty() {
                q["description"] = serde_json::json!(q_description);
            }

            // Extract question type and options.
            if let Some(question) = item.get("questionItem").and_then(|qi| qi.get("question")) {
                if let Some(choice) = question.get("choiceQuestion") {
                    let q_type = choice.get("type").and_then(|t| t.as_str()).unwrap_or("CHOICE");
                    q["type"] = serde_json::json!(q_type);
                    if let Some(options) = choice.get("options").and_then(|o| o.as_array()) {
                        let opts: Vec<&str> = options.iter()
                            .filter_map(|o| o.get("value").and_then(|v| v.as_str()))
                            .collect();
                        q["options"] = serde_json::json!(opts);
                    }
                } else if question.get("textQuestion").is_some() {
                    q["type"] = serde_json::json!("TEXT");
                } else if question.get("scaleQuestion").is_some() {
                    q["type"] = serde_json::json!("SCALE");
                } else if question.get("dateQuestion").is_some() {
                    q["type"] = serde_json::json!("DATE");
                } else if question.get("timeQuestion").is_some() {
                    q["type"] = serde_json::json!("TIME");
                }

                if let Some(required) = question.get("required").and_then(|r| r.as_bool()) {
                    q["required"] = serde_json::json!(required);
                }
            }

            questions.push(q);
        }
    }

    let mut result = serde_json::json!({
        "type": "form",
        "form_id": form_id,
        "title": title,
        "description": description,
        "questions": questions,
    });

    // Include response summary if available.
    if let Some(resp_data) = responses {
        if let Some(resp_array) = resp_data.get("responses").and_then(|r| r.as_array()) {
            result["response_count"] = serde_json::json!(resp_array.len());

            let mut all_answers: Vec<serde_json::Value> = Vec::new();
            for response in resp_array {
                let mut answer_map = serde_json::Map::new();
                if let Some(answers) = response.get("answers").and_then(|a| a.as_object()) {
                    for (_q_id, answer) in answers {
                        let q_title_from_resp = answer.pointer("/textAnswers/answers")
                            .and_then(|a| a.as_array())
                            .map(|arr| {
                                arr.iter()
                                    .filter_map(|a| a.get("value").and_then(|v| v.as_str()))
                                    .collect::<Vec<_>>()
                                    .join(", ")
                            })
                            .unwrap_or_default();

                        if let Some(q_id) = answer.get("questionId").and_then(|q| q.as_str()) {
                            answer_map.insert(q_id.to_string(), serde_json::json!(q_title_from_resp));
                        }
                    }
                }
                if let Some(create_time) = response.get("createTime").and_then(|t| t.as_str()) {
                    answer_map.insert("submitted_at".to_string(), serde_json::json!(create_time));
                }
                all_answers.push(serde_json::Value::Object(answer_map));
            }
            result["responses"] = serde_json::json!(all_answers);
        }
    }

    result
}

/// Convert a raw Google Slides API response into a readable structured format.
fn convert_presentation(raw: &serde_json::Value) -> serde_json::Value {
    let title = raw.get("title")
        .and_then(|t| t.as_str())
        .unwrap_or("Untitled Presentation");

    let presentation_id = raw.get("presentationId")
        .and_then(|v| v.as_str())
        .unwrap_or("");

    let mut slides = Vec::new();

    if let Some(raw_slides) = raw.get("slides").and_then(|s| s.as_array()) {
        for (i, slide) in raw_slides.iter().enumerate() {
            let mut texts = Vec::new();

            if let Some(elements) = slide.get("pageElements").and_then(|e| e.as_array()) {
                for element in elements {
                    if let Some(shape) = element.get("shape") {
                        if let Some(text_elements) = shape
                            .pointer("/text/textElements")
                            .and_then(|t| t.as_array())
                        {
                            let mut slide_text = String::new();
                            for te in text_elements {
                                if let Some(content) = te
                                    .pointer("/textRun/content")
                                    .and_then(|c| c.as_str())
                                {
                                    slide_text.push_str(content);
                                }
                            }
                            let trimmed = slide_text.trim().to_string();
                            if !trimmed.is_empty() {
                                texts.push(trimmed);
                            }
                        }
                    }

                    // Also handle tables in slides.
                    if let Some(table) = element.get("table") {
                        let mut table_rows = Vec::new();
                        if let Some(rows) = table.get("tableRows").and_then(|r| r.as_array()) {
                            for row in rows {
                                let mut cells = Vec::new();
                                if let Some(table_cells) = row.get("tableCells").and_then(|c| c.as_array()) {
                                    for cell in table_cells {
                                        let mut cell_text = String::new();
                                        if let Some(text_elements) = cell
                                            .pointer("/text/textElements")
                                            .and_then(|t| t.as_array())
                                        {
                                            for te in text_elements {
                                                if let Some(content) = te
                                                    .pointer("/textRun/content")
                                                    .and_then(|c| c.as_str())
                                                {
                                                    cell_text.push_str(content);
                                                }
                                            }
                                        }
                                        cells.push(cell_text.trim().to_string());
                                    }
                                }
                                table_rows.push(cells);
                            }
                        }
                        texts.push(format!("[table: {} rows]", table_rows.len()));
                    }
                }
            }

            slides.push(serde_json::json!({
                "slide_number": i + 1,
                "content": texts
            }));
        }
    }

    serde_json::json!({
        "type": "presentation",
        "presentation_id": presentation_id,
        "title": title,
        "slides": slides
    })
}
