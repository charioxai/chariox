# Arroba v1 Specification

## Status

Draft v1.

This document defines the implementation target for Arroba v1. It is more specific than the high-level architecture summary in `agents/AGENTS.md` and is intended to guide code, schema, and protocol design.

Implementation baseline choices are documented in `docs/ARCHITECTURE.md` under **Implementation Choices (v1 baseline)**.
Daemon v1 implementation language baseline is Rust.

## 1. Product Definition

Arroba v1 is a daemon-centered session orchestrator for native AI coding CLIs.

It provides:

- native provider terminal passthrough
- daemon-owned capabilities invoked outside normal provider input
- lightweight provider integration where needed for file attachment
- local-first session hosting with optional remote attachment through a relay server

Arroba is a wrapper, not a replacement for provider CLIs. Provider-native behavior remains primary.

## 2. Goals

- Preserve the exact native terminal experience of supported providers.
- Allow multiple local or remote clients to attach to the same session.
- Let users invoke Arroba-specific actions through a command palette or overlay, not by typing special commands into the provider terminal.
- Support daemon-owned capabilities for shell, file, git, screenshot, scheduling, and transfer workflows.
- Support transferring a file to the daemon host and attaching it to the active provider when the provider supports attachment.
- Keep the provider control boundary intentionally small in v1.
- Add memory management so users do not need to repeatedly restate durable project context across runs, providers, or machines.

## 3. Non-Goals

- Replacing provider-native session state or hidden context mechanisms.
- Persisting full prompts or model outputs on the server by default.
- Building a provider-agnostic rich RPC surface beyond what v1 strictly needs.
- Requiring structured provider integration for a provider to function at all.

## 4. Core Principles

- Provider-native first: provider PTY behavior must remain intact.
- Minimal interference: Arroba should not reinterpret ordinary provider traffic.
- Daemon-centered runtime: the daemon is the source of truth for live session state.
- Local-first execution: sessions run on the user's machine.
- Graceful degradation: a provider without structured control support must still work through raw PTY passthrough.
- Cross-platform consistency: terminal behavior should be consistent across web, CLI, desktop, and mobile clients by following a shared terminal protocol/conformance profile.

## 5. Runtime Components

Arroba v1 has four runtime components:

- Client
- Machine
- Daemon
- Server

### 5.1 Client

Clients are terminal interfaces that attach to daemon-managed sessions.

Examples:

- local CLI client
- web terminal client
- future desktop or mobile clients
- third-party messaging clients (for example Telegram, Discord, Slack, or WhatsApp adapters)

Responsibilities:

- render the provider terminal stream
- render Arroba overlays and command palette UI
- send raw terminal input to the active provider PTY
- invoke daemon capabilities
- upload artifacts for transfer when requested
- show controller and observer state

### 5.2 Machine

A machine is a host where Arroba can run agent workloads through its daemon.

Properties:

- each machine has one daemon per OS user account
- a user may register and use multiple machines for the same Arroba account
- machines are the execution hosts for session workspaces, provider processes, and artifacts

### 5.3 Daemon

There is one daemon per machine OS user account.

The daemon is responsible for:

- hosting sessions
- launching and parking provider runs
- managing PTYs
- managing client attachments
- executing capabilities
- tracking worktrees and git state
- running scheduled jobs
- coordinating file transfer and file attachment

The daemon is the source of truth for live runtime state.

### 5.4 Server

The server is intentionally lightweight.

Responsibilities:

- authentication
- machine registry
- session discovery
- WebSocket relay
- presence tracking
- controller lease tracking
- schedule metadata storage
- operational metadata storage

The server should not depend on interpreting user content.

Security boundary requirement:

- the server relays encrypted payloads and should not require plaintext access to user-generated content
- end-to-end encryption keys are session-scoped so each session has an isolated cryptographic context

## 6. Interaction Lanes

Arroba v1 has three interaction lanes between clients, daemon, and providers.

### 6.1 Terminal Lane

The terminal lane carries the raw provider PTY stream and user terminal input.

Properties:

- preserves native provider CLI behavior
- transports provider stdout, stderr, and terminal control sequences
- transports user keystrokes as terminal input
- is the default interaction path for ordinary user work

Transmission requirement:

- user-generated information sent through this lane (for example prompts and terminal-entered content) must be protected with session-scoped end-to-end encryption whenever it traverses remote transport

