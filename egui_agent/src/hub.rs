//! The command queue bridging transport threads and the UI thread.
//!
//! Transports enqueue `(Command, reply sender)` and wake the UI with
//! `request_repaint()`; `Agent<A>` drains the queue inside `update()`.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::config::Config;
use crate::protocol::{AgentError, Command, Response};

/// One queued command plus its reply channel (a bounded-1 flume channel acts
/// as the oneshot; flume serves both sync and async consumers).
pub(crate) struct Envelope {
    pub cmd: Command,
    pub reply: flume::Sender<Response>,
}

/// Shared state between `Control`, transports, and the UI-side `Agent<A>`.
pub(crate) struct HubShared {
    pub cfg: Config,
    pub tx: flume::Sender<Envelope>,
    pub rx: flume::Receiver<Envelope>,
    /// Cloned egui context, bound once the app starts; used purely to
    /// request repaints from transport threads.
    pub ctx: Mutex<Option<egui::Context>>,
}

/// Enqueue-and-await handle used by transports, the MCP adapter, and tests.
///
/// Cloneable and `Send`; every enqueue wakes the UI thread.
#[derive(Clone)]
pub struct CommandHub {
    pub(crate) shared: Arc<HubShared>,
}

impl CommandHub {
    /// Enqueue a command and get a receiver for its eventual response.
    /// Wakes the UI thread via `request_repaint()`.
    pub fn submit(&self, cmd: Command) -> flume::Receiver<Response> {
        let (tx, rx) = flume::bounded(1);
        if !self.shared.cfg.enabled {
            let _ = tx.send(Err(AgentError::NotSupported(
                "agent control disabled".into(),
            )));
            return rx;
        }
        let _ = self.shared.tx.send(Envelope { cmd, reply: tx });
        self.request_repaint();
        rx
    }

    /// Enqueue and block for the response (transport threads).
    /// Times out per `Config::request_timeout`.
    pub fn request(&self, cmd: Command) -> Response {
        let timeout = self.shared.cfg.request_timeout;
        self.request_with_timeout(cmd, timeout)
    }

    pub fn request_with_timeout(&self, cmd: Command, timeout: Duration) -> Response {
        let rx = self.submit(cmd);
        match rx.recv_timeout(timeout) {
            Ok(resp) => resp,
            Err(_) => Err(AgentError::Timeout(format!(
                "UI thread did not answer within {timeout:?}"
            ))),
        }
    }

    /// Bind lazily from the UI thread if `Control::bind_context` wasn't called.
    pub(crate) fn bind_context_if_needed(&self, ctx: &egui::Context) {
        let mut guard = self.shared.ctx.lock().unwrap();
        if guard.is_none() {
            *guard = Some(ctx.clone());
        }
    }

    pub(crate) fn bind_context(&self, ctx: egui::Context) {
        *self.shared.ctx.lock().unwrap() = Some(ctx);
        // Wake immediately in case commands queued before the app started.
        self.request_repaint();
    }

    pub(crate) fn request_repaint(&self) {
        if let Some(ctx) = self.shared.ctx.lock().unwrap().as_ref() {
            ctx.request_repaint();
        }
    }

    pub(crate) fn config(&self) -> &Config {
        &self.shared.cfg
    }
}
