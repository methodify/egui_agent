//! M0 milestone tests: the full command lifecycle driven in-process via
//! `egui_kittest`, with no transport.

use egui_agent::{
    Action, ActionResult, AgentApp, AgentError, Command, Config, Node, Predicate, ResponseBody,
    Selector, Snapshot, SnapshotCtx, TestHarness,
};
use serde_json::json;

/// A hand-written AgentApp (design doc §7 "v1" style): explicit snapshot +
/// semantic dispatch to the same methods the UI calls.
#[derive(Default)]
struct Counter {
    count: i64,
    limit: i64,
}

impl Counter {
    fn increment(&mut self) -> bool {
        if self.count < self.limit {
            self.count += 1;
            true
        } else {
            false
        }
    }
}

impl AgentApp for Counter {
    fn snapshot(&self, _cx: &SnapshotCtx<'_>) -> Snapshot {
        Snapshot::new()
            .node(Node::value("count", self.count))
            .node(Node::button("increment", "Increment"))
            .with_data(&json!({ "limit": self.limit }))
    }

    fn dispatch(&mut self, action: &Action, _ctx: &egui::Context) -> ActionResult {
        match action.name() {
            "increment" | "click" => {
                if self.increment() {
                    ActionResult::with(&self.count)
                } else {
                    ActionResult::Rejected("limit reached".into())
                }
            }
            "set_limit" => match action.args().and_then(|v| v.as_i64()) {
                Some(limit) => {
                    self.limit = limit;
                    ActionResult::ok()
                }
                None => ActionResult::BadArgs("set_limit needs an integer".into()),
            },
            _ => ActionResult::Unhandled,
        }
    }
}

fn counter_harness() -> TestHarness<'static, Counter> {
    TestHarness::new(
        Counter {
            count: 0,
            limit: 10,
        },
        |app, ui| {
            ui.label(format!("count: {}", app.count));
            if ui.button("Increment").clicked() {
                app.increment();
            }
        },
    )
}

#[test]
fn snapshot_reflects_app_state() {
    let mut h = counter_harness();
    let body = h.send(Command::Snapshot).expect("snapshot succeeds");
    let ResponseBody::Snapshot(snap) = body else {
        panic!("expected snapshot body, got {body:?}")
    };
    let count = snap.select_unique(&Selector::Id("count".into())).unwrap();
    assert_eq!(count.value, Some(json!(0)));
    assert_eq!(snap.data, Some(json!({ "limit": 10 })));
}

#[test]
fn invoke_dispatches_and_returns_settled_snapshot() {
    let mut h = counter_harness();
    let body = h
        .send(Command::Invoke {
            target: None,
            action: "increment".into(),
            args: None,
        })
        .expect("invoke succeeds");
    let ResponseBody::Invoked { result, snapshot } = body else {
        panic!("expected invoked body, got {body:?}")
    };
    assert_eq!(result, Some(json!(1)));
    // The post-action snapshot proves settle semantics: captured on the frame
    // after execution, it must show the new state.
    let snap = snapshot.expect("post-action snapshot attached by default");
    let count = snap.select_unique(&Selector::Id("count".into())).unwrap();
    assert_eq!(count.value, Some(json!(1)));
    assert_eq!(h.app().count, 1);
}

#[test]
fn invoke_rejection_maps_to_action_rejected() {
    let mut h = TestHarness::new(Counter { count: 0, limit: 0 }, |_, _| {});
    let err = h
        .send(Command::Invoke {
            target: None,
            action: "increment".into(),
            args: None,
        })
        .expect_err("limit reached");
    assert_eq!(err, AgentError::ActionRejected("limit reached".into()));
}

