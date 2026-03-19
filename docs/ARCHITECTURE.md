# Arroba v1 Architecture

## Status

Draft architecture aligned with `docs/spec-v1.md`.

## 1. Purpose

This document translates the v1 specification into an implementation-oriented architecture view:

- component boundaries
- runtime ownership
- trust and security boundaries
- state ownership and storage boundaries
- critical runtime flows

## 2. System Topology

Arroba v1 is composed of four runtime components:

- Client
- Machine
- Daemon
- Server

High-level topology:

`Client <-> Daemon <-> Provider`

Remote topology:

`Remote Client <-> Server (relay) <-> Daemon <-> Provider`

A user may operate multiple clients and multiple machines in the same account.

## 2.1 Daemon-First, API-First, Multi-Client Design Rule

Arroba v1 architecture is explicitly daemon-first, API-first, and multi-client.

Normative rules:

- CLI is one client implementation, not the primary owner of business logic.
- Core session/capability/control behavior must be implemented in daemon and protocol layers.
- Terminal stream handling (unstructured I/O) must remain separate from structured control/state APIs.
- New features must be introduced in reusable daemon/protocol surfaces so they can be consumed by web, native, CLI, and extension clients (for example VS Code) with minimal duplication.

Implementation consequences:

- client code should be thin and focused on UI, input, and transport binding
- daemon services should expose stable operations that any client can invoke
- protocol/docs should be updated when reusable behavior contracts change

## 2.2 Multi-Surface, Multi-Transport Client Architecture Constraints

Arroba remote terminals MUST be architected as multi-surface, multi-transport clients. The architecture MUST NOT assume a single web app or a single local CLI surface.

### Normative Rules

1. Remote terminals are not tied to one UI surface
- A remote terminal MAY be implemented as web terminal, native app terminal, or CLI client attached through another terminal.
- Remote terminal CLI clients MUST be supported as first-class clients, both remotely and locally.

2. Third-party messaging apps are adapter surfaces
- Slack, Telegram, Discord, WhatsApp, and similar messaging channels MUST be modeled as constrained adapters/transports.
- These surfaces MAY support session control, prompt submission, approvals, notifications, summaries, and status queries.
- These surfaces MUST NOT be treated as full PTY clients unless verified to satisfy PTY semantics.

3. Terminal streaming vs structured control/state separation
- Full terminal clients MUST integrate through PTY streaming interfaces.
- Constrained clients and messaging adapters MUST integrate through structured control/state APIs.
- Non-terminal clients MUST NOT parse terminal text as their primary integration contract.
- Providers MAY also expose structured local session/event APIs; when they do, the daemon MAY use those APIs as the source of truth while still rendering a terminal-like experience to full terminal clients.

4. Client capability levels
- Every new feature MUST declare the minimum required client capability level.
- Capability levels MUST include:
  - `full_terminal`
  - `interactive_structured`
  - `message_transport`
  - `automation_only`

5. Remote CLI is first-class
- Remote CLI attachments MUST be treated as first-class full terminal clients.
- Session attachment model MUST consistently support local CLI, remote CLI, web terminals, and native terminal apps.

6. Core runtime below all clients
- Daemon/core services MUST own sessions, PTYs, provider runs, jobs, scheduling, worktrees, and runtime state.
- All client surfaces (terminal, structured, messaging, automation) MUST consume the same core APIs/protocols.

7. Reusable feature implementation
- Features MUST be implemented in reusable core layers (terminal streaming, structured control/state, adapter layer) as appropriate.
- Feature delivery MUST NOT be coupled to one UI surface.

## 2.3 Multi-Agent Workflow Architecture Rule

Arroba MUST support a workflow layer above single-agent sessions.

Delivery priority inside v1:

- circular topology is the earlier implementation target
- hierarchical topology remains in scope for v1, but is expected to land later in v1 after the lower-level runtime and protocol foundations are stable

Normative rules:

