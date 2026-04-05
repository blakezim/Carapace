//! Google Drive + Docs API HTTP client.

use std::sync::Arc;

use anyhow::{Context, Result};
use serde_json::json;

use crate::auth::TokenManager;
use crate::docs::types::*;

pub struct DocsClient {
    http_client: reqwest::Client,
    token_manager: Arc<TokenManager>,
    drive_base_url: String,
    docs_base_url: String,
    pub account: String,
}

impl DocsClient {
    pub fn new(
        token_manager: Arc<TokenManager>,
        account: String,
    ) -> Self {
        Self {
            http_client: reqwest::Client::new(),
            token_manager,
            drive_base_url: "https://www.googleapis.com/drive/v3".into(),
            docs_base_url: "https://docs.googleapis.com/v1".into(),
            account,
        }
    }

    async fn auth_header(&self) -> Result<String> {
        let token = self.token_manager.get_token().await?;
        Ok(format!("Bearer {token}"))
    }

    // ── Drive API ──────────────────────────────────────────────────────────

    /// Search for files in Google Drive.
    ///
    /// `query` uses Drive query syntax (e.g. `name contains 'budget'`).
    /// Automatically excludes trashed files.
    pub async fn search(
        &self,
        query: &str,
        max_results: u32,
        page_token: Option<&str>,
    ) -> Result<FileListResponse> {
        let auth = self.auth_header().await?;

        // Always exclude trashed files.
        let full_query = if query.is_empty() {
            "trashed = false".to_string()
        } else {
            format!("({query}) and trashed = false")
        };

        let mut req = self
            .http_client
            .get(format!("{}/files", self.drive_base_url))
            .header("Authorization", &auth)
            .query(&[
                ("q", full_query.as_str()),
                ("pageSize", &max_results.to_string()),
                ("fields", "files(id,name,mimeType,createdTime,modifiedTime,owners,webViewLink,starred,trashed),nextPageToken"),
                ("orderBy", "modifiedTime desc"),
            ]);
        if let Some(pt) = page_token {
            req = req.query(&[("pageToken", pt)]);
        }

        let resp = req.send().await.context("Drive search request failed")?;
        let resp = check_status(resp, "Drive search").await?;
        resp.json().await.context("failed to deserialize Drive search response")
    }

    /// Get file metadata by ID.
    pub async fn get_file(&self, file_id: &str) -> Result<DriveFile> {
        let auth = self.auth_header().await?;
        let resp = self
            .http_client
            .get(format!("{}/files/{file_id}", self.drive_base_url))
            .header("Authorization", &auth)
            .query(&[("fields", "id,name,mimeType,createdTime,modifiedTime,owners,webViewLink,starred,trashed")])
            .send()
            .await
            .context("Drive get_file request failed")?;
        let resp = check_status(resp, "Drive get_file").await?;
        resp.json().await.context("failed to deserialize file metadata")
    }

    /// Create a folder in Google Drive. Optionally place it inside a parent folder.
    pub async fn create_folder(&self, name: &str, parent_id: Option<&str>) -> Result<DriveFile> {
        let auth = self.auth_header().await?;

        let mut metadata = json!({
            "name": name,
            "mimeType": "application/vnd.google-apps.folder"
        });
        if let Some(pid) = parent_id {
            metadata["parents"] = json!([pid]);
        }

        let resp = self
            .http_client
            .post(format!("{}/files", self.drive_base_url))
            .header("Authorization", &auth)
            .query(&[("fields", "id,name,mimeType,createdTime,modifiedTime,owners,webViewLink")])
            .json(&metadata)
            .send()
            .await
            .context("Drive create_folder request failed")?;
        let resp = check_status(resp, "Drive create_folder").await?;
        resp.json().await.context("failed to deserialize folder metadata")
    }

