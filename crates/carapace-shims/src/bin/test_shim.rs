//! Test Shim – exercises the full Carapace gateway flow.
//!
//! This binary connects to the daemon and runs through each Phase 3 method,
//! printing results along the way. It's the quickest way to verify that
//! cross-user communication is working.
//!
//! # Usage
//!
//! ```bash
//! # With the default socket path:
//! test-shim
//!
//! # With a custom socket path (for local testing):
//! CARAPACE_SOCKET_PATH=/tmp/carapace-test.sock test-shim
//! ```

use carapace_client::GatewayClient;
use serde_json::json;

fn main() {
    println!("╔══════════════════════════════════════════════╗");
    println!("║     Carapace Gateway – Test Shim v0.1.0     ║");
    println!("╚══════════════════════════════════════════════╝");
    println!();

    // ── Connect ────────────────────────────────────────────────────────────

    print!("Connecting to daemon... ");
    let mut client = match GatewayClient::connect_default() {
        Ok(c) => {
            println!("OK ✓");
            c
        }
        Err(e) => {
            println!("FAILED ✗");
            eprintln!("  Error: {e}");
            eprintln!();
            eprintln!("Make sure the daemon is running:");
            eprintln!("  sudo -u carapace carapace-daemon");
            eprintln!();
            eprintln!("Or for local testing:");
            eprintln!("  CARAPACE_SOCKET_PATH=/tmp/carapace-test.sock carapace-daemon &");
            eprintln!("  CARAPACE_SOCKET_PATH=/tmp/carapace-test.sock test-shim");
            std::process::exit(1);
        }
    };

    let mut passed = 0u32;
    let mut failed = 0u32;

    // ── Test 1: ping ───────────────────────────────────────────────────────

    print!("1. ping .......... ");
    match client.call("ping", json!({})) {
        Ok(result) => {
            if result.get("pong") == Some(&json!(true)) {
                println!("PASS ✓  (pong: true)");
                passed += 1;
            } else {
                println!("FAIL ✗  (unexpected: {result})");
                failed += 1;
            }
        }
        Err(e) => {
            println!("FAIL ✗  ({e})");
            failed += 1;
        }
    }

    // ── Test 2: echo ───────────────────────────────────────────────────────

    print!("2. echo .......... ");
    let test_msg = "Hello from the other side!";
    match client.call("echo", json!({"message": test_msg})) {
        Ok(result) => {
            if result.get("echo").and_then(|v| v.as_str()) == Some(test_msg) {
                println!("PASS ✓  (echoed correctly)");
                passed += 1;
            } else {
                println!("FAIL ✗  (unexpected: {result})");
                failed += 1;
            }
        }
        Err(e) => {
            println!("FAIL ✗  ({e})");
            failed += 1;
        }
    }

    // ── Test 3: whoami ─────────────────────────────────────────────────────

    print!("3. whoami ........ ");
    match client.call("whoami", json!({})) {
        Ok(result) => {
            let user = result.get("user").and_then(|v| v.as_str()).unwrap_or("?");
            let uid = result.get("uid").and_then(|v| v.as_u64()).unwrap_or(0);
            println!("PASS ✓  (user: {user}, uid: {uid})");
            passed += 1;

            // Highlight the isolation proof.
            let my_user = std::env::var("USER").unwrap_or_else(|_| "unknown".into());
            if user != my_user {
                println!("         ↳ Isolation verified! Daemon runs as \"{user}\", you are \"{my_user}\"");
            } else {
                println!("         ↳ Note: daemon user matches yours – for full isolation,");
                println!("           run the daemon as the carapace user.");
            }
        }
        Err(e) => {
            println!("FAIL ✗  ({e})");
            failed += 1;
        }
    }

    // ── Test 4: execute ────────────────────────────────────────────────────

    print!("4. execute ....... ");
    match client.call(
        "execute",
        json!({"command": "echo", "args": ["cross-user execution works"]}),
    ) {
        Ok(result) => {
            let stdout = result
                .get("stdout")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .trim();
            let exit_code = result.get("exit_code").and_then(|v| v.as_i64()).unwrap_or(-1);

            if exit_code == 0 && stdout == "cross-user execution works" {
                println!("PASS ✓  (exit: {exit_code}, stdout: \"{stdout}\")");
                passed += 1;
            } else {
                println!("FAIL ✗  (exit: {exit_code}, stdout: \"{stdout}\")");
                failed += 1;
            }
        }
        Err(e) => {
            println!("FAIL ✗  ({e})");
            failed += 1;
        }
    }

    // ── Test 5: error handling (unknown method) ────────────────────────────

    print!("5. error case .... ");
    match client.call("nonexistent.method", json!({})) {
        Err(carapace_client::ClientError::Gateway { code, message }) => {
            if code == -32601 {
                println!("PASS ✓  (code: {code}, msg: \"{message}\")");
                passed += 1;
            } else {
                println!("FAIL ✗  (wrong error code: {code})");
                failed += 1;
            }
        }
        Ok(result) => {
            println!("FAIL ✗  (expected error, got: {result})");
            failed += 1;
        }
        Err(e) => {
            println!("FAIL ✗  (unexpected error type: {e})");
            failed += 1;
        }
    }

    // ── Summary ────────────────────────────────────────────────────────────

    println!();
    println!("─────────────────────────────────────────────");
    println!(
        "Results: {passed} passed, {failed} failed, {} total",
        passed + failed
    );

    if failed == 0 {
        println!("All tests passed! The gateway is working.");
    } else {
        println!("Some tests failed – check the output above.");
        std::process::exit(1);
    }
}