- A session MAY run in single-agent mode or multi-agent workflow mode.
- Multi-agent execution MUST be modeled as a general directed workflow graph.
- v1 validates only two workflow topologies:
  - circular
  - hierarchical
- The runtime MUST still be architecture-compatible with future DAGs, bounded cycles, conditional routing, richer aggregation, and other advanced topologies.
- Circular and hierarchical behavior MUST be implemented as validators and policies over a generic workflow engine, not as topology-specific logic scattered through unrelated services.

### Protocol-First, Capability-Based Interpretation

Arroba is protocol-first and capability-based:

- some clients are true terminals
- some clients are structured interactive clients
- some clients are message-based adapters
- all clients MUST integrate through stable core interfaces, not UI-specific logic

## 3. Component Responsibilities

### 3.1 Client

Client examples include local terminal clients, web terminals, and third-party messaging adapters (Telegram/Discord/Slack/WhatsApp).

Responsibilities:

- render terminal stream from active provider run
- capture terminal keystrokes and prompt/config interactions, then route them to the daemon through the appropriate runtime surface
- render Arroba slash-command completion, help, warnings, and command results
- upload files and display artifacts
- expose daemon-owned queue/config/session metadata

Current M2 runtime note:

- the local CLI is now a real daemon client over local IPC, not only a harness/test surface
- the primary local CLI implementation is now a TypeScript OpenTUI client
- `arroba-cli` currently exists as a Rust launcher for that TypeScript client
- the previous Rust-only CLI remains available as `arroba-cli-rust`, but it is phased out and should be treated as a fallback/debugging surface rather than the primary implementation target
- local client-daemon communication currently uses a Unix-socket transport on Unix-like systems

### 3.2 Machine

A machine is a host capable of running agent workloads.

Responsibilities:

- provide execution environment for daemon, providers, and artifacts
- host session worktree and runtime files
- participate in machine registration and reachability for remote use

Constraints:

- one daemon per OS user account on a machine
- sessions can have multiple eligible machines, but only one active execution host at a time

### 3.3 Daemon

The daemon is the runtime authority for live session state.

Responsibilities:

- session lifecycle and attachment lifecycle
- workflow scheduling and workflow-run state ownership
- inter-agent message routing and structured handoff processing
- worktree allocation and isolation enforcement for workflow branches
- PTY lifecycle for provider runs
- provider switching and parked-run management
- provider run lifecycle per workflow node when workflows are active
- capability execution (shell/tree/view/edit/screenshot/git/transfer/compact)
- memory management (short-term + long-term)
- context transfer package generation
- scheduler execution, failure propagation, retry hooks, and resource-limit enforcement
- reusable capability services with structured request/response contracts (starting with shell command execution)
- capability authorization and scoping checks tied to session attachments and worktree boundaries
- slash-command dispatch and command-registry resolution
- provider installation/auth-state probing and structured login warnings
- provider version probing, built-in command-catalog selection, and custom-command discovery when supported
- extension registry, binding resolution, and provider-view materialization
- MCP runtime lifecycle management

### 3.4 Server

The server is a lightweight control and relay layer.

Responsibilities:

- auth and identity
- machine/session discovery
- websocket relay
- presence plus queue/config/session metadata as needed
- schedule metadata and operational metadata

Non-responsibility:

- should not require plaintext access to user-generated session content

## 4. Runtime Ownership and State Authority

### 4.1 Authority Model

- **Daemon**: source of truth for active runtime state
- **Server**: source of truth for shared operational metadata
- **Provider process**: source of truth for provider-native behavior

### 4.2 Session Ownership

A session is bound to:

- one workspace
- one primary worktree in single-agent mode
- one active provider run in single-agent mode
- an eligible set of machines with one active host at a time

A session may include:

- many client attachments
- parked provider runs
- prompt queue state and canonical session config state
- a workflow definition and zero or one active workflow run
- node-scoped provider runs when workflow mode is active
- worktree assignments for isolated workflow branches
- schedules
- artifacts
- extension bindings resolved for top-level provider runs