    /// Copy a file. Returns the new file's metadata.
    pub async fn copy_file(&self, file_id: &str, new_title: Option<&str>) -> Result<DriveFile> {
        let auth = self.auth_header().await?;

        let body = if let Some(title) = new_title {
            json!({"name": title})
        } else {
            json!({})
        };

        let resp = self
            .http_client
            .post(format!("{}/files/{file_id}/copy", self.drive_base_url))
            .header("Authorization", &auth)
            .query(&[("fields", "id,name,mimeType,createdTime,modifiedTime,owners,webViewLink")])
            .json(&body)
            .send()
            .await
            .context("Drive copy request failed")?;
        let resp = check_status(resp, "Drive copy").await?;
        resp.json().await.context("failed to deserialize copy response")
    }

    // ── Google Docs API ────────────────────────────────────────────────────

    /// Read a Google Doc by document ID. Returns the full structured content.
    pub async fn get_document(&self, doc_id: &str) -> Result<Document> {
        let auth = self.auth_header().await?;
        let resp = self
            .http_client
            .get(format!("{}/documents/{doc_id}", self.docs_base_url))
            .header("Authorization", &auth)
            .send()
            .await
            .context("Docs get_document request failed")?;
        let resp = check_status(resp, "Docs get_document").await?;
        resp.json().await.context("failed to deserialize document")
    }

    // ── Google Sheets API ──────────────────────────────────────────────────

    /// Read a Google Spreadsheet. Returns sheet names and cell values.
    pub async fn get_spreadsheet(&self, spreadsheet_id: &str) -> Result<serde_json::Value> {
        let auth = self.auth_header().await?;
        let resp = self
            .http_client
            .get(format!(
                "https://sheets.googleapis.com/v4/spreadsheets/{spreadsheet_id}",
            ))
            .header("Authorization", &auth)
            .query(&[("includeGridData", "true")])
            .send()
            .await
            .context("Sheets get_spreadsheet request failed")?;
        let resp = check_status(resp, "Sheets get_spreadsheet").await?;
        resp.json().await.context("failed to deserialize spreadsheet")
    }

    // ── Google Slides API ────────────────────────────────────────────────

    /// Read a Google Slides presentation. Returns slide content.
    pub async fn get_presentation(&self, presentation_id: &str) -> Result<serde_json::Value> {
        let auth = self.auth_header().await?;
        let resp = self
            .http_client
            .get(format!(
                "https://slides.googleapis.com/v1/presentations/{presentation_id}",
            ))
            .header("Authorization", &auth)
            .send()
            .await
            .context("Slides get_presentation request failed")?;
        let resp = check_status(resp, "Slides get_presentation").await?;
        resp.json().await.context("failed to deserialize presentation")
    }

    // ── Google Forms API ──────────────────────────────────────────────────

    /// Read a Google Form. Returns questions and metadata.
    pub async fn get_form(&self, form_id: &str) -> Result<serde_json::Value> {
        let auth = self.auth_header().await?;
        let resp = self
            .http_client
            .get(format!(
                "https://forms.googleapis.com/v1/forms/{form_id}",
            ))
            .header("Authorization", &auth)
            .send()
            .await
            .context("Forms get_form request failed")?;
        let resp = check_status(resp, "Forms get_form").await?;
        resp.json().await.context("failed to deserialize form")
    }

    /// Read form responses.
    pub async fn get_form_responses(&self, form_id: &str) -> Result<serde_json::Value> {
        let auth = self.auth_header().await?;
        let resp = self
            .http_client
            .get(format!(
                "https://forms.googleapis.com/v1/forms/{form_id}/responses",
            ))
            .header("Authorization", &auth)
            .send()
            .await
            .context("Forms get_form_responses request failed")?;
        let resp = check_status(resp, "Forms get_form_responses").await?;
        resp.json().await.context("failed to deserialize form responses")
    }

