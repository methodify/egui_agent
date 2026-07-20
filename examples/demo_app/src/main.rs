//! Demo app: a small task list driven by egui_agent.
//!
//! Run with the control socket enabled:
//! ```text
//! DEMO_AGENT=1 cargo run -p demo_app
//! # then, from another terminal:
//! cargo run -p egui_agent_mcp --bin egui-agent-cli -- --socket /tmp/egui_agent_demo.sock snapshot
//! ```

use egui_agent::{Action, ActionResult, AgentApp, Snapshot, SnapshotCtx};

#[derive(Default)]
struct DemoApp {
    tasks: Vec<Task>,
    new_task: String,
    show_done: bool,
    total_added: u32,
}

struct Task {
    text: String,
    done: bool,
}

impl DemoApp {
    /// The same method the "Add" button calls — semantic dispatch routes here.
    fn add_task(&mut self) {
        let text = self.new_task.trim().to_owned();
        if text.is_empty() {
            return;
        }
        self.tasks.push(Task { text, done: false });
        self.new_task.clear();
        self.total_added += 1;
    }

    fn clear_done(&mut self) {
        self.tasks.retain(|t| !t.done);
    }
}

impl AgentApp for DemoApp {
    fn snapshot(&self, cx: &SnapshotCtx<'_>) -> Snapshot {
        // Registered widgets describe themselves; add domain state on top.
        Snapshot::new()
            .nodes(cx.registry_nodes())
            .with_data(&serde_json::json!({
                "tasks": self.tasks.iter().map(|t| {
                    serde_json::json!({ "text": t.text, "done": t.done })
                }).collect::<Vec<_>>(),
                "total_added": self.total_added,
            }))
    }

    fn dispatch(&mut self, action: &Action, _ctx: &egui::Context) -> ActionResult {
        match action.name() {
            "add_task" => {
                // Optional args override the text field, mirroring typing + click.
                if let Some(text) = action.args().and_then(|a| a.as_str()) {
                    self.new_task = text.to_owned();
                }
                self.add_task();
                ActionResult::with(&self.tasks.len())
            }
            "clear_done" => {
                self.clear_done();
                ActionResult::ok()
            }
            // Everything else (clicks/set_value on registered widgets) falls
            // through to automatic registry actuation.
            _ => ActionResult::Unhandled,
        }
    }
}

impl eframe::App for DemoApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        ui.heading("egui_agent demo — tasks");

        ui.horizontal(|ui| {
            egui_agent::widgets::text_edit_singleline(ui, "new_task", &mut self.new_task);
            if egui_agent::widgets::button(ui, "add", "Add").clicked() {
                self.add_task();
            }
        });

        egui_agent::widgets::checkbox(ui, "show_done", "Show done", &mut self.show_done);
        if egui_agent::widgets::button(ui, "clear_done", "Clear done").clicked() {
            self.clear_done();
        }

        ui.separator();
        for (i, task) in self.tasks.iter_mut().enumerate() {
            if !task.done || self.show_done {
                egui_agent::widgets::checkbox(
                    ui,
                    &format!("task_{i}"),
                    task.text.clone(),
                    &mut task.done,
                );
            }
        }
        egui_agent::widgets::expose(ui.ctx(), "task_count", self.tasks.len() as u32);
    }
}

fn main() -> eframe::Result {
    env_logger::init();
    let control = egui_agent::Control::spawn(egui_agent::Config {
        transport: egui_agent::Transport::UnixSocket("/tmp/egui_agent_demo.sock".into()),
        auth: egui_agent::Auth::from_env("DEMO_AGENT_TOKEN"),
        enabled: std::env::var("DEMO_AGENT").is_ok(),
        ..Default::default()
    });
    eframe::run_native(
        "egui_agent demo",
        Default::default(),
        Box::new(move |cc| {
            control.bind_context(cc.egui_ctx.clone());
            Ok(Box::new(egui_agent::wrap(DemoApp::default(), control)))
        }),
    )
}
