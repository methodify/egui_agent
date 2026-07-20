//! Predicate evaluation for [`Command::WaitFor`](crate::Command::WaitFor).

use crate::protocol::Predicate;
use crate::snapshot::Snapshot;

/// Evaluate a predicate against a snapshot.
///
/// Selector-based predicates are lenient: "no matching node" simply means the
/// condition doesn't hold (yet) — WaitFor is about waiting for state to
/// appear, so absence is not an error.
pub fn eval(pred: &Predicate, snap: &Snapshot) -> bool {
    match pred {
        Predicate::Exists(sel) => snap.select(sel).next().is_some(),
        Predicate::NotExists(sel) => snap.select(sel).next().is_none(),
        Predicate::ValueEq { target, value } => {
            snap.select(target).any(|n| n.value.as_ref() == Some(value))
        }
        Predicate::Enabled(sel) => snap.select(sel).any(|n| n.enabled),
        Predicate::Disabled(sel) => snap.select(sel).any(|n| !n.enabled),
        Predicate::Focused(sel) => snap.select(sel).any(|n| n.focused),
        Predicate::DataEq { pointer, value } => snap
            .data
            .as_ref()
            .and_then(|d| d.pointer(pointer))
            .is_some_and(|v| v == value),
        Predicate::All(ps) => ps.iter().all(|p| eval(p, snap)),
        Predicate::Any(ps) => ps.iter().any(|p| eval(p, snap)),
        Predicate::Not(p) => !eval(p, snap),
    }
}
