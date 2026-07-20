# Design Doc: `egui_agent` — a reusable agent control channel for egui/eframe apps

**Status:** Draft for implementation hand-off
**Working crate name:** `egui_agent` (placeholder — rename to taste)
**Audience:** the dev agent that will build this crate, and future-you integrating it across apps

---

## 1. Purpose

Provide a **drop-in, reusable crate** that lets an external agent (via MCP, a socket, or in-process tests) drive and introspect any egui/eframe application through **semantic commands and structured snapshots** rather than screenshots and pixel coordinates.

The integration target for any given app should be: implement one trait, wrap the app once at startup. Everything else — transport, queuing, repaint pumping, MCP exposure — lives in the crate.

## 2. Goals

- **Reusable across all my egui/eframe apps** with near-zero per-app boilerplate.
- **Semantic actuation**: the agent invokes named actions / sets named values; it never guesses coordinates.
- **Structured introspection**: the agent reads a serializable snapshot of the UI/app state (labels, roles, values, available actions), not an image.
- **Focus-independent and non-blocking**: the control channel runs on a background thread; driving the app never requires it to hold OS window focus or steal your desktop.
- **Transport-agnostic core**: MCP is *one* adapter over a small internal protocol; the same protocol also powers CLI tools and in-process tests.
- **Deterministic and testable**: the same command layer is exercisable headlessly via `egui_kittest`, so every app that adopts this also gets an integration-test harness for free.
- **Safe by default**: compiled out of release builds unless explicitly enabled; localhost/UDS-only; optional auth.

## 3. Non-goals

- Not a vision/screenshot agent. (A screenshot-on-demand command is an optional *fallback*, not the mechanism.)
- Not a general OS automation tool — it drives *your* apps, which cooperate by linking the crate.
- Not a remote-control/streaming UI. It's a command + state channel, not a pixel pipe.
- v1 does not attempt fully generic "click any widget with no app code." That's a later milestone (see §11, §14).

## 4. Background & constraints (why the design is shaped this way)

Three properties of egui/eframe drive every decision below:

1. **Immediate mode.** There is no retained widget tree to hold and call `.click()` on later. Widgets are re-created each frame. Therefore actuation cannot be "find object, invoke it"; it must be either (a) *semantic* — a command routed to the same app logic a click would trigger — or (b) *replayed input / AccessKit actions* fed into the next frame. v1 uses (a); (b) is a later enhancement.

2. **State lives on the UI thread.** The app's `&mut self` is only accessible inside `eframe::App::update`. Commands arriving on a transport thread **must be marshalled onto the UI thread** and executed inside `update`. This dictates the queue + oneshot-reply concurrency model in §6.

3. **Reactive repaint.** By default eframe sleeps until input arrives. A background command will not be processed until a frame runs, so the transport side must call `ctx.request_repaint()` to wake the UI thread. `egui::Context` is cheap to clone and `Send + Sync`, so a clone can safely live on the transport thread purely to request repaints.

A fourth property is an *asset*: eframe already integrates **AccessKit** (enabled by default), so egui builds a structured accessibility tree of labeled widgets with roles, values, and node ids. We can read that tree for introspection (§10) and, later, submit AccessKit `ActionRequest`s for generic actuation (§11).

## 5. High-level architecture

Three concentric layers, each independently testable:

```
        ┌─────────────────────────────────────────────┐
        │  Adapters (feature-gated, optional)          │
        │   • MCP server        • CLI / REPL           │
        │   • WebSocket / UDS / stdio transports       │
        └───────────────┬─────────────────────────────┘
                        │  Command / Response (serde)
        ┌───────────────▼─────────────────────────────┐
        │  Core (transport-agnostic)                   │
        │   • Command/Response protocol                │
        │   • command queue + oneshot replies          │
        │   • AgentApp trait (app implements)          │
        │   • frame lifecycle state machine            │
        └───────────────┬─────────────────────────────┘
                        │  runs inside update()
        ┌───────────────▼─────────────────────────────┐
        │  egui integration                            │
        │   • Agent<A> wrapper : eframe::App           │
        │   • drains queue on UI thread                │
        │   • request_repaint pump                     │
        │   • AccessKit snapshot capture               │
        └──────────────────────────────────────────────┘
```

The **core** knows nothing about sockets or MCP. Transports and MCP are thin adapters that translate an inbound request into a `Command`, push it into the queue, await the `Response`, and serialize it back. This is what makes it reusable: the same core serves MCP in production and `kittest` in CI.

## 6. Concurrency & lifecycle

**Command flow:**

