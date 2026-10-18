# Changelog

All notable changes to this workspace (`egui_agent`, `egui_agent_mcp`,
`egui_agent_macros`) are documented here. Versions are shared across the
workspace. Format follows [Keep a Changelog](https://keepachangelog.com);
versioning follows [SemVer](https://semver.org) (0.x: minor bumps may break).

## [Unreleased]

### Fixed

- **macOS build of the `pipe` (and `uds`) client** failed with E0275 (overflow evaluating
  `&Retained<_>: Read`). `Client::from_stream`'s `for<'a> &'a S: Read + Write` bound was checked
  before `S` was inferred, and on macOS objc2 (pulled in by eframe/winit) has a blanket
  `impl Read for &Retained<T>` that sent the solver into infinite recursion. `connect_pipe` and
  `connect_uds` now name the stream type explicitly. No API change.

## [0.3.1] - 2026-10-18

### Fixed

- **Apps with more than one native window** (deferred or immediate viewports).
  egui runs every plugin hook once per viewport pass, and eframe hands every
  captured screenshot to whichever viewport renders next. With a second window
  open, three things went wrong intermittently:
  - `screenshot` failed with "backend did not deliver a screenshot" when the
    capture landed in the other window's input. A small plugin now catches
    screenshot events in any viewport's input and `poll` finds the root's.
  - AccessKit actuation (plain-widget clicks, typing) could be injected into
    the other window's pass and lost. Queued events now wait for a root pass.
  - The AccessKit snapshot cache was replaced by whichever viewport finished
    last, often an empty secondary window. Only the root viewport's tree is
    kept now. Pass viewports are tracked as a stack because immediate
    viewports run nested inside their parent's pass.

  No API or wire-protocol change.

## [0.3.0] - 2026-09-16

### Changed (breaking for consumers)

- **Pinned to egui, eframe, and egui_kittest 0.36** (from 0.35). Apps that
  depend on this crate must move to egui/eframe 0.36 in the same change:
  egui types cross the API boundary, so mixed versions don't compile.
  accesskit stays at 0.24.1, which egui 0.36 also pins.
- Every version-specific binding was re-verified against the 0.36.2 source and
  none changed: `App::logic`/`ui` and the `glow`-dependent `on_exit`
  signature, the `Plugin` `input_hook`/`output_hook` capture and injection
  paths, `PlatformOutput::accesskit_update`, the `AccessKitActionRequest` and
  `Screenshot` events, the kittest `Harness` API, and the accesskit Click
  path that drives plain widgets. TextEdit still handles only
  `SetTextSelection`, so the emulated-typing path for string `set_value`
  stays.

No source changes were needed. The wire protocol is unchanged, so 0.2.x tools
(`egui-agent-mcp`, `egui-agent-cli`) keep working against 0.3.0 apps.

## [0.2.2] - 2026-07-20

### Added

- Regression tests pinning down CollapsingHeader actuation after a field
  report that headers "ignore" accesskit clicks. Not reproducible on 0.2.1:
  egui 0.35 headers register a `button` a11y node and its interact path
  honors accesskit Click requests — expand/collapse works by label and by
  node id, including nested in a ScrollArea. The report most likely predates
  the 0.2.0 node-id fixes, when corrupted node ids silently no-oped.
  If it recurs, capture the failing selector + `query` output.
- USAGE.md: guidance on collapsed/conditional UI (content nodes don't exist
  until drawn — click the header, `wait_for` the content, then interact; or
  prefer app-level dispatch verbs for scripted flows).

## [0.2.1] - 2026-07-20

### Fixed

- **`wait_for` no longer stalls on an idle or hidden app.** The predicate is
  re-evaluated once per frame, but an idle app produces no frames: repaint
  scheduling only ran at the end of the `ui` pass, which eframe skips entirely
  for hidden/minimized windows, so a pending `wait_for` could stop advancing.
  Two-sided fix: the transport thread now re-pumps `request_repaint()` every
  `poll_interval` while a `wait_for` is outstanding (works even when hidden —
  eframe still runs `logic` on repaint requests), and the agent also schedules
  pending-command repaints from the pre-pass, not just the post-pass.
- **Long waits are no longer killed by the liveness guard.** The transport
  bounded every command by `Config::request_timeout` (default 10s), so e.g. a
  300s `wait_for` died at 10s. `wait_for`'s transport deadline is now
  `timeout_ms + request_timeout`; other commands keep the flat guard.

## [0.2.0] - 2026-07-20

First-use feedback release. **Contains a breaking wire-format change** — keep
app and tools (`egui-agent-mcp` / `egui-agent-cli`) on matching versions.

### Changed (breaking)

- **Node ids are strings on the wire.** AccessKit node ids use the full u64
  range, which silently corrupts through any JSON stack that parses numbers as
  f64 (anything above 2^53 — notably JavaScript-based MCP hosts). Snapshot
  `node_id` fields and `{"node": ...}` selectors now serialize as strings
  (`{"node": "12345"}`). Plain JSON integers are still accepted on input when
  exact; precision-lossy floats are rejected with an error instead of being
  truncated.

### Added

- **`quiet` flag** on `invoke`, `set_value`, and `wait_for` (protocol field,
  MCP tool argument, and `--quiet` on `egui-agent-cli`): suppresses the echoed
  post-action snapshot, which is heavy for large UIs. Results and errors are
  unchanged; actions still settle before responding.
- **Unknown node ids now error.** Actuating `{"node": ...}` for a node absent
  from the a11y tree returns `target_not_found` instead of silently no-oping
  (egui drops ActionRequests for unknown targets, so stale/corrupted ids
  previously looked like success).

### Fixed

- MCP tool schemas now declare explicit types for `args` and `value`
  (previously untyped, which led at least one host to stringify structured
  arguments). Additionally, the server defensively parses JSON-encoded-string
  arguments for `target`, `predicate`, and invoke/custom `args` from
  noncompliant hosts. `set_value.value` is never de-stringified — a string is
  a first-class value for text fields; send numbers/booleans as native JSON.

## [0.1.0] - 2026-07-20

Initial implementation of the full design (`docs/egui_agent_design.md`,
milestones M0–M5), against egui/eframe/egui_kittest 0.35 and accesskit 0.24.

- Core: transport-agnostic `Command`/`Response` protocol, UI-thread command
  queue with settle semantics (`Queued → Executed → Settled → Responded`),
  `AgentApp` trait with registry-backed defaults, `Agent<A>` eframe wrapper,
  `WaitFor` predicates, domain-event stream, read-only mode, auth, audit log.
- Actuation tiers: semantic dispatch → registered-widget helpers → AccessKit
  `ActionRequest` injection for plain egui widgets (zero app code).
- Introspection: hand-written snapshots, registry auto-snapshots, and
  `Snapshot::from_accesskit` via egui's Plugin API.
- Transports: UDS (default), WebSocket (127.0.0.1), stdio, and cross-platform
  local socket / Windows named pipe (`pipe` feature) — all plain std threads.
- `egui_agent_mcp`: MCP stdio server + `egui-agent-cli` driver, both speaking
  `--socket` (UDS) and `--pipe` (Windows named pipe).
- `egui_agent_macros`: `#[derive(Snapshot)]`.
- In-process `TestHarness` over `egui_kittest`; Windows cross-compilation
  verified (`x86_64-pc-windows-msvc`).