    /// Create a new Google Doc with the given title, optionally in a specific folder.
    ///
    /// When `parent_id` is None, uses the Docs API directly (lands in Drive root).
    /// When `parent_id` is Some, creates via Drive API with the parent set,
    /// then fetches the doc via Docs API to return structured metadata.
    pub async fn create_document(&self, title: &str, parent_id: Option<&str>) -> Result<Document> {
        let auth = self.auth_header().await?;

        if let Some(pid) = parent_id {
            // Create via Drive API to set parent folder.
            let metadata = json!({
                "name": title,
                "mimeType": "application/vnd.google-apps.document",
                "parents": [pid]
            });
            let resp = self
                .http_client
                .post(format!("{}/files", self.drive_base_url))
                .header("Authorization", &auth)
                .query(&[("fields", "id")])
                .json(&metadata)
                .send()
                .await
                .context("Drive create doc request failed")?;
            let resp = check_status(resp, "Drive create doc").await?;
            let file: serde_json::Value = resp.json().await.context("failed to deserialize Drive file")?;
            let doc_id = file.get("id").and_then(|v| v.as_str())
                .context("missing id in Drive create response")?;
            // Fetch the doc via Docs API for structured response.
            self.get_document(doc_id).await
        } else {
            let resp = self
                .http_client
                .post(format!("{}/documents", self.docs_base_url))
                .header("Authorization", &auth)
                .json(&json!({"title": title}))
                .send()
                .await
                .context("Docs create request failed")?;
            let resp = check_status(resp, "Docs create").await?;
            resp.json().await.context("failed to deserialize new document")
        }
    }

    /// Append plain text to the end of a document using batchUpdate.
    pub async fn append_text(&self, doc_id: &str, text: &str) -> Result<()> {
        let auth = self.auth_header().await?;

        // To append, we insert at the end. We need to know the doc's end index.
        // The Docs API uses 1-based indexing; inserting at index 1 inserts at start.
        // We first get the document to find the end index, then insert there.
        let doc = self.get_document(doc_id).await?;
        let end_index = Self::doc_end_index(&doc);

        let requests = json!({
            "requests": [
                {
                    "insertText": {
                        "text": text,
                        "location": {
                            "index": end_index - 1  // Insert before the final newline
                        }
                    }
                }
            ]
        });

        let resp = self
            .http_client
            .post(format!("{}/documents/{doc_id}:batchUpdate", self.docs_base_url))
            .header("Authorization", &auth)
            .json(&requests)
            .send()
            .await
            .context("Docs batchUpdate request failed")?;
        check_status(resp, "Docs batchUpdate").await?;
        Ok(())
    }

    /// Find the end-of-body index of a document (for insertions).
    fn doc_end_index(doc: &Document) -> i64 {
        if let Some(ref body) = doc.body {
            if let Some(ref content) = body.content {
                // Each structural element has start_index and end_index.
                // The raw JSON has these but our typed struct doesn't track them.
                // A safe fallback: count characters + 1.
                let mut len: i64 = 1;
                for element in content {
                    if let Some(ref paragraph) = element.paragraph {
                        if let Some(ref elements) = paragraph.elements {
                            for elem in elements {
                                if let Some(ref text_run) = elem.text_run {
                                    if let Some(ref text) = text_run.content {
                                        len += text.len() as i64;
                                    }
                                }
                            }
                        }
                    }
                    if let Some(ref table) = element.table {
                        // Tables are complex; just add a rough estimate.
                        if let Some(ref rows) = table.table_rows {
                            for row in rows {
                                if let Some(ref cells) = row.table_cells {
                                    for cell in cells {
                                        if let Some(ref content) = cell.content {
                                            for el in content {
                                                if let Some(ref p) = el.paragraph {
                                                    if let Some(ref elems) = p.elements {
                                                        for pe in elems {
                                                            if let Some(ref tr) = pe.text_run {
                                                                if let Some(ref t) = tr.content {
                                                                    len += t.len() as i64;
                                                                }
                                                            }
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                        len += 2; // cell overhead
                                    }
                                }
                                len += 1; // row overhead
                            }
                        }
                    }
                }
                return len;
            }
        }
        1
    }

    pub fn token_manager(&self) -> Arc<TokenManager> {
        self.token_manager.clone()
    }
}

/// Check that the response status is 2xx. Consumes the response on error to include the body.
async fn check_status(resp: reqwest::Response, context: &str) -> Result<reqwest::Response> {
    let status = resp.status();
    if status.is_success() {
        Ok(resp)
    } else {
        let body = resp.text().await.unwrap_or_default();
        Err(anyhow::anyhow!("{context}: API error {status}: {body}"))
    }
}