Arroba must not require terminal traffic to be parsed into structured commands.

### 6.2 Capability Lane

The capability lane is used for Arroba commands invoked from the overlay or command palette.

Capabilities are executed by the daemon, not typed into the provider terminal.

The capability lane is used for:

- schedule management and execution
- screenshots
- file transfer into the daemon host
- git and worktree inspection
- directory tree display
- file view
- file edit flows
- shell command execution

Transmission requirement:

- user-generated capability payloads (for example uploaded files, prompt templates, and edit instructions) must be transmitted with session-scoped end-to-end encryption when crossing client, server, and daemon boundaries

### 6.3 Control Lane

The control lane is a structured daemon to provider adapter boundary for coordination that cannot be modeled as raw terminal input.

In v1, the canonical control surface contains three operations:

- `attach_file`
- `request_memory_update`
- `request_compaction_summary`

No other canonical control events or RPCs are part of v1.

`request_memory_update` is a daemon-owned control event used to gather memory-relevant signals from the active provider run (for example after provider-side compaction or reset).
`request_compaction_summary` is a daemon-owned control event used during Arroba-triggered compaction to request a compaction summary from the active provider run before warm-starting a fresh run.

Implications:

- memory-update inquiries are formal control-plane events in v1 and are distinct from normal user prompt/response traffic
- commit-description generation is not a formal control-plane request in v1
- provider functionality must not depend on the control lane except for enhanced attachment support, memory-update coordination, and Arroba-driven compaction coordination when available

## 7. Sessions and Provider Runs

### 7.1 Session

A session is the top-level execution unit.

A session is bound to:

- one workspace
- one worktree
- one active provider run at a time
- a set of eligible agent machines, with one active execution host at a time

A session may have:

- multiple attached clients
- multiple parked provider runs
- multiple eligible agent machine options (local or remote)
- scheduled jobs

Sessions do not move across workspaces in v1.

A session can be reassigned between its eligible agent machines over time, but only one machine hosts the active provider run at any moment.

### 7.2 Provider Run

A provider run is one live native provider process.

States:

- active
- parked
- terminated

Switching providers:

1. The user requests a switch through the Arroba UI.
2. Arroba prepares a transfer package from Arroba-managed memory (short-term and long-term) plus workspace state, and it must not rely on provider-private state.
3. A new provider process is launched.
4. The old provider run may be parked.
5. The user may resume a parked run later if supported by the provider process model.

Provider switching must remain minimally intrusive.

## 8. Attachments and Provider Adapter Model

### 8.1 Provider Adapter Requirement

Each supported provider has an adapter owned by Arroba.

The adapter is responsible for:

- launching the provider process
- exposing PTY integration details
- declaring whether provider file attachment and memory-update control are supported
- implementing canonical control operations when supported

### 8.2 Canonical Control Operations

v1 defines three structured provider control operations: `attach_file`, `request_memory_update`, and `request_compaction_summary`.

#### 8.2.1 `attach_file`

Inputs:

- session identifier
- provider run identifier
- absolute path to the transferred file on the daemon host
- optional attachment metadata such as display name or mime type

Expected behavior:

- if supported, the adapter requests the provider to import, attach, or otherwise reference the file in the active run
- if unsupported, the adapter returns a structured unsupported result without breaking the session

Outputs:

- success with provider-specific attachment reference if available
- unsupported
- failed with error details suitable for user-facing reporting

#### 8.2.2 `request_memory_update`

Purpose:

- allow the daemon to initiate an out-of-band memory-management inquiry to the active provider run
- collect memory-relevant signals when the provider has compacted, reset, or otherwise changed usable context

Inputs:

- session identifier
- provider run identifier
- reason code (for example `compaction_detected`, `user_requested_refresh`, `before_provider_switch`)
- optional policy hints indicating what the daemon is requesting (for example recency summary only vs full memory update)

Expected behavior:

- request is treated as control-lane traffic, not ordinary terminal prompt traffic
- provider adapter returns structured memory update payloads or a structured unsupported result
- failure or unsupported results do not terminate the provider run

Outputs:

- success with structured memory update payload for Arroba short-term/long-term memory pipelines
- unsupported
- failed with error details suitable for user-facing reporting

#### 8.2.3 `request_compaction_summary`

