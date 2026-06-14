# M27 Metaagents Stateful Planner Plan

## Summary

Update metaagents into stateful planning supervisors. A metaagent can read
workspace context, use recall, maintain a task plan, provision capabilities, and
delegate execution to regular agents. It always runs in provider plan mode, so
Arroba should not duplicate direct implementation-tool denial policy beyond what
plan mode and normal provider/runtime policy already enforce.

Metaagents can have one active task at a time. The task is created from the
original user prompt, is visible and editable in clients, and has a
kernel-managed plan document the metaagent can update while working.

## Key Changes

- Always force metaagents to `execution_mode = plan`.
- Do not force `permission_level = required`; inherit the normal
  user/session/agent permission configuration.
- Keep direct third-party/user MCP access unavailable by default.
- Allow slice/browser tools when the metaagent itself is deployed in a slice.
- Allow read-only planning access, including workspace reads and recall
  search/query.
- Allow secure vault secret entry for metaagents by default, configurable off.

## Runtime MCP Task Artifacts

Add metaagent-only runtime MCP calls for task artifacts:

- `arroba.meta.read_task`
- `arroba.meta.update_task`
- `arroba.meta.read_plan`
- `arroba.meta.update_plan`
- `arroba.meta.complete_task`
- `arroba.meta.mark_blocked`

These calls are exposed only to metaagent provider runs and are scoped by
session id plus metaagent id. The task prompt and plan documents are
kernel-managed artifacts, not arbitrary workspace files.

Metaagents may read and write these task documents only through the new
`arroba.meta.*` runtime MCP calls. Regular workspace file access remains normal
planning context: metaagents may read workspace files, but implementation work
is delegated to regular agents through plan-mode behavior and metaagent
supervision.

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

Keep the system prompt short. It should say:

```text
You are an Arroba metaagent. Read workspace context and recall when useful,
maintain your task plan, delegate execution to regular agents, and supervise
their results. Continue until the task is completed, paused, aborted, or
genuinely blocked. If the user edits the task, revise your plan as needed and
continue.
```

The prompt should avoid saying that metaagents cannot read files. The boundary
is direct implementation, not observation and planning.

## UI Updates

Update both the local TUI and web terminal:

- Show active metaagent task above the agent pane footer.
- Make the task area selectable and editable.
- Add task controls: edit, pause/resume, abort.
- Show compact status: `TASK`, `PAUSED`, `BLOCKED`, `DONE`, `ABORTED`.
- Allow viewing and editing the task prompt and plan document.
- Reflect kernel task updates from session snapshots or events.
- When the user edits the task, send the kernel task update and notify the
  metaagent.

## Test Plan

Update the capabilities drill:

- Assert the metaagent can read workspace files.
- Assert recall search/query are available.
- Assert the metaagent can read and update only its own task and plan documents
  through `arroba.meta.*` task tools.
- Assert task artifact calls fail for non-metaagents, other sessions, or other
  users' metaagents.
- Assert execution mode is plan while permission level is inherited.
- Assert direct third-party/user MCP tools remain unavailable by default.
- Assert secure secret entry is available through the configured vault path.

Add an autonomous one-prompt drill:

- Fixture: tiny JavaScript todo project with one failing test.
- Prompt: `The repo has a small failing JavaScript project. Figure out what is wrong, organize the work with regular agents, and get the project to a passing state. Report back with what changed and how you verified it.`
- Assert the metaagent reads context, creates or updates its plan, delegates to
  workers, does not require a second user prompt, and marks the task completed.
- Assert source changes are produced by regular workers, not by metaagent task
  artifact tools.
- Assert tests pass.

## Assumptions

- Plan mode is the primary mechanism for preventing direct implementation
  behavior.
- Metaagent task and plan documents are kernel-managed, session/metaagent-scoped
  runtime artifacts.
- Secret entry is allowed by default for metaagents through secure vault flows
  and can be disabled by configuration.
- TUI and web terminal should expose equivalent task lifecycle controls.
