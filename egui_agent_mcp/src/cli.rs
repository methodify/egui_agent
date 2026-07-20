//! `egui-agent-cli`: command-line driver for a running egui_agent app.
//!
//! ```text
//! egui-agent-cli --socket /tmp/app.sock snapshot
//! egui-agent-cli --socket /tmp/app.sock invoke click --target '{"id":"submit"}'
//! egui-agent-cli --socket /tmp/app.sock set-value '{"id":"name"}' '"Ada"'
//! egui-agent-cli --socket /tmp/app.sock wait-for '{"exists":{"id":"done"}}' --timeout 3000
//! egui-agent-cli --socket /tmp/app.sock repl
//! ```

use std::io::Write as _;
use std::path::PathBuf;

use egui_agent::{Command, Selector};
use egui_agent_mcp::AppConnection;
use serde_json::Value;

fn main() {
    let mut socket: Option<PathBuf> = std::env::var_os("EGUI_AGENT_SOCKET").map(Into::into);
    let mut pipe: Option<String> = std::env::var("EGUI_AGENT_PIPE").ok();
    let mut token: Option<String> = std::env::var("EGUI_AGENT_TOKEN").ok();
    let mut timeout_ms: u64 = 5000;
    let mut target: Option<String> = None;
    let mut args_json: Option<String> = None;
    let mut positional: Vec<String> = Vec::new();

    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--socket" => socket = args.next().map(Into::into),
            "--pipe" => pipe = args.next(),
            "--token" => token = args.next(),
            "--timeout" => {
                timeout_ms = args
                    .next()
                    .and_then(|s| s.parse().ok())
                    .unwrap_or_else(|| die("--timeout needs a millisecond value"))
            }
            "--target" => target = args.next(),
            "--args" => args_json = args.next(),
            "--help" | "-h" => {
                usage();
                return;
            }
            other => positional.push(other.to_owned()),
        }
    }

    let endpoint = egui_agent_mcp::resolve_endpoint(socket, pipe).unwrap_or_else(|msg| die(&msg));
    let mut conn = AppConnection::from_endpoint(endpoint, token);

    let Some(verb) = positional.first().cloned() else {
        usage();
        std::process::exit(2);
    };

    if verb == "repl" {
        repl(&mut conn);
        return;
    }

    let cmd = build_command(&verb, &positional[1..], target, args_json, timeout_ms);
    run(&mut conn, cmd);
}

fn build_command(
    verb: &str,
    rest: &[String],
    target: Option<String>,
    args_json: Option<String>,
    timeout_ms: u64,
) -> Command {
    let parse_selector = |s: &str| -> Selector {
        serde_json::from_str(s).unwrap_or_else(|e| die(&format!("invalid selector {s:?}: {e}")))
    };
    let parse_value = |s: &str| -> Value {
        serde_json::from_str(s).unwrap_or_else(|e| die(&format!("invalid JSON {s:?}: {e}")))
    };
    match verb {
        "snapshot" => Command::Snapshot,
        "query" => {
            let sel = rest
                .first()
                .unwrap_or_else(|| die("query needs a selector JSON argument"));
            Command::Query {
                target: parse_selector(sel),
            }
        }
        "invoke" => {
            let action = rest
                .first()
                .unwrap_or_else(|| die("invoke needs an action name"));
            Command::Invoke {
                target: target.as_deref().map(parse_selector),
                action: action.clone(),
                args: args_json.as_deref().map(parse_value),
            }
        }
        "set-value" => {
            let (sel, value) = match rest {
                [s, v, ..] => (s, v),
                _ => die("set-value needs a selector JSON and a value JSON"),
            };
            Command::SetValue {
                target: parse_selector(sel),
                value: parse_value(value),
            }
        }
        "wait-for" => {
            let pred = rest
                .first()
                .unwrap_or_else(|| die("wait-for needs a predicate JSON"));
            Command::WaitFor {
                predicate: serde_json::from_str(pred)
                    .unwrap_or_else(|e| die(&format!("invalid predicate: {e}"))),
                timeout_ms,
            }
        }
        "screenshot" => Command::Screenshot,
        "custom" => {
            let name = rest
                .first()
                .unwrap_or_else(|| die("custom needs a verb name"));
            Command::Custom {
                name: name.clone(),
                args: args_json.as_deref().map(parse_value).unwrap_or(Value::Null),
            }
        }
        other => die(&format!("unknown verb {other:?} (see --help)")),
    }
}

fn run(conn: &mut AppConnection, cmd: Command) {
    match conn.request(cmd) {
        Ok(Ok(body)) => {
            println!(
                "{}",
                serde_json::to_string_pretty(&body).expect("response serializes")
            );
        }
        Ok(Err(e)) => {
            eprintln!("agent error: {e}");
            std::process::exit(1);
        }
        Err(io) => {
            eprintln!("connection error: {io}");
            std::process::exit(1);
        }
    }
}

/// Line REPL: each line is a raw `Command` as JSON, e.g. `"snapshot"` or
/// `{"invoke": {"action": "click", "target": {"id": "submit"}}}`.
fn repl(conn: &mut AppConnection) {
    let stdin = std::io::stdin();
    let mut line = String::new();
    loop {
        eprint!("agent> ");
        let _ = std::io::stderr().flush();
        line.clear();
        match stdin.read_line(&mut line) {
            Ok(0) | Err(_) => return,
            Ok(_) => {}
        }
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed == "exit" || trimmed == "quit" {
            if trimmed.is_empty() {
                continue;
            }
            return;
        }
        match serde_json::from_str::<Command>(trimmed) {
            Ok(cmd) => match conn.request(cmd) {
                Ok(Ok(body)) => println!(
                    "{}",
                    serde_json::to_string_pretty(&body).expect("response serializes")
                ),
                Ok(Err(e)) => eprintln!("agent error: {e}"),
                Err(io) => eprintln!("connection error: {io}"),
            },
            Err(e) => eprintln!("invalid command: {e}"),
        }
    }
}

fn usage() {
    eprintln!(
        "egui-agent-cli: drive a running egui_agent app\n\n\
         Usage: egui-agent-cli (--socket <PATH> | --pipe <NAME>) [--token <TOKEN>] <verb> [args]\n\n\
         Verbs:\n\
         \x20 snapshot\n\
         \x20 query '<selector-json>'\n\
         \x20 invoke <action> [--target '<selector-json>'] [--args '<json>']\n\
         \x20 set-value '<selector-json>' '<value-json>'\n\
         \x20 wait-for '<predicate-json>' [--timeout <ms>]\n\
         \x20 screenshot\n\
         \x20 custom <name> [--args '<json>']\n\
         \x20 repl\n\n\
         --socket: Unix domain socket path (Linux/macOS)\n\
         --pipe:   local socket name / Windows named pipe (Transport::Pipe)\n\
         Env: EGUI_AGENT_SOCKET, EGUI_AGENT_PIPE, EGUI_AGENT_TOKEN"
    );
}

fn die(msg: &str) -> ! {
    eprintln!("{msg}");
    std::process::exit(2);
}
