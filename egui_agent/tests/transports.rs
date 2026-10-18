//! M1/M5 transport tests: real out-of-process control over UDS (with auth),
//! WebSocket, and the cross-platform local-socket ("pipe") transport.
//!
//! Pattern: the UI runs under kittest on the test thread (stepping in a
//! loop, as eframe would on repaint requests); a client thread connects
//! through the OS socket and issues commands.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use egui_agent::{Auth, Command, Config, Control, ResponseBody, Selector, Transport};
use serde_json::json;

/// Minimal registry-only app shared by the transport tests.
#[derive(Default)]
struct EchoApp {
    text: String,
    clicks: u32,
}

impl egui_agent::AgentApp for EchoApp {}

/// Drive a wrapped agent under kittest until `done` flips, with a deadline.
fn pump(agent: egui_agent::Agent<EchoApp>, done: Arc<AtomicBool>) {
    let mut harness = egui_kittest::Harness::new_ui_state(
        |ui, agent: &mut egui_agent::Agent<EchoApp>| {
            let ctx = ui.ctx().clone();
            agent.pre_frame(&ctx);
            {
                let app = agent.inner_mut();
                egui_agent::widgets::text_edit_singleline(ui, "text", &mut app.text);
                if egui_agent::widgets::button(ui, "bump", "Bump").clicked() {
                    app.clicks += 1;
                }
            }
            agent.post_frame(&ctx);
        },
        agent,
    );
    let deadline = Instant::now() + Duration::from_secs(10);
    while !done.load(Ordering::SeqCst) {
        assert!(Instant::now() < deadline, "transport test timed out");
        harness.step();
        std::thread::sleep(Duration::from_millis(2));
    }
}

