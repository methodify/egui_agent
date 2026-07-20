//! Optional audit trail: one JSON line per executed command.

use std::io::Write as _;

use crate::config::Config;
use crate::protocol::Command;

pub(crate) fn log_command(cfg: &Config, cmd: &Command) {
    log::debug!(target: "egui_agent::audit", "command: {cmd:?}");
    let Some(path) = &cfg.audit_log else { return };
    let ts_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    let line = serde_json::json!({ "ts_ms": ts_ms, "command": cmd });
    let result = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .and_then(|mut f| writeln!(f, "{line}"));
    if let Err(e) = result {
        log::warn!(target: "egui_agent::audit", "failed to write audit log {path:?}: {e}");
    }
}
