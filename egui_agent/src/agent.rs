//! `Agent<A>`: the wrapper that runs the command lifecycle on the UI thread.
//!
//! Per-command state machine (design doc §6):
//!
//! ```text
//! Queued → Executed(frame N) → Settled(frame N+1) → Responded
//! ```
//!
//! Simple commands (Snapshot/Query) resolve in one frame. Actuations settle
//! per [`SettleUntil`] so post-action snapshots aren't stale. Registry
//! actuations add one hand-off frame for the target widget to consume the
//! actuation while drawing.

use std::time::Instant;

use serde_json::Value;

use crate::app::{Action, ActionResult, AgentApp, SnapshotCtx};
use crate::config::{Config, SettleUntil};
use crate::hub::{CommandHub, Envelope};
use crate::protocol::{AgentError, Command, Predicate, Response, ResponseBody, Selector};
use crate::registry::{self, PendingActuation};
use crate::snapshot::Snapshot;

/// Wraps an [`AgentApp`] and drives queued commands inside the frame loop.
///
/// With the `eframe` feature (default), `Agent<A>` implements `eframe::App`
/// when `A` does — see [`crate::wrap`]. For headless/kittest use, call
/// [`Agent::run_frame`] yourself.
pub struct Agent<A> {
    inner: A,
    hub: CommandHub,
    pending: Vec<Pending>,
}

struct Pending {
    reply: flume::Sender<Response>,
    /// Always `Some` between frames; taken while being advanced.
    state: Option<PendingState>,
    /// Per-command override suppressing the response snapshot.
    quiet: bool,
}

enum ActKind {
    Invoke { result: Option<Value> },
    SetValue,
}

enum PendingState {
    /// Executed; waiting `frames_left` more frames (or idle) before the
    /// post-action snapshot is captured and the response sent.
    Settling { kind: ActKind, frames_left: u32 },
    /// A registry actuation was queued for `id`; waiting for the target
    /// widget to consume it while drawing.
    AwaitingConsume {
        id: String,
        kind: ActKind,
        grace_left: u32,
    },
    /// Re-evaluated every frame until satisfied or past `deadline`.
    WaitFor {
        predicate: Predicate,
        started: Instant,
        deadline: Instant,
    },
    /// Waiting for the backend to deliver the screenshot event.
    #[cfg(feature = "screenshot")]
    Screenshot { frames_left: u32 },
}

impl<A: AgentApp> Agent<A> {
    pub(crate) fn new(inner: A, hub: CommandHub) -> Self {
        Self {
            inner,
            hub,
            pending: Vec::new(),
        }
    }

    pub fn inner(&self) -> &A {
        &self.inner
    }

    pub fn inner_mut(&mut self) -> &mut A {
        &mut self.inner
    }

    pub fn into_inner(self) -> A {
        self.inner
    }

    /// Run one full frame manually: lifecycle pre-pass, the app's UI, then the
    /// lifecycle post-pass. This is the transport-free path used by
    /// [`crate::TestHarness`] and custom (non-eframe) integrations.
    pub fn run_frame(&mut self, ctx: &egui::Context, ui: impl FnOnce(&mut A, &egui::Context)) {
        self.pre_frame(ctx);
        ui(&mut self.inner, ctx);
        self.post_frame(ctx);
    }

    /// Lifecycle pre-pass: advance pending commands, then drain and execute
    /// newly queued ones. Runs on the UI thread before the app's UI.
    pub fn pre_frame(&mut self, ctx: &egui::Context) {
        self.hub.bind_context_if_needed(ctx);
        #[cfg(feature = "accesskit-introspection")]
        crate::introspection::ensure_installed(ctx);
        self.advance_pending(ctx);
        // Drain everything queued so far; commands arriving mid-frame wait for
        // the next repaint (transports request one on enqueue).
        while let Ok(env) = self.hub.shared.rx.try_recv() {
            self.execute(env, ctx);
        }
        // Schedule here as well as in post_frame: when the window is hidden,
        // eframe runs `logic` (→ pre_frame) but not `ui` (→ post_frame), and
        // pending commands must keep producing frames to advance.
        self.schedule_repaints(ctx);
    }

    /// Lifecycle post-pass: registry bookkeeping and repaint scheduling.
    /// Runs on the UI thread after the app's UI.
    pub fn post_frame(&mut self, ctx: &egui::Context) {
        registry::end_frame(ctx);
        self.schedule_repaints(ctx);
    }

