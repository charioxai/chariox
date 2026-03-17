# Arroba Task Board

A lightweight, repo-native task board so contributors and future agents can continue work without external PM tooling.

## How to use

- Keep tasks small and implementation-oriented.
- Move tasks between `Backlog` -> `In Progress` -> `Done`.
- When a task is completed, add a short completion note and link commit hash.
- Use IDs (`M0-001`, `M1-003`, etc.) so references stay stable.
- Do not delete completed tasks; keep historical context.

## Backlog

## In Progress

- [ ] **M2-001** Expand daemon capability surface beyond runtime baseline
  - Note: Started with a daemon-owned shell command capability plus local API exposure; continue with richer file/git/transfer/screenshot behavior.

## Backlog

- [ ] **M2-003** Add directory tree and file view/edit capability baselines
  - Note: Extend the new capability layer with structured filesystem inspection and edit flows.
- [ ] **M2-004** Add git/worktree inspection, transfer, and schedule baselines
  - Note: Build on the capability layer with git/worktree, file transfer, and schedule execution surfaces.

## Done

- [x] **M2-002** Create M2 implementation checklist and execution plan
  - Note: Added `docs/M2_IMPLEMENTATION_CHECKLIST.md` to break M2 into concrete capability workstreams, testing, and documentation steps.
  - Commit: _pending next commit_
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
