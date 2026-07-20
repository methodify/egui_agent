//! AccessKit auto-introspection and generic actuation
//! (feature `accesskit-introspection`).
//!
//! egui builds an AccessKit tree of every labeled widget each frame and hands
//! it to the backend in `FullOutput::platform_output::accesskit_update`. An
//! [`egui::Plugin`] gives us both directions with no backend cooperation:
//!
//! * `output_hook` — clone each `TreeUpdate` into a cache, giving
//!   [`Snapshot::from_accesskit`] and the default `AgentApp::snapshot`
//!   generic coverage of every labeled widget with zero per-widget code.
//! * `input_hook` — inject queued [`accesskit::ActionRequest`]s as
//!   `egui::Event::AccessKitActionRequest` raw-input events: the exact
//!   mechanism a screen reader (or `egui_kittest`) uses to click/actuate
//!   widgets, so plain `ui.button(..)` widgets respond with no app code.
//!
//! Verified against egui 0.35 / accesskit 0.24 (design doc §10/§11 research
//! tasks): `Context::add_plugin`, `Plugin::{input_hook,output_hook}`,
//! `PlatformOutput::accesskit_update`, `Event::AccessKitActionRequest`.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use accesskit::{NodeId as AkId, TreeUpdate};

use crate::snapshot::Node;

#[derive(Default)]
struct AkState {
    /// Last seen node set, keyed by AccessKit id (egui sends full trees).
    nodes: HashMap<AkId, accesskit::Node>,
    root: Option<AkId>,
    focus: Option<AkId>,
    /// Event batches waiting for injection; one batch is drained per pass so
    /// multi-step actuations (focus, then type) land in the right order.
    queue: Vec<Vec<egui::Event>>,
}

type Shared = Arc<Mutex<AkState>>;

/// The egui plugin doing capture + injection. Registered once per context.
struct AgentAccessKitPlugin {
    shared: Shared,
}

impl egui::Plugin for AgentAccessKitPlugin {
    fn debug_name(&self) -> &'static str {
        "egui_agent::accesskit"
    }

    fn input_hook(&mut self, ctx: &egui::Context, input: &mut egui::RawInput) {
        let mut st = self.shared.lock().unwrap();
        if !st.queue.is_empty() {
            input.events.extend(st.queue.remove(0));
            if !st.queue.is_empty() {
                ctx.request_repaint(); // later batches need further passes
            }
        }
    }

    fn output_hook(&mut self, _ctx: &egui::Context, output: &mut egui::FullOutput) {
        if let Some(update) = &output.platform_output.accesskit_update {
            self.shared.lock().unwrap().apply(update);
        }
    }
}

impl AkState {
    fn apply(&mut self, update: &TreeUpdate) {
        // egui rebuilds the full tree every pass, so replace wholesale.
        self.nodes = update.nodes.iter().cloned().collect();
        if let Some(tree) = &update.tree {
            self.root = Some(tree.root);
        }
        self.focus = Some(update.focus);
    }
}

fn shared(ctx: &egui::Context) -> Shared {
    let key = egui::Id::new("egui_agent::accesskit");
    ctx.data_mut(|d| d.get_temp_mut_or_default::<Shared>(key).clone())
}

/// Enable accesskit and register the capture/injection plugin (idempotent).
/// Called by the `Agent` wrapper every frame; cheap after the first call.
pub(crate) fn ensure_installed(ctx: &egui::Context) {
    let key = egui::Id::new("egui_agent::accesskit_installed");
    let installed = ctx.data(|d| d.get_temp::<bool>(key).unwrap_or(false));
    if installed {
        return;
    }
    ctx.enable_accesskit();
    ctx.add_plugin(AgentAccessKitPlugin {
        shared: shared(ctx),
    });
    ctx.data_mut(|d| d.insert_temp(key, true));
}

/// Queue an action request for injection into the next pass's raw input.
pub(crate) fn queue_action_request(ctx: &egui::Context, req: accesskit::ActionRequest) {
    queue_event_batches(ctx, vec![vec![egui::Event::AccessKitActionRequest(req)]]);
}

/// Queue raw event batches; each batch is injected into a successive pass.
pub(crate) fn queue_event_batches(ctx: &egui::Context, batches: Vec<Vec<egui::Event>>) {
    shared(ctx).lock().unwrap().queue.extend(batches);
    ctx.request_repaint();
}