    /// Keep frames coming while commands are pending: settling/actuating
    /// commands need the very next frame; WaitFor polls at `poll_interval`.
    fn schedule_repaints(&self, ctx: &egui::Context) {
        let mut needs_frame = false;
        let mut needs_poll = false;
        for p in &self.pending {
            match p.state {
                Some(PendingState::WaitFor { .. }) => needs_poll = true,
                _ => needs_frame = true,
            }
        }
        if needs_frame {
            ctx.request_repaint();
        } else if needs_poll {
            ctx.request_repaint_after(self.hub.config().poll_interval);
        }
    }

    fn config(&self) -> &Config {
        self.hub.config()
    }

    fn snapshot(&self, ctx: &egui::Context) -> Snapshot {
        self.inner.snapshot(&SnapshotCtx { ctx })
    }

    fn settle_frames(&self) -> u32 {
        match self.config().settle {
            SettleUntil::NextFrame => 1,
            SettleUntil::Idle { max_frames } => max_frames.max(1),
        }
    }

    fn advance_pending(&mut self, ctx: &egui::Context) {
        let mut i = 0;
        while i < self.pending.len() {
            if self.advance_one(i, ctx) {
                self.pending.remove(i);
            } else {
                i += 1;
            }
        }
    }

    /// Build and send the settled response for an actuation.
    fn respond_settled(&mut self, i: usize, kind: ActKind, ctx: &egui::Context) {
        let snapshot = (self.config().post_action_snapshot && !self.pending[i].quiet)
            .then(|| self.snapshot(ctx));
        let body = match kind {
            ActKind::Invoke { result } => ResponseBody::Invoked { result, snapshot },
            ActKind::SetValue => ResponseBody::ValueSet { snapshot },
        };
        let _ = self.pending[i].reply.send(Ok(body));
    }

    /// Advance pending command `i` by one frame; true when responded.
    fn advance_one(&mut self, i: usize, ctx: &egui::Context) -> bool {
        let state = self.pending[i].state.take().expect("pending state present");
        match state {
            PendingState::Settling { kind, frames_left } => {
                let frames_left = frames_left.saturating_sub(1);
                let idle_settled = matches!(self.config().settle, SettleUntil::Idle { .. })
                    && !ctx.has_requested_repaint();
                if frames_left == 0 || idle_settled {
                    self.respond_settled(i, kind, ctx);
                    true
                } else {
                    self.pending[i].state = Some(PendingState::Settling { kind, frames_left });
                    false
                }
            }
            PendingState::AwaitingConsume {
                id,
                kind,
                grace_left,
            } => {
                if !registry::unconsumed(ctx).contains(&id) {
                    // Consumed while the last frame drew; one frame has already
                    // elapsed since, so a NextFrame settle resolves right now.
                    let extra = self.settle_frames() - 1;
                    if extra == 0 {
                        self.respond_settled(i, kind, ctx);
                        true
                    } else {
                        self.pending[i].state = Some(PendingState::Settling {
                            kind,
                            frames_left: extra,
                        });
                        false
                    }
                } else if grace_left == 0 {
                    registry::cancel_actuation(ctx, &id);
                    let _ = self.pending[i]
                        .reply
                        .send(Err(AgentError::TargetNotFound(format!("id={id}"))));
                    true
                } else {
                    self.pending[i].state = Some(PendingState::AwaitingConsume {
                        id,
                        kind,
                        grace_left: grace_left - 1,
                    });
                    false
                }
            }
            PendingState::WaitFor {
                predicate,
                started,
                deadline,
            } => {
                let snap = self.snapshot(ctx);
                if crate::predicate::eval(&predicate, &snap) {
                    let waited_ms = started.elapsed().as_millis() as u64;
                    let snapshot = (self.config().post_action_snapshot && !self.pending[i].quiet)
                        .then_some(snap);
                    let _ = self.pending[i].reply.send(Ok(ResponseBody::Satisfied {
                        waited_ms,
                        snapshot,
                    }));
                    true
                } else if Instant::now() >= deadline {
                    let _ = self.pending[i].reply.send(Err(AgentError::Timeout(format!(
                        "predicate not satisfied after {} ms",
                        started.elapsed().as_millis()
                    ))));
                    true
                } else {
                    self.pending[i].state = Some(PendingState::WaitFor {
                        predicate,
                        started,
                        deadline,
                    });
                    false
                }
            }
            #[cfg(feature = "screenshot")]
            PendingState::Screenshot { frames_left } => {
                if let Some(resp) = crate::screenshot::poll(ctx) {
                    let _ = self.pending[i].reply.send(resp);
                    true
                } else if frames_left == 0 {
                    let _ = self.pending[i].reply.send(Err(AgentError::NotSupported(
                        "backend did not deliver a screenshot (renderer may not support it)".into(),
                    )));
                    true
                } else {
                    self.pending[i].state = Some(PendingState::Screenshot {
                        frames_left: frames_left - 1,
                    });
                    false
                }
            }
        }
    }

