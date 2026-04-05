//! Gmail API HTTP client.

use std::sync::Arc;

use anyhow::{Context, Result};
use serde::Serialize;

use crate::auth::TokenManager;
use crate::gmail::types::*;

pub struct GmailClient {
    http_client: reqwest::Client,
    token_manager: Arc<TokenManager>,
    base_url: String,
    /// The authenticated user's email address (for draft From: header).
    pub account: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WatchRequestBody<'a> {
    topic_name: &'a str,
    label_ids: &'a [String],
    label_filter_behavior: &'a str,
}

#[derive(Serialize)]
struct DraftBody {
    message: DraftMessage,
}

#[derive(Serialize)]
struct DraftMessage {
    raw: String,
}

impl GmailClient {
    pub fn new(token_manager: Arc<TokenManager>, base_url: String, account: String) -> Self {
        Self {
            http_client: reqwest::Client::new(),
            token_manager,
            base_url,
            account,
        }
    }

    async fn auth_header(&self) -> Result<String> {
        let token = self.token_manager.get_token().await?;
        Ok(format!("Bearer {token}"))
    }

    pub async fn search(
        &self,
        query: &str,
        max_results: u32,
        page_token: Option<&str>,
    ) -> Result<MessageListResponse> {
        let auth = self.auth_header().await?;
        let mut req = self
            .http_client
            .get(format!("{}/messages", self.base_url))
            .header("Authorization", &auth)
            .query(&[("q", query), ("maxResults", &max_results.to_string())]);
        if let Some(pt) = page_token {
            req = req.query(&[("pageToken", pt)]);
        }
        let resp = req.send().await.context("search request failed")?;
        check_status(&resp)?;
        resp.json().await.context("failed to deserialize search response")
    }

    pub async fn get_message(&self, id: &str) -> Result<Message> {
        let auth = self.auth_header().await?;
        let resp = self
            .http_client
            .get(format!("{}/messages/{id}", self.base_url))
            .header("Authorization", &auth)
            .query(&[("format", "full")])
            .send()
            .await
            .context("get_message request failed")?;
        check_status(&resp)?;
        resp.json().await.context("failed to deserialize message")
    }

    pub async fn get_thread(&self, id: &str) -> Result<ThreadResponse> {
        let auth = self.auth_header().await?;
        let resp = self
            .http_client
            .get(format!("{}/threads/{id}", self.base_url))
            .header("Authorization", &auth)
            .query(&[("format", "full")])
            .send()
            .await
            .context("get_thread request failed")?;
        check_status(&resp)?;
        resp.json().await.context("failed to deserialize thread")
    }

    pub async fn list_labels(&self) -> Result<LabelListResponse> {
        let auth = self.auth_header().await?;
        let resp = self
            .http_client
            .get(format!("{}/labels", self.base_url))
            .header("Authorization", &auth)
            .send()
            .await
            .context("list_labels request failed")?;
        check_status(&resp)?;
        resp.json().await.context("failed to deserialize labels")
    }

    pub async fn history(&self, start_history_id: u64) -> Result<HistoryResponse> {
        let auth = self.auth_header().await?;
        let resp = self
            .http_client
            .get(format!("{}/history", self.base_url))
            .header("Authorization", &auth)
            .query(&[
                ("startHistoryId", &start_history_id.to_string()),
                ("historyTypes", &"messageAdded".to_string()),
            ])
            .send()
            .await
            .context("history request failed")?;
        check_status(&resp)?;
        resp.json().await.context("failed to deserialize history response")
    }

    /// Create a draft email.
    ///
    /// Builds a minimal RFC 2822 message, base64url-encodes it, and posts to
    /// `drafts.create`. The OAuth scope `gmail.compose` is required.
    pub async fn create_draft(
        &self,
        to: &str,
        subject: &str,
        body: &str,
        cc: Option<&str>,
    ) -> Result<DraftResponse> {
        let auth = self.auth_header().await?;
        let raw = build_raw_message(&self.account, to, cc, subject, body);
        let payload = DraftBody {
            message: DraftMessage { raw },
        };
        let resp = self
            .http_client
            .post(format!("{}/drafts", self.base_url))
            .header("Authorization", &auth)
            .json(&payload)
            .send()
            .await
            .context("create_draft request failed")?;
        check_status(&resp)?;
        resp.json().await.context("failed to deserialize draft response")
    }

    pub fn token_manager(&self) -> Arc<TokenManager> {
        self.token_manager.clone()
    }
}

/// Check that the response status is 2xx, otherwise return an error with the body.
fn check_status(resp: &reqwest::Response) -> Result<()> {
    let status = resp.status();
    if status.is_success() {
        Ok(())
    } else {
        Err(anyhow::anyhow!("Gmail API error {status}"))
    }
}
