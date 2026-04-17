# AGENTS.md

## Project: Arroba

Arroba is a framework that allows developers to control AI coding agents such as Codex and Claude Code locally or remotely through a shared terminal interface. It acts as a thin wrapper around existing provider CLIs, enabling provider switching, multi-terminal collaboration, remote access to sessions running on a developer’s machine, and future extensions such as scheduling, screenshots, shell commands, and team support.

The goal is to preserve the exact native CLI experience while adding orchestration, remote terminals, and session management.

This document summarizes the current architectural decisions and constraints so future agents or developers can understand the design context without needing prior conversation history.

## Core Principles

1. Provider-native experience
   - Users must feel like they are running the provider CLI directly for ordinary prompt and terminal work.
   - Arroba owns the slash-command namespace.
   - Provider-specific commands are exposed through Arroba as `/agent ...`, not as raw provider `/...` commands.

2. Wrapper, not replacement
   - Arroba orchestrates sessions but does not replace provider logic.
   - All provider state remains owned by the provider.
   - Provider login remains provider-native and local to the host machine.

3. Terminal-first interface
   - The primary interface is a real terminal.
   - Arroba-specific actions are invoked through slash commands.
   - All important actions must be accessible directly from the terminal interface.

4. Daemon-centered architecture
   - A background daemon owns all active sessions.
   - Clients attach to daemon-managed sessions.

5. Local-first
   - Sessions run on the user’s machine.
   - Remote clients attach to those sessions through a relay server.

6. Minimal provider interference
   - Arroba should not attempt to manage or control provider internal state.
   - It only tracks enough context to support provider switching when requested.
   - Arroba must not become a provider credential store in v1.

7. Agent-scoped extensions
   - Skills, MCPs, command packs, and similar provider-facing assets are managed by Arroba on a per top-level agent basis.
   - Provider-native subagents are not separately orchestrated by Arroba.
   - MCP runtime ownership belongs to the daemon, not to provider CLIs.

8. Shared logging only
   - Arroba runtime diagnostics must use the shared Arroba logging system.
   - Do not add ad hoc debug env vars, temporary file-append loggers, or committed `eprintln!`/`console.log` debugging paths as separate mechanisms.
   - Logs should go under the shared machine-local Arroba log root with structured correlation fields so daemon and client activity can be inspected together.
   - Default logs should be metadata-first; prompt/output content is opt-in debug material, not default logging.

## Interaction Testing Guidance

Transport and runtime changes MUST be validated with layered interaction tests, not only unit tests or compile checks.

Required testing stack for kernel/CLI interaction work:

1. CLI transport contract tests
   - Keep request/response, subscribe/unsubscribe, and transport-close behavior covered in `apps/cli/src/ipc.test.ts`.
   - Extend these tests first when the TypeScript client transport or frame protocol changes.

2. Daemon kernel-transport integration tests
   - Keep real kernel-WebSocket coverage in `apps/kernel/tests/kernel_websocket_integration.rs`.
   - These tests should exercise the real daemon app plus the transport server and assert pushed kernel events, not only direct service calls.

3. Live program smoke validation
   - When changing transport, runtime fanout, or transcript update behavior, run the real daemon and CLI and verify the visible flow with logs enabled.
   - At minimum, confirm attach/subscribe, prompt submission, pushed transcript updates, and clean session-unavailable handling when the active workspace/session disappears.
   - Clean up any daemon, CLI, and provider processes you started for the smoke run after verification. Stale live-run processes can exhaust PTY devices on macOS and make PTY-backed tests fail with `failed to openpty` even when the PTY manager is correct.

4. Next coverage layer
   - Prefer adding deterministic kernel/CLI transcript-flow integration tests before adding more ad hoc manual checks.
   - PTY-driven CLI smoke tests are the next intended layer for visible terminal-state assertions.

Future agents MUST extend this layered setup instead of introducing one-off scripts or compile-only validation for interaction changes.

