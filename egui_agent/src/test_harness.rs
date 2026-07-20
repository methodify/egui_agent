//! In-process test harness: drive an [`Agent`]-wrapped app under
//! `egui_kittest` with no transport — fully deterministic, CI-friendly.
//!
//! ```no_run
//! # use egui_agent::{TestHarness, Command, AgentApp};
//! # #[derive(Default)] struct MyApp;
//! # impl AgentApp for MyApp {}
//! let mut h = TestHarness::new(MyApp::default(), |app, ui| {
//!     ui.label("app ui goes here");
//! });
//! let snapshot = h.send(Command::Snapshot).unwrap();
//! ```

use crate::agent::Agent;
use crate::app::AgentApp;
use crate::config::Config;
use crate::control::{wrap, Control};
use crate::hub::CommandHub;
use crate::protocol::{AgentError, Command, Response};

type UiFn<A> = Box<dyn FnMut(&mut A, &mut egui::Ui)>;

/// Opaque state held by the kittest harness (the wrapped agent + UI closure).
pub struct HarnessState<A> {
    agent: Agent<A>,
    ui: UiFn<A>,
}

/// Feeds [`Command`]s directly into an [`Agent<A>`] running under a kittest
/// harness and returns settled [`Response`]s.
pub struct TestHarness<'a, A> {
    harness: egui_kittest::Harness<'a, HarnessState<A>>,
    hub: CommandHub,
    /// Frames to run before declaring a command unanswered.
    pub max_frames_per_command: u32,
}

impl<'a, A: AgentApp + 'static> TestHarness<'a, A> {
    /// Harness with default config; `ui` is the app's per-frame UI (the
    /// equivalent of `eframe::App::ui`, minus the `Frame`).
    pub fn new(app: A, ui: impl FnMut(&mut A, &mut egui::Ui) + 'static) -> Self {
        Self::with_config(Config::default(), app, ui)
    }

    pub fn with_config(
        cfg: Config,
        app: A,
        ui: impl FnMut(&mut A, &mut egui::Ui) + 'static,
    ) -> Self {
        let control = Control::spawn(Config {
            transport: crate::config::Transport::None,
            ..cfg
        });
        let hub = control.hub();
        let agent = wrap(app, control);
        let state = HarnessState {
            agent,
            ui: Box::new(ui),
        };
        let harness = egui_kittest::Harness::new_ui_state(
            |ui, state: &mut HarnessState<A>| {
                let ctx = ui.ctx().clone();
                let HarnessState { agent, ui: app_ui } = state;
                agent.pre_frame(&ctx);
                app_ui(agent.inner_mut(), ui);
                agent.post_frame(&ctx);
            },
            state,
        );
        Self {
            harness,
            hub,
            max_frames_per_command: 32,
        }
    }

    /// Enqueue a command, run frames until it settles, return its response.
    pub fn send(&mut self, cmd: Command) -> Response {
        let rx = self.hub.submit(cmd);
        for _ in 0..self.max_frames_per_command {
            self.harness.step();
            if let Ok(resp) = rx.try_recv() {
                return resp;
            }
        }
        Err(AgentError::Timeout(format!(
            "command not settled after {} frames",
            self.max_frames_per_command
        )))
    }

    /// The enqueue handle, e.g. to submit concurrent commands.
    pub fn hub(&self) -> CommandHub {
        self.hub.clone()
    }

    /// Run a single frame without sending anything.
    pub fn step(&mut self) {
        self.harness.step();
    }

    /// The wrapped app.
    pub fn app(&self) -> &A {
        self.harness.state().agent.inner()
    }

    pub fn app_mut(&mut self) -> &mut A {
        self.harness.state_mut().agent.inner_mut()
    }

    /// The underlying kittest harness, for input simulation and queries.
    pub fn kittest(&mut self) -> &mut egui_kittest::Harness<'a, HarnessState<A>> {
        &mut self.harness
    }
}
