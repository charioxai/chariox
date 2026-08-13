# M27 Metaagents Stateful Planner Plan

> Updated model: stateful planning now belongs to an ordinary agent while it is
> in temporary `/meta` mode. The task and plan are kernel-managed for that
> active mode; completion, block, or abort exits meta mode and restores the
> previous provider profile.

## Summary

Update metaagents into stateful planning supervisors. A metaagent can read
workspace context, use recall, maintain a task plan, provision capabilities, and
delegate execution to regular agents. It always runs in provider plan mode, so
Chariox should not duplicate direct implementation-tool denial policy beyond what
plan mode and normal provider/runtime policy already enforce.

Metaagents can have one active task at a time. The task is created from the
original user prompt, is visible and editable in clients, and has a
kernel-managed plan document the metaagent can update while working.

The intended model is a "CEO" agent: it can observe, plan, provision, approve,
and supervise, but it should delegate implementation work to regular agents.

## Key Changes

- Always force metaagents to `execution_mode = plan`.
- Do not force `permission_level = required`; inherit the normal
  user/session/agent permission configuration.
- Keep direct third-party/user MCP access unavailable by default.
- Allow slice/browser tools when the metaagent itself is deployed in a slice.
- Allow read-only planning access, including workspace reads and recall
  search/query.
- Allow secure vault secret entry for metaagents by default, configurable off.
- Keep meta capability management available: MCP install/provisioning,
  vault-backed secret creation, worker capability grants, and worker permission
  confirmations remain metaagent capabilities.

Do not add a separate "deny implementation tools" layer for metaagents unless a
provider exposes direct mutation while in plan mode. Plan mode is the primary
enforcement mechanism for direct action. Runtime policy should focus on which
Chariox tools exist for metaagents, not on making metaagents blind.

## Runtime MCP Task Artifacts

Add metaagent-only runtime MCP calls for task artifacts:

- `chariox.meta.read_task`
- `chariox.meta.update_task`
- `chariox.meta.read_plan`
- `chariox.meta.update_plan`
- `chariox.meta.complete_task`
- `chariox.meta.mark_blocked`

These calls are exposed only to metaagent provider runs and are scoped by
session id plus metaagent id. The task prompt and plan documents are
kernel-managed artifacts, not arbitrary workspace files.

Metaagents may read and write these task documents only through the new runtime
MCP calls. Those calls are exposed only to metaagents and are scoped per session
and agent, so one metaagent cannot overwrite another metaagent's task state.
Regular workspace file access remains normal planning context: metaagents may
read workspace files, but implementation work is delegated to regular agents
through plan-mode behavior and metaagent supervision.

`chariox.read_artifact` should remain available to metaagents. Reading artifacts
is observation, not execution.

## Task Lifecycle

Add one task state per metaagent:

- `none`
- `active`
- `paused`
- `blocked`
- `completed`
- `aborted`

The original prompt creates the active task and is stored as editable markdown.
The metaagent updates the plan markdown as it works. User edits to the task
notify or steer the metaagent. Pause stops further metaagent work without
deleting state. Abort cancels current metaagent work and marks the task aborted.
Completion or blocking is explicitly declared by the metaagent through meta task
tools.

## Prompt Guidance

Keep the system prompt in a markdown template loaded by the prompt assembly
service, like the other system prompts. Keep it short. It should say:

```text
You are a Chariox metaagent. Read workspace context and recall when useful,
maintain your task plan, delegate execution to regular agents, and supervise
their results. Continue until the task is completed, paused, aborted, or
genuinely blocked. If the user edits the task, revise your plan as needed and
continue.
```

The prompt should avoid saying that metaagents cannot read files. The boundary
is direct implementation, not observation and planning.

## UI Updates

Update both the local TUI and web terminal surfaces:

- Show active metaagent task above the agent pane footer.
- Make the task area selectable and editable.
- Add task controls: edit, pause/resume, abort.
- Show compact status: `TASK`, `PAUSED`, `BLOCKED`, `DONE`, `ABORTED`.
- Allow viewing and editing the task prompt and plan document.
- Reflect kernel task updates from session snapshots or events.
- When the user edits the task, send the kernel task update and notify the
  metaagent.

The same kernel task model and local daemon requests should back both UIs. Do
not implement a web-only or TUI-only task state.

## Test Plan

Update the capabilities drill:

- Assert the metaagent can read workspace files.
- Assert recall search/query are available.
- Assert artifact reads are available.
- Assert the metaagent can read and update only its own task and plan documents
  through `chariox.meta.*` task tools.
- Assert task artifact calls fail for non-metaagents, other sessions, or other
  users' metaagents.
- Assert execution mode is plan while permission level is inherited.
- Assert direct third-party/user MCP tools remain unavailable by default.
- Assert secure secret entry is available through the configured vault path.
- Assert capability provisioning paths remain available: MCP install/setup,
  vault secret creation, grants to workers, and worker confirmation flows.

Add a real-provider observe-only code-fix drill:

- Fixture: tiny JavaScript todo project with one failing test.
- Prompt: `The repo has a small failing JavaScript project. Delegate the investigation, fix, and verification to regular agent(s), get the project to a passing state, then mark this task complete with a concise report of what changed and how it was verified.`
- The drill must use an actual provider-backed metaagent, never `dev-stub`.
- The harness may create the fixture, start the kernel/session, launch the
  metaagent, and submit that single prompt.
- After the prompt, the harness may only observe session state, metaagent
  events, workspace diffs, and test results. It must not call runtime MCP tools,
  append synthetic provider output, prompt workers directly, or write files.
- Assert the metaagent reads context, creates or updates its plan, delegates to
  workers, does not require a second user prompt, and marks the task completed.
- Assert source changes are produced by regular workers, not by metaagent task
  artifact tools or harness writes.
- Assert `src/todo.mjs` changes, the test file remains unchanged, and
  `npm test` passes.

## Assumptions

- Plan mode is the primary mechanism for preventing direct implementation
  behavior.
- Metaagent task and plan documents are kernel-managed, session/metaagent-scoped
  runtime artifacts.
- Secret entry is allowed by default for metaagents through secure vault flows
  and can be disabled by configuration.
- TUI and web terminal should expose equivalent task lifecycle controls.
