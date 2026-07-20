# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What this is

A greenfield Rust project building `egui_agent`: a reusable crate that lets an external agent (MCP, socket, or in-process tests) drive and introspect any egui/eframe app through semantic commands and structured snapshots — not screenshots or pixel coordinates.

**`docs/egui_agent_design.md` is the source of truth.** Read it before doing any design or implementation work. It specifies the architecture, protocol, milestones, and open research questions. Do not deviate from it silently; if implementation reality contradicts the doc, surface the conflict.

## Current state

Pre-implementation. No Cargo workspace exists yet; the repo is not yet a git repository. First implementation steps are `git init`, workspace scaffolding, and milestone M0.

## Commands

Standard Cargo workspace (once scaffolded):

- `cargo build --workspace` / `cargo check --workspace`
- `cargo test --workspace` — M0 onward is tested headlessly via `egui_kittest`; no display server needed
- `cargo test -p egui_agent <test_name>` — run a single test
- `cargo clippy --workspace --all-features` and `cargo fmt` before committing
- Feature-gated code needs explicit flags to compile-check: `cargo check -p egui_agent --features uds` etc.

## Architecture (from the design doc)

Three concentric layers, each independently testable:

1. **Core** (`egui_agent`, transport-agnostic): serde `Command`/`Response` protocol, command queue with oneshot replies, `AgentApp` trait (the only per-app surface: `snapshot()` + `dispatch()`), per-command lifecycle state machine (`Queued → Executed → Settled → Responded`).
2. **egui integration**: `Agent<A>` wrapper implementing `eframe::App` — drains the command queue on the UI thread inside `update()` before running the inner app, pumps `ctx.request_repaint()`, captures snapshots.
3. **Adapters** (feature-gated): transports (`uds` default, `ws`, `stdio`, Windows `pipe`) and a separate `egui_agent_mcp` crate so core stays free of tokio/MCP deps. Planned `egui_agent_macros` crate for derives/widget helpers.

Load-bearing constraints that shape everything (design doc §4):

- **Immediate mode**: no retained widgets to call `.click()` on. v1 actuation is semantic dispatch — commands route to the same named methods the app's buttons call.
- **State lives on the UI thread**: transport threads must marshal commands onto the UI thread via the queue; they never touch app state directly.
- **Reactive repaint**: eframe sleeps until input. Every enqueue must call `request_repaint()` on a cloned `egui::Context` (it's `Send + Sync`). Never switch to continuous rendering as a workaround.
- **Settle semantics**: commands whose result depends on post-command UI resolve over two frames (execute frame N, snapshot frame N+1), governed by a configurable settle policy.

Planned workspace layout (design doc §14): `egui_agent/` (core + transports behind features), `egui_agent_mcp/`, `egui_agent_macros/`.

## Build order

Milestones (design doc §17): **M0** core with no transport, driven entirely by `egui_kittest` → **M1** UDS transport → **M2** MCP adapter → M3–M5 (AccessKit introspection, generic actuation, hardening). Ship M0–M2 first. The core protocol and `AgentApp` trait are the load-bearing decisions — get those reviewed before building transports.

## Cautions

- Several egui/AccessKit API bindings are version-dependent and flagged in the doc as "research tasks" (§10, §11, §18): the AccessKit tree read path and in-process `ActionRequest` submission. **Pin an egui version and verify against it — do not assume API names from memory.**
- Security posture is part of the design, not an afterthought: whole crate behind an `agent` cargo feature, runtime opt-in via config, localhost/UDS only, optional auth token and read-only mode (§13).
- The same command layer must remain drivable in-process by `egui_kittest` (§16) — every feature should keep the transport-free test path working; use it to test the crate itself.
