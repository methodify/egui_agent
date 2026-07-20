//! MCP adapter for `egui_agent`.
//!
//! A thin MCP server (stdio transport, line-delimited JSON-RPC 2.0) that
//! exposes the core commands as tools and forwards each call as a
//! `Command` round-trip to a running app over its UDS control socket.
//!
//! No async runtime: MCP's stdio transport is a line protocol, and the app
//! connection is the blocking `egui_agent::client`.

use std::io::{BufRead, Write};
use std::path::PathBuf;

use egui_agent::client::Client;
use egui_agent::{AgentError, Command, Predicate, Response, Selector};
use serde_json::{json, Value};

/// Supported MCP protocol revisions (newest first).
const PROTOCOL_VERSIONS: &[&str] = &["2025-06-18", "2025-03-26", "2024-11-05"];

/// Connection settings for the app's control socket.
pub struct AppConnection {
    pub socket: PathBuf,
    pub token: Option<String>,
    client: Option<Client<std::os::unix::net::UnixStream>>,
}

impl AppConnection {
    pub fn new(socket: PathBuf, token: Option<String>) -> Self {
        Self {
            socket,
            token,
            client: None,
        }
    }

    /// Send a command, (re)connecting lazily. One reconnect attempt on IO
    /// error so an app restart doesn't wedge the MCP session.
    pub fn request(&mut self, cmd: Command) -> Result<Response, std::io::Error> {
        if self.client.is_none() {
            self.client = Some(self.connect()?);
        }
        match self.client.as_mut().unwrap().request(cmd.clone()) {
            Ok(resp) => Ok(resp),
            Err(_) => {
                self.client = Some(self.connect()?);
                self.client.as_mut().unwrap().request(cmd)
            }
        }
    }

    fn connect(&self) -> std::io::Result<Client<std::os::unix::net::UnixStream>> {
        egui_agent::client::connect_uds(&self.socket, self.token.as_deref())
    }
}

/// Tool definitions advertised by `tools/list`.
pub fn tool_definitions() -> Value {
    let selector_schema = json!({
        "description": "Widget selector: {\"id\": \"...\"} (stable semantic id, preferred), {\"label\": \"...\"}, {\"role\": {\"role\": \"button\", \"name\": \"...\"}}, or {\"node\": 123}.",
        "type": "object"
    });
    json!([
        {
            "name": "snapshot",
            "description": "Full structured snapshot of the app's current UI/state: nodes with id, role, label, value, enabled, focused, actions. Reason over this instead of screenshots. Call it first to discover available targets and actions.",
            "inputSchema": { "type": "object", "properties": {}, "additionalProperties": false }
        },
        {
            "name": "query",
            "description": "Query snapshot nodes matching a selector (cheaper than a full snapshot).",
            "inputSchema": {
                "type": "object",
                "properties": { "target": selector_schema },
                "required": ["target"]
            }
        },
        {
            "name": "invoke",
            "description": "Invoke a named action, optionally on a target widget. Actions are listed per-node in the snapshot (e.g. \"click\") plus app-level verbs.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "action": { "type": "string" },
                    "target": selector_schema,
                    "args": { "description": "Optional action arguments (any JSON)." }
                },
                "required": ["action"]
            }
        },
        {
            "name": "set_value",
            "description": "Set a targeted value: text field (string), slider (number), checkbox (bool)...",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "target": selector_schema,
                    "value": { "description": "New value (any JSON)." }
                },
                "required": ["target", "value"]
            }
        },
        {
            "name": "wait_for",
            "description": "Block until a predicate over the snapshot holds or the timeout expires. Predicates: {\"exists\": SELECTOR}, {\"not_exists\": SELECTOR}, {\"value_eq\": {\"target\": SELECTOR, \"value\": ...}}, {\"enabled\"|\"disabled\"|\"focused\": SELECTOR}, {\"data_eq\": {\"pointer\": \"/path\", \"value\": ...}}, {\"all\"|\"any\": [..]}, {\"not\": ..}.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "predicate": { "type": "object" },
                    "timeout_ms": { "type": "integer", "default": 5000 }
                },
                "required": ["predicate"]
            }
        },
        {
            "name": "screenshot",
            "description": "Optional fallback: PNG of the current frame (base64). Prefer snapshot/query.",
            "inputSchema": { "type": "object", "properties": {}, "additionalProperties": false }
        },
        {
            "name": "custom",
            "description": "Invoke an app-specific verb outside the standard action set.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "name": { "type": "string" },
                    "args": { "description": "Verb arguments (any JSON)." }
                },
                "required": ["name"]
            }
        }
    ])
}

fn parse_selector(v: &Value) -> Result<Selector, String> {
    serde_json::from_value(v.clone()).map_err(|e| format!("invalid selector: {e}"))
}

