//! Registered-widget helpers: draw a normal egui widget *and* make it
//! agent-visible (auto-snapshot) and agent-drivable (auto-actuation) under a
//! stable semantic id — zero `snapshot`/`dispatch` code per widget.
//!
//! Each helper returns a [`WidgetResult`] whose `clicked()`/`changed()` also
//! report agent actuations, so app logic like
//! `if egui_agent::widgets::button(ui, "submit", "Submit").clicked() { ... }`
//! behaves identically for human clicks and agent commands.

use std::ops::RangeInclusive;

use egui::emath::Numeric;

use crate::registry::{self, PendingActuation};
use crate::snapshot::Node;

/// An egui `Response` augmented with whether the agent actuated the widget
/// this frame.
pub struct WidgetResult {
    pub response: egui::Response,
    /// True when the agent clicked/changed this widget this frame.
    pub triggered: bool,
}

impl WidgetResult {
    /// Human click or agent `Invoke`.
    pub fn clicked(&self) -> bool {
        self.triggered || self.response.clicked()
    }

    /// Human edit or agent `SetValue`.
    pub fn changed(&self) -> bool {
        self.triggered || self.response.changed()
    }
}

impl std::ops::Deref for WidgetResult {
    type Target = egui::Response;
    fn deref(&self) -> &egui::Response {
        &self.response
    }
}

fn base_node(role: &str, id: &str, response: &egui::Response, enabled: bool) -> Node {
    let r = response.rect;
    Node::new(role)
        .with_id(id)
        .with_enabled(enabled)
        .with_focused(response.has_focus())
        .with_rect([r.min.x, r.min.y, r.max.x, r.max.y])
}

/// A button with semantic id `id`. Agent-invokable via `Invoke { target: Id(id), action: "click" }`.
pub fn button(ui: &mut egui::Ui, id: &str, text: impl Into<egui::WidgetText>) -> WidgetResult {
    let text: egui::WidgetText = text.into();
    let label = text.text().to_owned();
    let response = ui.button(text);
    let triggered = ui.is_enabled()
        && matches!(
            registry::take_pending(ui.ctx(), id),
            Some(PendingActuation::Click)
        );
    registry::register_node(
        ui.ctx(),
        base_node(
            crate::snapshot::roles::BUTTON,
            id,
            &response,
            ui.is_enabled(),
        )
        .with_label(label)
        .with_actions(["click"]),
    );
    WidgetResult {
        response,
        triggered,
    }
}

/// A checkbox bound to `checked`. Agent can `Invoke` (`click` toggles) or
/// `SetValue` with a bool.
pub fn checkbox(
    ui: &mut egui::Ui,
    id: &str,
    text: impl Into<egui::WidgetText>,
    checked: &mut bool,
) -> WidgetResult {
    let mut triggered = false;
    if ui.is_enabled() {
        match registry::take_pending(ui.ctx(), id) {
            Some(PendingActuation::Click) => {
                *checked = !*checked;
                triggered = true;
            }
            Some(PendingActuation::SetValue(v)) => {
                if let Some(b) = v.as_bool() {
                    *checked = b;
                    triggered = true;
                }
            }
            None => {}
        }
    }
    let text: egui::WidgetText = text.into();
    let label = text.text().to_owned();
    let response = ui.checkbox(checked, text);
    registry::register_node(
        ui.ctx(),
        base_node(
            crate::snapshot::roles::CHECKBOX,
            id,
            &response,
            ui.is_enabled(),
        )
        .with_label(label)
        .with_value(*checked)
        .with_actions(["click", "set_value"]),
    );
    WidgetResult {
        response,
        triggered,
    }
}

/// A single-line text edit bound to `text`. Agent-settable via `SetValue` with a string.
pub fn text_edit_singleline(ui: &mut egui::Ui, id: &str, text: &mut String) -> WidgetResult {
    let triggered = apply_text_actuation(ui, id, text);
    let response = ui.text_edit_singleline(text);
    register_text_node(ui, id, text, &response);
    WidgetResult {
        response,
        triggered,
    }
}

/// A multi-line text edit bound to `text`. Agent-settable via `SetValue` with a string.
pub fn text_edit_multiline(ui: &mut egui::Ui, id: &str, text: &mut String) -> WidgetResult {
    let triggered = apply_text_actuation(ui, id, text);
    let response = ui.text_edit_multiline(text);
    register_text_node(ui, id, text, &response);
    WidgetResult {
        response,
        triggered,
    }
}

fn apply_text_actuation(ui: &egui::Ui, id: &str, text: &mut String) -> bool {
    if !ui.is_enabled() {
        return false;
    }
    match registry::take_pending(ui.ctx(), id) {
        Some(PendingActuation::SetValue(v)) => {
            if let Some(s) = v.as_str() {
                *text = s.to_owned();
                true
            } else {
                *text = v.to_string();
                true
            }
        }
        _ => false,
    }
}

fn register_text_node(ui: &egui::Ui, id: &str, text: &str, response: &egui::Response) {
    registry::register_node(
        ui.ctx(),
        base_node(
            crate::snapshot::roles::TEXT_INPUT,
            id,
            response,
            ui.is_enabled(),
        )
        .with_value(text.to_owned())
        .with_actions(["set_value"]),
    );
}

/// A slider bound to `value`. Agent-settable via `SetValue` with a number
/// (clamped to `range`).
pub fn slider<N: Numeric>(
    ui: &mut egui::Ui,
    id: &str,
    value: &mut N,
    range: RangeInclusive<N>,
) -> WidgetResult {
    let mut triggered = false;
    if ui.is_enabled() {
        if let Some(PendingActuation::SetValue(v)) = registry::take_pending(ui.ctx(), id) {
            if let Some(x) = v.as_f64() {
                let clamped = x.max(range.start().to_f64()).min(range.end().to_f64());
                *value = N::from_f64(clamped);
                triggered = true;
            }
        }
    }
    let response = ui.add(egui::Slider::new(value, range.clone()));
    registry::register_node(
        ui.ctx(),
        base_node(
            crate::snapshot::roles::SLIDER,
            id,
            &response,
            ui.is_enabled(),
        )
        .with_value(value.to_f64())
        .with_actions(["set_value"]),
    );
    WidgetResult {
        response,
        triggered,
    }
}

/// Expose a read-only piece of app state as a snapshot node without drawing
/// anything. Useful for status the agent should observe.
pub fn expose(ctx: &egui::Context, id: &str, value: impl Into<serde_json::Value>) {
    registry::register_node(
        ctx,
        Node::new(crate::snapshot::roles::VALUE)
            .with_id(id)
            .with_value(value),
    );
}

/// A label the agent can read (registered under `id`).
pub fn label(ui: &mut egui::Ui, id: &str, text: impl Into<String>) -> egui::Response {
    let text = text.into();
    let response = ui.label(text.clone());
    registry::register_node(
        ui.ctx(),
        base_node(
            crate::snapshot::roles::LABEL,
            id,
            &response,
            ui.is_enabled(),
        )
        .with_label(text),
    );
    response
}