/// Find the unique a11y node matching a selector; returns its AccessKit id.
pub(crate) fn resolve(
    ctx: &egui::Context,
    sel: &crate::Selector,
) -> Result<AkId, crate::AgentError> {
    let nodes = accesskit_nodes(ctx);
    fn walk<'a>(nodes: &'a [Node], sel: &crate::Selector, hits: &mut Vec<&'a Node>) {
        for n in nodes {
            if n.matches(sel) {
                hits.push(n);
            }
            walk(&n.children, sel, hits);
        }
    }
    let mut hits = Vec::new();
    walk(&nodes, sel, &mut hits);
    match hits.as_slice() {
        [] => Err(crate::AgentError::TargetNotFound(sel.to_string())),
        [one] => one
            .node_id
            .map(AkId)
            .ok_or_else(|| crate::AgentError::TargetNotFound(sel.to_string())),
        _ => Err(crate::AgentError::AmbiguousSelector(sel.to_string())),
    }
}

/// Current toggle state of a cached a11y node, if it is toggleable.
pub(crate) fn node_toggled(ctx: &egui::Context, id: AkId) -> Option<bool> {
    let shared = shared(ctx);
    let st = shared.lock().unwrap();
    st.nodes
        .get(&id)?
        .toggled()
        .map(|t| matches!(t, accesskit::Toggled::True))
}

/// Snapshot nodes from the AccessKit tree egui built last pass.
///
/// Empty until a pass has completed after [`ensure_installed`]. Rects are in
/// physical pixels (the tree's root carries the pixels-per-point transform).
pub fn accesskit_nodes(ctx: &egui::Context) -> Vec<Node> {
    let shared = shared(ctx);
    let st = shared.lock().unwrap();
    let Some(root) = st.root else {
        return Vec::new();
    };
    // Skip the synthetic root window node; expose its children as top level.
    match st.nodes.get(&root) {
        Some(root_node) => root_node
            .children()
            .iter()
            .filter_map(|id| convert(&st, *id))
            .collect(),
        None => Vec::new(),
    }
}

fn convert(st: &AkState, id: AkId) -> Option<Node> {
    let ak = st.nodes.get(&id)?;
    let mut node = Node::new(role_name(ak.role()));
    node.node_id = Some(id.0);
    node.label = ak.label().map(str::to_owned);
    // AccessKit convention: a Label's text lives in `value`, not `label`.
    // Mirror it into `label` so `Selector::Label("...")` matches labels too.
    if node.label.is_none() && ak.role() == accesskit::Role::Label {
        node.label = ak.value().map(str::to_owned);
    }
    node.value = ak
        .value()
        .map(|s| serde_json::Value::from(s.to_owned()))
        .or_else(|| ak.numeric_value().map(serde_json::Value::from))
        .or_else(|| {
            ak.toggled()
                .map(|t| serde_json::Value::from(matches!(t, accesskit::Toggled::True)))
        });
    node.enabled = !ak.is_disabled();
    node.focused = st.focus == Some(id);
    for (action, name) in [
        (accesskit::Action::Click, "click"),
        (accesskit::Action::Focus, "focus"),
        (accesskit::Action::SetValue, "set_value"),
        (accesskit::Action::Increment, "increment"),
        (accesskit::Action::Decrement, "decrement"),
    ] {
        if ak.supports_action(action) {
            node.actions.push(name.to_owned());
        }
    }
    if let Some(b) = ak.bounds() {
        node.rect = Some([b.x0 as f32, b.y0 as f32, b.x1 as f32, b.y1 as f32]);
    }
    node.children = ak
        .children()
        .iter()
        .filter_map(|child| convert(st, *child))
        .collect();
    Some(node)
}

fn role_name(role: accesskit::Role) -> String {
    // Reuse the registry role names for the common widgets so selectors work
    // uniformly across both introspection sources.
    match role {
        accesskit::Role::Button => crate::roles::BUTTON.to_owned(),
        accesskit::Role::CheckBox => crate::roles::CHECKBOX.to_owned(),
        accesskit::Role::TextInput | accesskit::Role::MultilineTextInput => {
            crate::roles::TEXT_INPUT.to_owned()
        }
        accesskit::Role::Slider => crate::roles::SLIDER.to_owned(),
        accesskit::Role::Label => crate::roles::LABEL.to_owned(),
        accesskit::Role::Window => crate::roles::WINDOW.to_owned(),
        other => format!("{other:?}").to_lowercase(),
    }
}

impl crate::Snapshot {
    /// Build a snapshot straight from the AccessKit tree (design doc §10 v2):
    /// makes a hand-written `snapshot()` a one-liner, composable with
    /// app-specific nodes via [`crate::Snapshot::nodes`].
    pub fn from_accesskit(cx: &crate::SnapshotCtx<'_>) -> Self {
        Self {
            nodes: accesskit_nodes(cx.egui()),
            data: None,
        }
    }
}
