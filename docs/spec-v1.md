# Arroba v1 Specification

## Status

Draft v1.

This document defines the implementation target for Arroba v1. It is more specific than the high-level architecture summary in `agents/AGENTS.md` and is intended to guide code, schema, and protocol design.

Terminology note:

- the docs now use `Arroba Kernel` as the architectural term for the runtime authority hosted by the `arroba-daemon` process
- the docs now use `workspace` as the target term for the persistent collaboration domain that is still mostly implemented as a `session` in the current code

Implementation baseline choices are documented in `docs/ARCHITECTURE.md` under **Implementation Choices (v1 baseline)**.
Daemon v1 implementation language baseline is Rust.
Current primary local CLI implementation baseline is TypeScript/OpenTUI, with the previous Rust-only CLI retained only as a phased-out compatibility fallback.

Current delivery sequence:

1. close the one-provider development cycle around `opencode`
2. finish local agent interactions for harnessing and multi-machine session behavior on that same OpenCode-first path
3. polish the TypeScript CLI as the reference client
4. add multi-platform clients on the same daemon/protocol model, starting with web and then iOS/Android
5. only after that, expand to additional providers such as Claude Code and Codex and harden the provider-generic adapter/protocol design

## 1. Product Definition

Arroba v1 is an Arroba-Kernel-centered orchestrator for native AI coding CLIs and compatible agent runtimes.

It provides:

- native provider terminal passthrough
- daemon-owned capabilities invoked outside normal provider input
- lightweight provider integration where needed for file attachment
- local-first workspace hosting with optional remote attachment through a relay server
- one Arroba Kernel owning a local runtime domain that may include both local and relay-attached clients or agents

Arroba is a wrapper around provider CLIs, not a replacement for their execution engines.
Arroba owns the slash-command surface and routes provider behavior through adapters.

## 2. Goals

- Preserve native provider PTY behavior for ordinary prompt and terminal work.
- Allow multiple local or remote clients to attach to the same workspace.
- Allow multiple local or remote runtime members, including clients and agents, to attach to the same kernel-owned runtime domain.
- Finish one provider deeply before expanding provider breadth.
- Support multiple top-level Arroba-managed agents inside one session, each with its own runtime context.
- Let users invoke Arroba actions through daemon-owned slash commands.
- Reserve `/agent ...` as the provider-specific command namespace exposed by Arroba.
- Support daemon-owned capabilities for shell, file, git, screenshot, scheduling, and transfer workflows.
- Support transferring a file to the daemon host and attaching it to the active provider when the provider supports attachment.
- Keep the provider control boundary intentionally small in v1.
- Add memory management so users do not need to repeatedly restate durable project context across runs, providers, or machines.
- Add daemon-owned workspace coordination so multi-agent edits can be integrated safely without assuming a human will manually resolve every conflict.

## 3. Non-Goals

- Replacing provider-native session state or hidden context mechanisms.
- Persisting full prompts or model outputs on the server by default.
- Building a provider-agnostic rich RPC surface beyond what v1 strictly needs.
- Requiring structured provider integration for a provider to function at all.

## 4. Core Principles

- Provider-native PTY first: provider terminal behavior must remain intact for ordinary non-command traffic.
- Prefer the strongest provider-native contract: when a provider exposes a stable local structured protocol, Arroba should prefer that protocol over PTY-derived heuristics for prompt lifecycle, output ordering, and command discovery.
- Slash-command ownership: Arroba owns `/...` command parsing and completion.
- Kernel-centered runtime: the Arroba Kernel is the source of truth for live workspace state.
- Node-centered routing: the kernel is the source of truth for routing between local and remote members attached to the same runtime domain.
- Local-first execution: workspaces run on the user's machine.
- Relay is transport, not authority: relay infrastructure may forward connections but must not become the authority for session/runtime state.
- Directory is discovery, not relay: directory/discovery and relay/transport are separate concerns.
- Graceful degradation: a provider without structured control support must still work through raw PTY passthrough.
- Cross-platform consistency: terminal behavior should be consistent across web, CLI, desktop, and mobile clients by following a shared terminal protocol/conformance profile.
- OpenCode-first sequencing: v1 should finish the full local development loop around `opencode` before broadening supported provider families.
- Future-compatible abstraction without premature breadth: daemon, protocol, and adapter boundaries must stay compatible with later providers and clients, but OpenCode correctness and end-to-end UX take priority over early provider-generalization work.

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
- future web terminal client after the CLI is polished
- future desktop or mobile clients after the web path proves out
- third-party messaging clients (for example Telegram, Discord, Slack, or WhatsApp adapters)

