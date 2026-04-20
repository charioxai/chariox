# M7.5 Arroba Shell Plan

## Goal

Add `arroba-shell` as a kernel-facing command shell independent from the TUI while reusing the same command executor as CLI slash commands. The shell provides a bash-like interaction model for Arroba domain commands: an `@` prompt for input, command output printed below without the prompt marker, stateful context, and a path to scriptable command files.

`arroba-shell` is not a POSIX shell. It does not execute arbitrary OS commands by default and does not aim to support pipes, redirects, shell expansion, or process control in v1. It is an Arroba command REPL backed by kernel IPC.


## App And Package Boundaries

`arroba-shell` is a sibling app to the TUI CLI, not a TUI subcommand:

- `apps/cli`: OpenTUI client, waiting room, panes, command center, and TUI-only commands.
- `apps/shell`: standalone Arroba command REPL and script runner.
- `packages/kernel-client`: shared kernel IPC client, request builders, kernel-facing runtime types, shell parser, and shell executor.

The shell may share kernel/domain code with the CLI, but it must not import TUI rendering or TUI state modules.

## Design Principles

- One command executor: CLI slash commands and `arroba-shell` commands must normalize to the same command execution layer where possible.
- Shell independence: `arroba-shell` must be conceptually and operationally independent from the TUI client.
- Stateful by default, explicit when needed: commands use current shell context unless an explicit `--session`, `--agent`, `--workflow`, or related ref is provided.
- Scriptability without a language: v1 scripts are line-oriented Arroba commands with simple variables and stop-on-error behavior.
- Structured results first: commands should return structured command results that can be rendered as text, table, JSON, or errors.
- TUI-only behavior stays TUI-only: layout, waiting room, command-center, and pane controls are not kernel/domain shell commands.

## Command Classes

### Shared Kernel/Domain Commands

These should be executable from both the TUI slash-command surface and `arroba-shell`:

- `session new|attach|list|delete|alias|use`
- `agent spawn|list|focus|delete|cycle`
- `machine list|kernels|approve|forget|rename`
- `relay status|configure`
- `config show|set|unset|managed-io`
- `mcp install|list|show|import|grant|revoke|grants`
- `skill install|list|show|import|grant|revoke|grants`
- `workflow new|list|show|run|runs|cancel|resume`
- `workflow node|edge|endpoint|watchdog|config`
- `provider status|login|logout|reauth`
- `stop` for cancelling active prompt work in the current or explicitly selected session/agent

### Shell-Local Commands

These are not kernel mutations, but are valid in `arroba-shell`:

- `exit` / `quit`
- `help`
- `set provider <provider>`
- `set model <model>`
- `set effort <effort>`
- `use session <ref>`
- `use agent <ref>`
- `use workflow <ref>`
- `context`
- `pwd`
- `vars`
- `unset <name>`
- `source <file>` or `run <file>`

### TUI-Only Commands

These should remain outside `arroba-shell` because they only control the current TUI client:

- `/waiting`
- `/view split|individual`
- `Tab`, `Ctrl+Tab`, command-center mechanics
- `/exit` as TUI process exit; shell has its own `exit` / `quit`
- `/workflow terminal` when it means show the TUI workflow terminal pane; if log reading is needed, add a separate shared command such as `workflow logs`
- `/provider`, `/model`, and `/variant` as UI default selectors; shell equivalents are shell-local `set provider|model|effort`
- provider namespace commands such as `/opencode <cmd>` and `/codex <cmd>` until they are expressed as an explicit shared command, for example `agent input <agent-ref> <text>`

## Shell Context

The shell keeps this context in memory:

```ts
type ShellContext = {
  workspace: string
  worktree: string
  sessionId?: string
  agentId?: string
  workflowId?: string
  provider: string
  model: string
  effort: string
  variables: Record<string, string>
}
```

Implicit targeting uses context. Explicit refs override context for one command only:

```text
@ agent list
@ agent list --session session-abc
@ workflow run qa default --session session-other
@ mcp grant playwright --agent agent-7
```

Commands that create or attach resources update context by default:

```text
@ session new --worktree ../qa --branch qa/run as qa_session
created session session-abc in /path/to/qa
bound $qa_session = session-abc
current session = session-abc

@ agent spawn reviewer gpt-5.2 as reviewer
spawned agent agent-2 (reviewer)
bound $reviewer = agent-2
current agent = agent-2
```

## Variables And Scripts

V1 variables are simple string bindings:

- `as <name>` stores the primary returned resource id.
- `$name` substitutes that string in later commands.
- Variables are shell-local and ephemeral in v1.
- No arrays, arithmetic, conditionals, loops, pipes, or redirects in v1.

Script execution:

```bash
arroba-shell run setup.arroba
arroba-shell run setup.arroba --var session=session-1 --continue-on-error
```

From an active `arroba-shell` REPL or another script:

```text
source setup.arroba
run setup.arroba
```

Default script behavior:

