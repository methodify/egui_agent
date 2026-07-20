//! The transport-agnostic command/response protocol.
//!
//! Everything here is plain serde data: transports and the MCP adapter
//! serialize these types; the core and `Agent<A>` interpret them.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::snapshot::{Node, Snapshot};

/// A request from the agent to the app.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Command {
    /// Full structured view of the current UI/app state.
    Snapshot,
    /// Query a subset by selector (cheaper than a full snapshot).
    Query { target: Selector },
    /// Invoke a named action, optionally on a target widget.
    Invoke {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        target: Option<Selector>,
        action: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        args: Option<Value>,
        /// Suppress the post-action snapshot in the response (lighter for
        /// large UIs; overrides `Config::post_action_snapshot` per call).
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        quiet: bool,
    },
    /// Set a named/targeted value (text field, slider, toggle...).
    SetValue {
        target: Selector,
        value: Value,
        /// Suppress the post-action snapshot in the response.
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        quiet: bool,
    },
    /// Block until a predicate over the snapshot holds, or timeout.
    WaitFor {
        predicate: Predicate,
        timeout_ms: u64,
        /// Suppress the snapshot in the response.
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        quiet: bool,
    },
    /// Optional fallback: PNG of the current frame (requires the `screenshot` feature
    /// and a backend that honors `ViewportCommand::Screenshot`).
    Screenshot,
    /// Escape hatch for app-specific verbs.
    Custom { name: String, args: Value },
}

/// How a command names its target widget/state.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Selector {
    /// Developer-assigned stable semantic id (preferred).
    Id(String),
    /// Match by accessible label / name.
    Label(String),
    /// Match by role, optionally narrowed by name.
    Role {
        role: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        name: Option<String>,
    },
    /// AccessKit node id. Ephemeral unless the app guarantees stable id sources.
    ///
    /// On the wire this is a **string** (`{"node": "12345"}`): node ids use
    /// the full u64 range, which corrupts through JSON parsers that read
    /// numbers as f64 (anything above 2^53). Plain JSON numbers are still
    /// accepted on input when they are exact integers.
    Node(#[serde(with = "u64_as_string")] u64),
}

/// Serialize u64 as a JSON string; accept strings or exact JSON integers.
/// Node ids cover the full u64 range, and JavaScript-based MCP hosts corrupt
/// integers above 2^53 through f64 — strings survive every JSON stack.
pub(crate) mod u64_as_string {
    use serde::de::Visitor;

    pub fn serialize<S: serde::Serializer>(v: &u64, s: S) -> Result<S::Ok, S::Error> {
        s.collect_str(v)
    }

    pub fn deserialize<'de, D: serde::Deserializer<'de>>(d: D) -> Result<u64, D::Error> {
        struct V;
        impl Visitor<'_> for V {
            type Value = u64;
            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("a node id as a string (preferred) or exact integer")
            }
            fn visit_u64<E: serde::de::Error>(self, v: u64) -> Result<u64, E> {
                Ok(v)
            }
            fn visit_i64<E: serde::de::Error>(self, v: i64) -> Result<u64, E> {
                u64::try_from(v).map_err(|_| E::custom("node id cannot be negative"))
            }
            fn visit_f64<E: serde::de::Error>(self, v: f64) -> Result<u64, E> {
                // Reject anything that lost precision on the way in.
                if v.fract() == 0.0 && v >= 0.0 && v <= (1u64 << 53) as f64 {
                    Ok(v as u64)
                } else {
                    Err(E::custom(
                        "node id lost precision as a JSON number; send it as a string",
                    ))
                }
            }
            fn visit_str<E: serde::de::Error>(self, v: &str) -> Result<u64, E> {
                v.parse()
                    .map_err(|_| E::custom(format!("invalid node id {v:?}")))
            }
        }
        d.deserialize_any(V)
    }
}

/// `Option<u64>` variant of [`u64_as_string`].
pub(crate) mod opt_u64_as_string {
    use serde::de::Visitor;

    pub fn serialize<S: serde::Serializer>(v: &Option<u64>, s: S) -> Result<S::Ok, S::Error> {
        match v {
            Some(x) => s.collect_str(x),
            None => s.serialize_none(),
        }
    }

    pub fn deserialize<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<u64>, D::Error> {
        struct OptV;
        impl<'de> Visitor<'de> for OptV {
            type Value = Option<u64>;
            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("an optional node id as a string or exact integer")
            }
            fn visit_none<E>(self) -> Result<Self::Value, E> {
                Ok(None)
            }
            fn visit_unit<E>(self) -> Result<Self::Value, E> {
                Ok(None)
            }
            fn visit_some<D2: serde::Deserializer<'de>>(
                self,
                d: D2,
            ) -> Result<Self::Value, D2::Error> {
                super::u64_as_string::deserialize(d).map(Some)
            }
        }
        d.deserialize_option(OptV)
    }
}

