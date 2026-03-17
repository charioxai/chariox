# Contributing Guide

## Status

Working contributor conventions for Arroba v1.

## 1. Purpose

This document defines how to contribute implementation code and tests consistently across the repository.

Use this for:

- coding style expectations
- testing expectations
- PR/change hygiene

Use `docs/spec-v1.md`, `docs/ARCHITECTURE.md`, and `docs/PROTOCOL.md` for product/runtime behavior requirements.

## 2. Code Style and Structure

## 2.1 General

- Prefer small, focused changes.
- Keep feature behavior aligned with `spec-v1` and protocol docs.
- Avoid mixing unrelated refactors with behavior changes.
- Use clear naming that reflects domain terms (`session`, `provider run`, `attachment`, `capability`, `control op`).

## 2.2 Rust (Daemon Baseline)

- Follow `rustfmt` defaults.
- Treat Clippy warnings as action items where practical.
- Prefer explicit error propagation with contextual errors over silent fallback.
- Keep protocol/domain structs near their owning modules.

## 2.3 TypeScript/Frontend

- Prefer strict typing and avoid `any` unless unavoidable and documented.
- Keep UI behavior deterministic for terminal and overlay flows.
- Isolate adapter/transport logic from rendering components.

## 2.4 Protocol and Contracts

- Additive changes are preferred for v1 compatibility.
- When introducing a new event/field, document it in `docs/PROTOCOL.md` in the same change.
- Keep canonical control operations minimal and version-aware.

## 3. Testing Expectations

## 3.1 Required by Change Type

- **Behavior/runtime changes:** add or update automated tests.
- **Protocol changes:** add/adjust contract tests and protocol doc examples.
- **Bug fixes:** include a regression test when feasible.
- **Docs-only changes:** no mandatory runtime tests, but ensure docs stay internally consistent.

## 3.2 Test Layers

- Unit tests for pure/domain logic.
- Integration tests for daemon-session-provider interactions.
- Workflow validation tests for coordinator behavior, structured handoffs, graph/topology rules, and worktree isolation once those surfaces land.
- Conformance tests for terminal lane behavior across client types.
- End-to-end tests for critical workflows (attach, provider switch, memory update, compact flow).

## 3.3 Cross-Platform Terminal Conformance

For client implementations across web/mobile/desktop/CLI:

- validate prompt/config interactions plus `terminal.output` and `terminal.resize` behavior
- compare against xterm.js reference expectations where applicable
- preserve control-sequence fidelity and resize semantics

## 3.4 Baseline Commands

Use these commands from the repository root for the current repository baseline:

```bash
pnpm install
pnpm lint
pnpm build
pnpm test
pnpm smoke:daemon
cargo test --manifest-path apps/daemon/Cargo.toml
```

Recommended Rust quality checks:

```bash
cargo fmt --manifest-path apps/daemon/Cargo.toml --check
cargo clippy --manifest-path apps/daemon/Cargo.toml --all-targets --all-features -- -D warnings
```

## 4. Pull Request Expectations

- Explain **why** and **what** changed.
- List tests/checks run and their outcomes.
- Call out compatibility implications (especially protocol/control-lane changes).
- Update affected docs when behavior contracts change.

## 5. Recommended File Locations for Guidance

- Product/runtime behavior: `docs/spec-v1.md`
- Architecture and implementation baseline: `docs/ARCHITECTURE.md`
- Message and control contracts: `docs/PROTOCOL.md`
- Delivery planning: `docs/ROADMAP.md`
- Contributor style/testing workflow: `docs/CONTRIBUTING.md`

## 6. Lightweight Task Tracking (No PM Tool Required)

Use the repo-native tracker under `docs/ops/`:

- task board: `docs/ops/TASKS.md`
- chronological handoff notes: `docs/ops/PROGRESS_LOG.md`

Workflow:

- create tasks with stable IDs (example: `M0-001`)
- move tasks across `Backlog`, `In Progress`, `Done`
- when marking done, record a short note + commit hash
- append significant milestones/decisions in `PROGRESS_LOG.md` for future contributors/agents
