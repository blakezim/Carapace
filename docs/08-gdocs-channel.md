# Google Docs/Drive Channel

The GDocs channel provides read access to Google Docs, Sheets, Slides, and Forms, plus the ability to create new documents and folders.

## Architecture

```
Agent -> gdocs-mcp -> carapace-daemon -> gdocs-proxy -> Google APIs
                                                          |-- Drive API (search, copy, folders)
                                                          |-- Docs API (read/create documents)
                                                          |-- Sheets API (read spreadsheets)
                                                          |-- Slides API (read presentations)
                                                          |-- Forms API (read forms + responses)
```

## Accounts

| Account | Email | Config | Socket |
|---------|-------|--------|--------|
| hq | zimmermanhq@gmail.com | `/etc/carapace/gdocs-proxy-hq.toml` | `gdocs-proxy-hq.sock` |
| automations | automationsbz@gmail.com | `/etc/carapace/gdocs-proxy-automations.toml` | `gdocs-proxy-automations.sock` |

## OAuth Scopes

- `drive.readonly` — list, search, read file metadata
- `documents.readonly` — read Google Docs content
- `spreadsheets.readonly` — read Google Sheets content
- `presentations.readonly` — read Google Slides content
- `forms.body.readonly` — read Google Forms questions
- `forms.responses.readonly` — read Google Forms responses
- `drive.file` — create new files, edit files the app created

## What Agents Can Do

| Tool | What It Does |
|------|-------------|
| `gdocs_search` | Search Drive for files (query syntax: `name contains 'budget'`, `mimeType = '...'`) |
| `gdocs_read` | Read a file — auto-detects type (Doc, Sheet, Slides, Form) |
| `gdocs_file_info` | Get metadata for any file (name, type, owner, dates) |
| `gdocs_create` | Create a new Google Doc, optionally in a specific folder |
| `gdocs_copy` | Copy any file (the copy is owned by the agent, so it's editable) |
| `gdocs_append` | Append text to a document the agent created |
| `gdocs_create_folder` | Create a folder, optionally inside another folder |
| `gdocs_status` | Check proxy health and token status |

## What Agents Cannot Do

- Delete files or folders
- Change sharing permissions
- Edit files they didn't create (enforced by `drive.file` scope)
- Read PDFs, images, or binary files (only Google Workspace types)

## Read Output Formats

### Google Docs
```json
{
  "type": "document",
  "document_id": "...",
  "title": "Meeting Notes",
  "content": [
    {"type": "heading", "level": 1, "text": "Meeting Notes"},
    {"type": "paragraph", "text": "Discussed budget...", "links": []},
    {"type": "table", "rows": [["Item", "Cost"], ["Venue", "$5000"]]}
  ]
}
```

### Google Sheets
```json
{
  "type": "spreadsheet",
  "title": "Budget",
  "sheets": [
    {"name": "Sheet1", "rows": [["Name", "Amount"], ["Venue", "5000"]]}
  ]
}
```

### Google Slides
```json
{
  "type": "presentation",
  "title": "Deck",
  "slides": [
    {"slide_number": 1, "content": ["Title Slide", "Subtitle"]}
  ]
}
```

### Google Forms
```json
{
  "type": "form",
  "title": "RSVP",
  "questions": [
    {"title": "Will you attend?", "type": "RADIO", "options": ["Yes", "No"]}
  ],
  "response_count": 42,
  "responses": [...]
}
```

## Setup Steps

### 1. Enable APIs

In Google Cloud Console, enable:
- Google Drive API
- Google Docs API
- Google Sheets API
- Google Slides API
- Google Forms API

### 2. Create Config

```bash
sudo tee /etc/carapace/gdocs-proxy-hq.toml > /dev/null << 'EOF'
[auth]
client_id = "YOUR_CLIENT_ID"
client_secret = "YOUR_CLIENT_SECRET"
secrets_file = "secrets-gdocs-hq.toml"

[gdocs]
account = "you@gmail.com"

[scrub]
strip_links = false
redact_patterns = []

[proxy]
socket_path = "/var/run/carapace/gdocs-proxy-hq.sock"
EOF

sudo chown carapace /etc/carapace/gdocs-proxy-hq.toml
```

### 3. Pre-create Secrets File

```bash
sudo touch /etc/carapace/secrets-gdocs-hq.toml
sudo chown carapace /etc/carapace/secrets-gdocs-hq.toml
sudo chmod 600 /etc/carapace/secrets-gdocs-hq.toml
```

### 4. Run OAuth Setup

```bash
sudo -u carapace gdocs-proxy setup --config /etc/carapace/gdocs-proxy-hq.toml
```

### 5. Install LaunchDaemon and Add to Gateway Config

Same pattern as Gmail — create plist, bootstrap, add to daemon config, restart gateway.

## Multi-Account

Use `GDOCS_ACCOUNT` env var in MCP config:

```json
{
  "gdocs": {
    "command": "/usr/local/bin/gdocs-mcp",
    "env": { "GDOCS_ACCOUNT": "automations" }
  }
}
```