1. Transport thread receives a request, decodes a `Command`, and pushes `(Command, oneshot::Sender<Response>)` onto a channel (`flume` or `crossbeam` — must bridge sync UI thread and possibly-async transport thread).
2. Transport thread calls `ctx.request_repaint()` to wake the UI.
3. On the next frame, `Agent<A>::update` drains the queue **before** running the inner app, executes each command against `&mut inner` on the UI thread, and records results.
4. For commands whose result depends on the post-command UI (e.g. "invoke then return the resulting snapshot"), the command is not resolved same-frame. It advances a small per-command state machine and `request_repaint()` is called again so the snapshot is captured *after* the UI settles.
5. When settled, the response is sent on the oneshot; the transport thread serializes and replies.

**Per-command state machine** (prevents returning stale UI):

```
Queued → Executed(frame N) → Settled(frame N+1) → Responded
```

Most commands (e.g. `GetState`) resolve in one frame. Actuating commands that must report resulting UI take two. Provide a configurable settle policy (`SettleUntil::NextFrame` | `SettleUntil::Idle { max_frames }`), mirroring why `egui_kittest` has an explicit `harness.run()` step.

**Repaint pump:** the transport handle holds a cloned `egui::Context`. Every enqueue triggers `request_repaint()`. Do **not** switch the app to continuous rendering just for this — that wastes CPU on the user's machine; reactive + `request_repaint` is the correct pattern.

## 7. Integration ergonomics (the per-app surface)

The headline goal: adopting this in a new app is **one trait + one wrap call**.

```rust
struct MyApp { /* ... */ }

// The only app-specific code you write for v1:
impl egui_agent::AgentApp for MyApp {
    fn snapshot(&self, cx: &egui_agent::SnapshotCtx) -> egui_agent::Snapshot {
        // Describe current state semantically. Helpers reduce this to a few lines;
        // or return Snapshot::from_accesskit(cx) once §10 auto-introspection lands.
    }

    fn dispatch(&mut self, cmd: &egui_agent::Action, ctx: &egui::Context)
        -> egui_agent::ActionResult {
        // Route named actions to the SAME methods your buttons call.
        // e.g. match cmd.name.as_str() { "submit" => { self.submit(); Ok(()) } ... }
    }
}

impl eframe::App for MyApp {
    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        // ... your normal UI, unchanged ...
    }
}

fn main() -> eframe::Result<()> {
    let control = egui_agent::Control::spawn(egui_agent::Config {
        transport: egui_agent::Transport::UnixSocket("/tmp/myapp.agent.sock".into()),
        auth: egui_agent::Auth::from_env("MYAPP_AGENT_TOKEN"),
        enabled: std::env::var("MYAPP_AGENT").is_ok(), // off unless opted in
        ..Default::default()
    });

    eframe::run_native("MyApp", Default::default(), Box::new(move |cc| {
        control.bind_context(cc.egui_ctx.clone()); // for request_repaint
        let app = MyApp::new(cc);
        Ok(Box::new(egui_agent::wrap(app, control)))   // <-- single wrap
    }))
}
```

`egui_agent::wrap(app, control)` returns an `Agent<MyApp>` that implements `eframe::App`. Its `update` drains the queue, calls `inner.update`, and captures snapshots. The user's `update` is untouched.

**Boilerplate reduction (progressive):**
- v1: user hand-writes `snapshot`/`dispatch`.
- v2: `#[derive(Snapshot)]` for state structs, and registered-widget helpers like `egui_agent::button(ui, "submit", "Submit")` that draw the widget *and* register a semantic id `"submit"` in a per-frame registry, so `dispatch` can be auto-generated for those widgets.
- v2: `Snapshot::from_accesskit(cx)` makes `snapshot` a one-liner by reading the tree egui already builds (§10).

## 8. Core protocol

Small, semantic, serde-serializable. Keep the surface tight.

```rust
#[derive(Serialize, Deserialize)]
pub enum Command {
    /// Full structured view of the current UI/app state.
    Snapshot,
    /// Query a subset by selector (cheaper than full snapshot).
    Query { target: Selector },
    /// Invoke a named action, optionally on a target widget.
    Invoke { target: Option<Selector>, action: String, args: Option<Value> },
    /// Set a named/targeted value (text field, slider, toggle...).
    SetValue { target: Selector, value: Value },
    /// Block until a predicate over the snapshot holds, or timeout.
    WaitFor { predicate: Predicate, timeout_ms: u64 },
    /// Optional fallback: PNG of the current frame.
    Screenshot,
    /// Escape hatch for app-specific verbs.
    Custom { name: String, args: Value },
}

#[derive(Serialize, Deserialize)]
pub enum Selector {
    /// Developer-assigned stable semantic id (preferred).
    Id(String),
    /// Match by accessible label / name.
    Label(String),
    /// Match by role, optionally narrowed by name.
    Role { role: String, name: Option<String> },
    /// AccessKit node id (ephemeral unless derived from a stable egui Id — see §12).
    Node(u64),
}
```