### 4.2.1 Shared Attachment and Queue Ownership

In single-agent mode, attachments are shared session participants rather than exclusive control roles.

Required daemon-owned responsibilities:

- serializing prompt execution per session
- maintaining explicit scheduler state boundaries (`idle`, `runnable`, `running`, `waiting`) for queued work
- maintaining canonical queued-prompt state
- exposing canonical session state and runtime notices to attachments
- applying accepted config changes to canonical session config state
- rejecting unsafe config changes while a prompt is running
- notifying all other attachments when a prompt is queued
- propagating canonical config updates to all attachments after acceptance

Current M1 runtime note:

- the daemon now keeps explicit primary worktree assignment metadata for each session even in single-agent mode so later branch/worktree isolation can extend the same runtime shape

The daemon MUST treat prompt scheduling and config state as structured runtime state, not terminal-local behavior.

### 4.3 Workflow Ownership

When a session runs in multi-agent workflow mode, the daemon MUST treat the workflow as a generic directed graph execution problem.

Required runtime concepts:

- `WorkflowDefinition`
- `WorkflowNode`
- `WorkflowEdge`
- `WorkflowRun`
- `NodeRun`
- `NodeMessage`
- `WorktreeAssignment`
- `AggregationState` or equivalent barrier/fan-in state

Required rules:

- Every multi-agent workflow MUST have a designated coordinator node.
- The coordinator MUST receive the initial user prompt.
- The coordinator MUST decide whether execution continues, stops, or completes.
- In circular topology, the coordinator MUST be part of the cycle.
- In hierarchical topology, the coordinator MUST be the root.

Implementation priority note:

- circular topology should be implemented and stabilized first
- hierarchical topology should follow later in v1 on top of the same generic workflow engine

### 4.4 Inter-Agent Communication Ownership

Inter-agent communication MUST be daemon-orchestrated.

Required rules:

- Agents MUST NOT communicate directly.
- The daemon MUST own all routing and delivery between workflow nodes.
- Output from one node MUST be transformed into a standardized structured handoff payload before it is delivered to the next node.
- Inter-agent communication MUST NOT be modeled as raw terminal transcript forwarding.
- Workflow scheduling MUST advance from structured node completion reports, not arbitrary provider turns.

Each node completion artifact/report MUST include at least:

- `status`
- `summary`
- `artifacts` or changed files
- `handoff_payload`
- `stop_recommendation`

### 4.5 Worktree Isolation

Parallel code-writing branches MUST NOT share the same active worktree.

Required rules:

- Worktree assignment MUST be explicit in runtime state and the data model.
- In hierarchical workflows, each active code-writing branch or subtree SHOULD receive an isolated worktree and git branch.
- The daemon MUST reject or prevent concurrent mutation of the same active worktree by parallel code-writing nodes.

## 5. Interaction Lanes

### 5.1 Terminal Lane

- Carries raw PTY output and user keystrokes.
- Must preserve provider-native semantics.
- Must not be transformed into structured command traffic by default.
- Must not be used as the source of truth for prompt queue ordering or session config state.
- For providers with stable structured local protocols, daemon-rendered output derived from provider events is acceptable and preferred over PTY-idle heuristics.

### 5.2 Capability Lane

- Carries daemon capability requests/results.
- Used for shell, file ops, screenshot, git/worktree, schedules, transfers, and other Arroba-owned slash commands.

### 5.3 Control Lane

Structured daemon-to-provider adapter control boundary.

Canonical control operations in v1:

- `attach_file`
- `request_memory_update`
- `request_compaction_summary`

`request_memory_update` and `request_compaction_summary` are daemon-owned and distinct from normal user prompt/response traffic.

`/agent ...` commands are resolved by Arroba first, then dispatched into adapter-owned behavior through the control lane or adapter-specific execution hooks.

Provider authentication is not part of the control lane in v1; adapters probe and report auth state, but login itself remains a provider-native local CLI flow on the host machine.

