//! Registered-widget helper tests: auto-snapshot + auto-actuation with zero
//! per-app `snapshot`/`dispatch` code (the `AgentApp` defaults).

use egui_agent::{AgentApp, AgentError, Command, ResponseBody, Selector, TestHarness};
use serde_json::json;

/// Uses only the trait defaults: the registry is the entire integration.
#[derive(Default)]
struct FormApp {
    name: String,
    subscribed: bool,
    volume: f32,
    submitted: Option<String>,
}

impl AgentApp for FormApp {}

fn form_harness() -> TestHarness<'static, FormApp> {
    TestHarness::new(FormApp::default(), |app, ui| {
        egui_agent::widgets::text_edit_singleline(ui, "name", &mut app.name);
        egui_agent::widgets::checkbox(ui, "subscribed", "Subscribe", &mut app.subscribed);
        egui_agent::widgets::slider(ui, "volume", &mut app.volume, 0.0..=100.0);
        if egui_agent::widgets::button(ui, "submit", "Submit").clicked() {
            app.submitted = Some(app.name.clone());
        }
        egui_agent::widgets::expose(ui.ctx(), "submitted", json!(app.submitted));
    })
}

#[test]
fn registry_auto_snapshot_describes_widgets() {
    let mut h = form_harness();
    let ResponseBody::Snapshot(snap) = h.send(Command::Snapshot).unwrap() else {
        panic!()
    };

    let name = snap.select_unique(&Selector::Id("name".into())).unwrap();
    assert_eq!(name.role, "text_input");
    assert_eq!(name.value, Some(json!("")));

    let submit = snap.select_unique(&Selector::Id("submit".into())).unwrap();
    assert_eq!(submit.role, "button");
    assert_eq!(submit.label.as_deref(), Some("Submit"));
    assert!(submit.actions.contains(&"click".to_string()));
    assert!(submit.rect.is_some(), "registered widgets carry rects");
}

#[test]
fn set_value_actuates_text_edit() {
    let mut h = form_harness();
    let body = h
        .send(Command::SetValue {
            target: Selector::Id("name".into()),
            value: json!("Ada"),
            quiet: false,
        })
        .expect("set_value succeeds");
    let ResponseBody::ValueSet { snapshot } = body else {
        panic!("{body:?}")
    };
    assert_eq!(h.app().name, "Ada");
    // Post-settle snapshot must already show the applied value.
    let snap = snapshot.unwrap();
    let name = snap.select_unique(&Selector::Id("name".into())).unwrap();
    assert_eq!(name.value, Some(json!("Ada")));
}

#[test]
fn click_actuates_button_like_a_user() {
    let mut h = form_harness();
    h.send(Command::SetValue {
        target: Selector::Id("name".into()),
        value: json!("Grace"),
        quiet: false,
    })
    .unwrap();
    let body = h
        .send(Command::Invoke {
            target: Some(Selector::Id("submit".into())),
            action: "click".into(),
            args: None,
            quiet: false,
        })
        .expect("click succeeds");
    assert!(matches!(body, ResponseBody::Invoked { .. }));
    assert_eq!(h.app().submitted.as_deref(), Some("Grace"));
}

#[test]
fn click_by_label_resolves_through_registry() {
    let mut h = form_harness();
    h.send(Command::Invoke {
        target: Some(Selector::Label("Submit".into())),
        action: "click".into(),
        args: None,
        quiet: false,
    })
    .expect("click by label succeeds");
    assert!(h.app().submitted.is_some());
}

#[test]
fn set_value_actuates_checkbox_and_slider() {
    let mut h = form_harness();
    h.send(Command::SetValue {
        target: Selector::Id("subscribed".into()),
        value: json!(true),
        quiet: false,
    })
    .unwrap();
    assert!(h.app().subscribed);

    // Toggle via click too.
    h.send(Command::Invoke {
        target: Some(Selector::Id("subscribed".into())),
        action: "click".into(),
        args: None,
        quiet: false,
    })
    .unwrap();
    assert!(!h.app().subscribed);

    h.send(Command::SetValue {
        target: Selector::Id("volume".into()),
        value: json!(150.0),
        quiet: false,
    })
    .unwrap();
    assert_eq!(h.app().volume, 100.0, "slider values clamp to their range");
}

#[test]
fn actuating_missing_widget_times_out_to_target_not_found() {
    let mut h = form_harness();
    let err = h
        .send(Command::Invoke {
            target: Some(Selector::Id("ghost".into())),
            action: "click".into(),
            args: None,
            quiet: false,
        })
        .expect_err("no such widget");
    assert!(matches!(err, AgentError::TargetNotFound(_)), "{err:?}");
}

#[test]
fn widget_appearing_within_grace_frames_is_actuated() {
    // The target button only exists after the app's flag flips, simulating
    // conditional UI. Selector::Id is optimistic and waits grace frames.
    struct LateApp {
        show: bool,
        clicked: bool,
        frames: u32,
    }
    impl AgentApp for LateApp {}

    let mut h = TestHarness::new(
        LateApp {
            show: false,
            clicked: false,
            frames: 0,
        },
        |app, ui| {
            app.frames += 1;
            if app.frames >= 2 {
                app.show = true;
            }
            if app.show && egui_agent::widgets::button(ui, "late", "Late").clicked() {
                app.clicked = true;
            }
        },
    );
    h.send(Command::Invoke {
        target: Some(Selector::Id("late".into())),
        action: "click".into(),
        args: None,
        quiet: false,
    })
    .expect("late widget still actuated");
    assert!(h.app().clicked);
}

#[test]
fn expose_publishes_read_only_state() {
    let mut h = form_harness();
    let ResponseBody::Nodes(nodes) = h
        .send(Command::Query {
            target: Selector::Id("submitted".into()),
        })
        .unwrap()
    else {
        panic!()
    };
    assert_eq!(nodes.len(), 1);
    assert_eq!(nodes[0].value, Some(json!(null)));
}

#[test]
fn quiet_set_value_still_actuates() {
    let mut h = form_harness();
    let body = h
        .send(Command::SetValue {
            target: Selector::Id("name".into()),
            value: json!("Quiet Ada"),
            quiet: true,
        })
        .expect("quiet set_value succeeds");
    let ResponseBody::ValueSet { snapshot } = body else {
        panic!("{body:?}")
    };
    assert!(snapshot.is_none());
    assert_eq!(h.app().name, "Quiet Ada");
}
