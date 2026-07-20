# Changelog

All notable changes to this workspace (`egui_agent`, `egui_agent_mcp`,
`egui_agent_macros`) are documented here. Versions are shared across the
workspace. Format follows [Keep a Changelog](https://keepachangelog.com);
versioning follows [SemVer](https://semver.org) (0.x: minor bumps may break).

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
