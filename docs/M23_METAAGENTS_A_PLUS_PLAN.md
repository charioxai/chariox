# M23 Metaagents A+ Plan

> Superseded: this hardening plan predates temporary `/meta` mode. Current work
> should treat "metaagent" as an agent operating in meta mode for an active task,
> not as a separately created permanent role. Creation UX, command docs, runtime
> authorization, projection, and drills should use `/meta <task>` activation and
> mode-based checks.

## Objective

Take metaagents from a solid kernel-backed feature to professional-grade
software across `arroba` and `arroba-cloud`.

The target state is:

- Metaagent authority is explicit, typed, and kernel-verified.
- Metaagent state, events, and decisions survive restart and reconnect.
- Command discovery, docs, parsing, policy, and execution are generated from one
  source of truth.
- The web app exposes metaagents as a real supervisory workflow, not just as a
  spawn option and role badge.
- Validation covers local, remote, collaborative, restart, web, and hosted
  product flows.

Metaagent event notifications should remain visible runtime-origin prompts. Do
not hide these notifications in provider-only context.

## Current Assessment

The current implementation has the right shape:

- `AgentRole::Meta` is part of the serialized agent model.
- The kernel enforces one metaagent per user per session.
- Metaagents are denied slice placement and workflow node membership.
- Meta runtime MCP tools are shown only to metaagent provider runs.
- Metaagents can inspect session state, list/read/ack events, inspect turns, run
  scoped commands, prompt owned regular agents, and resolve owned regular-agent
  runtime interactions.
- `arroba-cloud` can create metaagent sessions/agents, marks metaagents in the
  terminal footer, and excludes metaagents from workflow node selection.

The remaining gap is not concept. It is product hardening: durability, audit,
authority typing, command-policy completeness, and first-class web supervision.

## Phase 1: Kernel Authority Hardening

Replace string-prefix metaagent detection with typed caller authority.

Current risk:

- Session actor lanes infer metaagent origin with
  `caller_id.starts_with("metaagent:")`.
- That is too weak for an authority boundary and makes future refactors easy to
  get wrong.

Target work:

- Add a typed caller variant such as `KernelCallerKind::Metaagent`.
- Include the metaagent id explicitly on metaagent-origin commands.
- Validate that the caller user owns that metaagent and that the metaagent
  belongs to the containing session before dispatch.
- Make session actor envelopes carry typed metaagent identity, not a boolean
  inferred from a string.
- Add tests that forged caller ids do not receive metaagent authority.

Acceptance:

- No authorization branch depends on `caller_id` string parsing.
- Metaagent-origin lifecycle suppression, command execution, and prompt routing
  all use verified typed identity.

## Phase 2: Durable Metaagent State And Audit

Persist metaagent records and decisions through the existing durable state log.

Current risk:

- The metaagent event store is in-memory.
- Restart loses event inbox records, read/ack state, optional subscriptions, and
  event prompt provenance.
- Runtime interaction resolution records a response payload, but durable audit
  does not identify the metaagent resolver.

Target work:

- Persist metaagent event records.
- Persist optional subscriptions and read/ack state.
- Persist metaagent command executions, including denials.
- Persist metaagent prompt submissions to other agents.
- Persist metaagent runtime interaction resolutions with:
  - session id
  - user id
  - metaagent id
  - target agent id
  - interaction id
  - choice/input metadata
  - provider run id when known
  - causation/correlation ids
  - timestamp
- Restore the metaagent event store from durable state on kernel startup.
- Add bounded retention or compaction rules for large event payloads.

Acceptance:

- Kernel restart preserves metaagent event inbox state.
- Restart preserves optional subscriptions.
- Read/ack state survives restart.
- Every metaagent-origin mutation has durable provenance.

## Phase 3: Unified Command Registry

Make command docs, search, parsing, policy, and routing share one source of
truth.

Current risk:

- The registry advertises some commands that are not routed.
- Example syntax can drift from the actual meta command parser.
- Policy lives partly in the registry and partly in router-specific code.

Target work:

- Define one descriptor per metaagent-routable command:
  - name and aliases
  - usage
  - examples
  - scope
  - mutates
  - policy
  - authority requirements
  - parser
  - request builder
  - denial message
- Generate `search_commands`, `list_commands`, and `command_docs` from those
  descriptors.
- Dispatch `run_command` through the same descriptors.
- Add table-driven tests that every documented routed command parses and every
  denied command rejects with the documented policy.
- Keep unsupported commands out of examples, or mark them explicitly as
  not-routed.

Acceptance:

- No documented `routed: true` command lacks an execution route.
- No routed command lacks policy tests.
- Command docs cannot drift from enforcement without tests failing.

