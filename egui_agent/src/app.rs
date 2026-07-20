//! The per-app integration surface: [`AgentApp`].

use serde_json::Value;

use crate::protocol::Selector;
use crate::registry;
use crate::snapshot::Snapshot;

/// An actuation delivered to [`AgentApp::dispatch`].
#[derive(Debug, Clone)]
pub enum Action {
    /// `Command::Invoke` / `Command::Custom`.
    Invoke {
        target: Option<Selector>,
        name: String,
        args: Option<Value>,
    },
    /// `Command::SetValue`.
    SetValue { target: Selector, value: Value },
}

impl Action {
    /// The action verb: the invoke name, or `"set_value"`.
    pub fn name(&self) -> &str {
        match self {
            Action::Invoke { name, .. } => name.as_str(),
            Action::SetValue { .. } => "set_value",
        }
    }

    pub fn target(&self) -> Option<&Selector> {
        match self {
            Action::Invoke { target, .. } => target.as_ref(),
            Action::SetValue { target, .. } => Some(target),
        }
    }

    pub fn args(&self) -> Option<&Value> {
        match self {
            Action::Invoke { args, .. } => args.as_ref(),
            Action::SetValue { value, .. } => Some(value),
        }
    }
}

/// Outcome of [`AgentApp::dispatch`].
#[derive(Debug, Clone)]
pub enum ActionResult {
    /// The app handled the action; optional return payload.
    Handled(Option<Value>),
    /// The app doesn't recognize this action/target — the wrapper falls back to
    /// registered-widget actuation, then errors with `UnknownAction`/`TargetNotFound`.
    Unhandled,
    /// The app recognized but refused the action.
    Rejected(String),
    /// The action name matched but its arguments didn't.
    BadArgs(String),
}

impl ActionResult {
    /// Convenience: `Handled(None)`.
    pub fn ok() -> Self {
        ActionResult::Handled(None)
    }

    /// Convenience: `Handled(Some(value))` from anything serializable.
    pub fn with<T: serde::Serialize>(value: &T) -> Self {
        ActionResult::Handled(serde_json::to_value(value).ok())
    }
}

/// Context handed to [`AgentApp::snapshot`], giving access to the crate's
/// automatic introspection sources so hand-written snapshots can compose
/// with them instead of starting from scratch.
pub struct SnapshotCtx<'a> {
    pub(crate) ctx: &'a egui::Context,
}

impl<'a> SnapshotCtx<'a> {
    /// The egui context, for focus queries etc.
    pub fn egui(&self) -> &egui::Context {
        self.ctx
    }

    /// Nodes registered by this frame's registered-widget helpers
    /// ([`crate::widgets`]). Empty if the app doesn't use them.
    pub fn registry_nodes(&self) -> Vec<crate::snapshot::Node> {
        registry::snapshot_nodes(self.ctx)
    }

    /// Recent domain events emitted via [`crate::events::emit`].
    pub fn event_nodes(&self) -> Vec<crate::snapshot::Node> {
        crate::events::event_nodes(self.ctx)
    }

    /// Nodes read from the AccessKit tree egui built last frame.
    /// Empty until a frame has completed with accesskit enabled.
    #[cfg(feature = "accesskit-introspection")]
    pub fn accesskit_nodes(&self) -> Vec<crate::snapshot::Node> {
        crate::introspection::accesskit_nodes(self.ctx)
    }
}

/// The only trait an app implements to become agent-drivable.
///
/// Both methods have working defaults when the app uses the registered-widget
/// helpers in [`crate::widgets`]; hand-written impls give full control.
pub trait AgentApp {
    /// Describe current state semantically. The default composes the
    /// registered-widget registry and (when enabled) the AccessKit tree.
    fn snapshot(&self, cx: &SnapshotCtx<'_>) -> Snapshot {
        let snap = Snapshot::new().nodes(cx.registry_nodes());
        #[cfg(feature = "accesskit-introspection")]
        let snap = snap.nodes(cx.accesskit_nodes());
        snap.nodes(cx.event_nodes())
    }

    /// Route named actions to the same methods your buttons call.
    /// Return [`ActionResult::Unhandled`] (the default) to let the wrapper
    /// actuate registered widgets automatically.
    fn dispatch(&mut self, action: &Action, ctx: &egui::Context) -> ActionResult {
        let _ = (action, ctx);
        ActionResult::Unhandled
    }
}
