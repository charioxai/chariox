# M0 Implementation Checklist

## Status

Execution checklist for **M0 - Foundations**.

Decisions confirmed:

- include Rust daemon bootstrap in M0
- use GitHub Actions for baseline CI now
- use Option A baseline package layout
- include smoke tests + minimal domain contract tests
- include initial Prisma schema now

## 1. Target M0 outcomes

From `docs/ROADMAP.md`, M0 outcomes are:

- monorepo/workspace setup and baseline CI
- initial shared domain model for user/machine/daemon/session/provider run
- developer docs baseline (`spec-v1`, architecture, protocol, roadmap)

Exit criteria:

- repository can build/lint/test baseline packages
- docs provide a coherent implementation target

## 2. Planned baseline structure (Option A)

```text
/
  apps/
    daemon/         # Rust crate (bootstrap now)
    server/         # Fastify TypeScript app (bootstrap now)
  packages/
    domain/         # shared TS domain types and contracts
  prisma/
    schema.prisma   # initial schema for core entities
  .github/
    workflows/
      ci.yml        # baseline CI for lint/build/test
```

## 3. Implementation checklist

## 3.1 Workspace + package bootstrapping

- [ ] Add workspace root config (`package.json`, `pnpm-workspace.yaml`, `tsconfig` baseline).
- [ ] Add `apps/server` with strict TypeScript setup and a minimal health endpoint.
- [ ] Add `packages/domain` with strict TypeScript and exported core model types.
- [ ] Add `apps/daemon` Rust crate with minimal executable and crate-level tests.
- [ ] Add root scripts:
  - `pnpm lint`
  - `pnpm build`
  - `pnpm test`
  - `cargo test -p arroba-daemon` (or equivalent in daemon crate)

## 3.2 Domain model + schema baseline

- [ ] Define core entities in `packages/domain`:
  - User
  - Machine
  - DaemonInstance
  - Workspace
  - Worktree
  - Session
  - ProviderRun
  - SessionAttachment
  - ControllerLease
  - Schedule
- [ ] Add minimal domain contract tests:
  - serialization/shape invariants
  - enum/status validation for key entities
- [ ] Add Prisma baseline schema in `prisma/schema.prisma` for the same core entities.
- [ ] Ensure naming aligns with docs (`session`, `provider run`, `attachment`, `capability`, `control op`).

## 3.3 CI baseline (GitHub Actions)

- [ ] Create `.github/workflows/ci.yml` with jobs for:
  - install dependencies
  - lint
  - build
  - test
  - Rust checks for daemon crate (`cargo fmt --check`, `cargo clippy`, `cargo test`)
- [ ] Cache pnpm store and cargo artifacts for reasonable CI time.
- [ ] Trigger on pull requests and pushes to main branches.

## 3.4 Verification commands (M0 gate)

Run and pass locally before claiming M0 complete:

```bash
pnpm install
pnpm lint
pnpm build
pnpm test
cargo test --manifest-path apps/daemon/Cargo.toml
```

Optional but recommended:

```bash
cargo fmt --manifest-path apps/daemon/Cargo.toml --check
cargo clippy --manifest-path apps/daemon/Cargo.toml --all-targets --all-features -- -D warnings
```

## 3.5 Documentation updates required in same PR set

- [ ] Update `docs/CONTRIBUTING.md` with exact baseline commands.
- [ ] Update `docs/ROADMAP.md` M0 notes if scope materially changes.
- [ ] Update `agents/AGENTS.md` **Current Status** once code scaffolding lands.

## 4. Definition of Done for M0

M0 is complete when all are true:

- [ ] Option A structure exists and builds.
- [ ] Rust daemon crate exists and passes baseline tests.
- [ ] Shared domain package + minimal contract tests pass.
- [ ] Prisma core schema exists and is coherent with domain docs.
- [ ] GitHub Actions CI passes lint/build/test checks for baseline packages.
- [ ] Documentation is updated and consistent.
