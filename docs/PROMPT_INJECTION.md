# Prompt injection and context projection

Chariox-owned natural-language instructions are catalogued by stable IDs in
`apps/kernel/src/prompt_assembly.rs` and stored as individual Markdown files
under `apps/kernel/src/provider/`. The kernel's `PromptAssemblyService` is the
only renderer for provider-turn hidden context, workflow system/node guidance,
meta-agent event/recovery guidance, granted-skill context, and utility prompts.
Skill discovery remains owned by the skill registry, but its agent-facing
prose is rendered through the `runtime/skill-context` catalog entry. User-authored workflow
and node instructions remain data owned by the workflow; provider-native prompt
text and UI/error copy remain outside this catalog.

Each catalog entry is materialized into the kernel's prompt directory, carries
scope/audience/provider/editability metadata, and is rendered with bounded
variable substitution. Overrides use optimistic revision and SHA-256 checks,
are atomically persisted, and are deterministic across restart. A bundled
template change updates an unmodified materialized default while preserving a
user override. Protected entries are catalogued and resettable but cannot be
edited because they explain security or protocol invariants that are also
enforced in code.

The prompt manifest records every template body used by a provider turn. The
manifest is part of the durable/replay-visible prompt envelope, so a replay can
identify the exact template revisions without re-reading mutable UI state.

Conditional workflow-node guidance is catalogued too: `workflow/node-max-turns`
renders the configured turn limit, while `workflow/wait-for-all-inputs` is added
only for nodes that wait for every incoming edge in the current iteration.

## Context policy

The always-on workflow contract is deliberately small. Workflow/node
instructions, bounded handoff payloads, edge contracts, event summaries, and
artifacts are projected only for the current turn. Large payloads use artifact
references rather than embedding raw provider data. Credentials and event
installation metadata never enter prompts.

Runtime tools are projected from the authenticated provider-run capability
snapshot and checked again at dispatch. Ordinary provider runs do not receive
workflow tools. Workflow runs receive the workflow contract tools. The
`reply_to_event` action is added only when the current event binding enables a
`thread` or `channel` reply mode; bindings with replies disabled do not add its
description to `tools/list`. Because providers cache tool discovery, a
capability change rotates an idle provider run. A busy run keeps its existing
snapshot until its admitted prompt completes, preserving FIFO semantics; the
next idle boundary applies the new snapshot. The same flag travels in
`RemoteWorkflowTurnContext` to leased workers, and the relay-peer protocol is
versioned when this shape changes.

Meta-agent tools remain conditionally projected by the existing meta-agent
policy. Every runtime tool validates the capability again at dispatch, so
omitting a description is an optimization and never an authorization boundary.

## Adding a prompt

1. Add one Markdown file under `apps/kernel/src/provider/`.
2. Add one stable catalog entry and metadata in `bundled_templates()` and
   `prompt_setting_metadata()`.
3. Use `{{VARIABLE}}` placeholders only when the renderer supplies them, and
   keep the rendered result below the validation limit.
4. Route assembly through `PromptAssemblyService` (or the workflow injection
   service for workflow-specific composition); do not add agent-directed prose
   literals to runtime code.
5. Add a catalog/render/reset/replay test and update the prompt settings UI
   catalog fallback only if the new entry is visible before a kernel connects.

The disconnected browser catalog is a read-only projection of this same
catalog, not an independent inventory. When a new entry is added, the Cloud
fallback must add the same stable ID and a matching focused test; otherwise a
browser without a compatible kernel would silently show an incomplete list.

The browser Settings workspace and TUI `/settings prompts` commands call the
authenticated kernel API for list, update, preview, reset-one, and reset-all.
Cloud never owns prompt content or provider credentials.
