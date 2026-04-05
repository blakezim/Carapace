//! Label-based message filtering.

use super::query::{QueryNode, reconstruct_with_label_exclusion};

pub struct LabelFilter {
    blocked_label_id: String,
    blocked_label_name: String,
}

impl LabelFilter {
    pub fn new(blocked_label_id: String, blocked_label_name: String) -> Self {
        Self { blocked_label_id, blocked_label_name }
    }

    /// Returns true if the message carries the blocked label ID.
    pub fn is_message_blocked(&self, label_ids: &[String]) -> bool {
        label_ids.iter().any(|id| id == &self.blocked_label_id)
    }

    /// Rebuild a query string with the blocked label excluded.
    pub fn secure_query_string(&self, node: &QueryNode) -> String {
        reconstruct_with_label_exclusion(node, &self.blocked_label_name)
    }

    pub fn blocked_label_id(&self) -> &str {
        &self.blocked_label_id
    }

    pub fn blocked_label_name(&self) -> &str {
        &self.blocked_label_name
    }
}