Responsibilities:

- render the provider terminal stream
- render focused-agent state and, when a session contains multiple top-level agents, render per-agent history/runtime views without making the client the runtime authority
- render Arroba slash-command help, completions, warnings, and command results
- send terminal keystrokes or structured prompt/config actions to the daemon through the appropriate surface
- invoke daemon capabilities
- upload artifacts for transfer when requested
- show queue/config/session state reported by the daemon

### 5.2 Machine

A machine is a host where Arroba can run agent workloads through its daemon.

Properties:

- each machine has one daemon per OS user account
- a user may register and use multiple machines for the same Arroba account
- machines are the execution hosts for session workspaces, provider processes, and artifacts

### 5.3 Arroba Kernel

There is one Arroba Kernel per machine OS user account, hosted by the `arroba-daemon` process.

The kernel is responsible for:

- hosting workspaces
- routing workspace events and prompt lifecycle to attached local or remote members
- launching and parking provider runs
- managing PTYs
- managing client attachments
- managing agent endpoint attachments and bindings
- executing capabilities
- tracking worktrees and git state
- running scheduled jobs
- coordinating file transfer and file attachment
- coordinating workspace claims, worktree allocation, and integration safety for top-level agents

The kernel is the source of truth for live runtime state.

Node membership model:

- a member of a kernel-owned runtime domain may be local or remote
- local and remote members attached to the same kernel are in the same runtime domain
- remote attachment is a transport detail; it does not create a second session authority

### 5.4 Server

The server is intentionally lightweight.

Responsibilities:

- authentication
- machine registry
- session discovery
- WebSocket relay
- presence tracking
- queued prompt and config-state metadata when server-side operational metadata is needed
- schedule metadata storage
- operational metadata storage

The server should not depend on interpreting user content.

Near-term architectural role:

- the server should evolve into relay infrastructure for kernel, client, and remote-agent connectivity
- same-kernel remote connections should ideally preserve the illusion of direct kernel membership even when the physical path is relayed
- directory and federation concerns remain later work and should stay intentionally lightweight in the current implementation phase

Security boundary requirement:

- the server relays encrypted payloads and should not require plaintext access to user-generated content
- end-to-end encryption keys are session-scoped so each session has an isolated cryptographic context

## 6. Interaction Lanes

Arroba v1 has three interaction lanes between clients, daemon, and providers.

### 6.1 Terminal Lane

The terminal lane carries the raw provider PTY stream and user terminal input.

Properties:

- preserves native provider CLI behavior for ordinary non-command traffic
- transports provider stdout, stderr, and terminal control sequences
- transports user keystrokes as terminal input when they are not intercepted as Arroba slash commands
- is the default interaction path for ordinary user work

Transmission requirement:

- user-generated information sent through this lane (for example prompts and terminal-entered content) must be protected with session-scoped end-to-end encryption whenever it traverses remote transport

Arroba must not require ordinary non-command terminal traffic to be parsed into structured commands.

Provider-specific note:

- some providers MAY expose a richer local session/event API in addition to PTY traffic
- when that API is stable and supported by the adapter, Arroba MAY derive provider output, turn lifecycle, and command discovery from that structured surface instead of from PTY silence or screen scraping
- OpenCode is the current reference provider-specific use of this model

### 6.2 Capability Lane

The capability lane is used for daemon-owned Arroba commands invoked through the slash-command dispatcher.

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