- execute non-empty, non-comment lines in order
- stop on first error
- print the command transcript and outputs
- print line-numbered stop/continue diagnostics for failing commands
- return non-zero on failure
- accept repeated `--var NAME=VALUE` seed bindings before the first command
- accept `--continue-on-error` for validation/drill scripts that should report every failing command
- `source <file>` and `run <file>` load another script from disk, resolve relative paths against the current shell worktree, and preserve context changes/variable bindings in the caller

Context inspection:

```text
context
pwd
```

- `context` prints the current workspace, worktree, session, attachment, agent, workflow, provider, model, effort, and shell variables.
- When a current session is available, `context` refreshes session state and marks the current agent as `(busy)` if it has an active or queued prompt.
- `pwd` prints the current shell worktree.
- `pwd` is shell-local; `context` calls the kernel only when a current session is set.

Prompt submission:

```text
prompt [agent-ref] <prompt> [--wait] [--show-reply|--show-summary]
```

- Submits a freeform prompt to the current agent, or to `agent-ref` when provided.
- Without `--wait`, returns immediately with the prompt id.
- `--show-reply` implies wait and prints all prompt history emitted after the user prompt.
- `--show-summary` implies wait and prints only final provider output entries.
- Multi-line prompt output is rendered as an aligned blob headed by the prompt id, for example `prompt-1 summary`.

Example script:

```text
session new --worktree ../qa --branch qa/run as s
agent spawn reviewer gpt-5.2 --session $s as reviewer
mcp grant playwright --agent $reviewer
workflow new qa-flow --session $s as wf
workflow node add $wf $reviewer
workflow run $wf default "Run QA"
```

## Command Result Model

Shared command execution should return structured results:

```ts
type ShellCommandResult = {
  ok: boolean
  message?: string
  data?: unknown
  format?: "text" | "table" | "json"
  bindings?: Record<string, string>
  contextUpdates?: Partial<ShellContext>
}
```

Renderers convert the result for standalone shell, TUI pane, and scripts.

## Implementation Slices

### Slice 1: Milestone Plan And Shell Core Skeleton

- Status: implemented in `packages/kernel-client/src/shell-core.ts` with focused coverage in `packages/kernel-client/src/shell-core.test.ts`.
- Added this milestone plan.
- Added a parser that normalizes shell commands without `/` and slash commands with `/` to a shared shape.
- Added shell context and command result types.
- Added unit tests for parsing, `as <name>`, variables, result rendering, context updates, and TUI-only command classification.

### Slice 2: Minimal Shared Executor

- Status: implemented in `packages/kernel-client/src/shell-executor.ts` with focused coverage in `packages/kernel-client/src/shell-executor.test.ts`.
- Added a minimal executor that consumes normalized shell commands and returns structured `ShellCommandResult` values.
- Added shell-local execution for `help`, `set`, `use`, `vars`, `unset`, `exit`, and `quit`.
- Added low-risk kernel-backed command coverage:
  - `session list`
  - `session new --dir|--worktree`
  - `session attach|use`
  - `agent list`
  - `agent spawn --dir|--worktree|--machine`
- Also added `agent focus|cycle`.
- Existing TUI slash commands keep using their current handlers; the shell executor is not yet wired into the TUI.
- Provider-run launch after `agent spawn` is intentionally not part of this slice. The shell executor creates/manages kernel resources; provider process lifecycle integration is part of the standalone shell/runtime integration slice.

### Slice 3: Standalone `arroba-shell`

- Status: implemented in `apps/shell/src/shell.ts` with focused coverage in `apps/shell/src/shell.test.ts`.
- Added an `arroba-shell` binary/script entrypoint via the `apps/shell` package `bin` field and `pnpm --filter @arroba/shell run start`.
- Connects to the kernel through the existing local IPC client using `--kernel-url`, `--socket`, or the same environment/default endpoint rules.
- Renders an `@` prompt and prints command outputs below submitted commands.
- Supports `exit`, `quit`, `help`, `set`, `use`, `vars`, and `unset` through the shared shell executor.
- Basic command history is deferred until the workspace-pane/script-runner slices if needed.

### Slice 4: Script Runner

- Status: implemented in `apps/shell/src/shell.ts` with focused coverage in `apps/shell/src/shell.test.ts`.
- The reusable script runner now lives in `packages/kernel-client/src/shell-script.ts` so both `arroba-shell` and the embedded workflow-pane shell execute scripts through the same parser, executor, renderer, variable propagation, and nested `source`/`run` logic.
- Added `arroba-shell run <file>`.
- Supports comments, blank lines, variables, and stop-on-error execution.
- Supports seeded variables through repeated `--var NAME=VALUE`.
- Supports `--continue-on-error` for audit/drill scripts while still returning non-zero if any command failed.
- Supports `source <file>` / `run <file>` from the REPL or nested scripts, preserving the caller's shell context.
- Emits line-numbered failure diagnostics for both structured command failures and thrown transport/kernel errors so script output can be mapped back to the command file.
- Standalone shell clients attach to sessions they create/use so commands that require an attachment, such as `stop` and `workflow max-turns`, work outside the TUI.
- Returns non-zero on failure.
- Added script fixture tests for session/agent setup commands using mocked IPC.