## Daemon-First, API-First, Multi-Client Rule

Arroba is daemon-first, API-first, and multi-client by design.

Required implications:

- CLI is only one client surface; it is not the architecture center.
- Core logic must live below the CLI in daemon services and shared protocol contracts.
- Terminal I/O must remain separated from structured control/state operations.
- New features must be implemented so they can be reused by multiple clients (web app, native app, VS Code extension, and future clients) without re-implementing business logic in each client.

Design guardrails:

- capability and control behaviors belong to daemon + protocol layers
- client layers should focus on rendering, interaction, and transport adaptation
- avoid feature implementations that are CLI-only unless explicitly marked as temporary

## Multi-Surface, Multi-Transport Client Architecture Constraints

Arroba remote terminals MUST be designed as multi-surface, multi-transport clients. Contributors MUST NOT assume a single web app or a single local CLI as the only valid client model.

### Normative Rules

1. Remote terminals are not tied to one UI surface
- A remote terminal MAY be implemented as a web terminal, native app terminal, or a CLI client running inside another terminal.
- The architecture MUST support a remote terminal CLI as a first-class client, both when run on a remote machine and when used locally.

2. Third-party messaging apps are supported through adapters, not by pretending they are full terminals
- Messaging apps such as Slack, Telegram, Discord, WhatsApp, and similar channels MUST be modeled as constrained transport/adapter surfaces.
- They MAY support session control, prompt submission, approvals, notifications, summaries, and status queries.
- They MUST NOT be assumed to support full PTY/terminal semantics unless explicitly validated.

3. Separate terminal streaming from structured control/state
- Full terminal clients MUST use a PTY/terminal streaming interface.
- Constrained clients and messaging integrations MUST use a structured control/state API.
- Non-terminal clients MUST NOT be forced to parse terminal text in order to integrate with Arroba.

4. Define client capability levels
- New features MUST specify which client capability level they require.
- Capability levels MUST include at least:
  - `full_terminal`
  - `interactive_structured`
  - `message_transport`
  - `automation_only`

5. Remote CLI clients are first-class citizens
- A CLI that attaches remotely to Arroba sessions MUST be treated as a first-class full terminal client, not as a debug tool or special case.
- The same session attachment model MUST support:
  - local CLI
  - remote CLI
  - web terminal
  - future native terminal apps

6. Core runtime must remain below all clients
- The daemon MUST own sessions, PTYs, provider runs, jobs, scheduling, worktrees, and runtime state.
- All remote terminal implementations, including messaging adapters and remote CLI clients, MUST build on the same daemon/core APIs.

7. New features must be reusable across client surfaces
- When adding a feature, contributors MUST classify whether it belongs in:
  - terminal streaming layer
  - structured control/state layer
  - adapter layer
- Contributors MUST NOT implement features in a way that only works in the web app or only works in the local CLI.

### Protocol-First, Capability-Based Model

Arroba MUST be treated as protocol-first and capability-based:

- some clients are true terminals
- some clients are structured interactive clients
- some clients are message-based adapters
- all integrations MUST use stable core interfaces rather than UI-specific logic

## High-Level System Overview

Arroba has three primary runtime components:

- Client layer
- Local daemon
- Relay/control server

Flow:

`Client <-> Daemon <-> Provider CLI`
and for remote access:
`Remote Client <-> Server <-> Daemon <-> Provider CLI`

## Clients

Clients are terminal interfaces that attach to sessions.

Examples:
- Local CLI client
- Web application terminal
- Future desktop/mobile apps
- Possible future chat adapters

Important rules:
- A client may attach to multiple sessions simultaneously.
- Multiple clients may attach to the same session.
- Multiple local CLI terminals may attach to the same session.

## Daemon

There is one daemon process per machine OS user account.

Hierarchy:

- Machine
  - OS user account
    - Arroba daemon
      - many Arroba sessions
      - many client attachments across those sessions

