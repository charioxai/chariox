# Arroba Task Board

A lightweight, repo-native task board so contributors and future agents can continue work without external PM tooling.

## How to use

- Keep tasks small and implementation-oriented.
- Move tasks between `Backlog` -> `In Progress` -> `Done`.
- When a task is completed, add a short completion note and link commit hash.
- Use IDs (`M0-001`, `M1-003`, etc.) so references stay stable.
- Do not delete completed tasks; keep historical context.

## In Progress

## Backlog

- [ ] **M3-002** Expand local capability surface after OpenCode baseline
  - Note: Continue with richer OpenCode event rendering and broader TypeScript CLI integration tests, then move into slash-command-driven shell, file, git, screenshot, transfer, and schedule surfaces.
- [ ] **M3-003** Add Claude Code and Codex provider support
  - Note: Reuse the same daemon-managed local CLI model after OpenCode is proven.
- [ ] **M4-001** Add local multi-agent workflow runtime
  - Note: Build daemon-owned workflow scheduling and worktree-safe multi-agent execution after the single-agent path is solid.
- [ ] **M5-001** Add relay/web surfaces on top of the local daemon model
  - Note: Remote relay and webapp come after the local CLI + provider path and workflow baseline.
- [ ] **M6-001** Add provider switching, memory, compaction, and per-agent extension management
  - Note: Defer control-lane-heavy features until after local runtime, provider support, and workflow foundations are working.

## Done

- [x] **M3-CLI-001** Improve TypeScript CLI transcript rendering with markdown-aware output and syntax-highlighted fenced code blocks
  - Note: Added markdown-aware assistant/reasoning rendering, parser bootstrap, fence-language normalization, and syntax-highlighted fenced code blocks without depending on LSP semantic coloring.
  - Commit: _pending next commit_
- [x] **M3-001** Implement persistent session management and no-session CLI state
  - Note: Added explicit session deletion, resumable detached sessions, unattached landing state, and commit-like ids plus optional aliases.
  - Commit: _pending next commit_

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
