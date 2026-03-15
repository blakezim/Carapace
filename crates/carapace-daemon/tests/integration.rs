//! Integration tests for the Carapace gateway daemon.
//!
//! Uses a mock `imsg` binary and a temporary Unix socket to exercise
//! the full stack: client → socket → server → middleware → handler → adapter.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::time::Duration;

use carapace_client::GatewayClient;
use serde_json::json;

/// A test daemon that starts `carapace-daemon` with a temp socket + config
/// pointing at the mock binary. Kills the daemon on drop.
struct TestDaemon {
    child: Child,
    socket_path: PathBuf,
    _temp_dir: tempfile::TempDir,
}

impl TestDaemon {
    fn start() -> Self {
        let temp_dir = tempfile::tempdir().expect("failed to create temp dir");
        let socket_path = temp_dir.path().join("gateway.sock");
        let config_path = temp_dir.path().join("config.toml");
        let audit_path = temp_dir.path().join("audit.log");
        let dead_letter_path = temp_dir.path().join("dead_letters");

        // Resolve path to mock binary (relative to this test file).
        let mock_binary = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("mock_imsg.sh");

        assert!(
            mock_binary.exists(),
            "mock binary not found at {}",
            mock_binary.display()
        );

        // Write config that points at the mock binary.
        let config = format!(
            r#"
[gateway]
socket_path = "{socket}"

[security]
audit_log_path = "{audit}"
dead_letter_path = "{dead_letter}"
audit_enabled = false

[security.rate_limit]
default = {{ requests = 100, per_seconds = 60 }}

[security.content_filter]
enabled = true

[[security.content_filter.patterns]]
pattern = '(?i)password\s*[:=]'
action = "block"

[channels.imsg]
enabled = true
real_binary = "{binary}"
db_path = "/nonexistent/chat.db"

[channels.imsg.outbound]
mode = "allowlist"
allowlist = ["+1111111111", "friend@icloud.com"]

[channels.imsg.inbound]
mode = "allowlist"
allowlist = ["+1111111111"]
"#,
            socket = socket_path.display(),
            audit = audit_path.display(),
            dead_letter = dead_letter_path.display(),
            binary = mock_binary.display(),
        );

        std::fs::write(&config_path, config).expect("failed to write config");

        // Find the daemon binary.
        let daemon_bin = env!("CARGO_BIN_EXE_carapace-daemon");

        let child = Command::new(daemon_bin)
            .arg("--config")
            .arg(&config_path)
            .arg("--socket")
            .arg(&socket_path)
            .env("RUST_LOG", "warn")
            .spawn()
            .expect("failed to start daemon");

        let daemon = TestDaemon {
            child,
            socket_path: socket_path.clone(),
            _temp_dir: temp_dir,
        };

        // Poll until the socket appears (max 5 seconds).
        for _ in 0..50 {
            if socket_path.exists() {
                // Give the daemon a moment to finish binding.
                std::thread::sleep(Duration::from_millis(50));
                return daemon;
            }
            std::thread::sleep(Duration::from_millis(100));
        }

        panic!(
            "daemon socket did not appear at {} within 5s",
            socket_path.display()
        );
    }

    fn client(&self) -> GatewayClient {
        GatewayClient::connect(&self.socket_path).expect("failed to connect to daemon")
    }
}