/// Map an MCP tool call onto a protocol `Command`.
pub fn command_for_tool(name: &str, args: &Value) -> Result<Command, String> {
    match name {
        "snapshot" => Ok(Command::Snapshot),
        "query" => Ok(Command::Query {
            target: parse_selector(args.get("target").ok_or("missing `target`")?)?,
        }),
        "invoke" => Ok(Command::Invoke {
            target: args.get("target").map(parse_selector).transpose()?,
            action: args
                .get("action")
                .and_then(Value::as_str)
                .ok_or("missing `action`")?
                .to_owned(),
            args: args.get("args").cloned(),
        }),
        "set_value" => Ok(Command::SetValue {
            target: parse_selector(args.get("target").ok_or("missing `target`")?)?,
            value: args.get("value").cloned().ok_or("missing `value`")?,
        }),
        "wait_for" => {
            let predicate: Predicate = serde_json::from_value(
                args.get("predicate")
                    .cloned()
                    .ok_or("missing `predicate`")?,
            )
            .map_err(|e| format!("invalid predicate: {e}"))?;
            let timeout_ms = args
                .get("timeout_ms")
                .and_then(Value::as_u64)
                .unwrap_or(5000);
            Ok(Command::WaitFor {
                predicate,
                timeout_ms,
            })
        }
        "screenshot" => Ok(Command::Screenshot),
        "custom" => Ok(Command::Custom {
            name: args
                .get("name")
                .and_then(Value::as_str)
                .ok_or("missing `name`")?
                .to_owned(),
            args: args.get("args").cloned().unwrap_or(Value::Null),
        }),
        other => Err(format!("unknown tool: {other}")),
    }
}

/// Run the MCP server loop over the given streams until EOF.
pub fn serve(conn: &mut AppConnection, reader: impl BufRead, mut writer: impl Write) {
    for line in reader.lines() {
        let Ok(line) = line else { break };
        if line.trim().is_empty() {
            continue;
        }
        let msg: Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(e) => {
                log::warn!("egui_agent_mcp: unparseable message: {e}");
                continue;
            }
        };
        let id = msg.get("id").cloned();
        let method = msg.get("method").and_then(Value::as_str).unwrap_or("");
        // Notifications (no id) get no response.
        let Some(id) = id else {
            log::debug!("egui_agent_mcp: notification {method}");
            continue;
        };
        let reply = handle_request(conn, method, msg.get("params").unwrap_or(&Value::Null));
        let out = match reply {
            Ok(result) => json!({ "jsonrpc": "2.0", "id": id, "result": result }),
            Err((code, message)) => json!({
                "jsonrpc": "2.0", "id": id,
                "error": { "code": code, "message": message }
            }),
        };
        if writeln!(writer, "{out}")
            .and_then(|_| writer.flush())
            .is_err()
        {
            break;
        }
    }
}

fn handle_request(
    conn: &mut AppConnection,
    method: &str,
    params: &Value,
) -> Result<Value, (i64, String)> {
    match method {
        "initialize" => {
            let requested = params
                .get("protocolVersion")
                .and_then(Value::as_str)
                .unwrap_or(PROTOCOL_VERSIONS[0]);
            let version = if PROTOCOL_VERSIONS.contains(&requested) {
                requested
            } else {
                PROTOCOL_VERSIONS[0]
            };
            Ok(json!({
                "protocolVersion": version,
                "capabilities": { "tools": {} },
                "serverInfo": {
                    "name": "egui_agent_mcp",
                    "version": env!("CARGO_PKG_VERSION"),
                },
                "instructions": "Drives a running egui app through semantic commands. Call `snapshot` first to discover nodes (ids, roles, values, actions), then `invoke`/`set_value` on stable ids. Use `wait_for` instead of polling.",
            }))
        }
        "ping" => Ok(json!({})),
        "tools/list" => Ok(json!({ "tools": tool_definitions() })),
        "tools/call" => {
            let name = params
                .get("name")
                .and_then(Value::as_str)
                .ok_or((-32602, "missing tool name".to_owned()))?;
            let default_args = json!({});
            let args = params.get("arguments").unwrap_or(&default_args);
            let cmd = match command_for_tool(name, args) {
                Ok(cmd) => cmd,
                Err(e) => return Ok(tool_error(e)),
            };
            match conn.request(cmd) {
                Ok(Ok(body)) => Ok(tool_ok(&body)),
                Ok(Err(agent_err)) => Ok(tool_error(format_agent_error(&agent_err))),
                Err(io) => Ok(tool_error(format!(
                    "cannot reach app at {:?}: {io}. Is the app running with its agent socket enabled?",
                    conn.socket
                ))),
            }
        }
        _ => Err((-32601, format!("method not found: {method}"))),
    }
}

fn tool_ok(body: &egui_agent::ResponseBody) -> Value {
    // Screenshots become proper MCP image content; everything else is JSON text.
    if let egui_agent::ResponseBody::Image { format, data, .. } = body {
        return json!({
            "content": [{ "type": "image", "data": data, "mimeType": format!("image/{format}") }],
            "isError": false
        });
    }
    let text =
        serde_json::to_string_pretty(body).unwrap_or_else(|e| format!("<serialize error: {e}>"));
    json!({ "content": [{ "type": "text", "text": text }], "isError": false })
}

fn tool_error(message: String) -> Value {
    json!({ "content": [{ "type": "text", "text": message }], "isError": true })
}

fn format_agent_error(e: &AgentError) -> String {
    let detail = serde_json::to_string(e).unwrap_or_default();
    format!("{e} ({detail})")
}