### Slice 5: Workspace Pane Integration

- Status: implemented for the workflow workspace screen.
- Added a right-side `arroba-shell` pane to the workflow view while keeping the workflow outline/canvas on the left.
- Reuses the shared shell parser/executor and result renderer from `packages/kernel-client`.
- Workflow-screen shell input is submitted through the main prompt with an `@ <command>` prefix so it does not collide with workflow endpoint prompts or slash commands.
- The pane renders command lines with `@` and prints command output below each input.
- The embedded shell supports `source <file>` and `run <file>` from the current worktree, using the same shared script runner as `arroba-shell`.
- Workflow-creating or workflow-selecting shell commands now update the selected workflow in the TUI when the command result context contains a valid workflow id. This keeps the visible workflow pane aligned with `workflow new`, `workflow show`, and loaded workflow scripts.
- TUI-only commands remain rejected by the shared shell parser/executor.
- Added a CLI automation socket (`--automation-socket <path>`) for reliable embedded-shell drills. The socket accepts JSONL actions (`ping`, `switch_screen`, `workspace_shell_exec`, `snapshot`, `wait_for`, `exit`) and returns structured UI/application snapshots instead of relying on raw PTY keystrokes.
- The automation snapshot includes the current workspace screen, selected workflow/node, workflow graph counts, workflow run summaries, shell context, shell transcript entries, and footer flash state.

### Slice 6: Broaden Command Coverage

- Status: in progress.
- Implemented read/status/show coverage in the shared executor for:
  - `machine list|kernels`
  - `relay status`
  - `config show`
  - `mcp list|show`
  - `skill list|show`
  - `provider status`
- Implemented MCP and skill mutation/grant coverage in the shared executor:
  - `mcp install|update|uninstall|import|grant|revoke|grants`
  - `skill install|update|uninstall|import|grant|revoke|grants`
- MCP installs support stdio servers through `--command`, repeated `--arg`, and repeated `--env`, plus streamable HTTP servers through `--url` and optional `--bearer-token-env-var`.
- Added basic text/JSON renderers for list/show/import/grant results.
- Implemented workflow coverage in the shared executor:
  - `workflow list|new|show|alias|run|runs|run-show|cancel|resume`
  - `workflow node add|remove`
  - `workflow edge add|remove`
  - `workflow endpoint new|alias|bind`
- Implemented config mutation coverage in the shared executor:
  - `config path|set|unset|managed-io`
- Implemented workflow advanced coverage in the shared executor:
  - `workflow launch-policy|flush-context|max-turns|run-output-schema|intermediate-output-schema`
  - `workflow node can-complete-run|can-emit-intermediate-output|intermediate-output-schema|max-turns`
  - `workflow watchdog add|list|enable|disable|remove`
  - `workflow queue list|flush|remove`
- Implemented provider and cancellation coverage in the shared executor:
  - `provider status|login|logout|reauth|processes`
  - `stop` / `cancel`
- Implemented freeform prompt submission in the shared executor:
  - `prompt [agent-ref] <prompt> [--wait] [--show-reply|--show-summary]`
  - no-wait mode returns the prompt id immediately
  - wait/show modes poll prompt completion, read session history, and render prompt-id-headed blobs for captured output
- Remaining command/action gaps compared with the TUI slash surface:
  - `session alias|delete`
  - `agent delete|destroy`
  - `machine approve|forget|rename`
  - `relay configure`
  - `workflow node instructions show|set|save|close`
  - `workflow endpoint remove`
  - provider-native namespace passthrough such as `/opencode <cmd>` and `/codex <cmd>`
- Provider-native namespace passthrough remains intentionally excluded until it is expressed as explicit shared commands. `workflow node instructions` remains editor-coupled in the TUI for now.

## Validation

Each slice must include focused tests and update docs before commit/push. Required validation by the end of the milestone:

- CLI slash commands still pass existing command-action tests.
- `arroba-shell` can create a session, spawn an agent, list sessions/agents, and update context.
- `arroba-shell run <file>` can set up a small workflow using variables.
- `arroba-shell run <file> --var NAME=VALUE --continue-on-error` can run a validation script, continue past failures, and still return non-zero if anything failed.
- `pnpm --filter @arroba/cli run shell:drill` passes against an isolated local kernel.
- `pnpm --filter @arroba/cli run embedded-shell:drill` launches the real CLI under a PTY, drives the embedded workflow shell through the automation socket, and verifies workflow pane state from structured snapshots.
- TUI workspace pane can run the same shell command executor without duplicating command logic.
- TUI-only commands are rejected or redirected with clear messages in `arroba-shell`.

## Deferred

- Full bash compatibility.
- Pipes, redirects, globbing, arithmetic, loops, conditionals.
- Persistent variable stores.
- Provider-native namespace passthrough (`opencode ...`, `codex ...`) until represented as a provider-neutral shared command.
- Workflow logs command unless needed to replace the TUI-only workflow terminal pane.