impl std::fmt::Display for Selector {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Selector::Id(id) => write!(f, "id={id}"),
            Selector::Label(l) => write!(f, "label={l}"),
            Selector::Role {
                role,
                name: Some(n),
            } => write!(f, "role={role} name={n}"),
            Selector::Role { role, name: None } => write!(f, "role={role}"),
            Selector::Node(n) => write!(f, "node={n}"),
        }
    }
}

/// A condition over the current snapshot, used by [`Command::WaitFor`].
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Predicate {
    /// A node matching the selector exists.
    Exists(Selector),
    /// No node matches the selector.
    NotExists(Selector),
    /// The (unique) matching node's value equals `value`.
    ValueEq { target: Selector, value: Value },
    /// The (unique) matching node is enabled.
    Enabled(Selector),
    /// The (unique) matching node is disabled.
    Disabled(Selector),
    /// The (unique) matching node has focus.
    Focused(Selector),
    /// The app-provided `data` payload at a JSON pointer equals `value`.
    DataEq { pointer: String, value: Value },
    /// All sub-predicates hold.
    All(Vec<Predicate>),
    /// At least one sub-predicate holds.
    Any(Vec<Predicate>),
    /// The sub-predicate does not hold.
    Not(Box<Predicate>),
}

/// Successful response payloads, one per command shape.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResponseBody {
    /// Reply to [`Command::Snapshot`].
    Snapshot(Snapshot),
    /// Reply to [`Command::Query`]: all matching nodes.
    Nodes(Vec<Node>),
    /// Reply to [`Command::Invoke`] / [`Command::Custom`].
    Invoked {
        /// Value returned by the app's dispatch, if any.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        result: Option<Value>,
        /// Post-settle snapshot (present unless disabled in [`crate::Config`]).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        snapshot: Option<Snapshot>,
    },
    /// Reply to [`Command::SetValue`].
    ValueSet {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        snapshot: Option<Snapshot>,
    },
    /// Reply to [`Command::WaitFor`] once the predicate holds.
    Satisfied {
        waited_ms: u64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        snapshot: Option<Snapshot>,
    },
    /// Reply to [`Command::Screenshot`].
    Image {
        format: String,
        width: u32,
        height: u32,
        /// Base64-encoded image bytes.
        data: String,
    },
}

/// Structured errors the agent can reason about.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, thiserror::Error)]
#[serde(tag = "kind", content = "detail", rename_all = "snake_case")]
pub enum AgentError {
    #[error("no node matches selector: {0}")]
    TargetNotFound(String),
    #[error("selector matches more than one node: {0}")]
    AmbiguousSelector(String),
    #[error("action rejected: {0}")]
    ActionRejected(String),
    #[error("unknown action: {0}")]
    UnknownAction(String),
    #[error("bad arguments: {0}")]
    BadArgs(String),
    #[error("timed out: {0}")]
    Timeout(String),
    #[error("unauthorized")]
    Unauthorized,
    #[error("control channel is in read-only mode")]
    ReadOnly,
    #[error("not supported: {0}")]
    NotSupported(String),
    #[error("bad request: {0}")]
    BadRequest(String),
    #[error("internal error: {0}")]
    Internal(String),
}

/// The reply to a [`Command`].
pub type Response = Result<ResponseBody, AgentError>;

/// One request on the wire (newline-delimited JSON transports and WS text frames).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WireRequest {
    pub id: u64,
    pub command: Command,
}

/// One response on the wire. Exactly one of `ok`/`err` is set.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WireResponse {
    pub id: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ok: Option<ResponseBody>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub err: Option<AgentError>,
}

impl WireResponse {
    pub fn from_response(id: u64, resp: Response) -> Self {
        match resp {
            Ok(body) => Self {
                id,
                ok: Some(body),
                err: None,
            },
            Err(e) => Self {
                id,
                ok: None,
                err: Some(e),
            },
        }
    }

    pub fn into_response(self) -> Response {
        match (self.ok, self.err) {
            (Some(body), _) => Ok(body),
            (None, Some(e)) => Err(e),
            (None, None) => Err(AgentError::Internal("empty wire response".into())),
        }
    }
}

/// First line a client must send on socket transports when an auth token is configured.
/// (Only referenced by transport features, hence the allow.)
#[allow(dead_code)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthRequest {
    pub auth: String,
}