Provider-facing extension projection is also adapter-owned: the daemon resolves the authoritative extension bindings, and the adapter materializes the provider-specific runtime view.

### 5.3.1 OpenCode Structured Adapter

OpenCode is the first provider where Arroba intentionally prefers a structured local provider protocol over PTY-only inference.

Target runtime flow:

- daemon launches `opencode serve` in the assigned worktree or workspace context
- daemon waits for the local OpenCode server health endpoint
- daemon creates or binds an OpenCode session for the Arroba provider run
- daemon submits prompts through the OpenCode session API
- daemon subscribes to the OpenCode SSE event stream
- daemon maps OpenCode session/message events into Arroba prompt lifecycle, notices, and client-facing output

Target signal mapping:

- prompt submit: OpenCode session prompt API
- `/agent ...`: OpenCode command list plus session command API
- turn abort: OpenCode session abort API
- turn busy/idle: OpenCode session status events
- incremental text: OpenCode message-part delta and part-update events
- assistant completion: OpenCode assistant message updates with completion timestamps
- provider errors: OpenCode session error events plus adapter protocol errors when the event/session transport itself fails

Implication:

- PTY process exit remains a provider-run liveness signal
- PTY idleness is not the completion signal for OpenCode once the structured adapter path exists

### 5.4 Workflow Lane Semantics

Workflow scheduling and node-to-node handoffs belong to the daemon's structured state/control surfaces, not the terminal lane.

Implications:

- node handoffs MUST use structured daemon-owned payloads
- node completion reports MUST be machine-parseable
- workflow barriers, fan-in, aggregation, and termination decisions MUST operate on structured runtime state
- PTY traffic MAY be observed by the daemon for runtime coordination when needed, but MUST NOT be reused as the inter-agent contract

## 6. Security and Trust Boundaries

### 6.1 E2E Encryption Scope

User-generated content in transit must use session-scoped end-to-end encryption when crossing remote transport boundaries, including:

- terminal-entered prompts/content
- capability payloads (edit instructions, prompt templates)
- uploaded file payloads
- memory transfer package payloads
- compaction summary payloads

### 6.2 Relay Trust Model

Server acts as relay/registry and should not require content plaintext to perform core duties.

### 6.3 Session Key Isolation

Cryptographic context is session-scoped. A key compromise in one session must not imply compromise of other sessions.

## 7. Memory Architecture

## 7.1 Dual Memory Model

Arroba memory has two scopes:

- short-term memory: recent transcript/task continuity
- long-term memory: durable user/project guidance

## 7.2 Memory Update Mechanism

Daemon may call `request_memory_update` to refresh memory-relevant signals after provider compaction/reset or before transfer.
Daemon may call `request_compaction_summary` during user-triggered Arroba compaction before starting a fresh warmed run.

Fallback:

- if unsupported, daemon continues with Arroba-managed memory sources
- memory refresh failure must not terminate provider run

## 7.3 Context Transfer Package

Transfer package composes:

- selected short-term snapshot
- relevant long-term entries
- workspace state

Requirements:

- deterministic and auditable composition
- user control over long-term entry inclusion
- encrypted in transit

## 7.4 Extension Architecture

Arroba manages extensions in two phases:

- install: register an extension on the machine
- bind: make that extension available to a top-level Arroba-managed agent or provider run

The daemon owns:

- extension installation metadata
- compatibility and validation checks
- per-agent binding resolution
- provider-view materialization inputs
- MCP runtime lifecycle for bound MCP servers

Provider-native subagents are not separate extension targets; they inherit whatever their parent top-level provider run can access.

### 7.5 Arroba-Driven Context Compaction

Arroba provides a user-triggered compaction command: `/compact`.

Compaction sequence:

1. daemon requests compaction summary via `request_compaction_summary`
2. daemon stores summary as a session artifact/memory input
3. daemon launches fresh provider run with empty context window
4. daemon warms new run with compaction summary + selected Arroba memory/workspace context