The daemon is responsible for:
- Hosting sessions
- Launching provider CLIs
- Managing PTYs
- Handling client attachments
- Running scheduled tasks
- Executing capabilities
- Managing git worktrees
- Tracking short-term context for provider transfer

The daemon is the source of truth for runtime state.

There is not one daemon per session. A single daemon manages many sessions.

## Server

The server is intentionally lightweight.

Responsibilities:
- Authentication
- Machine registry
- Session discovery
- WebSocket relay
- Presence tracking
- Controller lease tracking
- Schedule metadata storage
- Operational metadata storage

The server should not need to interpret user content. It mainly acts as a relay and registry.

Local CLI clients communicate directly with the daemon over a local socket.
Remote clients communicate through the server, which relays messages to the daemon.

The server does not necessarily need to store encrypted content. Current assumption:
- prompts and model outputs may be relayed only and not persisted
- only operational metadata needs to be stored unless future features require persistence

## Sessions

An Arroba session is the top-level execution unit.

Every session MUST operate in one of two runtime modes:

- single-agent mode
- multi-agent workflow mode

A session is bound to:
- one workspace
- one primary worktree in single-agent mode
- one active provider run at a time in single-agent mode

A session may have:
- multiple attached terminals
- multiple parked provider runs
- scheduled jobs

In multi-agent workflow mode, the session additionally owns a daemon-orchestrated workflow graph, node-scoped provider runs, and explicit worktree assignments for isolated branches. Parallel workflow branches MAY therefore have multiple active provider runs at once, subject to daemon-owned concurrency and resource limits.

Sessions do not hop across workspaces.

## Multi-Agent Workflow Layer

Arroba MUST support a workflow layer above the single-agent session model.

Delivery priority inside v1:

- circular topology is the earlier implementation target
- hierarchical topology remains in scope for v1, but is expected to land later in v1 after the lower-level runtime and protocol foundations are stable

Normative rules:

- Multi-agent execution MUST be modeled as a general directed workflow graph.
- Contributors MUST NOT implement multi-agent behavior as hardcoded topology-specific branching scattered across the codebase.
- v1 validates only two workflow topologies:
  - circular
  - hierarchical
- The runtime MUST still be designed so future DAGs, bounded loops, conditional routing, richer fan-in/fan-out rules, and other topologies can be added without redesigning the core runtime model.

## Workflow Coordinator Model

Every multi-agent workflow MUST have a designated coordinator node.

Coordinator rules:

- the coordinator receives the initial user prompt
- the coordinator decides whether the workflow should continue, stop, or declare completion
- in circular topology, the coordinator is part of the cycle
- in hierarchical topology, the coordinator is the root node

## Multi-Agent Topology Rules

### Circular Topology

The circular topology is valid in v1 only if all are true:

- each node has exactly one incoming edge and one outgoing edge
- the last node connects back to the coordinator
- execution is serialized
- the workflow uses bounded iteration or round limits
- execution stops when either:
  - the configured max iteration/round limit is reached, or
  - the coordinator declares completion or stop

### Hierarchical Topology

The hierarchical topology is valid in v1 only if all are true:

- the workflow forms a rooted tree
- the coordinator is the root
- parent nodes may fan out to multiple children
- child branches may execute in parallel
- parent fan-in waits for all children by default
- child results propagate upward through structured aggregation
- the coordinator decides final stop or continue behavior

Implementation priority note:

- circular topology should be implemented and stabilized first
- hierarchical topology should reuse the same generic workflow engine and follow later in v1 rather than being deferred beyond v1

## Inter-Agent Communication

Inter-agent communication MUST be daemon-orchestrated.

Required rules:

- agents MUST NOT communicate directly with one another
- the daemon MUST own all inter-agent routing and message passing
- outputs from one node MUST be transformed into a standardized structured handoff payload before being routed to another node
- contributors MUST NOT model inter-agent communication as raw terminal transcript forwarding
- workflow progression MUST be driven by structured node completion reports, not arbitrary provider turns