    fn execute(&mut self, env: Envelope, ctx: &egui::Context) {
        crate::audit::log_command(self.config(), &env.cmd);
        let Envelope { cmd, reply } = env;
        match cmd {
            Command::Snapshot => {
                let _ = reply.send(Ok(ResponseBody::Snapshot(self.snapshot(ctx))));
            }
            Command::Query { target } => {
                let snap = self.snapshot(ctx);
                let nodes: Vec<_> = snap.select(&target).cloned().collect();
                let _ = reply.send(Ok(ResponseBody::Nodes(nodes)));
            }
            Command::Invoke {
                target,
                action,
                args,
                quiet,
            } => {
                if self.reject_if_read_only(&reply) {
                    return;
                }
                let act = Action::Invoke {
                    target,
                    name: action,
                    args,
                };
                self.dispatch_action(act, quiet, reply, ctx);
            }
            Command::Custom { name, args } => {
                if self.reject_if_read_only(&reply) {
                    return;
                }
                let act = Action::Invoke {
                    target: None,
                    name,
                    args: Some(args),
                };
                self.dispatch_action(act, false, reply, ctx);
            }
            Command::SetValue {
                target,
                value,
                quiet,
            } => {
                if self.reject_if_read_only(&reply) {
                    return;
                }
                let act = Action::SetValue { target, value };
                self.dispatch_action(act, quiet, reply, ctx);
            }
            Command::WaitFor {
                predicate,
                timeout_ms,
                quiet,
            } => {
                let now = Instant::now();
                let state = PendingState::WaitFor {
                    predicate,
                    started: now,
                    deadline: now + std::time::Duration::from_millis(timeout_ms),
                };
                // Evaluate at least once immediately: resolves same-frame when
                // the predicate already holds.
                self.pending.push(Pending {
                    reply,
                    state: Some(state),
                    quiet,
                });
                let i = self.pending.len() - 1;
                if self.advance_one(i, ctx) {
                    self.pending.remove(i);
                }
            }
            Command::Screenshot => {
                #[cfg(feature = "screenshot")]
                {
                    crate::screenshot::request(ctx);
                    self.pending.push(Pending {
                        reply,
                        state: Some(PendingState::Screenshot { frames_left: 10 }),
                        quiet: false,
                    });
                }
                #[cfg(not(feature = "screenshot"))]
                {
                    let _ = ctx;
                    let _ = reply.send(Err(AgentError::NotSupported(
                        "built without the `screenshot` feature".into(),
                    )));
                }
            }
        }
    }

    fn reject_if_read_only(&self, reply: &flume::Sender<Response>) -> bool {
        if self.config().read_only {
            let _ = reply.send(Err(AgentError::ReadOnly));
            true
        } else {
            false
        }
    }

    fn dispatch_action(
        &mut self,
        action: Action,
        quiet: bool,
        reply: flume::Sender<Response>,
        ctx: &egui::Context,
    ) {
        match self.inner.dispatch(&action, ctx) {
            ActionResult::Handled(result) => {
                let kind = match &action {
                    Action::Invoke { .. } => ActKind::Invoke { result },
                    Action::SetValue { .. } => ActKind::SetValue,
                };
                if self.config().post_action_snapshot && !quiet {
                    self.pending.push(Pending {
                        reply,
                        state: Some(PendingState::Settling {
                            kind,
                            frames_left: self.settle_frames(),
                        }),
                        quiet,
                    });
                } else {
                    let body = match kind {
                        ActKind::Invoke { result } => ResponseBody::Invoked {
                            result,
                            snapshot: None,
                        },
                        ActKind::SetValue => ResponseBody::ValueSet { snapshot: None },
                    };
                    let _ = reply.send(Ok(body));
                }
            }
            ActionResult::Rejected(msg) => {
                let _ = reply.send(Err(AgentError::ActionRejected(msg)));
            }
            ActionResult::BadArgs(msg) => {
                let _ = reply.send(Err(AgentError::BadArgs(msg)));
            }
            ActionResult::Unhandled => self.registry_fallback(action, quiet, reply, ctx),
        }
    }