The control lane may also carry `/agent ...` command invocations after Arroba resolves the active provider command catalog and target adapter behavior.

OpenCode-specific v1.1 target:

- OpenCode should use its local server/session/event protocol as the primary adapter contract
- prompt submission should map to OpenCode session operations rather than PTY writes
- turn completion should map to OpenCode session/message lifecycle signals rather than daemon idle timers
- PTY integration remains the fallback path for providers that do not expose a comparable structured surface
- OpenCode remains the reference provider for the current development cycle; later provider adapters must fit behind the same daemon/client contract rather than forcing that contract to be generalized prematurely

### 6.3.1 Node Transport Direction

Current implementation baseline:

- daemon-client communication is still local request/response IPC
- daemon-OpenCode communication already combines local HTTP control with an event stream

Target direction:

- daemon-client and daemon-agent communication should converge toward one daemon-owned bidirectional node protocol
- that protocol should support both local and relayed connections without changing session semantics
- current transport differences are implementation history, not a long-term architectural principle

### 6.4 Slash Command System

Arroba owns the slash-command namespace.

Required rules:

- `/...` is reserved for Arroba command dispatch and completion.
- `/agent ...` is the provider-specific namespace exposed by the active Arroba adapter.
- command completion is daemon-managed and may depend on session, provider, and attachment context.
- ordinary non-command input continues to flow through the terminal lane unchanged.

OpenCode-specific note:

- OpenCode command discovery SHOULD use machine-readable provider surfaces before falling back to shipped Arroba catalogs
- this includes provider-exposed command, agent, and skill listing where available

Provider command discovery policy:

- Arroba ships built-in provider command catalogs for explicitly supported provider version families.
- Arroba augments those catalogs by reading supported custom-command files or config locations when the provider supports that model.
- Arroba does not rely on scraping human-oriented `/help` output as the primary compatibility mechanism.
- if the detected provider version is unsupported, Arroba MUST warn the user but MUST still keep best-effort `/agent` completions enabled.

### 6.5 Agent-Scoped Extensions

Arroba manages provider-facing extensions as daemon-owned assets rather than treating project-local provider files as the authoritative source of truth.

Extension classes include:

- skills
- MCP server definitions
- custom command packs
- instruction packs
- hooks or provider-specific plugin-like assets

Required rules:

- extensions are installed once on the machine and bound per Arroba-managed top-level agent or provider run
- extension visibility defaults to the bound agent only
- Arroba materializes provider-specific extension views at launch time rather than requiring the project directory to be the canonical source of truth
- provider-facing generated files MAY be written for compatibility, but the daemon-owned registry remains authoritative
- top-level Arroba agents are the scoping boundary for extensions; provider-native subagents are not separately orchestrated by Arroba

Extension visibility policies MAY include:

- `agent_private`
- `session_shared`

## 7. Node and Agent Endpoint Model

Arroba should distinguish between the kernel's internal runtime model and provider-specific integration details.

### 7.1 Node Members

Possible members of the same kernel-owned runtime domain include:

- local CLI clients
- remote CLI clients attached through relay
- local agent endpoints
- remote agent endpoints attached through relay

All such members may participate in the same workspace/workflow domain when attached to the same kernel.

### 7.2 Agent Endpoint Modes

Arroba should support both:

- `managed` endpoints launched by the kernel
- `external` endpoints discovered/configured and connected to by the kernel

This allows:

- first-party local convenience integrations
- third-party agent runtimes Arroba does not ship itself
- remote agent runtimes attached through relay while still belonging to the same kernel-owned runtime domain

### 7.3 Workspace Coordination

Multi-agent orchestration must not assume human-mediated conflict resolution as the only safety mechanism.

The kernel should therefore evolve a workspace-coordination subsystem responsible for:

- branch/worktree allocation per top-level agent
- file or workspace claims
- mergeability and integration validation
- conflict detection before shared integration

The baseline coordination strategy should begin with worktree/branch isolation and file-level coordination before any more advanced region-level locking.

Scope rule:

- coordination is workspace-scoped, not repo-scoped across every workspace on a machine
- different workspaces may still collide later in the same way independent PRs can conflict
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
- one primary worktree in single-agent mode
- one active provider run at a time in single-agent mode
- a set of eligible agent machines, with one active execution host at a time

A session may have:

- multiple top-level Arroba-managed agents
- multiple attached clients
- multiple parked provider runs
- agent-scoped provider runs when multi-agent session mode or workflow mode is active
- agent-scoped histories and worktree assignments when multi-agent session mode or workflow mode is active
- multiple node-scoped provider runs when workflow mode is active and daemon resource policy allows it
- multiple worktree assignments when workflow mode is active
- multiple eligible agent machine options (local or remote)
- scheduled jobs

Sessions do not move across workspaces in v1.

A session can be reassigned between its eligible agent machines over time, but only one machine hosts the active provider run at any moment.

### 7.1.1 Session Agent Model

Arroba-managed top-level agents are first-class session entities in v1.

Required rules:

- a session MAY contain one or more top-level Arroba-managed agents
- each top-level agent MUST have its own stable agent id within the session
- each top-level agent SHOULD carry its own provider context, prompt target, history, and worktree-assignment metadata even when the initial implementation reuses shared session infrastructure
- the daemon MUST track a focused top-level agent for direct user interaction in multi-agent session mode
- direct prompt submission in multi-agent session mode MUST target the focused agent, not merely the session at large
- clients SHOULD make focused-agent changes visible in session chrome and in the main transcript/work area
- pane-based clients SHOULD render one visible sub-area per top-level agent when multiple agents are active in the session

Implementation note for the current codebase:

- the daemon and TypeScript CLI now expose a first real multi-agent session slice: `spawn`, `destroy`, `focus`, `list`, `cycle`, focused-agent prompt targeting, agent-scoped history metadata, and initial split-pane transcript behavior
- this slice still needs OpenCode-path stabilization, broader pane/layout completion, and multi-machine/runtime hardening before the one-provider development cycle can be considered closed

### 7.1.2 Session Lifecycle Semantics

Arroba sessions are intended to be persistent by default.

Required rules:

- detaching a client MUST NOT delete the session
- exiting a client SHOULD detach from the session rather than terminate it
- sessions SHOULD remain attachable after the last client disconnects until the user explicitly deletes them
- session deletion is an explicit user action and MUST NOT be implicit in ordinary client exit flows
- deleting a session MUST tear down provider runs, prompt/runtime state, and live attachments for that session

User-facing session operations in v1 SHOULD converge on:

- create
- list
- attach
- detach
- delete

The current daemon API still exposes an internal `end` operation, but the intended user-facing lifecycle is explicit session deletion rather than implicit session ending on CLI exit.

Deleting the currently attached session is valid. The client should transition to an unattached "no session" state rather than being forced to terminate the entire CLI process.

### 7.1.3 Session Identity and Aliases

Session references should become user-friendly and resumable.

Required rules:

- each session MUST have a stable primary id
- session ids SHOULD use short commit-like lowercase hexadecimal strings rather than `session-N` numeric labels
- each session MAY have one optional human-readable alias
- session references supplied by users MAY resolve by:
  - full session id
  - unique session-id prefix
  - alias
  - unique alias prefix
- if a provided reference is ambiguous, the daemon/client MUST reject it and require disambiguation instead of guessing

Recommended v1 default:

- 16-character lowercase hexadecimal ids
- exact alias match preferred over prefix match
- prefix resolution allowed only when unambiguous

Current implementation note:

- the current local implementation now uses 16-character lowercase hexadecimal ids with optional aliases
- alias matching is workspace-scoped and normalized to lowercase

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

### 7.2.1 Provider Authentication Model

Arroba uses a wrapper-style provider authentication model.

Required rules:

- Arroba launches native provider CLIs on the host machine and reuses their native local login state.
- Arroba MUST NOT require Anthropic API keys or other provider API credentials when the provider CLI already supports native end-user login/subscription access.
- Arroba MUST NOT store, mint, proxy, or relay provider credentials in v1.
- provider authentication remains local to the machine hosting the provider run.
- Arroba account/server authentication is separate from provider authentication.

