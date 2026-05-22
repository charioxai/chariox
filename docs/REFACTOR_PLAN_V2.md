# Refactor Plan (New Design Components)

## Status

Living plan for refactoring the daemon to match the updated kernel component boundaries:

- **Scheduler**: workflow execution scheduling only (node readiness, graph progression).
- **Transport**: transport contract + prompt flow control (queue advancement, idle/timeout completion, cancellation/complete transitions).
- **I/O Collision Manager**: resource locking and conflict prevention across agents.

This plan tracks architecture-aligned extractions and boundary enforcement, not general cleanup.

## Phases

### Phase 1. Transport Flow Control Extraction

Goal:

- move prompt activity tracking and idle/settlement logic into transport
- ensure transport owns queue advancement triggers for prompt lifecycle

Status: **Complete**

Shipped:

- transport flow control module (`apps/kernel/src/transport/flow_control.rs`)
- prompt activity tracking moved behind transport flow control
- local prompt lifecycle paths updated to use transport flow control helpers

### Phase 2. Scheduler Boundary (Workflow Only)

Goal:

- scheduler owns workflow node scheduling only
- non-workflow prompt lifecycle routes through transport

Status: **Complete**

Shipped:

- scheduler reduced to `schedule_workflow_node_prompt` only
- local API prompt submission, completion, cancellation routed through transport
- kernel websocket prompt pumping routed through transport

### Phase 3. Provider Adapter / Transport Contract Clarification

Goal:

- keep provider adapters responsible only for translating provider events into structured signals
- keep transport responsible for flow control and prompt lifecycle transitions

Status: **Complete**

Shipped:

- provider adapters now translate provider-specific events into a shared transport-facing prompt signal batch
- `DaemonApp` prompt pumping no longer branches on OpenCode vs Codex result types
- transport/prompt lifecycle now consumes only generic output chunks, assistant completions, completion signals, idle signals, and notices

### Phase 4. Workflow Scheduler Simplification

Goal:

- isolate workflow graph progression policy from prompt lifecycle
- make workflow scheduler independent of transport idle heuristics

Status: **Complete**

Shipped:

- workflow scheduling, workflow prompt composition, workflow dispatch fanout, workflow completion snapshot building, and workflow control mailbox writing now live in `apps/kernel/src/scheduler/runtime.rs`
- workflow prompt injection is centralized in the scheduler prompt-injection renderer and is shared by local dispatch, remote dispatch, retry/replay paths, and tests
- workflow prompt lifecycle callbacks (`started/completed/cancelled`) are handled by scheduler runtime, not `DaemonApp`
- transport owns workflow prompt dispatch and cancellation cleanup
- `app/workflow_runtime.rs` is reduced to the workflow invocation entrypoint and delegates scheduling logic to scheduler runtime

### Phase 5. I/O Collision Manager Integration

Goal:

- centralize resource locking under the kernel collision manager
- ensure transport/scheduler consult locks for file and port writes

Status: **Planned**

Planned:

- introduce resource lock interface and worktree scoping
- wire lock checks into workflow-driven file writes

### Phase 6. MCP Runtime Tools

Goal:

- expose workflow runtime tools through one daemon-owned Arroba MCP surface
- automate MCP attachment for managed provider runs
- keep tool semantics provider-agnostic while leaving adapter-owned projection details per provider

Status: **Partially Complete**

Shipped:

- shared transport-owned runtime-tool dispatch reused by local APIs and MCP
- daemon-owned Arroba MCP server for runtime tools
- automated managed-run MCP attachment for supported Codex and OpenCode runs
- live-managed MCP path verified for:
  - `ack_workflow_turn`
  - `validate_workflow_handoff`
  - OpenCode workflow turns
  - Codex workflow turns
  - mixed-provider workflow runs
- structured workflow failure events on workflow runs
- default `notify` failure policy with mailbox routing to source and affected sink nodes
- structured audit retrieval through workflow runtime surfaces
- simplified resume for stopped active node runs using preserved turn envelopes
- CLI workflow inspector now surfaces runtime failure/audit state for the selected run/node
- live stop/resume drills now cover:
  - single-provider workflow runs
  - mixed-provider workflow runs

Still ahead before this phase can be considered release-ready:

- dynamic per-turn tool scoping
- stronger per-run MCP isolation/auth hardening
- MCP connection health/reconnect handling
- failure taxonomy refinement so user/runtime stops are not reported as provider failures

Related next architecture step:

- consolidate cancel/interrupt, workflow MCP runtime tools, and later canonical provider control operations under one transport-owned control subsystem
- keep mailboxing as scheduler-owned retry/control context, not as the interactive control lane
