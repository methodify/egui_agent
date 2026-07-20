//! AccessKit auto-introspection (feature `accesskit-introspection`).
//!
//! egui builds an AccessKit tree of every labeled widget each frame. This
//! module captures that tree and converts it into snapshot [`Node`]s, so
//! `snapshot()` can be a one-liner with zero per-widget code.
//!
//! Implementation pending verification of the tree read path against the
//! pinned egui version (design doc §10 research task).

use crate::snapshot::Node;

/// Nodes read from the AccessKit tree egui built last frame.
pub fn accesskit_nodes(ctx: &egui::Context) -> Vec<Node> {
    let _ = ctx;
    Vec::new()
}
