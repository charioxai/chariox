# Arroba Task Board

A lightweight, repo-native task board so contributors and future agents can continue work without external PM tooling.

## How to use

- Keep tasks small and implementation-oriented.
- Move tasks between `Backlog` -> `In Progress` -> `Done`.
- When a task is completed, add a short completion note and link commit hash.
- Use IDs (`M0-001`, `M1-003`, etc.) so references stay stable.
- Do not delete completed tasks; keep historical context.

## Backlog

- [ ] _None currently_

## Done

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