This flow is daemon-orchestrated and separate from ordinary user prompt traffic.

## 8. Failure and Degradation

Mandatory behavior:

- adapter lacking `attach_file`, `request_memory_update`, and/or `request_compaction_summary` does not break core PTY usage
- control operation failures are isolated and user-visible
- remote client disconnect does not terminate session by default
- workflow node failure propagation and retry policy MUST remain daemon-owned and explicit
- workflow concurrency/resource limits MUST be centrally enforced by the daemon runtime
- unsupported provider versions emit compatibility warnings but retain best-effort `/agent` completions
- provider-auth failures are surfaced as structured local host warnings and MUST NOT cause Arroba to take ownership of provider credentials

## 9. Deployment and Evolution Notes

v1 is local-first and single-active-host-per-session. Architecture should remain forward-compatible with:

- richer multi-machine scheduling/migration
- expanded provider adapter capabilities
- optional content persistence policies
- more advanced workflow topologies
- bounded loops and other cycle policies
- multi-user and team workflows
- richer aggregation policies and barrier behavior
- explicit merge or reconciliation stages
- per-node provider, model, and account selection

## 10. Implementation Choices (v1 baseline)

Contributor workflow conventions (coding style, testing, PR hygiene) are documented in `docs/CONTRIBUTING.md`.

This section captures current implementation choices for v1 so engineering work has a stable baseline. These are implementation defaults, not product invariants, and may evolve with explicit architecture updates.

### 10.1 Monorepo and Package Management

- monorepo layout
- pnpm workspaces

### 10.2 Client Stack

- React
- TypeScript
- xterm.js for terminal rendering

### 10.3 Daemon Stack

- Rust (required for v1 daemon implementation baseline)

### 10.4 Backend Stack

- Fastify

### 10.5 Data Layer

- Prisma
- SQLite for early/local phases
- Postgres as scale-up target

### 10.6 Transport and Local IPC

- WebSockets for remote relay transport
- Unix socket on Unix-like systems for local client-daemon communication
- named pipe on Windows for local client-daemon communication

Current M2 runtime note:

- the Unix-socket local transport is now implemented for the daemon + local CLI baseline
- Windows local transport remains a later follow-up

### 10.6.1 OpenCode Integration Strategy

- M2 baseline: PTY-launched OpenCode wrapper path
- current M3 direction: daemon-launched `opencode serve` plus local HTTP/SSE adapter
- adapter-owned OpenCode session/event handling should remain behind daemon/provider abstractions so later providers can still use PTY or their own structured surfaces without changing client contracts

### 10.7 Governance

Implementation choices should be revised when they materially change runtime architecture, protocol assumptions, security posture, or operational behavior.

### 10.8 Cross-Platform Terminal Consistency Strategy

Arroba should use a shared terminal behavior contract while allowing platform-native implementation languages.

Approach:

- define canonical terminal behavior in protocol/conformance terms (PTY byte stream handling, resize semantics, key mapping expectations, control-sequence fidelity)
- use xterm.js as the web/remote reference implementation and golden-behavior baseline
- keep slash-command parsing, completion semantics, and warning behavior consistent across clients

Platform framework options for xterm.js-consistent rendering:

- Web: browser-hosted xterm.js
- iOS: `WKWebView` hosting xterm.js plus native shell for platform integration
- Android: `android.webkit.WebView` hosting xterm.js plus native shell for platform integration
- macOS desktop: `WKWebView` (AppKit/SwiftUI host) with xterm.js
- Windows desktop: WebView2 (`Microsoft.Web.WebView2`) with xterm.js
- Linux desktop: embedded Chromium/WebKit host (for example Electron or GTK WebKit) with xterm.js
- CLI/TUI clients: native terminal stack is allowed, but must pass the same conformance profile for input/output/resize semantics

Result:

- consistent remote terminal behavior across platforms
- freedom to use standard language/tooling per target platform
