# Arroba

Arroba is a daemon-centered framework for running and orchestrating native AI coding CLIs and compatible agent runtimes through a shared terminal interface.

The project is intentionally local-first. A daemon node owns live sessions on the user's machine, local or remote members attach to that node, and a lightweight server can relay remote connections without becoming the authority for runtime state or provider behavior.

## Status

M0, "Foundations", is complete in this repository.
M1, "Core Session Runtime", is complete.
M2, "End-to-End Local OpenCode Baseline", is complete.
M3 is largely delivered for the local baseline hardening work.
M4 is now in progress with the first manual multi-agent session runtime slice landed in `main`.
M4.5, "Kernel Runtime Refactor", is now in progress in parallel with the remaining M4 stabilization work.

v1 scope includes both:

- single-agent sessions
- manually directed multi-agent sessions
- multi-agent workflow execution

Current delivery priority:

- first: close the OpenCode-first runtime cycle, including capabilities, agent harnessing behavior, and multi-machine session work
- then: finish the kernel runtime refactor so interactive commands stay responsive while background work, provider I/O, history reads, and replay/reconnect paths run concurrently
- then: polish the TypeScript CLI as the reference client
- then: add multi-platform clients such as web and iOS/Android on the same daemon/protocol model
- finally: add more providers such as Claude Code and Codex and harden the generic provider-adapter/protocol shape

The current codebase provides:

- a pnpm workspace for TypeScript packages
- a minimal Fastify server with a health endpoint
- a shared domain package for workflow-oriented core v1 entities
- a Rust daemon runtime with config/bootstrap wiring, in-memory session lifecycle, shared attachment participation, provider-run orchestration, prompt queueing/config propagation, and PTY-backed terminal fan-out
- a real local daemon IPC surface, a TypeScript OpenTUI local CLI with an OpenCode-inspired transcript/prompt layout, and a working OpenCode baseline path with prompt submission and live streamed output
- a kernel-hosted WebSocket transport for the TypeScript CLI, including pushed events, resumable subscriptions, heartbeat/liveness, and reconnect-friendly behavior
- the first M4.5 kernel runtime slices: normalized kernel commands, an event log service with replay-gap handling, a command router, bounded interactive routing, safe command-id retry handling, typed CLI replay-gap notices, provider-run actor coverage for structured submit/cancel/poll paths, `KernelSessionService` for session lifecycle/focus/resize/end/delete operations, `KernelAgentService` for prompt submit/cancel/complete/queue-advance lifecycle behavior, per-agent prompt command mailboxes, per-session UI/lifecycle command mailboxes with projected delete/detach lane lookup, projected focused-agent fallback for untargeted prompt routing, projection-first reads for warmed session/list/history/provider-run/process/catalog and agent/workflow inspection state, daemon health queue snapshots, explicit file-writing capability worktree claims, provider prompt lifecycle worktree claims, and workflow node blocking/retry on workspace claims
- a real manual multi-agent session slice in the daemon and TypeScript CLI: agent records, focused-agent prompt routing, per-agent provider-run ownership/history metadata, `/agent ...` management commands, `Ctrl+A` focus cycling, and `individual`/`split` response views
- a Rust compatibility launcher for the TypeScript CLI
- a local daemon smoke harness for managed-session flows
- a Prisma schema aligned with workflow-oriented runtime entities
- baseline CI for TypeScript and Rust checks

Current implementation caveat:

- the OpenCode-backed multi-agent path still needs stabilization, but the current daemon and CLI suites are green
- the current split-pane TypeScript CLI is still an initial slice centered on the primary transcript plus up to two auxiliary panes
- the M4.5 refactor is not complete yet: `DaemonApp` still exists as a compatibility facade, and service-owned prompt/session state still needs to move fully behind actor/projection ownership before relay scale-out
- current workspace claims are a bounded safety and scheduling layer, not the final I/O-conflict-control design; deeper file-level, port-level, sandbox, or transactional patch coordination is deferred until after actor/projection ownership is complete
- generic agent transport is intentionally deferred for now; OpenCode continues to use its native local HTTP + SSE adapter path

The project specification and architecture remain the primary source of truth for behavior beyond this bootstrap.

## Repository Structure