`Response` is `Result<ResponseBody, AgentError>`, where `ResponseBody` carries a `Snapshot`, a queried node, an `ActionResult`, or an image. Errors are structured (`TargetNotFound`, `AmbiguousSelector`, `ActionRejected`, `Timeout`, `Unauthorized`).

`Snapshot` is a flat or tree list of nodes: `{ id, role, label, value, enabled, focused, actions: [..], rect? }`. This is the "structured text instead of a screenshot" payload the agent reasons over.

## 9. Transports

Feature-gated, all binding to the local machine only:

- `uds` — Unix domain socket, newline-delimited JSON. Simplest; default on Linux/macOS.
- `ws` — WebSocket (`tokio` + `tokio-tungstenite`) for browser-based or remote-dev agents.
- `stdio` — line protocol over stdin/stdout, convenient for spawning under an MCP host.
- `pipe` — named pipe on Windows.

Each transport is a small adapter implementing `trait Transport { async fn serve(self, hub: CommandHub) }`, where `CommandHub` is the enqueue-and-await handle from the core. Keep `tokio` behind the `ws`/`mcp` features so the default (`uds`) build stays lightweight and runtime-agnostic.

## 10. Introspection strategy

**v1 — app-provided snapshot.** The app's `snapshot()` returns whatever structured state it wants the agent to see. Full control, zero magic, works immediately.

**v2 — AccessKit auto-introspection.** eframe already builds an AccessKit tree of every labeled widget with role/value/actions/node-id. The crate captures the current tree each frame and offers `Snapshot::from_accesskit(cx)`, reducing `snapshot()` to a one-liner and giving generic coverage without per-widget code.

> **Version-binding to confirm (research task):** the exact API for reading egui's live AccessKit tree from inside `update` depends on the pinned egui version. Candidate paths: intercepting the accesskit `TreeUpdate` in the frame's `PlatformOutput`, or a dedicated `Context` accessor. Pin egui, confirm the accessor, and cache the last `TreeUpdate` at end-of-frame. Do **not** assume a method name; verify against the pinned version.

The two strategies compose: apps can auto-populate from AccessKit and then augment with domain state the a11y tree doesn't capture.

## 11. Actuation strategy

**v1 — semantic dispatch (primary, always works).** `Invoke`/`SetValue` are routed through `AgentApp::dispatch` to the same methods your widgets call. Deterministic, version-proof, no egui-internals spelunking. The one discipline it asks of the app: factor button/field handlers into named methods that both the UI and `dispatch` call. The registered-widget helpers in §7 make this automatic for common widgets.

**v2 — AccessKit `ActionRequest` injection (generic, optional).** egui already *implements the receiving side* of programmatic actuation: it handles AccessKit actions like `Click`, `Focus`, `SetValue`, `Increment`, `Decrement`, and `SetTextSelection` — that's how a screen reader activates a widget. If we can submit an `ActionRequest` into egui-winit's accesskit action channel *in-process*, we get generic "click/type any labeled widget" with no per-app code.

> **Version-binding to confirm (research task):** whether the pinned egui-winit exposes an in-process hook to submit an `ActionRequest` (the platform adapter normally receives these from the OS AT). If no public hook exists, options are: (a) a small upstream patch/PR, (b) synthesizing `egui::Event`s (`PointerButton`, `Text`, `Key`) into the next frame's `RawInput` if a public push exists, or (c) staying on v1 semantic dispatch. Fall back gracefully; keep v1 as the guaranteed path.

Because immediate mode means a target must be resolvable each frame, actuation resolves `Selector` → target via the registry (semantic id) or the AccessKit tree (label/role/node) captured last frame.

## 12. Selector stability

