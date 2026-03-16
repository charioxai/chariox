# Arroba Task Board

A lightweight, repo-native task board so contributors and future agents can continue work without external PM tooling.

## How to use

- Keep tasks small and implementation-oriented.
- Move tasks between `Backlog` -> `In Progress` -> `Done`.
- When a task is completed, add a short completion note and link commit hash.
- Use IDs (`M0-001`, `M1-003`, etc.) so references stay stable.
- Do not delete completed tasks; keep historical context.

## Backlog

- [x] **M0-001** Bootstrap workspace root (`pnpm-workspace`, root scripts, TS base config)
  - Note: Added workspace root `package.json`, `pnpm-workspace.yaml`, and `tsconfig.base.json`.
  - Commit: _pending next commit_
- [x] **M0-002** Create `packages/domain` with initial shared model types
  - Note: Added strict TypeScript domain package with initial core interfaces.
  - Commit: _pending next commit_
- [ ] **M0-003** Add minimal domain contract tests (shape/enum invariants)
- [x] **M0-004** Create `apps/server` Fastify stub with strict TypeScript
  - Note: Added Fastify server bootstrap with `/health` endpoint and strict TS config.
  - Commit: _pending next commit_
- [x] **M0-005** Bootstrap `apps/daemon` Rust crate and crate tests
  - Note: Added Rust daemon lib/bin with a baseline unit test.
  - Commit: _pending next commit_
- [ ] **M0-006** Add Prisma core schema for M0 entities
- [ ] **M0-007** Add GitHub Actions CI for lint/build/test (+ Rust checks)
- [ ] **M0-008** Align docs after scaffolding (`CONTRIBUTING`, `ROADMAP`, `AGENTS` status)

## In Progress

- [ ] _None currently_

## Done

- [x] **OPS-001** Establish repo-native task tracking docs for cross-agent continuity
  - Note: Added this board and progress log in `docs/ops/`.
  - Commit: _pending next commit_