Each node MUST emit a structured completion artifact/report containing at least:

- status
- summary
- artifacts or changed files
- handoff payload
- stop recommendation

## Worktree Isolation in Multi-Agent Mode

Parallel code-writing branches MUST NOT share the same active worktree.

Required rules:

- worktree assignment MUST be explicit in the runtime and data model
- in hierarchical workflows, each active code-writing branch or subtree SHOULD operate in an isolated worktree and git branch
- the daemon MUST NOT allow parallel code-writing agents to mutate the same worktree concurrently

## Workflow Runtime Ownership

The daemon MUST own all workflow execution concerns, including:

- workflow scheduling
- runnable, waiting, and completed node state
- inter-agent message routing
- worktree allocation
- provider run lifecycle per node
- termination conditions
- failure propagation and retry hooks
- concurrency and resource limits

The workflow runtime MUST think in generic graph-execution terms such as message availability, barriers, aggregation state, and termination policy. Circular and hierarchical behavior MUST be implemented as validators or policies over a generic workflow engine.

## Provider Runs

A provider run represents one live native provider process.

Examples:
- Codex process
- Claude Code process

Provider run states:
- active
- parked
- terminated

Switching providers does not immediately destroy the previous provider run.

Current intended behavior:
1. User requests a provider switch
2. Arroba asks whether context should be transferred
3. If yes, Arroba uses session memory (short-term + long-term) to initialize the new provider run
4. Previous provider run becomes parked
5. If user switches back before truly continuing with the new provider, the parked provider run can be resumed
6. Otherwise the old run may be terminated later, but the design preference is to be minimally intrusive and let the user decide when possible

Important clarification:
- Arroba session != provider-native session
- A provider-native session may be restarted or reset by the user directly inside the provider
- Arroba should tolerate that and adapt by resetting its own context tracking window when needed

## Provider Switching

When switching providers:
1. User triggers switch via an Arroba slash command
2. Arroba asks whether context should be transferred
3. If yes, Arroba uses:
   - short-term memory (recent transcript/task state)
   - long-term memory (durable user/project guidance)
   - current workspace state
4. New provider process starts
5. Old provider process becomes parked

Arroba should not depend on provider internal hidden session state.

If a user resets or compacts the provider context internally, Arroba should simply reset its own context tracking window. It should not create a new Arroba session automatically.

## Provider State Changes

Arroba should interfere as little as possible with provider-native PTY behavior.

Users must be able to:
- change permissions from within the provider
- use provider functionality exposed by Arroba under `/agent ...`
- start new native provider sessions
- reset conversations inside the provider

Arroba only really needs to care about:
1. context compaction / reset
2. permission requests that need to be relayed to attached terminals
3. daemon-initiated memory update inquiries to refresh Arroba memory after provider compaction/reset
4. user-triggered Arroba compaction flow (`/compact`) and compaction summary retrieval

Current design preference:
- be permission-agnostic
- forward permission requests and responses between provider and user
- let the user manage provider permission settings directly if possible
- avoid building a heavy permission policy layer unless later needed

## Terminal Attachments

Each session may have many attached terminals.

Examples:
- Local CLI terminal 1
- Local CLI terminal 2
- Web terminal
- Another remote terminal

Arroba command example:
- `/compact` triggers daemon-managed context compaction

All attachments in a session MAY submit prompts and request supported config changes.

Required rules:

- the daemon MUST remain the single source of truth for prompt scheduling and session config state
- the daemon MUST own explicit scheduler state boundaries for queued work
- attachments MUST NOT maintain conflicting local authority over queued work or session config
- at most one prompt may execute at a time per single-agent session
- if a prompt is submitted while another prompt is running, the daemon MUST enqueue it
- when a prompt is enqueued, the daemon MUST notify all other attachments in the session that a queued message exists and expose the canonical queue state
- attachments MUST be able to fetch canonical session state and daemon notices through a structured daemon-owned API surface
- attachments SHOULD render daemon-reported queue/config state rather than assuming local state is authoritative

