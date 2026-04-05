# Control Channel Plan

## Purpose

This document defines the target first-class control subsystem for Arroba and reconciles it with the current code state.

Today, control behavior exists, but it is split across prompt lifecycle, workflow runtime tools, provider adapters, and scheduler mailboxing. The goal of this plan is to converge those pieces into one explicit transport-owned control channel.

## Target Model

Arroba should have one structured control subsystem with these properties:

- owned by transport
- separate from terminal byte traffic
- separate from workflow scheduling
- auditable
- adapter-projected to provider-native mechanisms
- available to workflow and non-workflow agent interactions

Suggested component:

- `apps/daemon/src/transport/control.rs`

Suggested service:

- `ControlService`

Responsibilities:

- define canonical control operations
- dispatch control requests
- route provider-facing control actions through adapters
- route runtime-facing control actions through runtime handlers
- record control events for audit
- expose capability/availability information

## Control Operation Classes

The control subsystem should group operations into three classes.

### 1. Provider Run Control

These control an active provider turn or run.

Examples:

- `interrupt_turn`
- `cancel_prompt`
- `stop_workflow_run_active_turn`
- later: `pause_run`, `resume_run` if provider semantics support them

These operations are currently real in code, but they are not modeled as first-class control operations.

### 2. Runtime Workflow Control

These are runtime-owned operations exposed to agents through MCP.

Examples:

- `ack_workflow_turn`
- `validate_workflow_output`

These are already first-class in behavior and are the most mature part of the current control surface.

### 3. Canonical Provider Integration Control

These are the provider-facing structured operations already described in the spec.

Examples:

- `attach_file`
- `request_memory_update`
- `request_compaction_summary`

These belong in the same subsystem even if some are not implemented yet.

## Boundaries

### Transport owns

- control operation definitions
- control dispatch
- control-lane audit events
- adapter projection
- MCP/runtime-tool exposure for control operations that agents can call directly

### Scheduler owns

- workflow graph progression
- mailbox routing decisions
- when a node should run again

Scheduler may request a control action, but it should not own the control subsystem.

### Provider adapters own

- mapping control operations to provider-native APIs
- declaring unsupported operations
- reporting structured control results/failures

### Terminal lane does not own control

Terminal traffic remains raw prompt/input/output.
Slash commands may resolve into control operations, but the control action itself belongs to the control subsystem, not to terminal byte flow.

## Current Code State

Current control-related behavior is split across several places.

### A. Prompt and run interruption

Main files:

- `apps/daemon/src/transport/mod.rs`
- `apps/daemon/src/app/prompt_lifecycle.rs`
- `apps/daemon/src/provider/service.rs`
- `apps/daemon/src/provider/codex_runtime.rs`

What exists:

- local/API prompt cancellation
- runtime prompt cancellation
- adapter-backed abort for structured providers
- prompt settlement handling

What is missing:

- explicit control-operation model
- unified audit surface for control actions
- transport-owned control service boundary

### B. Workflow runtime tools

Main files:

- `apps/daemon/src/transport/runtime_tools.rs`
- `apps/daemon/src/transport/mcp_server.rs`

What exists:

- first-class MCP tool exposure
- authenticated runtime tool dispatch
- structured results

What is missing:

- these tools live beside transport, but not under a broader control subsystem
- no unifying abstraction between runtime tools and provider-run control

### C. Workflow mailbox / control feedback

Main file:

- `apps/daemon/src/scheduler/runtime.rs`

What exists:

- structured failure routing
- mailbox injection into workflow-level prompt

What is missing:

- mailbox is control context, not interactive control
- it should remain scheduler-owned context, not become the control channel itself

### D. Spec-defined control lane

Docs already define canonical provider control operations, but code has not yet consolidated them into one subsystem.

## Gap Analysis

### Gap 1. No first-class control component

The codebase does not have a dedicated `control` module or `ControlService`.

Consequence:

- control semantics are fragmented
- capability negotiation is scattered
- future additions will continue to land in unrelated modules

### Gap 2. Prompt cancellation is operational, not modeled

`cancel_active_prompt` and provider abort paths are implemented, but they are expressed as prompt-lifecycle behavior rather than control operations.

Consequence:

- hard to reason about “commands to agents” as a single concept
- difficult to extend to other run-control operations cleanly

### Gap 3. Runtime tools and provider control are separate universes

`ack_workflow_turn` and `validate_workflow_output` are structured and first-class, but they do not live in the same abstraction as cancel/interrupt.

Consequence:

- the system has two partial control surfaces instead of one explicit one

### Gap 4. Audit is incomplete at the control-operation level

Workflow failure events exist, but control actions themselves do not yet have a unified event model.

Consequence:

- later operational debugging and policy work will be harder than necessary

## Recommended Target API

Suggested internal operation enum:

- `ControlOperation`
  - `InterruptTurn`
  - `CancelPrompt`
  - `AckWorkflowTurn`
  - `ValidateWorkflowOutput`
  - `AttachFile`
  - `RequestMemoryUpdate`
  - `RequestCompactionSummary`

Suggested result type:

- `ControlResult`
  - `accepted`
  - `completed`
  - `unsupported`
  - `failed`

Suggested event type:

- `ControlEvent`
  - operation
  - target session / provider run / workflow node run
  - source
  - timestamp
  - outcome
  - message

This should be transport-owned and auditable.

## Implementation Phases

### Phase C1. Create transport-owned control subsystem

Add:

- `apps/daemon/src/transport/control.rs`

Move or wrap:

- runtime tool dispatch
- prompt cancel/interrupt dispatch

Do not change behavior yet. First step is boundary consolidation.

### Phase C2. Re-express prompt cancellation as control operations

Wrap existing cancel paths behind `ControlService`.

Scope:

- `cancel_active_prompt`
- runtime cancel for workflow runs
- provider adapter abort mapping

### Phase C3. Move runtime tools under control subsystem

Keep MCP server where it is, but make it dispatch through `ControlService` rather than a separate runtime-tools-only path.

Result:

- one control abstraction for:
  - agent-called control
  - daemon-called control

### Phase C4. Add control audit events

Add structured control events so interrupts, cancellations, runtime tool calls, and later memory/compaction control requests are auditable.

### Phase C5. Land remaining canonical control operations

Implement or integrate:

- `attach_file`
- `request_memory_update`
- `request_compaction_summary`

These should not invent a second control mechanism.

## Non-Goals

This plan does not move mailboxing into the control subsystem.

Mailbox remains:

- scheduler-owned retry/control context
- prompt content for the next turn

It is related to control, but it is not the interactive control lane itself.

## Recommended Next Step

Start with Phase C1 only:

- introduce `transport/control.rs`
- define the control operation model
- route existing runtime tools and cancel/interrupt through it without changing user-visible behavior

That gives Arroba a real control-channel foundation before more control features are added.
