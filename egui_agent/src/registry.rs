//! Per-frame registry of semantic widgets.
//!
//! The registered-widget helpers in [`crate::widgets`] draw a normal egui
//! widget *and* record a [`Node`] here each frame, so the default
//! [`AgentApp::snapshot`](crate::AgentApp::snapshot) can describe them and the
//! wrapper can actuate them (`Invoke`/`SetValue`) with no per-app dispatch code.
//!
//! Because egui is immediate mode, actuation is a hand-off across one frame:
//! the wrapper queues a [`PendingActuation`] before running the app's UI; the
//! helper for the targeted widget consumes it while drawing and reports the
//! interaction (click/changed) to the app as if the user had done it.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use serde_json::Value;

use crate::snapshot::Node;

/// An actuation queued for a registered widget to consume when it draws.
#[derive(Debug, Clone)]
pub enum PendingActuation {
    Click,
    SetValue(Value),
}

#[derive(Default)]
struct RegistryState {
    /// Nodes registered during the frame currently being drawn.
    current: Vec<Node>,
    /// Nodes from the last completed frame (what snapshots report).
    completed: Vec<Node>,
    /// Queued actuations, keyed by semantic id.
    pending: HashMap<String, PendingActuation>,
}

type Shared = Arc<Mutex<RegistryState>>;

fn shared(ctx: &egui::Context) -> Shared {
    let key = egui::Id::new("egui_agent::registry");
    ctx.data_mut(|d| d.get_temp_mut_or_default::<Shared>(key).clone())
}

/// Record a node for the frame being drawn. Called by the widget helpers;
/// public so custom widgets can participate in auto-snapshots too.
pub fn register_node(ctx: &egui::Context, node: Node) {
    shared(ctx).lock().unwrap().current.push(node);
}

/// Consume a queued actuation for `id`, if any. Called by widget helpers.
pub fn take_pending(ctx: &egui::Context, id: &str) -> Option<PendingActuation> {
    shared(ctx).lock().unwrap().pending.remove(id)
}

/// Queue an actuation for a registered widget (wrapper-internal).
pub(crate) fn queue_actuation(ctx: &egui::Context, id: String, act: PendingActuation) {
    shared(ctx).lock().unwrap().pending.insert(id, act);
}

/// Ids of actuations no widget has consumed yet (wrapper-internal).
pub(crate) fn unconsumed(ctx: &egui::Context) -> Vec<String> {
    shared(ctx)
        .lock()
        .unwrap()
        .pending
        .keys()
        .cloned()
        .collect()
}

/// Drop a queued actuation (wrapper-internal; used when it expires).
pub(crate) fn cancel_actuation(ctx: &egui::Context, id: &str) {
    shared(ctx).lock().unwrap().pending.remove(id);
}

/// Nodes from the last completed frame.
pub(crate) fn snapshot_nodes(ctx: &egui::Context) -> Vec<Node> {
    shared(ctx).lock().unwrap().completed.clone()
}

/// End-of-frame bookkeeping: promote this frame's nodes to `completed`.
/// Called by the wrapper after the inner app has drawn.
pub(crate) fn end_frame(ctx: &egui::Context) {
    let shared = shared(ctx);
    let mut st = shared.lock().unwrap();
    let current = std::mem::take(&mut st.current);
    st.completed = current;
}