```text
.
├── agents/              # project-level instructions and status for coding agents
├── apps/
│   ├── cli/             # TypeScript OpenTUI CLI client
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

The current local baseline is one local CLI, one provider (`opencode`), one prompt path, and live streamed output through the daemon. The near-term plan is to close that one-provider cycle fully before broadening to more clients or more providers.

Architecturally, the daemon is now better thought of as a node runtime:

- it owns sessions and prompt routing
- it will eventually route both local and relay-attached members in the same session domain
- it will eventually own workspace coordination to reduce edit/integration conflicts between top-level agents

The primary CLI path now uses the kernel WebSocket event stream. The longer-term daemon implementation target is an actor/event/projection kernel: commands enter through a router, actors own mutation, ordered kernel events drive recovery and replay, and clients read projections. The current `DaemonApp` shape should be treated as a compatibility facade while that refactor lands, not as the long-term concurrency model.

The primary local CLI is now the TypeScript OpenTUI app in `apps/cli`. `arroba-cli` remains the familiar entrypoint by launching that TypeScript client through a small Rust compatibility wrapper.

### `apps/cli`

This package is the new local terminal client. It uses the same OpenTUI stack as OpenCode and intentionally borrows the OpenCode prompt/transcript visual language: a boxed transcript pane, sticky bottom scrolling, a visible side scrollbar, and a boxed multiline prompt composer.

The CLI remains daemon-first: it is only a transport client over the kernel-owned transport surface, not a second runtime authority.

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
- `docs/RUNNING_LOCAL.md`: how to run the current local daemon + CLI path
- `docs/LOGGING.md`: shared logging setup, configuration, and inspection
- `docs/ROADMAP.md`: milestone plan
- `docs/M4_5_KERNEL_RUNTIME_REFACTOR_PLAN.md`: implementation plan for the actor/event/projection kernel refactor
- `docs/CONTRIBUTING.md`: contributor workflow and testing expectations
- `docs/M0_IMPLEMENTATION_CHECKLIST.md`: M0 definition of done and execution checklist
- `docs/M1_IMPLEMENTATION_CHECKLIST.md`: detailed execution checklist for the core session runtime milestone
- `docs/ops/TASKS.md`: lightweight repo-native task tracking
- `docs/ops/PROGRESS_LOG.md`: chronological handoff log

## Getting Started

### Prerequisites

- Node.js 22 or later
- pnpm 9.15.0
- Bun 1.2 or later for the TypeScript OpenTUI CLI runtime
- Rust stable toolchain with `cargo`, `rustfmt`, and `clippy`

### Install

```bash
pnpm install
```

### Run The Local OpenCode Baseline

For a fuller local-runtime guide, see [RUNNING_LOCAL.md](/Users/miguel/arroba/docs/RUNNING_LOCAL.md).

The current local runtime is two processes:

- `arroba-daemon`
- `arroba-cli` (Rust shim that launches the TypeScript OpenTUI client)

OpenCode setup currently requires:

- `opencode` installed locally and reachable on `PATH`, or `ARROBA_OPENCODE_BIN` set to the executable path
- `ARROBA_OPENCODE_PORT` set to an explicit local TCP port for `opencode serve`
- `bun` installed locally and reachable on `PATH`, or `BUN_BIN` set to the executable path

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

Direct TypeScript CLI development path:

```bash
export ARROBA_OPENCODE_PORT=43111
pnpm --filter @arroba/cli run dev
```

Current migration status:

- `apps/cli` is the primary local client implementation
- `arroba-cli` is a compatibility launcher for that TypeScript client

Current local CLI controls:

- `/stop` requests cancellation of the active provider turn; queued work advances only after the provider confirms the stop
- `/exit` detaches from the current session and exits the CLI
- `/session create [alias]` creates and attaches to a new session
- `/session attach <ref>` attaches to a session by full id, unique id prefix, alias, or unique alias prefix within the current workspace
- `/session delete [ref]` deletes the current or referenced session; deleting the active session returns the CLI to its no-session landing state
- manual multi-agent session commands exist today:
  - `/agent spawn [alias] [model]`
  - `/agent spawn <number_of_agents>` spawns that many agents
  - `/agent delete [name-or-alias]`
  - `/agent focus <id>`
  - `/agent list`
  - `/agent cycle`
  - `Ctrl+A` cycles focused agent in the current session
- `/view <split|individual>` switches between split-pane and single-transcript multi-agent views
- these agent controls now drive the local manual multi-agent runtime path: prompts follow the focused agent, the daemon tracks provider runs per agent, and the TypeScript CLI can render individual or split multi-agent transcript views

Optional executable override:

```bash
export ARROBA_OPENCODE_BIN=/absolute/path/to/opencode
```

Optional Bun override:

```bash
export BUN_BIN=/absolute/path/to/bun
```

### Logging And Debugging

For the full logging guide, see [LOGGING.md](/Users/miguel/arroba/docs/LOGGING.md).

Arroba now uses one machine-local shared log root for runtime processes instead of ad hoc debug files.

Default log root resolution:

- `ARROBA_LOG_DIR`, if set
- `XDG_STATE_HOME/arroba/logs`
- `~/.local/state/arroba/logs`
- `./.arroba/logs` as a final fallback

Current process coverage:

- `arroba-daemon`
- the Rust `arroba-cli` launcher
- the primary TypeScript CLI process in `apps/cli`
- the Fastify server process in `apps/server`

Current defaults:

- log format: NDJSON, one JSON record per line
- retention: roughly 7 days and 200 MB total per log root
- privacy: metadata, lifecycle, warnings, and errors by default; prompt/provider content should only be captured via explicit debug-oriented changes

Useful env vars:

```bash
export ARROBA_LOG_DIR=/absolute/path/to/arroba-logs
export ARROBA_LOG_LEVEL=debug
```

Inspect logs directly:

```bash
tail -f ~/.local/state/arroba/logs/*.ndjson
jq 'select(.session_id=="session-1")' ~/.local/state/arroba/logs/*.ndjson
```

Or use the built-in viewer:

```bash
cargo run --manifest-path apps/daemon/Cargo.toml --bin arroba-cli -- logs --follow
cargo run --manifest-path apps/daemon/Cargo.toml --bin arroba-cli -- logs --session session-1
pnpm --filter @arroba/cli run dev -- logs --process-kind daemon --level error
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