AccessKit `NodeId`s in egui derive from egui `Id`s (hashes of a widget's id-source), so they are **stable frame-to-frame only as long as the widget's id-source is stable** (e.g. constant labels or explicit `ui.push_id`). Guidance baked into the docs:

- Prefer `Selector::Id` (developer-assigned semantic ids) for anything the agent will target repeatedly.
- `Selector::Label` / `Selector::Role` are good for discovery and one-shot actions.
- Treat `Selector::Node` as ephemeral within a session unless the app guarantees stable id-sources.

## 13. Security

- **Compiled out by default.** Gate the whole crate behind an `agent` cargo feature so release builds can exclude it entirely.
- **Opt-in at runtime** via `Config::enabled` (env-var driven in the example) so even a feature-enabled build is inert unless asked.
- **Local only.** UDS with filesystem perms, or WS/TCP bound to `127.0.0.1`. Never bind a public interface.
- **Auth token** on connect for socket/WS transports.
- **Read-only mode**: a config flag that permits `Snapshot`/`Query`/`Screenshot` but rejects `Invoke`/`SetValue`, for safer observation sessions.
- **Audit log** of every command (behind a flag) for debugging and trust.

## 14. Crate & workspace layout

```
egui_agent/                # core: protocol, AgentApp, Agent<A> wrapper, queue, lifecycle
  features:
    default = ["uds"]
    uds, ws, stdio, pipe   # transports
    accesskit-introspection
    screenshot
egui_agent_mcp/            # optional separate crate: MCP server → Command mapping (keeps tokio/mcp out of core)
egui_agent_macros/         # optional: #[derive(Snapshot)], registered-widget helpers
```

Keeping MCP in its own crate means apps that only want socket/test control don't pull the MCP/tokio dependency tree.

## 15. MCP adapter (`egui_agent_mcp`)

A thin MCP server that exposes core commands as tools:

- `snapshot()` → `Command::Snapshot`
- `query(selector)` → `Command::Query`
- `invoke(target?, action, args?)` → `Command::Invoke`
- `set_value(target, value)` → `Command::SetValue`
- `wait_for(predicate, timeout)` → `Command::WaitFor`
- `screenshot()` → `Command::Screenshot` (if feature enabled)

The MCP server connects to a running app over the chosen transport (or spawns it), and each tool call is a `Command` round-trip. Tool schemas should describe the `Selector` shapes and the app's registered semantic ids where discoverable (the server can call `snapshot` on connect to advertise available targets).

## 16. Testing strategy (a reuse bonus worth calling out)

Because the core is transport-agnostic and commands map to app logic, the **same command dispatch is drivable in-process by `egui_kittest`**. Provide a `TestHarness` helper that feeds `Command`s directly into an `Agent<A>` running under a kittest `Harness`, asserting on resulting `Snapshot`s — no socket, fully deterministic, CI-friendly. Every app that adopts `egui_agent` therefore gets both an agent control surface *and* an integration-test harness from a single integration. Lean on this to test the crate itself, too.

## 17. Milestones

- **M0 — Core, no transport.** `Command`/`Response`, `AgentApp`, `Agent<A>` wrapper, queue + lifecycle state machine. Drive it via `egui_kittest` in-process. *Proves the model end-to-end with no I/O.*
- **M1 — UDS transport.** Newline-JSON over a Unix socket, auth token, `request_repaint` pump, `Snapshot`/`Query`/`Invoke`/`SetValue`. First real out-of-process control.
- **M2 — MCP adapter.** `egui_agent_mcp` mapping tools → commands; connect Claude/agent and drive a sample app.
- **M3 — AccessKit auto-introspection.** `Snapshot::from_accesskit`; resolve the version-binding in §10.
- **M4 — Generic actuation + ergonomics.** Registered-widget helpers, `#[derive(Snapshot)]`, and (if feasible) AccessKit `ActionRequest` injection per §11.
- **M5 — Streaming & hardening.** Optional event stream (domain events / toasts / navigation the agent can observe), `WaitFor`, settle-policy hardening, `ws`/`stdio`/Windows `pipe` transports, read-only mode, audit log.

Ship M0–M2 first; that already delivers reliable, focus-free, screenshot-free agent control for any app willing to write `snapshot`/`dispatch`. M3–M4 drive per-app boilerplate toward zero.

## 18. Open questions / research tasks for the build agent

1. **egui version pin.** Choose and pin an egui/eframe version; several bindings below depend on it.
2. **AccessKit tree read path** (§10) — confirm the exact accessor in the pinned version; cache last `TreeUpdate`.
3. **In-process `ActionRequest` submission** (§11) — determine if a public hook exists; else scope M4 to registered-widget dispatch and file an upstream request.
4. **Sync/async channel bridge** — pick the queue primitive (`flume` is async+sync friendly) so `uds`/`ws`/`mcp` (possibly tokio) and the sync UI thread interoperate cleanly.
5. **Settle semantics** — validate the `NextFrame` vs `Idle{max_frames}` policy against animated/async-loading UIs so post-action snapshots aren't stale.
6. **Selector ergonomics** — decide whether to auto-assign semantic ids from stable egui `Id`s or require explicit registration; document the stability contract (§12).
7. **Multi-window / viewports** — decide whether the snapshot spans egui viewports and how selectors disambiguate across them.

---

*Recommended build order: M0 in-process (kittest-driven) → M1 UDS → M2 MCP, then layer introspection/actuation ergonomics. The core protocol and `AgentApp` trait are the load-bearing design decisions; get those reviewed before building transports.*
