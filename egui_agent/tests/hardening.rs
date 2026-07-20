//! M5 hardening tests: event stream, Idle settle policy, audit log, and the
//! headless screenshot fallback path.

use egui_agent::{
    AgentApp, AgentError, Command, Config, Predicate, ResponseBody, Selector, SettleUntil,
    TestHarness,
};
use serde_json::json;

#[derive(Default)]
struct EventApp {
    saves: u32,
}

impl AgentApp for EventApp {}

#[test]
fn events_are_observable_and_waitable() {
    let mut h = TestHarness::new(EventApp::default(), |app, ui| {
        if egui_agent::widgets::button(ui, "save", "Save").clicked() {
            app.saves += 1;
            egui_agent::events::emit(ui.ctx(), "saved", json!({ "n": app.saves }));
        }
    });

    // No events yet.
    let ResponseBody::Nodes(nodes) = h
        .send(Command::Query {
            target: Selector::Role {
                role: "event".into(),
                name: None,
            },
        })
        .unwrap()
    else {
        panic!()
    };
    assert!(nodes.is_empty());

    // Concurrent WaitFor observes the event emitted by a later click.
    let rx = h.hub().submit(Command::WaitFor {
        predicate: Predicate::Exists(Selector::Label("saved".into())),
        timeout_ms: 10_000,
        quiet: false,
    });
    h.send(Command::Invoke {
        target: Some(Selector::Id("save".into())),
        action: "click".into(),
        args: None,
        quiet: false,
    })
    .unwrap();
    h.step();
    let body = rx
        .try_recv()
        .expect("wait_for resolved by the event")
        .expect("satisfied");
    assert!(matches!(body, ResponseBody::Satisfied { .. }));

    // The event node carries seq/name/data.
    let ResponseBody::Nodes(nodes) = h
        .send(Command::Query {
            target: Selector::Role {
                role: "event".into(),
                name: None,
            },
        })
        .unwrap()
    else {
        panic!()
    };
    assert_eq!(nodes.len(), 1);
    assert_eq!(nodes[0].label.as_deref(), Some("saved"));
    assert_eq!(nodes[0].value.as_ref().unwrap()["data"]["n"], json!(1));
}

#[test]
fn idle_settle_policy_resolves() {
    #[derive(Default)]
    struct App {
        count: u32,
    }
    impl AgentApp for App {
        fn snapshot(&self, _cx: &egui_agent::SnapshotCtx<'_>) -> egui_agent::Snapshot {
            egui_agent::Snapshot::new().node(egui_agent::Node::value("count", self.count))
        }
        fn dispatch(
            &mut self,
            action: &egui_agent::Action,
            _ctx: &egui::Context,
        ) -> egui_agent::ActionResult {
            match action.name() {
                "bump" => {
                    self.count += 1;
                    egui_agent::ActionResult::ok()
                }
                _ => egui_agent::ActionResult::Unhandled,
            }
        }
    }

    let cfg = Config {
        settle: SettleUntil::Idle { max_frames: 8 },
        ..Default::default()
    };
    let mut h = TestHarness::with_config(cfg, App::default(), |_, _| {});
    let body = h
        .send(Command::Invoke {
            target: None,
            action: "bump".into(),
            args: None,
            quiet: false,
        })
        .expect("bump settles under Idle policy");
    let ResponseBody::Invoked {
        snapshot: Some(snap),
        ..
    } = body
    else {
        panic!()
    };
    let node = snap.select_unique(&Selector::Id("count".into())).unwrap();
    assert_eq!(node.value, Some(json!(1)));
}

#[test]
fn audit_log_records_commands() {
    let path = std::env::temp_dir().join(format!(
        "egui_agent_audit_{}_{:?}.jsonl",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_file(&path);

    let cfg = Config {
        audit_log: Some(path.clone()),
        ..Default::default()
    };
    let mut h = TestHarness::with_config(cfg, EventApp::default(), |_, _| {});
    h.send(Command::Snapshot).unwrap();
    h.send(Command::Query {
        target: Selector::Id("x".into()),
    })
    .unwrap();

    let log = std::fs::read_to_string(&path).expect("audit file written");
    let lines: Vec<&str> = log.lines().collect();
    assert_eq!(lines.len(), 2);
    let first: serde_json::Value = serde_json::from_str(lines[0]).unwrap();
    assert!(first["ts_ms"].as_u64().is_some());
    assert_eq!(first["command"], json!("snapshot"));
    let _ = std::fs::remove_file(&path);
}

#[test]
fn screenshot_without_renderer_is_not_supported() {
    let mut h = TestHarness::new(EventApp::default(), |_, _| {});
    let err = h
        .send(Command::Screenshot)
        .expect_err("no renderer headlessly");
    assert!(matches!(err, AgentError::NotSupported(_)), "{err:?}");
}