## Phase 4: Event Delivery And Replay

Keep metaagent awareness prompt-first and visible.

Target work:

- Treat runtime-origin metaagent event prompts as visible prompts in the
  transcript, clearly marked as Arroba runtime events.
- Track prompt delivery status on each metaagent event:
  - recorded
  - submitted
  - steered
  - queued
  - delivered
  - failed
- Retry pending event prompts after provider restart/recovery.
- Preserve ordering guarantees for required events per metaagent.
- Make large event details discoverable through `read_event`, `turn_overview`,
  and `turn_blob`, while keeping the visible prompt compact and useful.
- Document provider-specific behavior for active-turn steering versus queueing
  across Codex, OpenCode, and Claude.

Acceptance:

- Event prompts are visible, attributable, and replayable.
- Missing provider runs are surfaced as liveness faults, not silent drops.
- A metaagent can reconstruct what happened after reconnect or restart.

## Phase 5: Collaboration And Remote Boundaries

Close the edge cases around multi-user and remote execution.

Target work:

- Add tests for one metaagent per collaborator in the same shared session.
- Verify a metaagent can mutate only its owner's regular agents.
- Verify a metaagent can observe workflow events according to session workflow
  policy.
- Verify a metaagent cannot inspect another user's non-workflow turn blobs.
- Verify remote-backed metaagents use home-kernel authority for meta runtime
  tools.
- Verify remote-backed metaagents cannot use worker-local shortcuts to bypass
  home policy.
- Verify slice-backed metaagent launch stays denied while slice management
  commands remain policy-gated.

Acceptance:

- Collaboration rules are enforced in kernel tests, not just UI behavior.
- Remote metaagent behavior is covered by home/worker tests and drills.

## Phase 6: Arroba Cloud Product Experience

Turn metaagents into a first-class supervisory surface in `arroba-cloud`.

Current state:

- The web app exposes creation toggles.
- The terminal footer can show a meta role marker.
- Workflow agent lists exclude metaagents.

Target work:

- Add a metaagent side panel or tab for the focused metaagent.
- Show session overview:
  - owned regular agents
  - agent status
  - active/queued prompts
  - workflow state
  - pending interactions
  - event counts
- Add an event inbox:
  - filter by kind/status
  - read detail
  - ack selected/all
  - jump to related agent/turn/workflow
- Add pending interaction controls:
  - approve/deny/select choice
  - show target agent and reason
  - clearly mark when human action is still required
- Add owned-agent task controls:
  - prompt an owned regular agent
  - focus agent
  - spawn regular helper agent
  - rename/delete owned regular agent
- Add a command palette backed by `arroba.meta.search_commands`.
- Add turn inspection:
  - call `turn_overview`
  - show trace items compactly
  - fetch selected blobs through `turn_blob`
- Keep all mutations routed through kernel requests or meta runtime tools. Do
  not create Cloud-only metaagent behavior.

Acceptance:

- A user can supervise a multi-agent session from the web UI without knowing MCP
  tool names.
- Every web control maps to an existing kernel authority path.
- Kernel rejection messages surface clearly in product UI.

## Phase 7: Validation Gates

Keep existing tests and add higher-value drills.

OSS tests:

- Typed metaagent caller authority.
- Durable event restore.
- Durable subscription restore.
- Durable read/ack restore.
- Durable interaction-resolution provenance.
- Command registry docs/enforcement parity.
- Cross-user mutation denial.
- Cross-user turn blob denial.
- Remote metaagent home-scope tool dispatch.
- Slice launch denial and slice management policy.

Cloud tests:

- Metaagent panel projection.
- Event inbox read/ack UI.
- Pending interaction UI.
- Command palette results.
- Owned-agent task controls.
- Workflow canvas still excludes metaagents.
- Duplicate metaagent spawn rejection remains visible.

Drills:

- Local web metaagent event replay after kernel restart.
- Hosted staging creation and duplicate rejection.
- Hosted staging regular-agent prompt visible in metaagent pane.
- Remote-worker metaagent tool availability and home-scope enforcement.
- Runtime interaction resolved by metaagent through web UI.
- Subscribed workflow event shown in metaagent inbox.

Acceptance:

- CI protects protocol shape, authority, command policy, and core state
  durability.
- Product drills prove the feature through real user flows, not direct kernel
  shortcuts.

## Release Criteria

Call the feature A+ only when:

- Authority is typed and kernel-verified.
- Metaagent events and decisions are durable.
- Command policy and docs are generated from one registry.
- Event prompts are visible, clearly attributed, and replayable.
- Web users have a real supervision UI.
- Remote and collaborative boundaries are covered by tests and drills.
- Hosted Cloud uses the same kernel/relay paths as local and remote clients.
