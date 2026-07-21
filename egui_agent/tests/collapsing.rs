//! Section-gated widgets (CollapsingHeader) via accesskit actuation.
//!
//! Investigated after a field report that CollapsingHeader "ignores" accesskit
//! clicks. On egui 0.35 the header registers an a11y node (Role::Button, via
//! `widget_info`) and egui's generic interact path honors accesskit Click
//! requests (`FAKE_PRIMARY_CLICKED`), so header clicks work; the report most
//! likely predates the 0.2.0 node-id fixes (corrupted ids silently no-oped).
//! These tests pin the working behavior down so a regression is caught loudly.

#![cfg(feature = "accesskit-introspection")]

use egui_agent::{AgentApp, Command, Predicate, ResponseBody, Selector, TestHarness};

#[derive(Default)]
struct SectionApp {
    inner_clicks: u32,
}
impl AgentApp for SectionApp {}

fn section_harness() -> TestHarness<'static, SectionApp> {
    let mut h = TestHarness::new(SectionApp::default(), |app, ui| {
        egui::CollapsingHeader::new("Advanced").show(ui, |ui| {
            if ui.button("Inner").clicked() {
                app.inner_clicks += 1;
            }
        });
    });
    h.send(Command::WaitFor {
        predicate: Predicate::Exists(Selector::Label("Advanced".into())),
        timeout_ms: 5000,
        quiet: true,
    })
    .expect("header visible in a11y tree");
    h
}

#[test]
fn collapsing_header_expands_via_label_click() {
    let mut h = section_harness();

    // Collapsed content is not in the a11y tree (it isn't drawn) — expected.
    let ResponseBody::Nodes(nodes) = h
        .send(Command::Query {
            target: Selector::Label("Inner".into()),
        })
        .unwrap()
    else {
        panic!()
    };
    assert!(nodes.is_empty(), "collapsed content should be absent");

    // The header itself is a clickable button node.
    let ResponseBody::Nodes(nodes) = h
        .send(Command::Query {
            target: Selector::Label("Advanced".into()),
        })
        .unwrap()
    else {
        panic!()
    };
    assert_eq!(nodes.len(), 1);
    assert_eq!(nodes[0].role, "button");
    assert!(nodes[0].actions.contains(&"click".to_string()));

    // Click it; the section content becomes reachable.
    h.send(Command::Invoke {
        target: Some(Selector::Label("Advanced".into())),
        action: "click".into(),
        args: None,
        quiet: true,
    })
    .expect("header click");
    h.send(Command::WaitFor {
        predicate: Predicate::Exists(Selector::Label("Inner".into())),
        timeout_ms: 2000,
        quiet: true,
    })
    .expect("section expanded");

    // And the revealed widget is actuatable like any other.
    h.send(Command::Invoke {
        target: Some(Selector::Label("Inner".into())),
        action: "click".into(),
        args: None,
        quiet: true,
    })
    .expect("inner click");
    assert_eq!(h.app().inner_clicks, 1);
}

#[test]
fn collapsing_header_toggles_via_node_id() {
    let mut h = section_harness();
    let ResponseBody::Nodes(nodes) = h
        .send(Command::Query {
            target: Selector::Label("Advanced".into()),
        })
        .unwrap()
    else {
        panic!()
    };
    let node_id = nodes[0].node_id.expect("header carries a node id");

    // Expand by node id...
    h.send(Command::Invoke {
        target: Some(Selector::Node(node_id)),
        action: "click".into(),
        args: None,
        quiet: true,
    })
    .expect("expand via node id");
    h.send(Command::WaitFor {
        predicate: Predicate::Exists(Selector::Label("Inner".into())),
        timeout_ms: 2000,
        quiet: true,
    })
    .expect("expanded");

    // ...and collapse again the same way.
    h.send(Command::Invoke {
        target: Some(Selector::Node(node_id)),
        action: "click".into(),
        args: None,
        quiet: true,
    })
    .expect("collapse via node id");
    h.send(Command::WaitFor {
        predicate: Predicate::NotExists(Selector::Label("Inner".into())),
        timeout_ms: 2000,
        quiet: true,
    })
    .expect("collapsed again");
}

#[test]
fn collapsing_header_inside_scroll_area_and_window() {
    // Same flow nested inside containers, closer to a real app layout.
    #[derive(Default)]
    struct NestedApp;
    impl AgentApp for NestedApp {}

    let mut h = TestHarness::new(NestedApp, |_, ui| {
        egui::ScrollArea::vertical().show(ui, |ui| {
            egui::CollapsingHeader::new("Settings").show(ui, |ui| {
                ui.label("nested content");
            });
        });
    });
    h.send(Command::WaitFor {
        predicate: Predicate::Exists(Selector::Label("Settings".into())),
        timeout_ms: 5000,
        quiet: true,
    })
    .expect("nested header visible");
    h.send(Command::Invoke {
        target: Some(Selector::Label("Settings".into())),
        action: "click".into(),
        args: None,
        quiet: true,
    })
    .expect("nested header click");
    h.send(Command::WaitFor {
        predicate: Predicate::Exists(Selector::Label("nested content".into())),
        timeout_ms: 2000,
        quiet: true,
    })
    .expect("nested section expanded");
}
