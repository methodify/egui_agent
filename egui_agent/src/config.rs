//! Runtime configuration for the control channel.

use std::path::PathBuf;
use std::time::Duration;

/// Which transport(s) the control channel serves.
#[derive(Debug, Clone, Default, PartialEq)]
pub enum Transport {
    /// No external transport; in-process only (tests, CLI embedding).
    #[default]
    None,
    /// Unix domain socket at the given path, newline-delimited JSON.
    #[cfg(all(feature = "uds", unix))]
    UnixSocket(PathBuf),
    /// WebSocket bound to `127.0.0.1:port`.
    #[cfg(feature = "ws")]
    WebSocket { port: u16 },
    /// Newline-delimited JSON over stdin/stdout.
    #[cfg(feature = "stdio")]
    Stdio,
    /// Cross-platform local socket (Windows named pipe / UDS elsewhere) by name.
    #[cfg(feature = "pipe")]
    Pipe(String),
}

/// Auth policy for socket transports.
#[derive(Debug, Clone, Default, PartialEq)]
pub enum Auth {
    /// No token required (still local-only by construction).
    #[default]
    None,
    /// Clients must present this token as their first message.
    Token(String),
}

impl Auth {
    /// Read a token from an environment variable; `Auth::None` if unset/empty.
    pub fn from_env(var: &str) -> Self {
        match std::env::var(var) {
            Ok(t) if !t.is_empty() => Auth::Token(t),
            _ => Auth::None,
        }
    }
}

/// When an actuating command's response snapshot is considered settled.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum SettleUntil {
    /// Capture the post-action snapshot on the next frame (default).
    #[default]
    NextFrame,
    /// Run frames until no repaint is pending or `max_frames` elapse.
    Idle { max_frames: u32 },
}

/// Control channel configuration. Start from `Config::default()` and override.
#[derive(Debug, Clone)]
pub struct Config {
    /// Master switch: when false, `Control::spawn` starts nothing and every
    /// enqueued command errors. Wire this to an env var so even feature-enabled
    /// builds stay inert unless opted in.
    pub enabled: bool,
    pub transport: Transport,
    pub auth: Auth,
    /// Permit Snapshot/Query/WaitFor/Screenshot but reject Invoke/SetValue/Custom.
    pub read_only: bool,
    /// Append a JSON line per command to this file (audit trail).
    pub audit_log: Option<PathBuf>,
    /// Settle policy for post-action snapshots.
    pub settle: SettleUntil,
    /// Attach a post-settle snapshot to Invoke/SetValue responses.
    pub post_action_snapshot: bool,
    /// How long a transport waits for the UI thread before answering Timeout.
    pub request_timeout: Duration,
    /// Repaint poll cadence while WaitFor/settle commands are pending.
    pub poll_interval: Duration,
    /// Frames an actuation may wait for its target widget to appear before
    /// resolving to TargetNotFound (registry-dispatched commands only).
    pub actuation_grace_frames: u32,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            enabled: true,
            transport: Transport::default(),
            auth: Auth::default(),
            read_only: false,
            audit_log: None,
            settle: SettleUntil::default(),
            post_action_snapshot: true,
            request_timeout: Duration::from_secs(10),
            poll_interval: Duration::from_millis(50),
            actuation_grace_frames: 2,
        }
    }
}
