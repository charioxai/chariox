# Chariox v1 Capability and Evolution Specification Details

Extracted from [spec-v1.md](spec-v1.md) to keep the main v1 specification below the line cap while preserving detailed capability, scheduling, memory, storage, failure, and entity notes.

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

Runs a Workspace Live Sync file edit flow.

Requirements:

- initiated through a Chariox slash command such as `/edit`
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

This capability is triggered by the Chariox slash command `/compact`.

It is user-triggered and daemon-orchestrated.

Flow:

1. User triggers `/compact`.
2. Daemon invokes provider adapter `request_compaction_summary` on the active run.
3. Daemon stores the returned summary as a compaction artifact/memory input.
4. Daemon starts a fresh provider run with an empty context window.
5. Daemon warms the new run using the compaction summary plus Chariox-selected memory/workspace state.
6. Previous run is parked or terminated according to session policy.

If `request_compaction_summary` is unsupported, Chariox falls back to Chariox-managed memory summaries and still allows fresh-run warm-up.

## 10. Scheduling Model

Schedules are daemon-owned jobs bound to a session.

Schedules are stored as session metadata and execute only while:

- the daemon is online
- the session exists
- the workspace and worktree remain available

v1 schedule execution types:

- send a prompt into the active provider terminal workflow
- run a Chariox capability
- run a small workflow composed of Chariox steps

Example workflow shapes:

- run shell command
- inspect git status
- request user-visible approval if required by policy
- perform commit or other git operation

The schedule system belongs to the capability lane, not the control lane.

## 11. Memory Management and Context Transfer

Chariox v1 memory management is designed to reduce repeated user instructions while staying compatible with provider-native behavior.

### 11.1 Dual Memory Model

Chariox maintains two complementary memory scopes per session:

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

When context transfer is requested (for provider switch, machine reassignment, or resumed work), Chariox composes a transfer package from:

- selected short-term memory snapshot
- relevant long-term memory entries
- current workspace state

Requirements:

- transfer package generation is deterministic and auditable at the Chariox layer
- users can inspect or constrain what long-term memory is included
- transfer data remains encrypted in transit under per-session end-to-end encryption rules
- daemon may trigger `request_memory_update` before package generation to refresh memory state after provider-side compaction/reset signals
- for Chariox-driven compaction, daemon may trigger `request_compaction_summary` and use the output as warm-up context for a fresh run

### 11.5 Boundaries

Memory management must follow these boundaries:

- Chariox memory augments, but does not replace, provider-native hidden session state
- provider internals are not required for Chariox memory continuity
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
- workflow definitions
- workflow runs
- workflow nodes and edges
- node runs and node messages
- worktree assignments
- aggregation state metadata
- attachments
- queued prompt and session config metadata
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
- structured node completion reports
- structured handoff payload artifacts when retained for audit or replay

Artifacts should be stored on the daemon host and referenced by metadata.

## 14. Failure and Compatibility Rules

The following rules are mandatory in v1:

- A provider must remain usable if the adapter only supports PTY launch and no control operations.
- `attach_file` failure must not terminate the provider run.
- `request_memory_update` failure must not terminate the provider run.
- `request_compaction_summary` failure must not terminate the provider run.
- Capability failures must be reported separately from provider terminal traffic.
- A lost remote client must not terminate the session by default.
- The daemon must remain the authority for prompt queue state and effective session config state.
- The daemon must remain the authority for workflow scheduling, node state, and inter-agent routing.
- Workflow failures, retries, and termination policies must be explicit daemon-owned runtime decisions.
- Circular and hierarchical topologies must be implemented as policies over a generic workflow engine.

## 15. Suggested Core Entities

Likely entities for v1:

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
- Artifact

## 16. Summary

Chariox v1 is defined by three lanes:

- terminal lane for raw provider-native interaction
- capability lane for Chariox-owned commands and workflows
- control lane for three narrow provider integration points: `attach_file`, `request_memory_update`, and `request_compaction_summary`

This keeps Chariox faithful to the native CLI experience while still supporting practical daemon-owned features such as scheduling, screenshots, file transfer, memory-aware context transfer, git inspection, file operations, and attachment-aware workflows.

In workflow mode, Chariox extends that same daemon-owned model to multi-agent execution through a generic graph runtime, structured handoffs, explicit worktree isolation, and coordinator-driven completion decisions.