    /// Actuate a widget when the app's dispatch didn't handle the action:
    /// first via the registered-widget registry (semantic ids), then — with
    /// the `accesskit-introspection` feature — via an injected AccessKit
    /// `ActionRequest`, which drives plain egui widgets with no app code.
    fn registry_fallback(
        &mut self,
        action: Action,
        quiet: bool,
        reply: flume::Sender<Response>,
        ctx: &egui::Context,
    ) {
        let Some(target) = action.target().cloned() else {
            let _ = reply.send(Err(AgentError::UnknownAction(action.name().to_owned())));
            return;
        };

        let registry_err = match self.try_registry(&action, &target, ctx) {
            Ok((id, actuation, kind)) => {
                registry::queue_actuation(ctx, id.clone(), actuation);
                self.pending.push(Pending {
                    reply,
                    state: Some(PendingState::AwaitingConsume {
                        id,
                        kind,
                        grace_left: self.config().actuation_grace_frames,
                    }),
                    quiet,
                });
                return;
            }
            Err(e) => e,
        };

        #[cfg(feature = "accesskit-introspection")]
        match self.try_accesskit(&action, &target, ctx) {
            Ok((kind, injection_frames)) => {
                // Injected over `injection_frames` passes, then settled.
                self.pending.push(Pending {
                    reply,
                    state: Some(PendingState::Settling {
                        kind,
                        frames_left: self.settle_frames() + injection_frames,
                    }),
                    quiet,
                });
            }
            Err(ak_err) => {
                // Report the more specific failure.
                let err = if matches!(registry_err, AgentError::TargetNotFound(_))
                    && !matches!(ak_err, AgentError::TargetNotFound(_))
                {
                    ak_err
                } else {
                    registry_err
                };
                let _ = reply.send(Err(err));
            }
        }

        #[cfg(not(feature = "accesskit-introspection"))]
        {
            let _ = reply.send(Err(registry_err));
        }
    }

    /// Registry (tier 2) actuation: map the action onto a semantic-id widget.
    fn try_registry(
        &self,
        action: &Action,
        target: &Selector,
        ctx: &egui::Context,
    ) -> Result<(String, PendingActuation, ActKind), AgentError> {
        let (actuation, kind) = match action {
            Action::Invoke { name, .. } => {
                if name != "click" && name != "toggle" {
                    return Err(AgentError::UnknownAction(name.clone()));
                }
                (PendingActuation::Click, ActKind::Invoke { result: None })
            }
            Action::SetValue { value, .. } => {
                (PendingActuation::SetValue(value.clone()), ActKind::SetValue)
            }
        };
        let id = self.resolve_semantic_id(target, ctx)?;
        Ok((id, actuation, kind))
    }