Operational behavior:

- if a provider CLI is already logged in for the selected local profile, the daemon may launch it directly
- if a provider CLI is installed but not logged in, the daemon surfaces `not_logged_in` status and instructs the user to complete the provider-native login flow on that machine
- if the provider login has expired or become invalid, the daemon surfaces `expired` status and a reauthentication hint
- remote Arroba clients may observe and acknowledge provider-auth state, but the actual provider login flow remains provider-native on the host machine

`account_profile` semantics:

- `account_profile` selects a provider-native local account/config context
- it is not an Arroba-managed credential container
- adapters MAY map it to provider-specific config roots, profile names, or environment selections

### 7.2.1.1 Provider Rollout Order

Provider breadth is intentionally sequenced late.

Required rules:

- OpenCode is the only provider that needs to be fully closed end-to-end before provider expansion begins.
- Multi-provider abstractions in v1 MUST be designed so later adapters can fit cleanly, but they MUST NOT block finishing the OpenCode-first runtime, harnessing, multi-machine, and client UX work.
- Claude Code, Codex, and broader provider-generic adapter/protocol work come after the OpenCode-first local cycle and after the reference CLI plus multi-platform client surfaces have stabilized.

### 7.2.2 Provider-Native Subagents

Provider-native subagents are not first-class Arroba workflow agents in v1.

Required rules:

- Arroba orchestrates only top-level session agents or workflow nodes that it launches explicitly
- any subagents internally spawned by a provider run are treated as provider-owned implementation details
- Arroba MUST NOT require separate scheduling, extension binding, or worktree allocation per provider-native subagent by default
- provider-native subagents may use the skills, MCPs, commands, and instructions available to their parent top-level provider run

Future implementations MAY surface debug or telemetry metadata about provider-native subagents when a provider exposes stable support for that, but such subagents remain outside Arroba's orchestration model.

### 7.3 Workflow Layer Above Workspaces

Arroba v1 MUST support both manually directed multi-agent workspaces and a workflow layer above the single-agent workspace model.

Delivery priority inside v1:

- circular topology is the earlier implementation target
- hierarchical topology remains in scope for v1, but is expected to land later in v1 after the lower-level runtime and protocol foundations are stable

Normative rules:

- A workspace MAY run in single-agent mode, multi-agent manual mode, or multi-agent workflow mode.
- Multi-agent manual mode is user-directed: the user selects the active top-level agent and the kernel routes direct interaction to that agent.
- Multi-agent execution MUST be modeled as a general directed workflow graph.
- v1 validates only two workflow topologies:
  - circular
  - hierarchical
- The runtime MUST still be designed so future DAGs, bounded loops, conditional routing, richer aggregation, and more advanced topologies can be added without redesigning the core workflow engine.
- Contributors MUST NOT implement multi-agent behavior as topology-specific special cases scattered through unrelated codepaths.

### 7.4 Entry Node and Workflow Endpoint

Every workflow definition MUST have exactly one designed entry node.

Rules:

- initial input enters the workflow through the entry node
- the same logical workflow endpoint MAY be invoked by a terminal user or an external system
- the workflow should be agnostic to the source of the initial input once it has been normalized into the workflow message model

### 7.5 Inter-Agent Communication Contract

Inter-agent communication in workflow mode MUST be kernel-orchestrated.

Required rules:

- agents MUST NOT communicate directly
- the kernel MUST own all routing and message passing between nodes
- inter-agent communication MUST NOT be modeled as raw terminal transcript forwarding
- workflow progression MUST advance on kernel-owned message routing and turn state, not arbitrary provider turns

Message model:

- `message`
- `recipients`
- `artifacts`

Rules:

- the kernel should not impose richer semantic fields by default
- artifacts may be text, JSON, files, paths, URLs, images, or lists of those
- a sender may emit at most one message per recipient in one turn

