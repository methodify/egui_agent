# egui_agent

A reusable **agent control channel** for [egui](https://github.com/emilk/egui)/eframe apps: let an external agent (MCP host, socket client, or in-process test) drive and introspect your app through **semantic commands and structured snapshots** — not screenshots and pixel coordinates.

Integration is one trait + one wrap call. Everything else — transport, queuing, repaint pumping, MCP exposure — lives in the crate.

> **Not on crates.io yet** — consume it straight from GitHub. **[docs/USAGE.md](docs/USAGE.md)** walks through adding the git dependency, installing the MCP server/CLI with `cargo install --git`, connecting an MCP host, and using the test harness.

```rust
use egui_agent::{AgentApp, Action, ActionResult, Snapshot, SnapshotCtx};

#[derive(Default)]
struct MyApp { count: u32 }

impl AgentApp for MyApp {} // defaults cover registered widgets + AccessKit

impl eframe::App for MyApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        // Helper draws a normal button AND makes it agent-visible/drivable:
        if egui_agent::widgets::button(ui, "increment", "+1").clicked() {
            self.count += 1;
        }
    }
}

fn main() -> eframe::Result {
    let control = egui_agent::Control::spawn(egui_agent::Config {
        transport: egui_agent::Transport::UnixSocket("/tmp/myapp.agent.sock".into()),
        auth: egui_agent::Auth::from_env("MYAPP_AGENT_TOKEN"),
        enabled: std::env::var("MYAPP_AGENT").is_ok(), // inert unless opted in
        ..Default::default()
    });
    eframe::run_native("MyApp", Default::default(), Box::new(move |cc| {
        control.bind_context(cc.egui_ctx.clone());
        Ok(Box::new(egui_agent::wrap(MyApp::default(), control)))
    }))
}
```

Then, from anywhere:

```console
$ egui-agent-cli --socket /tmp/myapp.agent.sock snapshot
$ egui-agent-cli --socket /tmp/myapp.agent.sock invoke click --target '{"id":"increment"}'
```

Or connect an MCP host (e.g. Claude) to `egui-agent-mcp --socket /tmp/myapp.agent.sock` and drive the app with the `snapshot` / `query` / `invoke` / `set_value` / `wait_for` / `screenshot` / `custom` tools. On Windows, both tools take `--pipe <name>` to reach an app serving `Transport::Pipe`.

## How it works

- **Commands** (`Snapshot`, `Query`, `Invoke`, `SetValue`, `WaitFor`, `Screenshot`, `Custom`) arrive on a transport thread, are queued, and executed **on the UI thread** inside the frame loop — egui state never crosses threads. Every enqueue wakes the (reactive) UI via `request_repaint()`.
- **Per-command lifecycle**: `Queued → Executed(frame N) → Settled(frame N+1) → Responded`, so post-action snapshots reflect the settled UI, never a stale frame. Settle policy is configurable (`NextFrame` or `Idle { max_frames }`).
- **Actuation tiers** (first match wins):
  1. **Semantic dispatch** — `AgentApp::dispatch` routes named actions to the same methods your buttons call.
  2. **Registered widgets** — `egui_agent::widgets::{button, checkbox, text_edit_singleline, slider, …}` helpers auto-snapshot and auto-actuate under stable semantic ids.
  3. **AccessKit injection** (`accesskit-introspection` feature) — plain egui widgets are clicked/toggled/typed-into by injecting `ActionRequest`s into the input stream, the same mechanism a screen reader uses. Zero app code.
- **Introspection**: hand-written snapshots, registry auto-snapshots, and `Snapshot::from_accesskit` (the a11y tree egui already builds) compose freely. Domain events (`egui_agent::events::emit`) surface toasts/navigation to the agent.

## Workspace

| Crate | What |
|---|---|
| `egui_agent` | Core: protocol, queue, `AgentApp`, `Agent<A>` wrapper, transports (feature-gated), test harness |
| `egui_agent_mcp` | MCP stdio server (`egui-agent-mcp`) + CLI/REPL driver (`egui-agent-cli`) |
| `egui_agent_macros` | `#[derive(Snapshot)]` for app state structs |
| `examples/demo_app` | Task-list demo wired for UDS control |

### Features (`egui_agent`)

`default = ["eframe", "uds"]` · transports: `uds`, `ws`, `stdio`, `pipe` (Windows named pipe / local socket) · `accesskit-introspection` · `screenshot` · `test-harness` (kittest-backed in-process harness) · `glow` (match eframe's glow feature — it changes `App::on_exit`'s signature)

## Security posture

Off unless opted in (`Config::enabled`), local-only transports (UDS perms / 127.0.0.1), optional auth token, `read_only` mode (observe but never actuate), optional audit log of every command. Gate the crate behind a cargo feature in your app to compile it out of release builds entirely.

## Testing your app with it

The same command layer is drivable in-process via `egui_kittest` — no socket, fully deterministic:

```rust
let mut h = egui_agent::TestHarness::new(MyApp::default(), |app, ui| { /* app ui */ });
let snap = h.send(egui_agent::Command::Snapshot).unwrap();
```

Every app that adopts `egui_agent` gets an integration-test harness for free.

## Version pinning

Built and verified against **egui/eframe/egui_kittest 0.35** and **accesskit 0.24** (the version egui 0.35 pins). The AccessKit capture/injection paths use egui's `Plugin` API (`input_hook`/`output_hook`) — see `docs/egui_agent_design.md` §10/§11 for the design rationale.
