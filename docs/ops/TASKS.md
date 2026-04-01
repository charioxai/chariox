# Arroba Task Board

A lightweight, repo-native task board so contributors and future agents can continue work without external PM tooling.

## How to use

- Keep tasks small and implementation-oriented.
- Move tasks between `Backlog` -> `In Progress` -> `Done`.
- When a task is completed, add a short completion note and link commit hash.
- Use IDs (`M0-001`, `M1-003`, etc.) so references stay stable.
- Do not delete completed tasks; keep historical context.

## In Progress

- [ ] **M3-002** Close the OpenCode-first capability and local-runtime cycle
  - Note: Finish the remaining shell/file/git/screenshot/transfer/schedule-facing productization and local slash-command UX without adding another provider family yet.
- [ ] **M4-003** Stabilize the OpenCode-backed multi-agent runtime path
  - Note: The core daemon integration suite is green again, but the OpenCode-first path still needs further stabilization around transcript/pane UX and deeper live runtime drills.
  - Progress: queued prompts now stay bound to their target agent run, and queued work can advance onto another healthy agent run after the active run exits unexpectedly.
  - Future transport drills still pending:
    - deeper live replay/catch-up validation during active streaming output
    - long-idle heartbeat/liveness validation on the kernel WebSocket path
- [ ] **M4-004** Expand the TypeScript CLI split-pane model beyond the current first slice
  - Note: The daemon/runtime data model can handle more agents than the current visible split-pane surface; the current UI still centers on the primary transcript plus up to two auxiliary panes.
- [ ] **M4-005** Add multi-machine session ownership and resume behavior on the OpenCode-first path
  - Note: Close machine reassignment/resume semantics before moving to multi-platform clients or more providers.

## Backlog

- [ ] **M4-002** Add daemon-scheduled multi-agent workflow runtime
  - Note: Build circular-first workflow scheduling and structured handoffs on top of the same top-level session-agent runtime.
- [ ] **M5-001** Polish the TypeScript CLI as the reference Arroba client
  - Note: Finish the local UX, pane behavior, and command flow on the OpenCode-first path before adding more client surfaces.
- [ ] **M6-001** Add multi-platform clients on the same daemon/protocol model
  - Note: Web comes first, then iOS/Android, all reusing the semantics proven by the polished CLI.
- [ ] **M7-001** Add Claude Code, Codex, and generalized provider-adapter/protocol support
  - Note: Multi-provider expansion is intentionally the last major breadth step after OpenCode, harnessing, multi-machine behavior, and multi-platform clients are settled.

## Done

- [x] **M3-CLI-001** Improve TypeScript CLI transcript rendering with markdown-aware output and syntax-highlighted fenced code blocks
  - Note: Added markdown-aware assistant/reasoning rendering, parser bootstrap, fence-language normalization, and syntax-highlighted fenced code blocks without depending on LSP semantic coloring.
  - Commit: _pending next commit_
- [x] **M3-001** Implement persistent session management and no-session CLI state
  - Note: Added explicit session deletion, resumable detached sessions, unattached landing state, and commit-like ids plus optional aliases.
  - Commit: _pending next commit_

- [x] **M4-001** Deliver manual multi-agent session runtime and split-pane CLI UX
  - Note: Landed real session-agent execution targets, focused-agent prompt routing, per-agent provider-run ownership/history metadata, `/view split|individual`, and the initial multi-agent split-pane TypeScript CLI surface.
  - Commit: `23829c2`

- [x] **M2-001** Deliver end-to-end local OpenCode daemon + CLI baseline
  - Note: Shipped one working local flow: launch OpenCode through the daemon, submit prompts from a CLI input field, and stream output live back into the terminal.
  - Commit: `727a97f`
- [x] **M2-002** Add real local daemon transport
  - Note: Replaced the in-process-only local harness assumption with a real local IPC path that a CLI can connect to.
  - Commit: `727a97f`
- [x] **M2-003** Add minimal local CLI client
  - Note: Added a usable CLI that connects to the daemon, submits prompts, and renders live output.
  - Commit: `727a97f`
- [x] **M2-004** Add real OpenCode provider adapter
  - Note: Replaced `dev-stub` for the main M2 path with a PTY-launched OpenCode adapter.
  - Commit: `727a97f`
