# Arroba

Arroba is a daemon-centered framework for running and orchestrating native AI coding CLIs through a shared terminal interface.

The project is intentionally local-first. A daemon owns live sessions on the user's machine, clients attach to those sessions, and a lightweight server can relay remote connections without becoming the authority for runtime state or provider behavior.

## Status

M0, "Foundations", is complete in this repository.
M1, "Core Session Runtime", is complete.
M2, "End-to-End Local OpenCode Baseline", is complete.
The immediate next milestone is M3, "Local Capability Surface and Provider Expansion".

v1 scope includes both:

- single-agent sessions
- multi-agent workflow execution

Current delivery priority:

- first: local capabilities, slash commands, and more providers on top of the now-working local OpenCode path
- then: multi-agent workflows
- then: relay/web surfaces
- then: provider switching, memory, compaction, and per-agent extensions such as MCPs and skills

The current codebase provides:

- a pnpm workspace for TypeScript packages
- a minimal Fastify server with a health endpoint
- a shared domain package for workflow-oriented core v1 entities
- a Rust daemon runtime with config/bootstrap wiring, in-memory session lifecycle, shared attachment participation, provider-run orchestration, prompt queueing/config propagation, and PTY-backed terminal fan-out
- a real local daemon IPC surface, a minimal local CLI, and a working OpenCode baseline path with prompt submission and live streamed output
- a local daemon smoke harness for managed-session flows
- a Prisma schema aligned with workflow-oriented runtime entities
- baseline CI for TypeScript and Rust checks

The project specification and architecture remain the primary source of truth for behavior beyond this bootstrap.

## Repository Structure

```text
.
├── agents/              # project-level instructions and status for coding agents
├── apps/
│   ├── daemon/          # Rust daemon crate
│   └── server/          # Fastify TypeScript server bootstrap
├── docs/                # product, architecture, protocol, roadmap, and ops docs
├── packages/
│   └── domain/          # shared TypeScript domain model
├── prisma/              # baseline Prisma schema
├── package.json         # root workspace scripts and dev tooling
├── pnpm-workspace.yaml  # pnpm workspace package discovery
└── tsconfig.base.json   # shared TypeScript compiler baseline
```

## Key Components

### `apps/daemon`

The daemon is the runtime authority in Arroba v1. It is responsible for hosting sessions, managing PTYs, coordinating provider runs, and eventually owning the capability and control lanes described in the architecture docs.

The current local baseline is one local CLI, one provider (`opencode`), one prompt path, and live streamed output through the daemon. Broader capability work, more providers, workflows, relay/web support, and memory-oriented features follow in later milestones.

### `apps/server`

The server is the future relay and control-plane surface. In v1 it is intended to stay lightweight: authentication, discovery, presence, and relay responsibilities should live here, not session execution or provider logic.

At M0 it exposes a single `/health` endpoint and a smoke test.

### `packages/domain`

This package defines shared TypeScript contracts for the core v1 entities used across the repo. It exists to keep terminology and shape definitions consistent between daemon, server, and future client surfaces.

The package includes runtime enum constants and a minimal contract test suite so the baseline shapes are verified, not just typed.

### `prisma/schema.prisma`

The Prisma schema is the initial persistence model for the same core entities defined in the domain package and docs. It is included now to establish naming, relationships, and status enums early, before implementation details spread across the codebase.

## Documentation Map

- `agents/AGENTS.md`: high-level design constraints and current status
- `docs/spec-v1.md`: the product specification for Arroba v1
- `docs/ARCHITECTURE.md`: implementation-oriented architecture view
- `docs/PROTOCOL.md`: protocol lanes and structured message contracts
- `docs/ROADMAP.md`: milestone plan
- `docs/CONTRIBUTING.md`: contributor workflow and testing expectations
- `docs/M0_IMPLEMENTATION_CHECKLIST.md`: M0 definition of done and execution checklist
- `docs/M1_IMPLEMENTATION_CHECKLIST.md`: detailed execution checklist for the core session runtime milestone
- `docs/ops/TASKS.md`: lightweight repo-native task tracking
- `docs/ops/PROGRESS_LOG.md`: chronological handoff log

## Getting Started

### Prerequisites

- Node.js 22 or later
- pnpm 9.15.0
- Rust stable toolchain with `cargo`, `rustfmt`, and `clippy`

### Install

```bash
pnpm install
```

### Run The Local OpenCode Baseline

The current local runtime is two processes:

- `arroba-daemon`
- `arroba-cli`

OpenCode setup currently requires:

- `opencode` installed locally and reachable on `PATH`, or `ARROBA_OPENCODE_BIN` set to the executable path
- `ARROBA_OPENCODE_PORT` set to an explicit local TCP port for `opencode serve`

Example:

```bash
export ARROBA_OPENCODE_PORT=43111
cargo run --manifest-path apps/daemon/Cargo.toml --bin arroba-daemon
```

Then in another terminal:

```bash
export ARROBA_OPENCODE_PORT=43111
cargo run --manifest-path apps/daemon/Cargo.toml --bin arroba-cli
```

Current local CLI controls:

- `/stop` requests cancellation of the active provider turn; queued work advances only after the provider confirms the stop
- `/exit` detaches or ends the local session and exits the CLI

Optional executable override:

```bash
export ARROBA_OPENCODE_BIN=/absolute/path/to/opencode
```

### Verification Commands

Run the current repository verification set from the repository root:

```bash
pnpm lint
pnpm build
pnpm test
cargo test --manifest-path apps/daemon/Cargo.toml
```

Optional Rust checks:

```bash
cargo fmt --manifest-path apps/daemon/Cargo.toml --check
cargo clippy --manifest-path apps/daemon/Cargo.toml --all-targets --all-features -- -D warnings
```

## Design Constraints

The code in this repository is guided by a few non-negotiable rules:

- preserve the native provider terminal experience
- keep the daemon as the runtime authority
- keep the server lightweight
- separate raw terminal streaming from structured capability and control lanes
- implement reusable behavior below any one client surface

These constraints are described in more detail in `agents/AGENTS.md`, `docs/spec-v1.md`, and `docs/ARCHITECTURE.md`.
