//! `Control`: owns the queue and (optionally) serves a transport.

use std::sync::{Arc, Mutex};

use crate::agent::Agent;
use crate::app::AgentApp;
use crate::config::{Config, Transport};
use crate::hub::{CommandHub, HubShared};

/// Handle created at startup (before the UI exists) that owns the command
/// queue and any transport threads. Clone freely; hand one clone to
/// [`wrap`] and keep others for in-process control.
#[derive(Clone)]
pub struct Control {
    hub: CommandHub,
}

impl Control {
    /// Create the queue and start the configured transport (if any and if
    /// `cfg.enabled`). Call before `eframe::run_native`; transports queue
    /// commands even before the UI starts (they're answered once frames run).
    pub fn spawn(cfg: Config) -> Self {
        let (tx, rx) = flume::unbounded();
        let hub = CommandHub {
            shared: Arc::new(HubShared {
                cfg,
                tx,
                rx,
                ctx: Mutex::new(None),
            }),
        };
        let control = Self { hub };
        if control.hub.config().enabled {
            control.start_transport();
        } else {
            log::info!("egui_agent: control channel disabled by config");
        }
        control
    }

    /// Bind the egui context used to wake the UI thread on enqueue.
    /// Usually called from the eframe creation closure; `Agent<A>` also
    /// self-binds on its first frame as a fallback.
    pub fn bind_context(&self, ctx: egui::Context) {
        self.hub.bind_context(ctx);
    }

    /// The enqueue-and-await handle (for in-process control and custom transports).
    pub fn hub(&self) -> CommandHub {
        self.hub.clone()
    }

    fn start_transport(&self) {
        match &self.hub.config().transport {
            Transport::None => {}
            #[cfg(all(feature = "uds", unix))]
            Transport::UnixSocket(path) => {
                crate::transport::uds::serve(self.hub.clone(), path.clone());
            }
            #[cfg(feature = "ws")]
            Transport::WebSocket { port } => {
                crate::transport::ws::serve(self.hub.clone(), *port);
            }
            #[cfg(feature = "stdio")]
            Transport::Stdio => {
                crate::transport::stdio::serve(self.hub.clone());
            }
            #[cfg(feature = "pipe")]
            Transport::Pipe(name) => {
                crate::transport::pipe::serve(self.hub.clone(), name.clone());
            }
        }
    }
}

/// Wrap an app for agent control: `Agent<A>` implements `eframe::App` (when
/// `A` does) and drives the command lifecycle around the app's own `update`.
pub fn wrap<A: AgentApp>(app: A, control: Control) -> Agent<A> {
    Agent::new(app, control.hub())
}