Config update rules:

- config changes that are safe during execution MAY be applied immediately
- config changes that are not safe during execution MUST be rejected with an explicit busy-state error while a prompt is running
- after an accepted config change, the daemon MUST propagate the canonical updated config state to all attachments in the session

A single client may interact with multiple sessions at once if it has multiple tabs or attachments.

## Command System

Arroba owns the slash-command namespace.

Rules:

- `/...` is reserved for Arroba command parsing and completion.
- `/agent ...` is the provider-specific command namespace exposed by the active adapter.
- ordinary non-command input continues through the PTY path.
- unsupported provider versions produce warnings but do not disable best-effort `/agent` completions.

Examples of target commands:
- switch session
- switch provider
- show provider history
- show directory tree
- run shell command
- take screenshot
- show git status
- show worktree info

Provider command discovery policy:

- Arroba ships built-in command catalogs for explicitly supported provider version families.
- Arroba augments those catalogs by reading supported custom-command files/config when the provider supports that pattern.
- Arroba does not rely on scraping human-oriented slash help output as its primary compatibility mechanism.

## Capabilities

The daemon exposes capabilities independent of the provider.

Current v1 capabilities to model:
- run shell command
- show directory tree
- show file contents
- edit file
- take screenshot
- git operations
- scheduled prompts

Capabilities are daemon features, not provider features.

## Git Integration

Each session operates in its own git worktree.

Git automation may include:
- auto worktree creation
- auto commit after interactions
- optional push
- optional merge when session ends

These should be configurable.

## Scheduled Tasks

Users may schedule prompts.

Schedules:
- belong to sessions
- are stored as metadata on the server
- are executed by the daemon

Schedules should run as long as:
- daemon is online
- session exists
- workspace/worktree is available

Schedules do not spawn new sessions in v1.

## Shell Commands

Shell commands can be invoked via Arroba capability.

Behavior:
- executed in a subprocess
- scoped to the session workspace/worktree
- should not mutate daemon process state implicitly

Example concern:
- if a shell command runs `cd ..`, that directory change should affect only that subprocess, not redefine the daemon’s working directory

Provider-terminal-native commands such as `cd` inside the actual provider PTY behave normally there.

No separate Arroba `change working directory` abstraction is required for now.

## Workspace Model

A workspace represents a project.

Contains:
- repository
- worktrees
- sessions

Future direction:
- multiple users on the same workspace
- team support
- access control
- collaborative session visibility

So the architecture/schema should already be friendly to future entities such as:
- WorkspaceMembership
- Session ACL
- Team encryption keys if needed later

## Multi-Machine Future

Current design:
- one session is hosted by one daemon
- one daemon runs on one machine OS user account

Future support should allow:
- same workspace across multiple machines
- eventual session migration between daemons
- multi-machine coordination

Schema should already anticipate fields like:
- host_machine_id
- host_daemon_id

But v1 remains single-host-per-session.

## Permissions

Current direction is simplified compared with earlier ideas.

Arroba should be mostly permission-agnostic:
- provider requests permissions
- Arroba relays those permission requests/responses
- user can manage provider-specific permission settings directly

Key requirement:
- changing permissions inside the provider while using Arroba must feel exactly as it would when running the provider CLI directly

Arroba should avoid imposing a complex permission model unless later needed.

## Directory Tree

Users should be able to request a directory snapshot from within the terminal slash-command surface.

The daemon returns:
- workspace tree
- filtered by ignore rules where appropriate

This acts as a terminal-friendly equivalent of an IDE file explorer.

## Memory Management and Context Transfer

Arroba uses a dual memory model to support provider transfer and reduce repeated user instructions.

Memory scopes:
- short-term memory for immediate conversational/task continuity
- long-term memory for durable user/project guidance

Tracked items may include:
- recent transcript and task state
- context summaries/checkpoints
- persistent user-approved notes and constraints
- workspace state