### 7.5.1 Queue and Turn Semantics

Each workflow node/agent MUST have an inbound queue.

Rules:

- when an agent is idle and eligible, the kernel MAY start a turn
- at turn start, the agent MUST use a kernel-owned tool to consume the currently available queued inputs for that turn
- once a turn starts, the visible input set for that turn is fixed
- messages arriving while a turn is running remain queued for a later turn
- the running agent MUST NOT re-check the queue mid-turn

Required kernel tools:

- `consume_input_messages`
- `validate_output_messages`

### 7.5.2 Sync vs Async Workflow Execution

The workflow model should support both:

- `sync`
  - validated messages are delivered when the turn ends
- `async`
  - validated messages are delivered as soon as they are produced during the turn

In both modes, every delivered message is considered final. Arroba does not distinguish a separate intermediate-message type by default.

### 7.6 Circular Topology Rules

Circular topology is valid in v1 only if all are true:

- each node has exactly one incoming edge and one outgoing edge
- the last node connects back to the coordinator
- execution is serialized
- the workflow uses bounded iteration or round limits
- execution stops when either:
  - the max iteration or round limit is reached, or
  - the coordinator declares completion or stop

### 7.7 Hierarchical Topology Rules

Hierarchical topology is valid in v1 only if all are true:

- the workflow forms a rooted tree
- the coordinator is the root
- parent nodes may fan out to multiple children
- child branches may run in parallel
- parent fan-in waits for all children by default
- results propagate upward through structured aggregation
- the coordinator decides final stop or continue behavior

Implementation priority note:

- circular topology should be implemented and stabilized first
- hierarchical topology should follow later in v1 on top of the same generic workflow engine

### 7.8 Worktree Isolation Rules

Parallel code-writing branches MUST NOT share the same active worktree.

Required rules:

- in hierarchical workflows, each active code-writing branch or subtree SHOULD operate in an isolated worktree and git branch
- worktree assignment MUST be explicit in runtime state and the data model
- the daemon MUST NOT allow parallel code-writing agents to mutate the same worktree concurrently

## 8. Attachments and Provider Adapter Model

### 8.0 Shared Attachment Semantics

Attachments are shared session participants, not exclusive control roles.

Required rules:

- every attachment MAY submit prompts
- every attachment MAY request supported config changes
- the daemon MUST remain the single source of truth for prompt scheduling and effective session config state
- at most one prompt may execute at a time per single-agent session
- if a prompt arrives while another prompt is running, the daemon MUST enqueue it rather than dropping or interleaving it
- when a prompt is enqueued, the daemon MUST notify all other attachments in the session that a queued message exists and expose the canonical queue state
- attachments MUST be able to fetch canonical session state and daemon notices through a structured daemon-owned surface
- attachments MUST render daemon-owned queue and config state, not rely on locally assumed state

Scheduler/runtime boundary rule:

- the daemon MUST own explicit scheduler state for session work such as `idle`, `runnable`, `running`, and `waiting`
- queue advancement and prompt completion semantics MUST remain daemon-owned runtime decisions, even when a client triggers the completion action through a structured API
- for providers with structured turn lifecycle signals, the daemon SHOULD drive prompt completion from adapter-reported provider state rather than PTY output quiet windows

Config behavior:

- config changes that are safe during execution MAY be applied immediately
- config changes that are unsafe during execution MUST be rejected with an explicit busy-state error while a prompt is running
- after an accepted config change, the daemon MUST propagate the canonical updated config state to all session attachments

Worktree compatibility rule:

- even in single-agent mode, the runtime SHOULD keep explicit worktree-assignment metadata so future isolated branches/worktrees can extend the same runtime shape without redesign

### 8.1 Provider Adapter Requirement

Each supported provider has an adapter owned by Arroba.

The adapter is responsible for:

- launching the provider process
- exposing PTY integration details
- declaring whether provider file attachment and memory-update control are supported
- implementing canonical control operations when supported
- probing provider installation and authentication status before launch when possible
- probing the installed provider version
- selecting a shipped built-in command catalog for supported versions when available
- discovering custom commands from provider-supported files or config when available
- projecting daemon-managed extensions into provider-compatible runtime views when required

