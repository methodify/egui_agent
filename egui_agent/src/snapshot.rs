//! Structured snapshots: the "text instead of a screenshot" payload.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::protocol::Selector;

/// Well-known role strings. Roles are open-ended (`Node::role` is a `String`);
/// these constants cover the common widgets and match what the registry helpers emit.
pub mod roles {
    pub const BUTTON: &str = "button";
    pub const CHECKBOX: &str = "checkbox";
    pub const TEXT_INPUT: &str = "text_input";
    pub const SLIDER: &str = "slider";
    pub const LABEL: &str = "label";
    pub const VALUE: &str = "value";
    pub const WINDOW: &str = "window";
    pub const UNKNOWN: &str = "unknown";
}

/// One UI/state node in a snapshot.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Node {
    /// Developer-assigned stable semantic id, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    /// Widget/state role, e.g. `"button"`. See [`roles`].
    pub role: String,
    /// Accessible label / display name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// Current value (text content, slider position, checked state...).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<Value>,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub focused: bool,
    /// Action names invokable on this node.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub actions: Vec<String>,
    /// Screen rect `[min_x, min_y, max_x, max_y]`, when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rect: Option<[f32; 4]>,
    /// AccessKit node id, when the node came from the a11y tree.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub node_id: Option<u64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub children: Vec<Node>,
}

fn default_true() -> bool {
    true
}

impl Node {
    pub fn new(role: impl Into<String>) -> Self {
        Self {
            id: None,
            role: role.into(),
            label: None,
            value: None,
            enabled: true,
            focused: false,
            actions: Vec::new(),
            rect: None,
            node_id: None,
            children: Vec::new(),
        }
    }

    pub fn button(id: impl Into<String>, label: impl Into<String>) -> Self {
        Self::new(roles::BUTTON)
            .with_id(id)
            .with_label(label)
            .with_actions(["click"])
    }

    pub fn checkbox(id: impl Into<String>, label: impl Into<String>, checked: bool) -> Self {
        Self::new(roles::CHECKBOX)
            .with_id(id)
            .with_label(label)
            .with_value(checked)
            .with_actions(["toggle", "set_value"])
    }

    pub fn text_input(id: impl Into<String>, text: impl Into<String>) -> Self {
        Self::new(roles::TEXT_INPUT)
            .with_id(id)
            .with_value(text.into())
            .with_actions(["set_value"])
    }

    pub fn slider(id: impl Into<String>, value: f64) -> Self {
        Self::new(roles::SLIDER)
            .with_id(id)
            .with_value(value)
            .with_actions(["set_value"])
    }

    pub fn label(text: impl Into<String>) -> Self {
        Self::new(roles::LABEL).with_label(text)
    }

    /// A named piece of app state that isn't a widget.
    pub fn value(id: impl Into<String>, value: impl Into<Value>) -> Self {
        Self::new(roles::VALUE).with_id(id).with_value(value)
    }

    pub fn with_id(mut self, id: impl Into<String>) -> Self {
        self.id = Some(id.into());
        self
    }

    pub fn with_label(mut self, label: impl Into<String>) -> Self {
        self.label = Some(label.into());
        self
    }

    pub fn with_value(mut self, value: impl Into<Value>) -> Self {
        self.value = Some(value.into());
        self
    }

    pub fn with_actions<S: Into<String>>(mut self, actions: impl IntoIterator<Item = S>) -> Self {
        self.actions = actions.into_iter().map(Into::into).collect();
        self
    }

    pub fn with_enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    pub fn with_focused(mut self, focused: bool) -> Self {
        self.focused = focused;
        self
    }

    pub fn with_rect(mut self, rect: [f32; 4]) -> Self {
        self.rect = Some(rect);
        self
    }

    pub fn with_children(mut self, children: Vec<Node>) -> Self {
        self.children = children;
        self
    }

    pub fn matches(&self, sel: &Selector) -> bool {
        match sel {
            Selector::Id(id) => self.id.as_deref() == Some(id.as_str()),
            Selector::Label(l) => self.label.as_deref() == Some(l.as_str()),
            Selector::Role { role, name } => {
                self.role == *role
                    && name.as_ref().is_none_or(|n| {
                        self.label.as_deref() == Some(n.as_str())
                            || self.id.as_deref() == Some(n.as_str())
                    })
            }
            Selector::Node(n) => self.node_id == Some(*n),
        }
    }
}

/// A structured view of the current UI/app state.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Snapshot {
    /// UI/state nodes (a tree; flat lists are trees of depth 1).
    #[serde(default)]
    pub nodes: Vec<Node>,
    /// Free-form app-specific state the agent can also reason over.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

impl Snapshot {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn node(mut self, node: Node) -> Self {
        self.nodes.push(node);
        self
    }

    pub fn nodes<I: IntoIterator<Item = Node>>(mut self, nodes: I) -> Self {
        self.nodes.extend(nodes);
        self
    }

    /// Attach serializable app state as the free-form `data` payload.
    pub fn with_data<T: Serialize>(mut self, data: &T) -> Self {
        self.data = serde_json::to_value(data).ok();
        self
    }

    /// Depth-first iteration over all nodes.
    pub fn iter(&self) -> impl Iterator<Item = &Node> {
        let mut stack: Vec<&Node> = self.nodes.iter().rev().collect();
        std::iter::from_fn(move || {
            let n = stack.pop()?;
            stack.extend(n.children.iter().rev());
            Some(n)
        })
    }

    /// All nodes matching a selector, depth-first.
    pub fn select<'a>(&'a self, sel: &'a Selector) -> impl Iterator<Item = &'a Node> + 'a {
        self.iter().filter(move |n| n.matches(sel))
    }

    /// The unique node matching `sel`, or a structured error.
    pub fn select_unique(&self, sel: &Selector) -> Result<&Node, crate::AgentError> {
        let mut found: Option<&Node> = None;
        for n in self.iter() {
            if n.matches(sel) {
                if found.is_some() {
                    return Err(crate::AgentError::AmbiguousSelector(sel.to_string()));
                }
                found = Some(n);
            }
        }
        found.ok_or_else(|| crate::AgentError::TargetNotFound(sel.to_string()))
    }
}

/// Types that can describe themselves as snapshot nodes.
///
/// Implemented by hand or via `#[derive(Snapshot)]` from `egui_agent_macros`.
pub trait ToSnapshot {
    fn to_snapshot_nodes(&self) -> Vec<Node>;

    fn to_snapshot(&self) -> Snapshot {
        Snapshot {
            nodes: self.to_snapshot_nodes(),
            data: None,
        }
    }
}
