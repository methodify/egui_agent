//! M2 milestone test: a full MCP session (initialize → tools/list →
//! tools/call) bridged to a live app over a real UDS control socket.

use std::io::Cursor;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use egui_agent::{Auth, Config, Control, Transport};
use serde_json::{json, Value};

#[derive(Default)]
struct MiniApp {
    label: String,
}

impl egui_agent::AgentApp for MiniApp {}

/// Which control-channel flavor a session should use.
enum Channel {
    Uds,
    Pipe,
}

/// Run a scripted MCP session against a served app; returns one parsed
/// JSON-RPC response per request line.
fn run_session_over(channel: Channel, requests: Vec<Value>, token: Option<&str>) -> Vec<Value> {
    let unique = format!(
        "egui_agent_mcp_test_{}_{:?}",
        std::process::id(),
        std::thread::current().id()
    );
    let sock = std::env::temp_dir().join(format!("{unique}.sock"));
    let _ = std::fs::remove_file(&sock);

    let (transport, endpoint) = match channel {
        Channel::Uds => (
            Transport::UnixSocket(sock.clone()),
            egui_agent_mcp::Endpoint::Uds(sock.clone()),
        ),
        Channel::Pipe => (
            Transport::Pipe(unique.clone()),
            egui_agent_mcp::Endpoint::Pipe(unique.clone()),
        ),
    };

    let control = Control::spawn(Config {
        transport,
        auth: token.map(|t| Auth::Token(t.into())).unwrap_or(Auth::None),
        ..Default::default()
    });
    let agent = egui_agent::wrap(MiniApp::default(), control);

    let done = Arc::new(AtomicBool::new(false));
    let done2 = done.clone();
    let token = token.map(str::to_owned);
    let mcp = std::thread::spawn(move || {
        // Give the listener a moment to bind; the connection itself also
        // retries lazily inside AppConnection.
        std::thread::sleep(Duration::from_millis(50));
        let mut conn = egui_agent_mcp::AppConnection::from_endpoint(endpoint, token);
        let input = requests
            .iter()
            .map(|r| serde_json::to_string(r).unwrap())
            .collect::<Vec<_>>()
            .join("\n");
        let mut output = Vec::new();
        egui_agent_mcp::serve(&mut conn, Cursor::new(input), &mut output);
        done2.store(true, Ordering::SeqCst);
        String::from_utf8(output).unwrap()
    });

    // Pump UI frames until the MCP session ends.
    let mut harness = egui_kittest::Harness::new_ui_state(
        |ui, agent: &mut egui_agent::Agent<MiniApp>| {
            let ctx = ui.ctx().clone();
            agent.pre_frame(&ctx);
            {
                let app = agent.inner_mut();
                egui_agent::widgets::text_edit_singleline(ui, "label", &mut app.label);
            }
            agent.post_frame(&ctx);
        },
        agent,
    );
    let deadline = Instant::now() + Duration::from_secs(10);
    while !done.load(Ordering::SeqCst) {
        assert!(Instant::now() < deadline, "MCP test timed out");
        harness.step();
        std::thread::sleep(Duration::from_millis(2));
    }

    let out = mcp.join().unwrap();
    let _ = std::fs::remove_file(sock);
    out.lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect()
}

fn run_session(requests: Vec<Value>, token: Option<&str>) -> Vec<Value> {
    run_session_over(Channel::Uds, requests, token)
}

#[test]
fn full_mcp_session() {
    let responses = run_session(
        vec![
            json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize",
                    "params": { "protocolVersion": "2025-06-18",
                                 "clientInfo": { "name": "test", "version": "0" },
                                 "capabilities": {} } }),
            json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }),
            json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list" }),
            json!({ "jsonrpc": "2.0", "id": 3, "method": "tools/call",
                    "params": { "name": "wait_for", "arguments": {
                        "predicate": { "exists": { "id": "label" } },
                        "timeout_ms": 5000 } } }),
            json!({ "jsonrpc": "2.0", "id": 4, "method": "tools/call",
                    "params": { "name": "set_value", "arguments": {
                        "target": { "id": "label" }, "value": "from MCP" } } }),
            json!({ "jsonrpc": "2.0", "id": 5, "method": "tools/call",
                    "params": { "name": "snapshot", "arguments": {} } }),
            json!({ "jsonrpc": "2.0", "id": 6, "method": "tools/call",
                    "params": { "name": "no_such_tool", "arguments": {} } }),
            json!({ "jsonrpc": "2.0", "id": 7, "method": "bogus/method" }),
        ],
        None,
    );

    // Notification produced no response: 7 responses for 8 lines.
    assert_eq!(responses.len(), 7);

    let init = &responses[0];
    assert_eq!(init["result"]["protocolVersion"], "2025-06-18");
    assert_eq!(init["result"]["serverInfo"]["name"], "egui_agent_mcp");

    let tools = init_tools(&responses[1]);
    for expected in [
        "snapshot",
        "query",
        "invoke",
        "set_value",
        "wait_for",
        "screenshot",
        "custom",
    ] {
        assert!(
            tools.contains(&expected.to_string()),
            "missing tool {expected}"
        );
    }

    assert_eq!(
        responses[2]["result"]["isError"], false,
        "wait_for: {:?}",
        responses[2]
    );
    assert_eq!(
        responses[3]["result"]["isError"], false,
        "set_value: {:?}",
        responses[3]
    );

    let snap_text = responses[4]["result"]["content"][0]["text"]
        .as_str()
        .unwrap();
    assert!(
        snap_text.contains("from MCP"),
        "snapshot should show the set value: {snap_text}"
    );

    assert_eq!(
        responses[5]["result"]["isError"], true,
        "unknown tool is a tool error"
    );
    assert_eq!(
        responses[6]["error"]["code"], -32601,
        "unknown method is a JSON-RPC error"
    );
}

#[test]
fn mcp_session_with_auth_token() {
    let responses = run_session(
        vec![json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/call",
                    "params": { "name": "wait_for", "arguments": {
                        "predicate": { "exists": { "id": "label" } },
                        "timeout_ms": 5000 } } })],
        Some("hunter2"),
    );
    assert_eq!(responses.len(), 1);
    assert_eq!(
        responses[0]["result"]["isError"], false,
        "{:?}",
        responses[0]
    );
}

fn init_tools(list_response: &Value) -> Vec<String> {
    list_response["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap().to_owned())
        .collect()
}

#[test]
fn mcp_session_over_pipe() {
    let responses = run_session_over(
        Channel::Pipe,
        vec![
            json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/call",
                    "params": { "name": "wait_for", "arguments": {
                        "predicate": { "exists": { "id": "label" } },
                        "timeout_ms": 5000 } } }),
            json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/call",
                    "params": { "name": "set_value", "arguments": {
                        "target": { "id": "label" }, "value": "via pipe" } } }),
            json!({ "jsonrpc": "2.0", "id": 3, "method": "tools/call",
                    "params": { "name": "snapshot", "arguments": {} } }),
        ],
        Some("pipe-token"),
    );
    assert_eq!(responses.len(), 3);
    assert_eq!(
        responses[0]["result"]["isError"], false,
        "{:?}",
        responses[0]
    );
    assert_eq!(
        responses[1]["result"]["isError"], false,
        "{:?}",
        responses[1]
    );
    let snap_text = responses[2]["result"]["content"][0]["text"]
        .as_str()
        .unwrap();
    assert!(snap_text.contains("via pipe"), "{snap_text}");
}
