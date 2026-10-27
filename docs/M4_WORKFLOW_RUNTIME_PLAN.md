# M4 Workflow Runtime Plan

## Goal

Turn the current workflow editor into a real daemon-scheduled workflow runtime.

The first slice should focus on on-demand workflow runs started from existing workflow endpoints. Time-based schedules should come after the execution engine exists and is stable.

## Current Baseline

Already implemented:

- workflow definitions stored on the daemon session state
- workflow node, edge, and endpoint creation/removal/binding
- workflow and endpoint alias resolution
- workflow canvas rendering in the CLI
- manual multi-agent session runtime with top-level session agents
- `WorkflowRun`, `WorkflowNodeRun`, and `WorkflowMessage` runtime entities
- local API invoke/list/get/cancel flow for workflow runs
- daemon scheduling of the entry node for endpoint-triggered runs when an invocation prompt is present
- daemon-owned downstream routing that creates structured handoff messages and schedules downstream node prompts
- CLI `/workflow run`, `/workflow runs`, and `/workflow cancel` commands
- basic workflow canvas runtime visibility for the selected workflow and its nodes
- interval watchdogs with `skip` and `queue` policies

Not implemented yet:

- daemon runtime internals have been modularized:
  - `session/service` is split into focused modules
  - `local/api` is split into module, types, and tests

- explicit per-node policy overrides (`input_gate` / `output_release`)
- richer run inspection/history UI beyond the current selected-workflow status view
- full cron syntax for recurring schedules
- daemon-owned provider process inspection/teardown is landed

## Key Gaps To Resolve First

### 1. Reconcile the workflow model

The long-form architecture/spec docs describe a richer workflow model than the runtime currently stores. The current daemon workflow definition is still only:

- workflow id and optional alias
- nodes that point to existing session agents
- directed edges
- endpoints bound to one entry node

Before execution work starts, we should lock the v1 runtime model for:

- workflow runs
- node runs
- node input queues / handoff messages
- run and node statuses
- execution validation rules

### 2. Separate two meanings of scheduling

There are two different scheduling problems:

- execution scheduling: deciding which workflow node runs next
- wall-clock scheduling: cron-like or recurring invocation of a workflow

The next milestone should implement execution scheduling first. Wall-clock schedules should stay out of scope until manual workflow runs are working end to end.

## Recommended Delivery Order

### Phase 1. Runtime model and protocol surface

Add daemon/runtime types for:

- `WorkflowRun`
- `WorkflowRunStatus`
- `WorkflowNodeRun`
- `WorkflowNodeRunStatus`
- `WorkflowMessage` or equivalent structured handoff payload

Add local API surface for:

- invoke workflow endpoint
- list workflow runs
- get workflow run
- cancel workflow run

Add pushed runtime events/notices for:

- run started
- node started
- node completed
- message routed
- run completed
- run failed
- run cancelled

### Phase 2. Manual run creation from endpoints

Add the first user-facing command set:

- `/workflow run <workflow-ref> <endpoint-ref> [prompt]`
- `/workflow runs [workflow-ref]`
- `/workflow cancel <run-ref>`

Rules for the first slice:

- a run starts only from an existing endpoint
- endpoint validation happens before any agent work begins
- missing-agent nodes or invalid endpoint targets fail fast with explicit errors

Status:

- daemon-side endpoint invocation is landed through the local API
- CLI slash-command wiring is landed for run/runs/cancel

### Phase 3. First execution scheduler

Build the daemon-owned execution scheduler on top of the existing top-level session-agent runtime.

The first scheduler slice should be intentionally narrow:

- on-demand runs only
- one active turn per agent
- no retries
- no cron/recurring schedules
- no attempt to support every graph shape on day one

Recommended initial execution policy:

- support linear and DAG workflows first
- defer cycles until bounded-iteration policy exists
- keep `output_release = on_completion` until incremental output emission exists

Status:

- the first scheduler slice is landed for the entry node, simple downstream routing, and explicit join-node buffering
- endpoint invocation now submits a workflow-owned prompt onto the existing prompt queue and auto-launches a provider run for the bound agent if needed
- node completion now creates one structured handoff message per outgoing edge, stores those messages on the target side, and consumes messages into downstream node runs; nodes marked `wait_for_all_inputs` consume one synchronized same-iteration set from every incoming source
- runs become `Completed` when no downstream work remains, or `Running`/`Waiting` as downstream node work is scheduled

### Phase 4. Node completion and handoff contract

Define how a node turn tells the daemon:

- what outputs to route
- which downstream nodes should receive them
- whether the node considers itself complete, failed, or asking to stop

This contract must be daemon-owned and machine-parseable. Do not rely on ad hoc natural-language parsing for workflow control.

At minimum the daemon needs a daemon-owned, machine-parseable completion payload with:

- summary
- optional explicit output message
- optional artifact references when a node explicitly produces them
- source workflow/node/run references
- enough generic metadata to let downstream nodes continue without relying on ad hoc prose parsing

Status:

