//! `egui-agent-mcp`: MCP server (stdio) bridging an MCP host to a running
//! egui app's control socket.
//!
//! Usage: `egui-agent-mcp --socket /tmp/myapp.agent.sock [--token TOKEN]`
//! (or env `EGUI_AGENT_SOCKET` / `EGUI_AGENT_TOKEN`).

use std::path::PathBuf;

use egui_agent_mcp::AppConnection;

fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"))
        .target(env_logger::Target::Stderr)
        .init();

    let mut socket: Option<PathBuf> = std::env::var_os("EGUI_AGENT_SOCKET").map(Into::into);
    let mut token: Option<String> = std::env::var("EGUI_AGENT_TOKEN").ok();

    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--socket" => socket = args.next().map(Into::into),
            "--token" => token = args.next(),
            "--help" | "-h" => {
                eprintln!(
                    "egui-agent-mcp: MCP stdio server for egui_agent apps\n\n\
                     Usage: egui-agent-mcp --socket <PATH> [--token <TOKEN>]\n\
                     Env:   EGUI_AGENT_SOCKET, EGUI_AGENT_TOKEN"
                );
                return;
            }
            other => {
                eprintln!("unknown argument: {other} (see --help)");
                std::process::exit(2);
            }
        }
    }

    let Some(socket) = socket else {
        eprintln!("missing --socket (or EGUI_AGENT_SOCKET)");
        std::process::exit(2);
    };

    let mut conn = AppConnection::new(socket, token);
    let stdin = std::io::stdin().lock();
    let stdout = std::io::stdout().lock();
    egui_agent_mcp::serve(&mut conn, stdin, stdout);
}
