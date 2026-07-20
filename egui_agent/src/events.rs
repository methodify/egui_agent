//! Optional domain-event stream (design doc §17 M5): lets the app surface
//! moments the agent should observe — toasts, navigation, completed jobs —
//! without polling app state.
//!
//! Events land in a bounded ring buffer and appear in the default snapshot as
//! `role: "event"` nodes (newest last), each carrying `{seq, name, data}`.
//! Agents observe them with `Query { role: "event" }` or
//! `WaitFor { predicate: { exists: { label: "<name>" } } }`, and can use
//! `seq` to ignore events they've already seen.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use serde_json::Value;

use crate::snapshot::{roles, Node};

const CAPACITY: usize = 256;

#[derive(Default)]
struct EventBuffer {
    next_seq: u64,
    events: VecDeque<(u64, String, Value)>,
}

type Shared = Arc<Mutex<EventBuffer>>;

fn shared(ctx: &egui::Context) -> Shared {
    let key = egui::Id::new("egui_agent::events");
    ctx.data_mut(|d| d.get_temp_mut_or_default::<Shared>(key).clone())
}

/// Emit a domain event the agent can observe. Callable from anywhere with a
/// context (UI code, `dispatch`, background tasks holding a cloned context —
/// it requests a repaint so observers wake).
pub fn emit(ctx: &egui::Context, name: impl Into<String>, data: impl Into<Value>) {
    let shared = shared(ctx);
    let mut buf = shared.lock().unwrap();
    let seq = buf.next_seq;
    buf.next_seq += 1;
    buf.events.push_back((seq, name.into(), data.into()));
    while buf.events.len() > CAPACITY {
        buf.events.pop_front();
    }
    drop(buf);
    ctx.request_repaint();
}

/// Recent events as snapshot nodes (used by the default `AgentApp::snapshot`).
pub fn event_nodes(ctx: &egui::Context) -> Vec<Node> {
    let shared = shared(ctx);
    let buf = shared.lock().unwrap();
    buf.events
        .iter()
        .map(|(seq, name, data)| {
            Node::new(roles::EVENT)
                .with_id(format!("event:{seq}"))
                .with_label(name.clone())
                .with_value(serde_json::json!({ "seq": seq, "name": name, "data": data }))
        })
        .collect()
}
