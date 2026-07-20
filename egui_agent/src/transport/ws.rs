//! WebSocket transport bound to 127.0.0.1 (blocking `tungstenite`; no async
//! runtime). One text frame per request/response, same JSON as the line
//! transports.

use std::net::TcpListener;

use tungstenite::Message;

use crate::config::Auth;
use crate::hub::CommandHub;
use crate::protocol::{AgentError, AuthRequest, WireRequest, WireResponse};

pub(crate) fn serve(hub: CommandHub, port: u16) {
    std::thread::Builder::new()
        .name("egui_agent-ws".into())
        .spawn(move || {
            let listener = match TcpListener::bind(("127.0.0.1", port)) {
                Ok(l) => l,
                Err(e) => {
                    log::error!("egui_agent: failed to bind 127.0.0.1:{port}: {e}");
                    return;
                }
            };
            log::info!("egui_agent: listening on ws://127.0.0.1:{port}");
            for stream in listener.incoming() {
                let Ok(stream) = stream else { continue };
                let hub = hub.clone();
                let _ = std::thread::Builder::new()
                    .name("egui_agent-ws-conn".into())
                    .spawn(move || serve_socket(hub, stream));
            }
        })
        .expect("failed to spawn WS listener thread");
}

fn serve_socket(hub: CommandHub, stream: std::net::TcpStream) {
    let mut ws = match tungstenite::accept(stream) {
        Ok(ws) => ws,
        Err(e) => {
            log::warn!("egui_agent: WS handshake failed: {e}");
            return;
        }
    };

    let send = |ws: &mut tungstenite::WebSocket<std::net::TcpStream>,
                value: &dyn erased_json::SerializeJson|
     -> bool {
        match value.to_json() {
            Ok(json) => ws.send(Message::text(json)).is_ok(),
            Err(_) => false,
        }
    };

    if let Auth::Token(expected) = &hub.config().auth {
        let ok = match next_text(&mut ws) {
            Some(line) => serde_json::from_str::<AuthRequest>(&line)
                .is_ok_and(|a| a.auth.as_bytes() == expected.as_bytes()),
            None => false,
        };
        if !ok {
            let resp = WireResponse::from_response(0, Err(AgentError::Unauthorized));
            send(&mut ws, &resp);
            let _ = ws.close(None);
            return;
        }
        send(
            &mut ws,
            &serde_json::json!({ "id": 0, "ok": "authenticated" }),
        );
    }

    while let Some(text) = next_text(&mut ws) {
        let resp = match serde_json::from_str::<WireRequest>(&text) {
            Ok(req) => WireResponse::from_response(req.id, hub.request(req.command)),
            Err(e) => WireResponse::from_response(
                0,
                Err(AgentError::BadRequest(format!("invalid request: {e}"))),
            ),
        };
        if !send(&mut ws, &resp) {
            break;
        }
    }
}

fn next_text(ws: &mut tungstenite::WebSocket<std::net::TcpStream>) -> Option<String> {
    loop {
        match ws.read() {
            Ok(Message::Text(t)) => return Some(t.to_string()),
            Ok(Message::Ping(_) | Message::Pong(_) | Message::Binary(_) | Message::Frame(_)) => {}
            Ok(Message::Close(_)) | Err(_) => return None,
        }
    }
}

/// Tiny object-safe serialization shim so `send` can take both `WireResponse`
/// and ad-hoc `serde_json::Value`s.
mod erased_json {
    pub(super) trait SerializeJson {
        fn to_json(&self) -> Result<String, serde_json::Error>;
    }

    impl<T: serde::Serialize> SerializeJson for T {
        fn to_json(&self) -> Result<String, serde_json::Error> {
            serde_json::to_string(self)
        }
    }
}
