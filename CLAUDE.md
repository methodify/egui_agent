# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What this is

`egui_agent`: a reusable crate that lets an external agent (MCP, socket, or in-process tests) drive and introspect any egui/eframe app through semantic commands and structured snapshots — not screenshots or pixel coordinates.

**`docs/egui_agent_design.md` is the design source of truth**; `README.md` describes the shipped surface. If implementation reality contradicts the design doc, surface the conflict — don't deviate silently.

## Current state

All milestones M0–M5 implemented and tested (50+ tests, headless via `egui_kittest`). Pinned to egui/eframe/egui_kittest **0.36**, accesskit **0.24** (egui 0.36's pin; unchanged from 0.35). The design doc's open research questions (§10/§11/§18) are resolved — see "Resolved version bindings" below.

## Commands

- `cargo build --workspace` / `cargo check --workspace`
- `cargo test --workspace` — everything runs headlessly; no display server needed. Transport tests use real sockets in `/tmp`.
- `cargo test -p egui_agent --test accesskit` — run one integration-test file; add a name filter for a single test
- `cargo clippy --workspace --all-features --all-targets` and `cargo fmt --all` before committing (both must be clean)
- Feature-gated checks: `cargo check -p egui_agent --no-default-features`, `--all-features`
- Live demo: `DEMO_AGENT=1 cargo run -p demo_app`, then drive it with
  `cargo run -p egui_agent_mcp --bin egui-agent-cli -- --socket /tmp/egui_agent_demo.sock snapshot`

## Architecture

Three layers (design doc §5), all in `egui_agent/src/`:

1. **Core (transport-agnostic)**: `protocol.rs` (serde `Command`/`Response`/`Selector`/`Predicate` + wire types), `hub.rs` (`CommandHub`: flume queue + bounded-1 reply channels bridging transport threads → UI thread), `app.rs` (`AgentApp` trait — the only per-app surface), `config.rs`, `predicate.rs`, `events.rs` (domain-event ring buffer).
2. **egui integration**: `agent.rs` — `Agent<A>` runs the per-command state machine (`Queued → Executed(frame N) → Settled(frame N+1) → Responded`) in `pre_frame`/`post_frame` around the app's UI. `registry.rs` + `widgets.rs` — per-frame registry powering auto-snapshot/auto-actuation of registered widgets. `introspection.rs` — AccessKit tree capture + ActionRequest injection via an `egui::Plugin`. `test_harness.rs` — kittest-backed `TestHarness`.
3. **Adapters**: `transport/` (uds/ws/stdio/pipe, all blocking std threads — no tokio anywhere), `client.rs` (blocking client used by the MCP crate), `egui_agent_mcp` (hand-rolled MCP stdio JSON-RPC server + `egui-agent-cli`), `egui_agent_macros` (`#[derive(Snapshot)]`).

**Actuation resolves in tiers** (`Agent::dispatch_action` → `registry_fallback`): app `dispatch` first; `Unhandled` falls through to registry actuation (semantic ids, one hand-off frame for the widget helper to consume); then AccessKit `ActionRequest` injection for plain egui widgets. Keep this ordering — apps override behavior by handling actions in `dispatch`.

Load-bearing constraints (design doc §4): commands only execute on the UI thread inside the frame loop; every enqueue calls `request_repaint()` on a cloned context (never switch to continuous rendering); actuating commands settle over an extra frame so snapshots aren't stale.

## Resolved version bindings (egui 0.36 — re-verify on every egui upgrade)

All of these were re-verified against 0.36.2 source when the pin moved from 0.35 (2026-09-16); none changed.

- **eframe (since 0.35) splits `App::update` into `logic()` + `ui()`.** `Agent<A>` maps `pre_frame` onto `logic` and `post_frame` onto the end of `ui`. `App::on_exit`'s signature changes with eframe's `glow` feature — mirrored by the `egui_agent/glow` passthrough feature.
- **AccessKit read path**: an `egui::Plugin`'s `output_hook` clones `FullOutput::platform_output::accesskit_update` into a cache (`introspection.rs`); requires `ctx.enable_accesskit()`.
- **Generic actuation**: the same plugin's `input_hook` injects `egui::Event::AccessKitActionRequest` events — the mechanism kittest and screen readers use. egui 0.36 widgets handle `Click` (buttons/checkboxes), `SetValue` (sliders, NumericValue only). **TextEdit does not handle `SetValue`** — text is set by emulating a user: `Focus` request, then select-all + backspace + `Event::Text` batches injected over successive passes.
- A11y label quirk: `Role::Label` nodes carry their text in the accesskit `value` property; `introspection::convert` mirrors it into `label` so `Selector::Label` matches.
- **Multiple viewports (0.3.1+)**: egui runs every plugin hook once per viewport pass, and immediate viewports run nested inside the parent's pass, so `introspection.rs` tracks the pass viewport as a stack. The agent drives the root viewport only: event injection waits for a root pass, only the root's a11y tree is cached, and `screenshot.rs` stashes `Event::Screenshot` from any viewport's input because eframe routes captures to whichever viewport renders next. Keep new hooks viewport-aware.

## Cautions

- Wire-format invariant (0.2.0+): AccessKit node ids serialize as **strings** in JSON (`node_id`, `{"node": "..."}` selectors) because they exceed JSON's f64-safe integer range — don't regress them to numbers. Protocol changes require a version bump and a `CHANGELOG.md` entry.

- Security posture is part of the design (§13): off unless `Config::enabled`, local-only transports, optional auth token, `read_only` mode, audit log. Don't weaken defaults.
- Every feature must keep the transport-free kittest path working (§16) — test new commands through `TestHarness`, not just over sockets.
- The MCP server is deliberately hand-rolled (line-delimited JSON-RPC, no tokio); keep the core free of async runtimes.
- `egui_agent` has a self-dev-dependency enabling test features so plain `cargo test --workspace` runs the full suite — don't remove it.
