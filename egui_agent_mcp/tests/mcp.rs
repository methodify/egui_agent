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

/// Run a scripted MCP session against a UDS-served app; returns one parsed
/// JSON-RPC response per request line.
fn run_session(requests: Vec<Value>, token: Option<&str>) -> Vec<Value> {
    let sock = std::env::temp_dir().join(format!(
        "egui_agent_mcp_test_{}_{:?}.sock",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_file(&sock);

    let control = Control::spawn(Config {
        transport: Transport::UnixSocket(sock.clone()),
        auth: token.map(|t| Auth::Token(t.into())).unwrap_or(Auth::None),
        ..Default::default()
    });
    let agent = egui_agent::wrap(MiniApp::default(), control);

    let done = Arc::new(AtomicBool::new(false));
    let done2 = done.clone();
    let sock2 = sock.clone();
    let token = token.map(str::to_owned);
    let mcp = std::thread::spawn(move || {
        // Wait until the socket exists before starting the MCP loop.
        for _ in 0..200 {
            if sock2.exists() {
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        let mut conn = egui_agent_mcp::AppConnection::new(sock2, token);
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