#[test]
fn bad_args_and_unknown_action() {
    let mut h = counter_harness();
    let err = h
        .send(Command::Invoke {
            target: None,
            action: "set_limit".into(),
            args: Some(json!("x")),
        })
        .expect_err("bad args");
    assert!(matches!(err, AgentError::BadArgs(_)), "{err:?}");

    let err = h
        .send(Command::Invoke {
            target: None,
            action: "no_such_verb".into(),
            args: None,
        })
        .expect_err("unknown action");
    assert_eq!(err, AgentError::UnknownAction("no_such_verb".into()));
}

#[test]
fn custom_command_routes_through_dispatch() {
    let mut h = counter_harness();
    let body = h
        .send(Command::Custom {
            name: "set_limit".into(),
            args: json!(3),
        })
        .expect("custom succeeds");
    assert!(matches!(body, ResponseBody::Invoked { .. }));
    assert_eq!(h.app().limit, 3);
}

#[test]
fn query_selects_by_id_label_and_role() {
    let mut h = counter_harness();
    let by_label = h
        .send(Command::Query {
            target: Selector::Label("Increment".into()),
        })
        .expect("query succeeds");
    let ResponseBody::Nodes(nodes) = by_label else {
        panic!()
    };
    assert_eq!(nodes.len(), 1);
    assert_eq!(nodes[0].id.as_deref(), Some("increment"));

    let by_role = h
        .send(Command::Query {
            target: Selector::Role {
                role: "button".into(),
                name: None,
            },
        })
        .expect("query succeeds");
    let ResponseBody::Nodes(nodes) = by_role else {
        panic!()
    };
    assert_eq!(nodes.len(), 1);

    let miss = h
        .send(Command::Query {
            target: Selector::Id("nope".into()),
        })
        .expect("empty query is not an error");
    let ResponseBody::Nodes(nodes) = miss else {
        panic!()
    };
    assert!(nodes.is_empty());
}

#[test]
fn read_only_mode_rejects_actuation_but_allows_snapshot() {
    let cfg = Config {
        read_only: true,
        ..Default::default()
    };
    let mut h = TestHarness::with_config(
        cfg,
        Counter {
            count: 0,
            limit: 10,
        },
        |_, _| {},
    );
    assert!(h.send(Command::Snapshot).is_ok());
    let err = h
        .send(Command::Invoke {
            target: None,
            action: "increment".into(),
            args: None,
        })
        .expect_err("read-only");
    assert_eq!(err, AgentError::ReadOnly);
    let err = h
        .send(Command::SetValue {
            target: Selector::Id("count".into()),
            value: json!(5),
        })
        .expect_err("read-only");
    assert_eq!(err, AgentError::ReadOnly);
}

#[test]
fn wait_for_already_satisfied_resolves_same_frame() {
    let mut h = counter_harness();
    let body = h
        .send(Command::WaitFor {
            predicate: Predicate::ValueEq {
                target: Selector::Id("count".into()),
                value: json!(0),
            },
            timeout_ms: 1000,
        })
        .expect("wait_for succeeds");
    assert!(matches!(body, ResponseBody::Satisfied { .. }));
}

#[test]
fn wait_for_resolves_when_state_changes_later() {
    let mut h = counter_harness();
    let rx = h.hub().submit(Command::WaitFor {
        predicate: Predicate::ValueEq {
            target: Selector::Id("count".into()),
            value: json!(1),
        },
        timeout_ms: 10_000,
    });
    h.step();
    assert!(rx.try_recv().is_err(), "predicate should not hold yet");
    h.app_mut().count = 1;
    h.step();
    h.step();
    let body = rx
        .try_recv()
        .expect("resolved after state change")
        .expect("satisfied");
    let ResponseBody::Satisfied { snapshot, .. } = body else {
        panic!()
    };
    let snap = snapshot.expect("snapshot attached");
    let count = snap.select_unique(&Selector::Id("count".into())).unwrap();
    assert_eq!(count.value, Some(json!(1)));
}