- [x] **M2-005** Add end-to-end smoke coverage for local daemon + CLI + OpenCode
  - Note: Milestone completion is now covered through the real transport and CLI path rather than only the in-process harness.
  - Commit: `727a97f`
- [x] **M2-006** Create and later reshape the M2 implementation checklist and execution plan
  - Note: Added `docs/M2_IMPLEMENTATION_CHECKLIST.md` to break M2 into concrete capability workstreams, testing, and documentation steps.
  - Commit: `727a97f`
- [x] **M1-001** Add daemon runtime skeleton and module layout
  - Note: Added a lean daemon bootstrap around config, structured runtime errors, shutdown handling, and expandable real services instead of placeholder scaffolding.
  - Commit: _pending next commit_
- [x] **M1-002** Implement in-memory session lifecycle service
  - Note: Added an in-memory Rust session store with create/get/list/end flows, explicit session transitions, and encapsulated runtime session state.
  - Commit: _pending next commit_
- [x] **M1-003** Implement attachment lifecycle and shared-session interaction model
  - Note: Refactored attachments to shared participation, added daemon-owned prompt queueing/config propagation semantics, and kept in-memory attachment event recording.
  - Commit: _pending next commit_
- [x] **M1-004** Add provider adapter baseline and provider run service
  - Note: Added a provider adapter trait, a deterministic dev-stub adapter, and in-memory provider run management with active/parked transitions.
  - Commit: _pending next commit_
- [x] **M1-005** Add PTY manager and terminal stream fan-out
  - Note: Added a `portable-pty` backed PTY manager, terminal input/output routing, and multi-attachment fan-out through daemon-owned terminal records.
  - Commit: _pending next commit_
- [x] **M1-006** Add local daemon API or harness for managed-session flows
  - Note: Added a local request/response daemon API, a smoke harness binary, and runtime tests proving managed-session PTY flow through the daemon.
  - Commit: _pending next commit_
- [x] **M1-007** Add session runtime and PTY-oriented integration tests
  - Note: Added daemon integration coverage for lifecycle cleanup, prompt queue notifications, active run switching, and local managed-session PTY/config flow.
  - Commit: _pending next commit_
- [x] **M1-008** Align protocol and architecture docs with concrete M1 runtime behavior
  - Note: Updated protocol, architecture, checklist, and status docs to match shared attachments, prompt queueing, session-state reads, notice polling, and config propagation.
  - Commit: _pending next commit_
- [x] **M0-001** Bootstrap workspace root (`pnpm-workspace`, root scripts, TS base config)
  - Note: Added workspace root `package.json`, `pnpm-workspace.yaml`, and `tsconfig.base.json`.
  - Commit: _pending next commit_
- [x] **M0-002** Create `packages/domain` with initial shared model types
  - Note: Expanded the domain package to cover the full M0 entity baseline and runtime status constants.
  - Commit: _pending next commit_
- [x] **M0-003** Add minimal domain contract tests (shape/enum invariants)
  - Note: Added TypeScript contract tests for shared runtime constants and core entity serialization.
  - Commit: _pending next commit_
- [x] **M0-004** Create `apps/server` Fastify stub with strict TypeScript
  - Note: Added a smoke test for the server health endpoint.
  - Commit: _pending next commit_
- [x] **M0-005** Bootstrap `apps/daemon` Rust crate and crate tests
  - Note: Baseline daemon crate remains in place and passes cargo tests.
  - Commit: _pending next commit_
- [x] **M0-006** Add Prisma core schema for M0 entities
  - Note: Added `prisma/schema.prisma` for the baseline M0 entity set and relationships.
  - Commit: _pending next commit_
- [x] **M0-007** Add GitHub Actions CI for lint/build/test (+ Rust checks)
  - Note: Added `.github/workflows/ci.yml` for pnpm and Rust verification.
  - Commit: _pending next commit_
- [x] **M0-008** Align docs after scaffolding (`CONTRIBUTING`, `ROADMAP`, `AGENTS` status)
  - Note: Updated root README and M0-related docs to reflect the completed foundation state.
  - Commit: _pending next commit_
- [x] **OPS-001** Establish repo-native task tracking docs for cross-agent continuity
  - Note: Added this board and progress log in `docs/ops/`.
  - Commit: _pending next commit_
