//! Data types for Google Drive and Docs API responses.

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Google Drive API types
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileListResponse {
    pub files: Option<Vec<DriveFile>>,
    pub next_page_token: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct DriveFile {
    pub id: String,
    pub name: String,
    pub mime_type: String,
    #[serde(default)]
    pub created_time: Option<String>,
    #[serde(default)]
    pub modified_time: Option<String>,
    #[serde(default)]
    pub owners: Option<Vec<FileOwner>>,
    #[serde(default)]
    pub web_view_link: Option<String>,
    #[serde(default)]
    pub starred: Option<bool>,
    #[serde(default)]
    pub trashed: Option<bool>,
    #[serde(default)]
    pub parents: Option<Vec<String>>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct FileOwner {
    pub display_name: String,
    pub email_address: Option<String>,
}

// ---------------------------------------------------------------------------
// Google Docs API types
// ---------------------------------------------------------------------------

/// Top-level Google Docs document response.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Document {
    pub document_id: String,
    pub title: String,
    pub body: Option<DocBody>,
    pub revision_id: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DocBody {
    pub content: Option<Vec<StructuralElement>>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StructuralElement {
    pub paragraph: Option<Paragraph>,
    pub table: Option<Table>,
    pub section_break: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Paragraph {
    pub elements: Option<Vec<ParagraphElement>>,
    pub paragraph_style: Option<ParagraphStyle>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ParagraphStyle {
    pub named_style_type: Option<String>,
    pub heading_id: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ParagraphElement {
    pub text_run: Option<TextRun>,
    pub inline_object_element: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TextRun {
    pub content: Option<String>,
    pub text_style: Option<TextStyle>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TextStyle {
    pub bold: Option<bool>,
    pub italic: Option<bool>,
    pub underline: Option<bool>,
    pub strikethrough: Option<bool>,
    pub link: Option<Link>,
}

#[derive(Debug, Deserialize)]
pub struct Link {
    pub url: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Table {
    pub rows: u32,
    pub columns: u32,
    pub table_rows: Option<Vec<TableRow>>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TableRow {
    pub table_cells: Option<Vec<TableCell>>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TableCell {
    pub content: Option<Vec<StructuralElement>>,
}

// ---------------------------------------------------------------------------
// Structured output types (what the proxy returns to clients)
// ---------------------------------------------------------------------------

/// Structured representation of a Google Doc, suitable for agent consumption.
#[derive(Debug, Serialize)]
pub struct StructuredDoc {
    pub document_id: String,
    pub title: String,
    pub revision_id: Option<String>,
    pub content: Vec<ContentBlock>,
}

/// A single content block in a structured document.
#[derive(Debug, Serialize)]
#[serde(tag = "type")]
pub enum ContentBlock {
    #[serde(rename = "heading")]
    Heading {
        level: u8,
        text: String,
    },
    #[serde(rename = "paragraph")]
    Paragraph {
        text: String,
        #[serde(skip_serializing_if = "Vec::is_empty")]
        links: Vec<InlineLink>,
    },
    #[serde(rename = "table")]
    Table {
        rows: Vec<Vec<String>>,
    },
}

#[derive(Debug, Serialize)]
pub struct InlineLink {
    pub text: String,
    pub url: String,
}

// ---------------------------------------------------------------------------
// Conversion: Document → StructuredDoc
// ---------------------------------------------------------------------------

impl Document {
    /// Convert a raw Google Docs API response into a structured representation.
    pub fn to_structured(&self) -> StructuredDoc {
        let mut blocks = Vec::new();

        if let Some(ref body) = self.body {
            if let Some(ref content) = body.content {
                for element in content {
                    if let Some(block) = convert_element(element) {
                        blocks.push(block);
                    }
                }
            }
        }

        StructuredDoc {
            document_id: self.document_id.clone(),
            title: self.title.clone(),
            revision_id: self.revision_id.clone(),
            content: blocks,
        }
    }
}

fn convert_element(element: &StructuralElement) -> Option<ContentBlock> {
    if let Some(ref paragraph) = element.paragraph {
        return convert_paragraph(paragraph);
    }
    if let Some(ref table) = element.table {
        return Some(convert_table(table));
    }
    None
}

fn convert_paragraph(paragraph: &Paragraph) -> Option<ContentBlock> {
    let mut text = String::new();
    let mut links = Vec::new();

    if let Some(ref elements) = paragraph.elements {
        for elem in elements {
            if let Some(ref text_run) = elem.text_run {
                let content = text_run.content.as_deref().unwrap_or("");
                text.push_str(content);

                // Collect links.
                if let Some(ref style) = text_run.text_style {
                    if let Some(ref link) = style.link {
                        if let Some(ref url) = link.url {
                            let link_text = content.trim().to_string();
                            if !link_text.is_empty() {
                                links.push(InlineLink {
                                    text: link_text,
                                    url: url.clone(),
                                });
                            }
                        }
                    }
                }
            }
        }
    }

    // Trim trailing newline that Google Docs adds to every paragraph.
    let text = text.trim_end_matches('\n').to_string();

    // Skip empty paragraphs (section breaks, etc.).
    if text.is_empty() {
        return None;
    }

    // Determine if this is a heading.
    if let Some(ref style) = paragraph.paragraph_style {
        if let Some(ref named_style) = style.named_style_type {
            let heading_level = match named_style.as_str() {
                "HEADING_1" => Some(1),
                "HEADING_2" => Some(2),
                "HEADING_3" => Some(3),
                "HEADING_4" => Some(4),
                "HEADING_5" => Some(5),
                "HEADING_6" => Some(6),
                "TITLE" => Some(1),
                "SUBTITLE" => Some(2),
                _ => None,
            };

            if let Some(level) = heading_level {
                return Some(ContentBlock::Heading { level, text });
            }
        }
    }

    Some(ContentBlock::Paragraph { text, links })
}

fn convert_table(table: &Table) -> ContentBlock {
    let mut rows = Vec::new();

    if let Some(ref table_rows) = table.table_rows {
        for row in table_rows {
            let mut cells = Vec::new();
            if let Some(ref table_cells) = row.table_cells {
                for cell in table_cells {
                    let mut cell_text = String::new();
                    if let Some(ref content) = cell.content {
                        for element in content {
                            if let Some(ref paragraph) = element.paragraph {
                                if let Some(ref elements) = paragraph.elements {
                                    for elem in elements {
                                        if let Some(ref text_run) = elem.text_run {
                                            if let Some(ref content) = text_run.content {
                                                cell_text.push_str(content);
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                    cells.push(cell_text.trim_end_matches('\n').to_string());
                }
            }
            rows.push(cells);
        }
    }

    ContentBlock::Table { rows }
}

// ---------------------------------------------------------------------------
// Token types (shared with auth)
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct TokenResponse {
    pub access_token: String,
    pub expires_in: u64,
    pub token_type: String,
}

// ---------------------------------------------------------------------------
// Request types
// ---------------------------------------------------------------------------

/// Request body for `POST /docs` (create new document).
#[derive(Debug, Deserialize)]
pub struct CreateDocRequest {
    pub title: String,
    /// Optional initial plain-text content.
    #[serde(default)]
    pub content: Option<String>,
    /// Optional folder ID to create the document in.
    #[serde(default)]
    pub folder_id: Option<String>,
}

/// Request body for `PUT /doc/{id}` (append/replace content).
#[derive(Debug, Deserialize)]
pub struct UpdateDocRequest {
    /// Plain text to append to the document.
    #[serde(default)]
    pub append_text: Option<String>,
}

/// Sanitized file listing for search results.
#[derive(Debug, Serialize)]
pub struct FileResult {
    pub id: String,
    pub name: String,
    pub mime_type: String,
    pub created_time: Option<String>,
    pub modified_time: Option<String>,
    pub owner: Option<String>,
    pub web_view_link: Option<String>,
}

impl DriveFile {
    pub fn to_result(&self) -> FileResult {
        let owner = self.owners.as_ref().and_then(|owners| {
            owners.first().map(|o| {
                o.email_address
                    .as_deref()
                    .unwrap_or(&o.display_name)
                    .to_string()
            })
        });
        FileResult {
            id: self.id.clone(),
            name: self.name.clone(),
            mime_type: self.mime_type.clone(),
            created_time: self.created_time.clone(),
            modified_time: self.modified_time.clone(),
            owner,
            web_view_link: self.web_view_link.clone(),
        }
    }
}