impl Drop for TestDaemon {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

// ── Tests ──────────────────────────────────────────────────────────────

#[test]
fn ping_round_trip() {
    let daemon = TestDaemon::start();
    let mut client = daemon.client();
    let result = client.call("ping", json!({})).unwrap();
    assert_eq!(result["pong"], true);
}

#[test]
fn echo_round_trip() {
    let daemon = TestDaemon::start();
    let mut client = daemon.client();
    let result = client.call("echo", json!({"message": "integration"})).unwrap();
    assert_eq!(result["echo"], "integration");
}

#[test]
fn whoami_round_trip() {
    let daemon = TestDaemon::start();
    let mut client = daemon.client();
    let result = client.call("whoami", json!({})).unwrap();
    assert!(result.get("user").is_some());
    assert!(result.get("uid").is_some());
}

#[test]
fn channel_send_success() {
    let daemon = TestDaemon::start();
    let mut client = daemon.client();
    let result = client
        .call(
            "channel.send",
            json!({
                "channel": "imsg",
                "recipient": "+1111111111",
                "message": "hello"
            }),
        )
        .unwrap();
    assert_eq!(result["success"], true);
}

#[test]
fn channel_send_blocked_by_allowlist() {
    let daemon = TestDaemon::start();
    let mut client = daemon.client();
    let err = client
        .call(
            "channel.send",
            json!({
                "channel": "imsg",
                "recipient": "+9999999999",
                "message": "hello"
            }),
        )
        .unwrap_err();
    match err {
        carapace_client::ClientError::Gateway { code, .. } => {
            assert_eq!(code, -32001); // NOT_IN_ALLOWLIST
        }
        other => panic!("expected Gateway error, got: {other}"),
    }
}

#[test]
fn channel_send_blocked_by_content_filter() {
    let daemon = TestDaemon::start();
    let mut client = daemon.client();
    let err = client
        .call(
            "channel.send",
            json!({
                "channel": "imsg",
                "recipient": "+1111111111",
                "message": "password= secret123"
            }),
        )
        .unwrap_err();
    match err {
        carapace_client::ClientError::Gateway { code, .. } => {
            assert_eq!(code, -32003); // CONTENT_BLOCKED
        }
        other => panic!("expected Gateway error, got: {other}"),
    }
}

#[test]
fn channel_list_chats() {
    let daemon = TestDaemon::start();
    let mut client = daemon.client();
    let result = client
        .call("channel.list_chats", json!({"channel": "imsg"}))
        .unwrap();
    let chats = result.as_array().expect("expected array");
    assert_eq!(chats.len(), 2);
    assert_eq!(chats[0]["chat_id"], "chat001");
}

#[test]
fn channel_get_history() {
    let daemon = TestDaemon::start();
    let mut client = daemon.client();
    let result = client
        .call(
            "channel.get_history",
            json!({"channel": "imsg", "chat_id": "chat001"}),
        )
        .unwrap();
    let messages = result.as_array().expect("expected array");
    assert_eq!(messages.len(), 2);
    assert_eq!(messages[0]["sender"], "+1111111111");
}

#[test]
fn channel_status() {
    let daemon = TestDaemon::start();
    let mut client = daemon.client();
    let result = client
        .call("channel.status", json!({"channel": "imsg"}))
        .unwrap();
    assert_eq!(result["channel"], "imsg");
    assert_eq!(result["configured"], true);
}

#[test]
fn channel_watch_receives_filtered_messages() {
    let daemon = TestDaemon::start();
    let client = daemon.client();

    let (_ack, subscription) = client
        .subscribe("channel.watch", json!({"channel": "imsg"}))
        .unwrap();

    // Collect all events with a timeout.
    let events: Vec<serde_json::Value> = subscription
        .take_while(|r| r.is_ok())
        .map(|r| r.unwrap())
        .collect();

    // The mock emits 3 events but one is from +9999999999 which is not
    // in the inbound allowlist. We should only see 2 events.
    assert_eq!(
        events.len(),
        2,
        "expected 2 events after inbound filtering, got {}: {:?}",
        events.len(),
        events
    );
    assert_eq!(events[0]["sender"], "+1111111111");
    assert_eq!(events[1]["sender"], "+1111111111");
}

#[test]
fn unknown_method_returns_error() {
    let daemon = TestDaemon::start();
    let mut client = daemon.client();
    let err = client
        .call("nonexistent.method", json!({}))
        .unwrap_err();
    match err {
        carapace_client::ClientError::Gateway { code, .. } => {
            assert_eq!(code, -32601); // METHOD_NOT_FOUND
        }
        other => panic!("expected Gateway error, got: {other}"),
    }
}

#[test]
fn malformed_json_returns_parse_error() {
    let daemon = TestDaemon::start();

    // We need to send raw bytes, so use a raw UnixStream.
    use std::io::{BufRead, BufReader};
    use std::os::unix::net::UnixStream;

    let stream = UnixStream::connect(&daemon.socket_path).unwrap();
    let mut writer = stream.try_clone().unwrap();
    let mut reader = BufReader::new(stream);

    writer.write_all(b"not valid json\n").unwrap();
    writer.flush().unwrap();

    let mut line = String::new();
    reader.read_line(&mut line).unwrap();

    let resp: serde_json::Value = serde_json::from_str(line.trim()).unwrap();
    assert_eq!(resp["error"]["code"], -32700); // PARSE_ERROR
}
