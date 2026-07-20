//! `egui_agent` — a reusable agent control channel for egui/eframe apps.
//!
//! Lets an external agent (MCP, socket, or in-process tests) drive and
//! introspect an egui app through **semantic commands and structured
//! snapshots**, not screenshots or pixel coordinates.
//!
//! Integration is one trait + one wrap call:
//!
//! ```no_run
//! use egui_agent::{AgentApp, Action, ActionResult, Snapshot, SnapshotCtx};
//!
//! #[derive(Default)]
//! struct MyApp { count: u32 }
//!
//! impl AgentApp for MyApp {
//!     fn snapshot(&self, cx: &SnapshotCtx<'_>) -> Snapshot {
//!         Snapshot::new()
//!             .node(egui_agent::Node::value("count", self.count))
//!             .nodes(cx.registry_nodes())
//!     }
//!     fn dispatch(&mut self, action: &Action, _ctx: &egui::Context) -> ActionResult {
//!         match action.name() {
//!             "increment" => { self.count += 1; ActionResult::ok() }
//!             _ => ActionResult::Unhandled,
//!         }
//!     }
//! }
//!
//! impl eframe::App for MyApp {
//!     fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
//!         if egui_agent::widgets::button(ui, "increment", "+1").clicked() {
//!             self.count += 1;
//!         }
//!     }
//! }
//!
//! fn main() -> eframe::Result {
//!     let control = egui_agent::Control::spawn(egui_agent::Config {
//!         transport: egui_agent::Transport::UnixSocket("/tmp/myapp.agent.sock".into()),
//!         auth: egui_agent::Auth::from_env("MYAPP_AGENT_TOKEN"),
//!         enabled: std::env::var("MYAPP_AGENT").is_ok(),
//!         ..Default::default()
//!     });
//!     eframe::run_native("MyApp", Default::default(), Box::new(move |cc| {
//!         control.bind_context(cc.egui_ctx.clone());
//!         Ok(Box::new(egui_agent::wrap(MyApp::default(), control)))
//!     }))
//! }
//! ```
//!
//! See `docs/egui_agent_design.md` in the repository for the full design.

mod agent;
mod app;
mod audit;
mod config;
mod control;
pub mod events;
mod hub;
mod predicate;
mod protocol;
pub mod registry;
mod snapshot;
pub mod widgets;

#[cfg(any(feature = "uds", feature = "ws", feature = "stdio", feature = "pipe"))]
mod transport;

#[cfg(feature = "uds")]
pub mod client;

#[cfg(feature = "accesskit-introspection")]
pub mod introspection;

#[cfg(feature = "screenshot")]
mod screenshot;

#[cfg(feature = "test-harness")]
mod test_harness;

pub use agent::Agent;
pub use app::{Action, ActionResult, AgentApp, SnapshotCtx};
pub use config::{Auth, Config, SettleUntil, Transport};
pub use control::{wrap, Control};
pub use hub::CommandHub;
pub use predicate::eval as eval_predicate;
pub use protocol::{
    AgentError, Command, Predicate, Response, ResponseBody, Selector, WireRequest, WireResponse,
};
pub use snapshot::{roles, Node, Snapshot, ToSnapshot};

#[cfg(feature = "test-harness")]
pub use test_harness::TestHarness;

// Re-export for macro use and version alignment.
pub use serde_json;
pub use serde_json::Value;