### 8.1.1 Extension Projection Requirement

Provider adapters MUST translate Arroba's daemon-owned extension bindings into the provider-specific shape expected by that provider runtime.

Examples:

- generated skills or command files under a provider-specific runtime view
- provider config overlays or environment variables
- scoped MCP configuration visible only to the selected top-level run

Isolation rule:

- if a provider supports custom config roots, home directories, or equivalent launch-time overrides, Arroba SHOULD use them to keep agent-scoped extension views isolated
- if a provider only supports project-local extension files, Arroba SHOULD rely on worktree or projected-workspace isolation to preserve per-agent visibility boundaries

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

Provider-specific structured adapter note:

- some adapters MAY expose richer provider-owned operations beyond the canonical v1 control trio when Arroba needs them for correctness
- OpenCode is expected to use provider-specific session operations for prompt submit, command invoke, turn abort, and event subscription
- those richer provider-specific operations remain adapter-internal and do not change Arroba's provider-agnostic user-facing model

In that case Arroba must:

- store the transferred file in the session workspace or a session-scoped staging location
- surface the local path to the user
- avoid pretending the provider has received the file
- continue memory transfer using Arroba-managed memory sources without requiring provider-side memory update signals
- if compaction summary is unsupported, allow Arroba-driven compaction using Arroba-managed memory snapshots as fallback warm-up

### 8.4 Provider Command Compatibility

Arroba MUST surface provider command compatibility status to the user.

Minimum compatibility state:

- detected provider name and version
- matched Arroba catalog version or version family when one exists
- support status (`supported` | `best_effort` | `unsupported_not_installed`)
- warning text when Arroba does not officially support the detected version

Compatibility rule:

- unsupported provider versions MUST NOT disable `/agent` completions by default
- instead, Arroba continues with best-effort completions from the nearest shipped catalog plus discovered custom commands and surfaces a warning that behavior may drift

### 8.5 Provider Login Procedure

Provider login remains provider-native.

Required rules:

- Arroba SHOULD prefer prompting the user to use the provider's normal CLI login flow rather than reimplementing login itself
- Arroba MAY surface provider-specific login instructions, but those instructions are advisory and adapter-owned
- Arroba MUST NOT treat provider login as a server-side or relay-side concern
- if the provider CLI supports multiple local profiles, the adapter MAY expose those through `account_profile`

Minimum provider auth states:

- `authenticated`
- `not_logged_in`
- `expired`
- `unknown`
- `provider_not_installed`

### 8.6 Extension and MCP Management

Arroba owns extension installation and binding in v1.

Required rules:

- skills, MCP definitions, command packs, and similar extension assets are installed through Arroba-managed flows rather than delegated to provider CLIs
- installation is machine-scoped; availability is determined by per-agent binding
- MCP servers are daemon-managed runtime components and MAY be launched or terminated independently of provider CLIs
- only the MCPs bound to a given top-level provider run should be exposed to that run
- extension installation metadata, compatibility state, and bindings should be inspectable through daemon-owned APIs

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

- initiated through an Arroba slash command such as `/edit`
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

This capability is triggered by the Arroba slash command `/compact`.

It is user-triggered and daemon-orchestrated.

Flow:

1. User triggers `/compact`.
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

Arroba v1 is defined by three lanes:

- terminal lane for raw provider-native interaction
- capability lane for Arroba-owned commands and workflows
- control lane for three narrow provider integration points: `attach_file`, `request_memory_update`, and `request_compaction_summary`

This keeps Arroba faithful to the native CLI experience while still supporting practical daemon-owned features such as scheduling, screenshots, file transfer, memory-aware context transfer, git inspection, file operations, and attachment-aware workflows.

In workflow mode, Arroba extends that same daemon-owned model to multi-agent execution through a generic graph runtime, structured handoffs, explicit worktree isolation, and coordinator-driven completion decisions.