#[test]
fn wait_for_times_out_with_structured_error() {
    let mut h = counter_harness();
    let err = h
        .send(Command::WaitFor {
            predicate: Predicate::Exists(Selector::Id("never".into())),
            timeout_ms: 1,
        })
        .expect_err("timeout");
    assert!(matches!(err, AgentError::Timeout(_)), "{err:?}");
}

#[test]
fn predicate_combinators() {
    let snap = Snapshot::new()
        .node(Node::value("a", 1).with_enabled(false))
        .node(Node::button("b", "B").with_focused(true))
        .with_data(&json!({ "nested": { "flag": true } }));

    use egui_agent::eval_predicate as eval;
    assert!(eval(&Predicate::Exists(Selector::Id("a".into())), &snap));
    assert!(eval(&Predicate::NotExists(Selector::Id("z".into())), &snap));
    assert!(eval(&Predicate::Disabled(Selector::Id("a".into())), &snap));
    assert!(eval(&Predicate::Focused(Selector::Id("b".into())), &snap));
    assert!(eval(
        &Predicate::DataEq {
            pointer: "/nested/flag".into(),
            value: json!(true)
        },
        &snap
    ));
    assert!(eval(
        &Predicate::All(vec![
            Predicate::Exists(Selector::Id("a".into())),
            Predicate::Not(Box::new(Predicate::Exists(Selector::Id("z".into())))),
        ]),
        &snap
    ));
    assert!(!eval(
        &Predicate::Any(vec![
            Predicate::Exists(Selector::Id("z".into())),
            Predicate::Focused(Selector::Id("a".into())),
        ]),
        &snap
    ));
}

#[test]
fn ambiguous_selector_is_reported() {
    #[derive(Default)]
    struct TwoButtons;
    impl AgentApp for TwoButtons {
        fn snapshot(&self, _cx: &SnapshotCtx<'_>) -> Snapshot {
            Snapshot::new()
                .node(Node::button("a", "Same"))
                .node(Node::button("b", "Same"))
        }
    }
    let mut h = TestHarness::new(TwoButtons, |_, _| {});
    let err = h
        .send(Command::Invoke {
            target: Some(Selector::Label("Same".into())),
            action: "click".into(),
            args: None,
        })
        .expect_err("ambiguous");
    assert!(matches!(err, AgentError::AmbiguousSelector(_)), "{err:?}");
}

#[test]
fn disabled_control_rejects_everything() {
    let cfg = Config {
        enabled: false,
        ..Default::default()
    };
    let mut h = TestHarness::with_config(cfg, Counter::default(), |_, _| {});
    let err = h.send(Command::Snapshot).expect_err("disabled");
    assert!(matches!(err, AgentError::NotSupported(_)), "{err:?}");
}

#[test]
fn protocol_serde_roundtrip() {
    let cmds = vec![
        Command::Snapshot,
        Command::Query {
            target: Selector::Role {
                role: "button".into(),
                name: Some("Add".into()),
            },
        },
        Command::Invoke {
            target: Some(Selector::Id("submit".into())),
            action: "click".into(),
            args: Some(json!({ "x": 1 })),
        },
        Command::SetValue {
            target: Selector::Label("Name".into()),
            value: json!("Ada"),
        },
        Command::WaitFor {
            predicate: Predicate::All(vec![Predicate::Exists(Selector::Node(7))]),
            timeout_ms: 250,
        },
        Command::Screenshot,
        Command::Custom {
            name: "reload".into(),
            args: json!(null),
        },
    ];
    for cmd in cmds {
        let json = serde_json::to_string(&cmd).unwrap();
        let back: Command = serde_json::from_str(&json).unwrap();
        // Round-trip must be lossless (compare via canonical JSON).
        assert_eq!(json, serde_json::to_string(&back).unwrap());
    }

    let err: AgentError =
        serde_json::from_str(r#"{"kind":"target_not_found","detail":"id=x"}"#).unwrap();
    assert_eq!(err, AgentError::TargetNotFound("id=x".into()));
}
