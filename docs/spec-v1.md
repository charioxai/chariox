# Arroba v1 Specification

## Status

Draft v1.

This document defines the implementation target for Arroba v1. It is more specific than the high-level architecture summary in `agents/AGENTS.md` and is intended to guide code, schema, and protocol design.

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

## 5. Runtime Components

Arroba v1 has three runtime components:

- Client
- Daemon
- Server

### 5.1 Client

Clients are terminal interfaces that attach to daemon-managed sessions.

Examples:

- local CLI client
- web terminal client
- future desktop or mobile clients

Responsibilities:

- render the provider terminal stream
- render Arroba overlays and command palette UI
- send raw terminal input to the active provider PTY
- invoke daemon capabilities
- upload artifacts for transfer when requested
- show controller and observer state

### 5.2 Daemon

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

### 5.3 Server

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

## 6. Interaction Lanes

Arroba v1 has three interaction lanes between clients, daemon, and providers.

### 6.1 Terminal Lane

The terminal lane carries the raw provider PTY stream and user terminal input.

Properties:

- preserves native provider CLI behavior
- transports provider stdout, stderr, and terminal control sequences
- transports user keystrokes as terminal input
- is the default interaction path for ordinary user work

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

### 6.3 Control Lane

The control lane is a structured daemon to provider adapter boundary for coordination that cannot be modeled as raw terminal input.

In v1, the canonical control surface contains exactly one operation:

- `attach_file`

No other canonical control events or RPCs are part of v1.

Implications:

- context compaction is not a formal control-plane event in v1
- commit-description generation is not a formal control-plane request in v1
- provider functionality must not depend on the control lane except for enhanced attachment support when available

## 7. Sessions and Provider Runs

### 7.1 Session

A session is the top-level execution unit.

A session is bound to:

- one workspace
- one worktree
- one active provider run at a time

A session may have:

- multiple attached clients
- multiple parked provider runs
- scheduled jobs

Sessions do not move across workspaces in v1.

### 7.2 Provider Run

A provider run is one live native provider process.

States:

- active
- parked
- terminated

Switching providers:

1. The user requests a switch through the Arroba UI.
2. Arroba may prepare transfer context using local transcript or workspace state, but it must not rely on provider-private state.
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
- declaring whether provider file attachment is supported
- implementing the canonical `attach_file` control operation when supported

### 8.2 Canonical Control Operation

`attach_file` is the only structured provider control operation in v1.

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

### 8.3 Degradation Rule

If a provider adapter does not implement `attach_file`, the session still functions normally through PTY passthrough.

In that case Arroba must:

- store the transferred file in the session workspace or a session-scoped staging location
- surface the local path to the user
- avoid pretending the provider has received the file

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

## 11. Git and File Operation Requirements

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

## 12. Data and Storage Boundaries

### 12.1 Server-Stored Operational Metadata

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

### 12.2 Session Content

By default, prompts and model outputs should be relayed rather than persisted by the server.

If content persistence is added later, it should be treated as a separate design decision.

### 12.3 Artifacts

Session artifacts may include:

- uploaded files
- screenshots
- generated session-side files needed for workflows

Artifacts should be stored on the daemon host and referenced by metadata.

## 13. Failure and Compatibility Rules

The following rules are mandatory in v1:

- A provider must remain usable if the adapter only supports PTY launch and no control operations.
- `attach_file` failure must not terminate the provider run.
- Capability failures must be reported separately from provider terminal traffic.
- A lost remote client must not terminate the session by default.
- The daemon must remain the authority for controller and observer state.

## 14. Suggested Core Entities

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

## 15. Summary

Arroba v1 is defined by three lanes:

- terminal lane for raw provider-native interaction
- capability lane for Arroba-owned commands and workflows
- control lane for one narrow provider integration point: `attach_file`

This keeps Arroba faithful to the native CLI experience while still supporting practical daemon-owned features such as scheduling, screenshots, file transfer, git inspection, file operations, and attachment-aware workflows.