Short-term memory resets when:
- provider compacts context
- user resets conversation
- provider-native session is effectively restarted

Long-term memory remains until user edits or removes it.

Arroba memory augments provider workflows but does not require or replace provider-native hidden state.

## Data Storage

Current assumption:
- server stores operational metadata
- prompts and model outputs may be relayed without persistence
- no strong requirement yet to store privacy-critical session content on the server

Operational metadata examples:
- users
- machines
- daemon instances
- workspaces
- worktrees
- sessions
- workflow definitions
- workflow runs
- workflow nodes and edges
- node runs and node messages
- worktree assignments
- attachments
- schedules
- provider run metadata
- prompt queue and session config state

If later features require persistence of content, storage design can be revisited.

## Suggested Core Data Model

Likely entities:

- User
- Machine
- DaemonInstance
- Workspace
- Worktree
- Session
- WorkflowDefinition
- WorkflowNode
- WorkflowEdge
- WorkflowRun
- NodeRun
- NodeMessage
- WorktreeAssignment
- AggregationState
- ProviderRun
- SessionAttachment
- PromptQueueItem
- SessionConfigState
- Schedule

Possible future entities:
- WorkspaceMembership
- SessionACL
- Checkpoint
- SessionEvent log
- Team roles

Useful conceptual fields:

### Machine
- id
- user_id
- hostname
- platform
- last_seen_at

### DaemonInstance
- id
- machine_id
- os_user
- version
- started_at
- status

### Workspace
- id
- user_id
- name
- repo_origin

### Worktree
- id
- workspace_id
- host_machine_id
- path
- branch
- base_branch
- status

### Session
- id
- workspace_id
- worktree_id
- host_machine_id
- host_daemon_id
- status
- active_provider_run_id
- execution_mode
- active_workflow_run_id
- created_at

### WorkflowDefinition
- id
- session_id
- topology_type
- coordinator_node_id
- version
- validation_policy

### WorkflowNode
- id
- workflow_definition_id
- node_key
- role
- provider
- model
- account_profile
- concurrency_group
- writes_code

### WorkflowEdge
- id
- workflow_definition_id
- from_node_id
- to_node_id
- edge_type
- routing_policy

### WorkflowRun
- id
- workflow_definition_id
- session_id
- status
- iteration_count
- max_iterations
- started_at
- completed_at

### NodeRun
- id
- workflow_run_id
- workflow_node_id
- status
- provider_run_id
- worktree_assignment_id
- started_at
- completed_at

### NodeMessage
- id
- workflow_run_id
- source_node_run_id
- target_node_run_id or target_node_id
- message_type
- handoff_payload
- delivery_status
- created_at

### WorktreeAssignment
- id
- workflow_run_id
- workflow_node_id or node_run_id
- worktree_id
- branch
- isolation_mode

### AggregationState
- id
- workflow_run_id
- workflow_node_id
- barrier_key
- expected_inputs
- received_inputs
- aggregation_policy
- status

### ProviderRun
- id
- session_id
- node_run_id
- provider
- account_profile
- model
- state

### SessionAttachment
- id
- session_id
- client_id
- transport_type
- connected_at
- last_seen_at

### PromptQueueItem
- id
- session_id
- source_attachment_id
- prompt_payload
- status
- submitted_at
- started_at
- completed_at

### SessionConfigState
- session_id
- version
- config_payload
- updated_by_attachment_id
- updated_at

### Schedule
- id
- session_id
- cron_expr
- prompt_template
- enabled
- last_run_at

## Component Diagram

Conceptual structure:

- Local CLI Client
  - terminal UI
  - slash-command help and completion UI

- Web App Client
  - xterm.js terminal
  - slash-command help and completion UI

- Arroba Daemon
  - session manager
  - PTY manager
  - provider adapter host
  - I/O collision manager
  - context tracker
- capability service
- git/worktree manager
- scheduler
- workflow prompt registry (workflow-level shared prompt)