#[test]
fn uds_round_trip_with_auth() {
    let dir = std::env::temp_dir().join(format!("egui_agent_test_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let sock = dir.join("uds_auth.sock");

    let control = Control::spawn(Config {
        transport: Transport::UnixSocket(sock.clone()),
        auth: Auth::Token("sekrit".into()),
        ..Default::default()
    });
    let agent = egui_agent::wrap(EchoApp::default(), control);

    let done = Arc::new(AtomicBool::new(false));
    let done2 = done.clone();
    let sock2 = sock.clone();
    let client = std::thread::spawn(move || {
        let result = (|| -> Result<(), String> {
            // Wait for the listener to bind.
            let mut conn = None;
            for _ in 0..100 {
                match egui_agent::client::connect_uds(&sock2, Some("sekrit")) {
                    Ok(c) => {
                        conn = Some(c);
                        break;
                    }
                    Err(_) => std::thread::sleep(Duration::from_millis(10)),
                }
            }
            let mut conn = conn.ok_or("could not connect")?;

            // Wrong token must be rejected.
            match egui_agent::client::connect_uds(&sock2, Some("wrong")) {
                Ok(_) => return Err("wrong token accepted".into()),
                Err(e) => {
                    if e.kind() != std::io::ErrorKind::PermissionDenied {
                        return Err(format!("unexpected auth error kind: {e}"));
                    }
                }
            }

            let body = conn
                .request(Command::SetValue {
                    target: Selector::Id("text".into()),
                    value: json!("over the wire"),
                    quiet: false,
                })
                .map_err(|e| e.to_string())?
                .map_err(|e| e.to_string())?;
            if !matches!(body, ResponseBody::ValueSet { .. }) {
                return Err(format!("unexpected body: {body:?}"));
            }

            let body = conn
                .request(Command::Invoke {
                    target: Some(Selector::Id("bump".into())),
                    action: "click".into(),
                    args: None,
                    quiet: false,
                })
                .map_err(|e| e.to_string())?
                .map_err(|e| e.to_string())?;
            let ResponseBody::Invoked {
                snapshot: Some(snap),
                ..
            } = body
            else {
                return Err(format!("unexpected body: {body:?}"));
            };
            let text = snap
                .select_unique(&Selector::Id("text".into()))
                .map_err(|e| e.to_string())?;
            if text.value != Some(json!("over the wire")) {
                return Err(format!("snapshot text wrong: {:?}", text.value));
            }
            Ok(())
        })();
        done2.store(true, Ordering::SeqCst);
        result
    });

    pump(agent, done);
    client.join().unwrap().expect("client thread");
    let _ = std::fs::remove_file(sock);
}

#[test]
fn ws_round_trip() {
    // Bind an ephemeral-ish port; retry a few candidates to dodge collisions.
    let port = {
        let l = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let p = l.local_addr().unwrap().port();
        drop(l);
        p
    };

    let control = Control::spawn(Config {
        transport: Transport::WebSocket { port },
        ..Default::default()
    });
    let agent = egui_agent::wrap(EchoApp::default(), control);

    let done = Arc::new(AtomicBool::new(false));
    let done2 = done.clone();
    let client = std::thread::spawn(move || {
        let result = (|| -> Result<(), String> {
            let url = format!("ws://127.0.0.1:{port}");
            let mut ws = None;
            for _ in 0..100 {
                match tungstenite::connect(&url) {
                    Ok((socket, _)) => {
                        ws = Some(socket);
                        break;
                    }
                    Err(_) => std::thread::sleep(Duration::from_millis(10)),
                }
            }
            let mut ws = ws.ok_or("could not connect ws")?;
            let req = serde_json::to_string(&egui_agent::WireRequest {
                id: 1,
                command: Command::Snapshot,
            })
            .unwrap();
            ws.send(tungstenite::Message::text(req))
                .map_err(|e| e.to_string())?;
            let reply = loop {
                match ws.read().map_err(|e| e.to_string())? {
                    tungstenite::Message::Text(t) => break t.to_string(),
                    _ => continue,
                }
            };
            let resp: egui_agent::WireResponse =
                serde_json::from_str(&reply).map_err(|e| e.to_string())?;
            let body = resp.into_response().map_err(|e| e.to_string())?;
            if !matches!(body, ResponseBody::Snapshot(_)) {
                return Err(format!("unexpected body: {body:?}"));
            }
            let _ = ws.close(None);
            Ok(())
        })();
        done2.store(true, Ordering::SeqCst);
        result
    });

    pump(agent, done);
    client.join().unwrap().expect("ws client thread");
}

#[test]
fn pipe_round_trip() {
    use interprocess::local_socket::traits::Stream as _;
    use interprocess::local_socket::{GenericNamespaced, Stream, ToNsName};

    let name = format!("egui_agent_test_{}.pipe", std::process::id());

    let control = Control::spawn(Config {
        transport: Transport::Pipe(name.clone()),
        ..Default::default()
    });
    let agent = egui_agent::wrap(EchoApp::default(), control);

    let done = Arc::new(AtomicBool::new(false));
    let done2 = done.clone();
    let client = std::thread::spawn(move || {
        let result = (|| -> Result<(), String> {
            let ns = name
                .clone()
                .to_ns_name::<GenericNamespaced>()
                .map_err(|e| e.to_string())?;
            let mut stream = None;
            for _ in 0..100 {
                match Stream::connect(ns.clone()) {
                    Ok(s) => {
                        stream = Some(s);
                        break;
                    }
                    Err(_) => std::thread::sleep(Duration::from_millis(10)),
                }
            }
            let stream = stream.ok_or("could not connect local socket")?;
            let mut conn = egui_agent::client::Client::<Stream>::from_stream(stream, None)
                .map_err(|e| e.to_string())?;
            // The first frames may not have completed yet; wait for the UI.
            conn.request(Command::WaitFor {
                predicate: egui_agent::Predicate::Exists(Selector::Id("bump".into())),
                timeout_ms: 5000,
                quiet: false,
            })
            .map_err(|e| e.to_string())?
            .map_err(|e| e.to_string())?;
            let body = conn
                .request(Command::Query {
                    target: Selector::Id("bump".into()),
                })
                .map_err(|e| e.to_string())?
                .map_err(|e| e.to_string())?;
            let ResponseBody::Nodes(nodes) = body else {
                return Err(format!("unexpected body: {body:?}"));
            };
            if nodes.len() != 1 {
                return Err(format!("expected 1 node, got {}", nodes.len()));
            }
            Ok(())
        })();
        done2.store(true, Ordering::SeqCst);
        result
    });

    pump(agent, done);
    client.join().unwrap().expect("pipe client thread");
}

/// Regression: a long `wait_for` must not be killed by the transport's
/// liveness guard (`request_timeout`), and the transport re-pumps repaints so
/// the predicate keeps evaluating.
#[test]
fn long_wait_for_outlives_the_liveness_guard() {
    let dir = std::env::temp_dir().join(format!("egui_agent_test_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let sock = dir.join("uds_longwait.sock");
    let _ = std::fs::remove_file(&sock);

    let control = Control::spawn(Config {
        transport: Transport::UnixSocket(sock.clone()),
        // Liveness guard far shorter than the wait: the old code returned
        // Timeout after this interval regardless of wait_for's own deadline.
        request_timeout: Duration::from_millis(200),
        poll_interval: Duration::from_millis(20),
        ..Default::default()
    });
    let agent = egui_agent::wrap(EchoApp::default(), control);

    let done = Arc::new(AtomicBool::new(false));
    let done2 = done.clone();
    let sock2 = sock.clone();
    let client = std::thread::spawn(move || {
        let result = (|| -> Result<(), String> {
            let mut conn = None;
            for _ in 0..100 {
                match egui_agent::client::connect_uds(&sock2, None) {
                    Ok(c) => {
                        conn = Some(c);
                        break;
                    }
                    Err(_) => std::thread::sleep(Duration::from_millis(10)),
                }
            }
            let mut conn = conn.ok_or("could not connect")?;
            let started = Instant::now();
            let body = conn
                .request(Command::WaitFor {
                    predicate: egui_agent::Predicate::ValueEq {
                        target: Selector::Id("ticked".into()),
                        value: json!(true),
                    },
                    timeout_ms: 10_000,
                    quiet: true,
                })
                .map_err(|e| e.to_string())?
                .map_err(|e| format!("agent error (killed after {:?}?): {e}", started.elapsed()))?;
            if !matches!(body, ResponseBody::Satisfied { .. }) {
                return Err(format!("unexpected body: {body:?}"));
            }
            let waited = started.elapsed();
            if waited < Duration::from_millis(500) {
                return Err(format!("resolved suspiciously early: {waited:?}"));
            }
            Ok(())
        })();
        done2.store(true, Ordering::SeqCst);
        result
    });

    // Pump frames; the exposed value flips ~600ms in — three liveness
    // intervals after the wait began.
    let start = Instant::now();
    let mut harness = egui_kittest::Harness::new_ui_state(
        |ui, agent: &mut egui_agent::Agent<EchoApp>| {
            let ctx = ui.ctx().clone();
            agent.pre_frame(&ctx);
            egui_agent::widgets::expose(
                ui.ctx(),
                "ticked",
                start.elapsed() >= Duration::from_millis(600),
            );
            agent.post_frame(&ctx);
        },
        agent,
    );
    let deadline = Instant::now() + Duration::from_secs(15);
    while !done.load(Ordering::SeqCst) {
        assert!(Instant::now() < deadline, "long wait_for test timed out");
        harness.step();
        std::thread::sleep(Duration::from_millis(2));
    }
    client.join().unwrap().expect("client thread");
    let _ = std::fs::remove_file(sock);
}