Purpose:

- allow the daemon to request a model-authored compaction summary during Arroba-driven user-triggered compaction
- produce a summary artifact suitable for warming a fresh provider run with an empty context window

Inputs:

- session identifier
- provider run identifier
- compaction intent (`user_triggered_arroba_compact`)
- optional output policy hints (for example target length or required headings)

Expected behavior:

- request is treated as control-lane traffic, not ordinary terminal prompt traffic
- provider adapter returns structured compaction summary payload or structured unsupported result
- failure or unsupported results do not terminate provider run

Outputs:

- success with structured compaction summary payload
- unsupported
- failed with error details suitable for user-facing reporting

### 8.3 Degradation Rule

If a provider adapter does not implement `attach_file`, `request_memory_update`, and/or `request_compaction_summary`, the session still functions normally through PTY passthrough.

In that case Arroba must:

- store the transferred file in the session workspace or a session-scoped staging location
- surface the local path to the user
- avoid pretending the provider has received the file
- continue memory transfer using Arroba-managed memory sources without requiring provider-side memory update signals
- if compaction summary is unsupported, allow Arroba-driven compaction using Arroba-managed memory snapshots as fallback warm-up

## 9. Capability Catalog

The following capabilities are first-class in v1.

### 9.1 Shell Command

Runs a subprocess scoped to the session workspace or worktree.

Requirements:

- must not mutate daemon process state implicitly
- must capture output for client display
- must record command metadata for the session runtime state

### 9.2 Directory Tree

Returns a terminal-friendly directory snapshot.

Requirements:

- scoped to the session workspace or worktree
- respects ignore rules where appropriate
- optimized for fast inspection rather than full filesystem indexing

### 9.3 View File

Returns a read-only terminal-friendly file view.

Requirements:

- line-oriented rendering
- supports large files through paging or chunking

### 9.4 Edit File

Runs an Arroba-managed file edit flow.

Requirements:

- initiated through the overlay or command palette
- applied to workspace files, not daemon internals
- able to report diffs or change summaries back to the client

### 9.5 Screenshot

Captures a screenshot on the daemon or agent host.

Requirements:

- the resulting artifact is stored as a session artifact
- the client can inspect, download, or forward the artifact

### 9.6 Git and Worktree Info

Provides session-relevant git state.

Minimum fields:

- repository path
- worktree path
- branch
- base branch if configured
- dirty state
- ahead and behind status when available
- relevant worktree list when useful

### 9.7 File Transfer

Transfers a client-side file to the daemon host.

Requirements:

- the transfer is associated with a session
- the daemon stores the file in a deterministic session-visible location
- the stored artifact can optionally be passed to `attach_file`

### 9.8 Attach Transferred File

This is a compound capability built on top of file transfer plus the control lane.

Flow:

1. Client uploads file to the daemon.
2. Daemon stores the file locally.
3. Daemon invokes provider adapter `attach_file`.
4. The result is surfaced to the user.

If provider attachment is unsupported, the local stored path is surfaced instead.

### 9.9 Compact Context

This capability is triggered by an Arroba command: `<reserved character for arroba commands>compact`.

It is user-triggered and daemon-orchestrated.

Flow:

1. User triggers `<reserved character for arroba commands>compact`.
2. Daemon invokes provider adapter `request_compaction_summary` on the active run.
3. Daemon stores the returned summary as a compaction artifact/memory input.
4. Daemon starts a fresh provider run with an empty context window.
5. Daemon warms the new run using the compaction summary plus Arroba-selected memory/workspace state.
6. Previous run is parked or terminated according to session policy.

If `request_compaction_summary` is unsupported, Arroba falls back to Arroba-managed memory summaries and still allows fresh-run warm-up.

## 10. Scheduling Model

Schedules are daemon-owned jobs bound to a session.

Schedules are stored as session metadata and execute only while:

- the daemon is online
- the session exists
- the workspace and worktree remain available

v1 schedule execution types:

- send a prompt into the active provider terminal workflow
- run an Arroba capability
- run a small workflow composed of Arroba steps

Example workflow shapes:

- run shell command
- inspect git status
- request user-visible approval if required by policy
- perform commit or other git operation

The schedule system belongs to the capability lane, not the control lane.

## 11. Memory Management and Context Transfer

