//! M3/M4 milestone tests: AccessKit auto-introspection and generic actuation
//! against *plain* egui widgets — no registry helpers, no dispatch code, the
//! default `AgentApp` impl. Everything flows through the a11y tree egui
//! already builds and the ActionRequest events egui already handles.

#![cfg(feature = "accesskit-introspection")]

use egui_agent::{AgentApp, Command, Predicate, ResponseBody, Selector, TestHarness};
use serde_json::json;

/// Completely vanilla egui app: the crate has no per-widget knowledge of it.
#[derive(Default)]
struct PlainApp {
    count: i32,
    dark: bool,
    text: String,
}

impl AgentApp for PlainApp {}

fn plain_harness() -> TestHarness<'static, PlainApp> {
    let mut h = TestHarness::new(PlainApp::default(), |app, ui| {
        if ui.button("Plus").clicked() {
            app.count += 1;
        }
        ui.checkbox(&mut app.dark, "Dark mode");
        ui.text_edit_singleline(&mut app.text);
        ui.label(format!("count is {}", app.count));
    });
    // Let the plugin install and a pass complete so the tree cache fills.
    h.send(Command::WaitFor {
        predicate: Predicate::Exists(Selector::Label("Plus".into())),
        timeout_ms: 5000,
        quiet: false,
    })
    .expect("a11y tree becomes visible");
    h
}

#[test]
fn accesskit_snapshot_describes_plain_widgets() {
    let mut h = plain_harness();
    let ResponseBody::Snapshot(snap) = h.send(Command::Snapshot).unwrap() else {
        panic!()
    };

    let button = snap
        .select_unique(&Selector::Label("Plus".into()))
        .expect("plain button visible via a11y tree");
    assert_eq!(button.role, "button");
    assert!(button.node_id.is_some(), "a11y nodes carry AccessKit ids");
    assert!(button.actions.contains(&"click".to_string()));

    let checkbox = snap
        .select_unique(&Selector::Label("Dark mode".into()))
        .expect("checkbox visible");
    assert_eq!(checkbox.role, "checkbox");
    assert_eq!(checkbox.value, Some(json!(false)));
}

#[test]
fn click_plain_button_via_label() {
    let mut h = plain_harness();
    h.send(Command::Invoke {
        target: Some(Selector::Label("Plus".into())),
        action: "click".into(),
        args: None,
        quiet: false,
    })
    .expect("click via injected ActionRequest");
    assert_eq!(h.app().count, 1);

    // And the post-action snapshot (captured after settle) shows the change.
    let ResponseBody::Snapshot(snap) = h.send(Command::Snapshot).unwrap() else {
        panic!()
    };
    assert!(
        snap.select(&Selector::Label("count is 1".into()))
            .next()
            .is_some(),
        "label updated in a11y snapshot"
    );
}

#[test]
fn click_plain_button_via_node_id() {
    let mut h = plain_harness();
    let ResponseBody::Nodes(nodes) = h
        .send(Command::Query {
            target: Selector::Label("Plus".into()),
        })
        .unwrap()
    else {
        panic!()
    };
    let node_id = nodes[0].node_id.expect("node id");

    h.send(Command::Invoke {
        target: Some(Selector::Node(node_id)),
        action: "click".into(),
        args: None,
        quiet: false,
    })
    .expect("click via raw node id");
    assert_eq!(h.app().count, 1);
}

#[test]
fn set_bool_value_toggles_plain_checkbox() {
    let mut h = plain_harness();
    h.send(Command::SetValue {
        target: Selector::Label("Dark mode".into()),
        value: json!(true),
        quiet: false,
    })
    .expect("toggle on");
    assert!(h.app().dark);

    // Setting to the current value is a no-op, not a toggle.
    h.send(Command::SetValue {
        target: Selector::Label("Dark mode".into()),
        value: json!(true),
        quiet: false,
    })
    .expect("idempotent set");
    assert!(h.app().dark);

    h.send(Command::SetValue {
        target: Selector::Label("Dark mode".into()),
        value: json!(false),
        quiet: false,
    })
    .expect("toggle off");
    assert!(!h.app().dark);
}

#[test]
fn set_string_value_on_plain_text_edit() {
    let mut h = plain_harness();
    let result = h.send(Command::SetValue {
        target: Selector::Role {
            role: "text_input".into(),
            name: None,
        },
        value: json!("hello from accesskit"),
        quiet: false,
    });
    // egui supports AccessKit SetValue on text edits; assert it works so a
    // regression in the pinned egui version is caught loudly.
    result.expect("SetValue on plain TextEdit via ActionRequest");
    assert_eq!(h.app().text, "hello from accesskit");
}

#[test]
fn unknown_label_is_target_not_found() {
    let mut h = plain_harness();
    let err = h
        .send(Command::Invoke {
            target: Some(Selector::Label("Ghost".into())),
            action: "click".into(),
            args: None,
            quiet: false,
        })
        .expect_err("nothing to click");
    assert!(
        matches!(err, egui_agent::AgentError::TargetNotFound(_)),
        "{err:?}"
    );
}

#[test]
fn unknown_node_id_errors_instead_of_silent_noop() {
    let mut h = plain_harness();
    // A node id that cannot exist in the tree: egui would silently drop the
    // ActionRequest, so the agent must reject it up front.
    let err = h
        .send(Command::Invoke {
            target: Some(Selector::Node(u64::MAX - 7)),
            action: "click".into(),
            args: None,
            quiet: false,
        })
        .expect_err("unknown node id");
    assert!(
        matches!(err, egui_agent::AgentError::TargetNotFound(_)),
        "{err:?}"
    );
    assert_eq!(h.app().count, 0, "nothing was clicked");
}