- the daemon now derives a human-facing summary for a completed node from actual provider output when that output is available
- workflow-owned prompts now instruct the node to emit a machine-parseable JSON envelope with separate `summary` and explicit downstream `output.message`
- workflow-owned prompts can include an optional workflow-level prompt shared across nodes
- the daemon includes optional artifact refs in the output payload by scanning workflow-owned artifact roots namespaced to the workflow source attachment
- downstream handoff payloads now include workflow run id, workflow id, source node run id, source node id, source agent id, target node id, the root invocation prompt, and the optional summary-plus-output completion payload
- completed node runs now persist the same summary-plus-output completion payload instead of treating summary itself as the downstream payload
- completion routing is machine-owned rather than parsed from model prose
- audit transcript remains in session history for later inspection and is intentionally not forwarded downstream as workflow output
- typed/domain-specific payloads are intentionally still out of scope
- richer completion data such as changed files and explicit stop recommendations is still pending

### Phase 4.1 Node-Level Release/Gating Policy

Execution policy should be modeled per node, not as a user-declared workflow topology mode.

Target model:

- `input_gate`
  - `first_input`
  - `all_inputs`
- `output_release`
  - `on_completion`
  - `immediate`

Default derivation:

- all nodes default to `first_input`
- explicit `wait_for_all_inputs = true` => `all_inputs`
- default `output_release = on_completion`

Status:

- docs now align on graph-derived execution and per-node policy
- the current runtime still behaves like `output_release = on_completion`
- explicit `all_inputs` barrier enforcement is landed for nodes marked `wait_for_all_inputs`
- true `output_release = immediate` is still pending

### Phase 5. CLI run visibility

Once runs exist, the workflow canvas should show runtime state, not just graph structure.

First-pass CLI additions:

- active run id and status in the workflow header
- per-node state such as idle, runnable, running, completed, failed
- lightweight event log or footer notices for run progress
- command-center entries for the new run commands

Status:

- CLI command wiring is landed for `/workflow run`, `/workflow runs`, and `/workflow cancel`
- the workflow canvas now shows the selected workflow's display run id/status plus per-node status derived from that run
- `/workflow resume` is now landed
- the workflow inspector now shows selected-node runtime state, failure events, turn-envelope state, mailbox snapshot, and handoff snapshot
- live stop/resume drills now cover both single-provider and mixed-provider workflow runs
- richer historical inspection and dedicated audit-pane UX are still pending

### Phase 6. Time-based schedules

Only after phases 1 through 5 are stable should we add full cron syntax.

That later slice can introduce:

- schedule metadata storage
- richer `/workflow watchdog ...` commands
- cron validation
- enable/disable controls
- daemon-online-only execution semantics

Status:

- interval watchdogs are landed
- full cron syntax is still pending
- watchdogs should now gain a bounded wakeup budget:
  - default bounded `max_wakeups`
  - explicit `null` to allow unbounded schedules when the user opts in

### Phase 7. Provider Process Ownership And Cleanup

The daemon should become the explicit owner of managed provider runtime processes.

Required slices:

- track managed provider processes and their owner provider runs
- track provider-native session ids per provider run when available
- expose tracked processes in the CLI through `/provider processes`
- support safe teardown only by default:
  - do not kill processes still needed by an attached session
  - do not kill processes backing an active prompt
  - do not kill processes backing an active workflow run
- make drill harnesses stop the daemon they launch so managed provider processes do not leak after live drills
- make drill harnesses end the sessions they create on exit

Late-stage follow-up:

- add durable daemon identity and workspace-level manager semantics
- persist a daemon runtime record and lock under `.chariox/runtime/daemon`
- make the CLI prefer attach-or-start against that workspace daemon record
- stamp managed provider children with daemon instance id so stale-child cleanup can be added later

CLI target:

- `/provider processes`
- `/provider processes <provider>`
- `/provider processes teardown`
- `/provider processes teardown <provider>`

Safety model:

- safe teardown should only stop daemon-tracked managed processes that are not attached and not actively executing work
- attached sessions must survive provider loss by degrading provider runs rather than losing the Chariox session itself
- CLI process inspection should explain why a process is blocked from safe teardown

## Suggested Immediate Next Steps

1. Lock the v1 execution model in code and docs before adding any slash command.
2. Add daemon/runtime types plus local API requests/responses for manual workflow runs.
3. Ship endpoint-triggered manual runs with a narrow DAG-first scheduler.
4. Add richer run inspection beyond the current selected-workflow header/node status view.
5. Enrich the completion contract with real node outputs, artifacts, and explicit stop/fail semantics.
6. Add default-bounded `max_wakeups` to watchdogs, with explicit `null` for unbounded schedules.
7. Add daemon-owned provider process inspection and safe teardown.
8. Add cron syntax only after interval watchdogs are proven in live drills.
9. Add buffered intermediate workflow outputs with workflow-level schema defaults and node-level schema overrides.

## Non-Goals For The First Slice

- cron syntax or calendar-aware recurring schedules
- arbitrary cyclic graph execution
- retries, backoff, or recovery orchestration
- external API publishing of workflow endpoints
- full historical replay/audit UI for runs
