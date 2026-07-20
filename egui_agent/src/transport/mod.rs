//! Transport adapters: translate an inbound byte stream into `Command`s,
//! push them through the [`CommandHub`], and serialize responses back.
//!
//! All transports are local-only by construction (UDS / named pipe /
//! 127.0.0.1) and run on plain std threads — no async runtime in the core.

#[cfg(feature = "uds")]
pub mod uds;

#[cfg(feature = "stdio")]
pub mod stdio;

#[cfg(feature = "ws")]
pub mod ws;

#[cfg(feature = "pipe")]
pub mod pipe;

#[cfg(any(feature = "uds", feature = "stdio", feature = "pipe"))]
mod line_protocol {
    use std::io::{BufRead, Write};

    use crate::config::Auth;
    use crate::hub::CommandHub;
    use crate::protocol::{AgentError, AuthRequest, WireRequest, WireResponse};

    /// Serve one newline-delimited-JSON connection until EOF.
    ///
    /// If an auth token is configured, the first line must be
    /// `{"auth": "<token>"}`; anything else closes the connection with an
    /// `unauthorized` error.
    pub(crate) fn serve_connection(hub: &CommandHub, reader: impl BufRead, mut writer: impl Write) {
        let mut lines = reader.lines();

        if let Auth::Token(expected) = &hub.config().auth {
            let ok = match lines.next() {
                Some(Ok(line)) => serde_json::from_str::<AuthRequest>(&line)
                    .is_ok_and(|a| constant_time_eq(a.auth.as_bytes(), expected.as_bytes())),
                _ => false,
            };
            if !ok {
                let resp = WireResponse::from_response(0, Err(AgentError::Unauthorized));
                let _ = write_line(&mut writer, &resp);
                return;
            }
            let _ = write_line(
                &mut writer,
                &serde_json::json!({ "id": 0, "ok": "authenticated" }),
            );
        }

        for line in lines {
            let Ok(line) = line else { break };
            if line.trim().is_empty() {
                continue;
            }
            let resp = match serde_json::from_str::<WireRequest>(&line) {
                Ok(req) => WireResponse::from_response(req.id, hub.request(req.command)),
                Err(e) => WireResponse::from_response(
                    0,
                    Err(AgentError::BadRequest(format!("invalid request: {e}"))),
                ),
            };
            if write_line(&mut writer, &resp).is_err() {
                break;
            }
        }
    }

    fn write_line(writer: &mut impl Write, value: &impl serde::Serialize) -> std::io::Result<()> {
        let json = serde_json::to_string(value)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        writeln!(writer, "{json}")?;
        writer.flush()
    }

    fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
        if a.len() != b.len() {
            return false;
        }
        a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
    }
}

#[cfg(any(feature = "uds", feature = "stdio", feature = "pipe"))]
pub(crate) use line_protocol::serve_connection;