    /// AccessKit (tier 3) actuation: queue an `ActionRequest` (or emulated
    /// input sequence) for injection into upcoming passes' raw input — the
    /// same path a screen reader uses. Returns the actuation kind and how
    /// many injection passes it needs before settling starts.
    #[cfg(feature = "accesskit-introspection")]
    fn try_accesskit(
        &self,
        action: &Action,
        target: &Selector,
        ctx: &egui::Context,
    ) -> Result<(ActKind, u32), AgentError> {
        use accesskit::{Action as AkAction, ActionData, ActionRequest};

        let node = match target {
            Selector::Node(n) => {
                let id = accesskit::NodeId(*n);
                // Validate against the cached tree: egui silently ignores
                // ActionRequests for unknown targets, which would otherwise
                // turn a stale/corrupted node id into a silent no-op.
                if !crate::introspection::node_exists(ctx, id) {
                    return Err(AgentError::TargetNotFound(target.to_string()));
                }
                id
            }
            other => crate::introspection::resolve(ctx, other)?,
        };
        let request = |action, data| ActionRequest {
            action,
            target_tree: accesskit::TreeId::ROOT,
            target_node: node,
            data,
        };

        match action {
            Action::Invoke { name, .. } => {
                let ak_action = match name.as_str() {
                    "click" | "toggle" => AkAction::Click,
                    "focus" => AkAction::Focus,
                    "increment" => AkAction::Increment,
                    "decrement" => AkAction::Decrement,
                    other => return Err(AgentError::UnknownAction(other.to_owned())),
                };
                crate::introspection::queue_action_request(ctx, request(ak_action, None));
                Ok((ActKind::Invoke { result: None }, 1))
            }
            Action::SetValue { value, .. } => {
                if let Some(s) = value.as_str() {
                    // egui 0.36's TextEdit handles SetTextSelection but not
                    // SetValue (verified in source), so emulate a user:
                    // focus the widget, then select-all + delete + type.
                    let select_all = egui::Event::Key {
                        key: egui::Key::A,
                        physical_key: None,
                        pressed: true,
                        repeat: false,
                        modifiers: egui::Modifiers::COMMAND,
                    };
                    let clear = egui::Event::Key {
                        key: egui::Key::Backspace,
                        physical_key: None,
                        pressed: true,
                        repeat: false,
                        modifiers: egui::Modifiers::NONE,
                    };
                    let mut second = vec![select_all, clear];
                    if !s.is_empty() {
                        second.push(egui::Event::Text(s.to_owned()));
                    }
                    crate::introspection::queue_event_batches(
                        ctx,
                        vec![
                            vec![egui::Event::AccessKitActionRequest(request(
                                AkAction::Focus,
                                None,
                            ))],
                            second,
                        ],
                    );
                    Ok((ActKind::SetValue, 2))
                } else if let Some(n) = value.as_f64() {
                    crate::introspection::queue_action_request(
                        ctx,
                        request(AkAction::SetValue, Some(ActionData::NumericValue(n))),
                    );
                    Ok((ActKind::SetValue, 1))
                } else if let Some(want) = value.as_bool() {
                    match crate::introspection::node_toggled(ctx, node) {
                        Some(current) if current == want => {} // already there
                        Some(_) => crate::introspection::queue_action_request(
                            ctx,
                            request(AkAction::Click, None),
                        ),
                        None => {
                            return Err(AgentError::BadArgs(
                                "target is not a toggleable widget".into(),
                            ));
                        }
                    }
                    Ok((ActKind::SetValue, 1))
                } else {
                    Err(AgentError::BadArgs(
                        "set_value via accesskit needs a string, number, or bool".into(),
                    ))
                }
            }
        }
    }

    /// Map a selector to the semantic id of a registered widget.
    /// `Selector::Id` passes through even if the widget hasn't drawn yet
    /// (it gets `actuation_grace_frames` to appear); other selectors resolve
    /// against the last completed frame's snapshot.
    fn resolve_semantic_id(
        &self,
        target: &Selector,
        ctx: &egui::Context,
    ) -> Result<String, AgentError> {
        if let Selector::Id(id) = target {
            return Ok(id.clone());
        }
        let snap = self.snapshot(ctx);
        let node = snap.select_unique(target)?;
        node.id.clone().ok_or_else(|| {
            AgentError::NotSupported(format!(
                "matched node has no semantic id; registry actuation needs one ({target})"
            ))
        })
    }
}

#[cfg(feature = "eframe")]
impl<A: AgentApp + eframe::App> eframe::App for Agent<A> {
    fn logic(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        // eframe calls `logic` once before each `ui` (and also when hidden):
        // the right place to drain and execute queued commands.
        self.pre_frame(ctx);
        self.inner.logic(ctx, frame);
    }

    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        self.inner.ui(ui, frame);
        let ctx = ui.ctx().clone();
        self.post_frame(&ctx);
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        self.inner.save(storage);
    }

    // eframe's `on_exit` signature changes with its `glow` feature; enable the
    // matching `egui_agent/glow` passthrough feature in glow apps.
    #[cfg(feature = "glow")]
    fn on_exit(&mut self, gl: Option<&eframe::glow::Context>) {
        self.inner.on_exit(gl);
    }

    #[cfg(not(feature = "glow"))]
    fn on_exit(&mut self) {
        self.inner.on_exit();
    }

    fn auto_save_interval(&self) -> std::time::Duration {
        self.inner.auto_save_interval()
    }

    fn clear_color(&self, visuals: &egui::Visuals) -> [f32; 4] {
        self.inner.clear_color(visuals)
    }

    fn persist_egui_memory(&self) -> bool {
        self.inner.persist_egui_memory()
    }

    fn raw_input_hook(&mut self, ctx: &egui::Context, raw_input: &mut egui::RawInput) {
        self.inner.raw_input_hook(ctx, raw_input);
    }
}

#[cfg(feature = "eframe")]
const _: () = {
    // Compile-time guard: Agent<A> must remain object-safe as an eframe::App
    // so `Box::new(egui_agent::wrap(app, control))` works in AppCreator.
    fn _assert_app<A: AgentApp + eframe::App + 'static>(a: Agent<A>) -> Box<dyn eframe::App> {
        Box::new(a)
    }
};