- Fastify Server
  - auth
  - machine/session registry
  - relay (transport member, not a second kernel)
  - presence
  - schedules metadata

- Database
  - Prisma
  - SQLite initially
  - Postgres later

## Runtime Ownership Diagram

Conceptual ownership:

- Machine OS User
  - Arroba Daemon
    - Session A
      - Active provider run
      - Parked provider run(s)
      - Multiple attachments
      - Schedule bindings
    - Session B
    - Session C

One daemon hosts many sessions.
One session has many client attachments.
One session has one active provider run at a time.

## Command / Control Flow

1. User types an Arroba slash command
2. Client requests completions/help from the daemon as needed
3. User confirms a command or subcommand
4. Command is sent:
   - directly to daemon for local CLI
   - through server relay for remote client
5. Daemon executes capability or session action
6. Daemon updates session state
7. All attached clients receive updates in real time

## Provider Switch Flow

1. User selects switch provider
2. Arroba asks whether to transfer context
3. If yes, Arroba may first request provider memory-update signals, then prepares a transfer package from short-term + long-term memory
4. New provider run is started
5. Existing provider run becomes parked
6. If user returns quickly, parked run may resume
7. If not, old run may later terminate or remain parked depending on implementation/user choice

## Arroba-Driven Compaction Flow

1. User triggers `/compact`
2. Daemon requests provider compaction summary via `request_compaction_summary`
3. Daemon stores summary as memory/artifact input
4. Daemon launches fresh provider run with empty context window
5. Daemon warms new run with compaction summary + selected Arroba memory/workspace state

## Permission Flow

Current simplified flow:
1. Provider asks for permission
2. Request is surfaced through attached terminal(s)
3. User responds
4. Response is delivered back to provider

Arroba does not currently impose a complex provider-independent permission system.
The design goal is to preserve native provider behavior.

## Technology Stack

Monorepo:
- pnpm workspaces

Frontend:
- React
- TypeScript
- xterm.js (reference terminal behavior baseline for web/remote clients)

Daemon:
- Rust (required daemon implementation baseline for v1)

Backend:
- Fastify

Database:
- Prisma
- SQLite initially
- Postgres later

Transport:
- WebSockets

Local client communication:
- Unix socket on Unix-like systems
- named pipe on Windows

Cross-platform terminal consistency:
- platform-native clients are allowed
- terminal behavior should conform to shared protocol/conformance expectations so remote and local experiences remain consistent
- iOS: `WKWebView` for xterm.js-hosted terminal surfaces
- Android: `android.webkit.WebView` for xterm.js-hosted terminal surfaces
- macOS: `WKWebView` host for xterm.js terminal surfaces
- Windows: WebView2 host for xterm.js terminal surfaces
- Linux desktop: embedded Chromium/WebKit host for xterm.js terminal surfaces

## Development Philosophy

Arroba should:
- behave like the real provider CLI
- add orchestration, not replace provider behavior
- minimize interference with native provider workflows
- keep the daemon as runtime authority
- keep the server lightweight
- remain extensible to multi-machine and multi-user collaboration later

## Current Status

M0 foundations are complete.
M1 is complete.
M2 is now in progress.
The repository includes workspace scaffolding, a strict TypeScript server bootstrap, a shared domain package with contract tests, a workflow-oriented Prisma schema baseline, and a Rust daemon runtime with config/bootstrap wiring, in-memory session lifecycle management, shared attachment participation, daemon-owned prompt queueing/config propagation, explicit scheduler state, worktree-assignment-compatible session state, PTY-backed terminal fan-out, a local daemon smoke harness, and an initial structured shell command capability. Baseline CI coverage for TypeScript and Rust verification remains in place.

Related architecture docs:
- docs/spec-v1.md
- docs/ARCHITECTURE.md
- docs/PROTOCOL.md
- docs/ROADMAP.md
- docs/CONTRIBUTING.md