Arroba v1 memory management is designed to reduce repeated user instructions while staying compatible with provider-native behavior.

### 11.1 Dual Memory Model

Arroba maintains two complementary memory scopes per session:

- short-term memory for immediate conversational and task continuity
- long-term memory for durable user/project guidance that should persist across provider switches and machine reassignment

### 11.2 Short-Term Memory

Short-term memory captures recent working context for high-fidelity continuation.

Typical contents:

- recent transcript window
- current task state and in-progress decisions
- latest workspace or git signals relevant to the active task

Lifecycle:

- updated continuously during active session work
- reset or compacted when provider context is reset/compacted or when user explicitly clears session recency

### 11.3 Long-Term Memory

Long-term memory stores durable context that users should not need to repeat.

Typical contents:

- project preferences and stable conventions
- recurring constraints, architecture guardrails, and team expectations
- user-approved persistent notes relevant to future tasks

Lifecycle:

- persisted as session-associated or workspace-associated memory records
- editable, reviewable, and removable by the user
- transferred across eligible agent machines for the same session through encrypted session transport

### 11.4 Context Transfer Package

When context transfer is requested (for provider switch, machine reassignment, or resumed work), Arroba composes a transfer package from:

- selected short-term memory snapshot
- relevant long-term memory entries
- current workspace state

Requirements:

- transfer package generation is deterministic and auditable at the Arroba layer
- users can inspect or constrain what long-term memory is included
- transfer data remains encrypted in transit under per-session end-to-end encryption rules
- daemon may trigger `request_memory_update` before package generation to refresh memory state after provider-side compaction/reset signals
- for Arroba-driven compaction, daemon may trigger `request_compaction_summary` and use the output as warm-up context for a fresh run

### 11.5 Boundaries

Memory management must follow these boundaries:

- Arroba memory augments, but does not replace, provider-native hidden session state
- provider internals are not required for Arroba memory continuity
- users must be able to clear short-term and long-term memory independently

## 12. Git and File Operation Requirements

Git and file inspection features are daemon responsibilities in v1.

### 11.1 Show Git Worktree

Must support:

- branch
- base branch if known
- path
- dirty state
- ahead and behind if available
- useful related worktree information

### 11.2 Show Directory Tree

Must support:

- workspace-scoped snapshot
- filtering by ignore rules
- terminal-oriented display

### 11.3 View File

Must support:

- read-only rendering
- sensible paging for long files

### 11.4 Edit File

Must support:

- daemon-mediated file modification flow
- clear user-facing confirmation of what changed

## 13. Data and Storage Boundaries

### 13.1 Server-Stored Operational Metadata

The server may store:

- users
- machines
- daemon instances
- workspaces
- worktrees
- sessions
- attachments
- controller lease state
- schedule metadata
- provider run metadata
- artifact metadata

### 13.2 Session Content

By default, prompts and model outputs should be relayed rather than persisted by the server.

All user-generated session content in transit (including prompts, model-visible instructions, uploaded files, and equivalent payloads) must use per-session end-to-end encryption so intermediary relay infrastructure does not require plaintext access.

If content persistence is added later, it should be treated as a separate design decision.

### 13.3 Artifacts

Session artifacts may include:

- uploaded files
- screenshots
- generated session-side files needed for workflows

Artifacts should be stored on the daemon host and referenced by metadata.

## 14. Failure and Compatibility Rules

The following rules are mandatory in v1:

- A provider must remain usable if the adapter only supports PTY launch and no control operations.
- `attach_file` failure must not terminate the provider run.
- `request_memory_update` failure must not terminate the provider run.
- `request_compaction_summary` failure must not terminate the provider run.
- Capability failures must be reported separately from provider terminal traffic.
- A lost remote client must not terminate the session by default.
- The daemon must remain the authority for controller and observer state.

## 15. Suggested Core Entities

Likely entities for v1:

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
- Artifact

## 16. Summary

Arroba v1 is defined by three lanes:

- terminal lane for raw provider-native interaction
- capability lane for Arroba-owned commands and workflows
- control lane for three narrow provider integration points: `attach_file`, `request_memory_update`, and `request_compaction_summary`

This keeps Arroba faithful to the native CLI experience while still supporting practical daemon-owned features such as scheduling, screenshots, file transfer, memory-aware context transfer, git inspection, file operations, and attachment-aware workflows.
