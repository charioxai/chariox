# M7.5 Arroba Shell Plan

## Goal

Add `arroba-shell` as a kernel-facing command shell independent from the TUI while reusing the same command executor as CLI slash commands. The shell provides a bash-like interaction model for Arroba domain commands: an `@` prompt for input, command output printed below without the prompt marker, stateful context, and a path to scriptable command files.

`arroba-shell` is not a POSIX shell. It does not execute arbitrary OS commands by default and does not aim to support pipes, redirects, shell expansion, or process control in v1. It is an Arroba command REPL backed by kernel IPC.

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
```

Default script behavior:

- execute non-empty, non-comment lines in order
- stop on first error
- print the command transcript and outputs
- return non-zero on failure

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

- Add this milestone plan.
- Add a parser that normalizes shell commands without `/` and slash commands with `/` to a shared shape.
- Add shell context and command result types.
- Add unit tests for parsing, `as <name>`, variables, and TUI-only command classification.

### Slice 2: Minimal Shared Executor

- Extract or wrap the existing command-action paths behind a shared command executor where possible.
- Start with low-risk commands:
  - `session list`
  - `session new --dir|--worktree`
  - `session attach|use`
  - `agent list`
  - `agent spawn --dir|--worktree|--machine`
- Existing TUI slash commands should keep working.

### Slice 3: Standalone `arroba-shell`

- Add an `arroba-shell` binary/script entrypoint.
- Connect to the kernel through the existing local IPC client.
- Render an `@` prompt.
- Print command outputs below submitted commands.
- Support `exit`, `quit`, `help`, `set`, `use`, `vars`, and basic history if low-risk.

### Slice 4: Script Runner

- Add `arroba-shell run <file>`.
- Support comments, blank lines, variables, and stop-on-error execution.
- Return non-zero on failure.
- Add script fixture tests for session/agent setup commands using mocked IPC.

### Slice 5: Workspace Pane Integration

- Add a right-side shell pane to the workspace view.
- Keep workflow outline/canvas on the left.
- Reuse the standalone shell engine and renderer where possible.
- Preserve TUI-only commands as TUI controls, not shell commands.

### Slice 6: Broaden Command Coverage

- Add remaining command families to the shared executor:
  - machine, relay, config
  - MCP, skills
  - workflows
  - provider auth/status
  - stop/cancel
- Add table/JSON renderers for list/show commands.

## Validation

Each slice must include focused tests and update docs before commit/push. Required validation by the end of the milestone:

- CLI slash commands still pass existing command-action tests.
- `arroba-shell` can create a session, spawn an agent, list sessions/agents, and update context.
- `arroba-shell run <file>` can set up a small workflow using variables.
- TUI workspace pane can run the same shell command executor without duplicating command logic.
- TUI-only commands are rejected or redirected with clear messages in `arroba-shell`.

## Deferred

- Full bash compatibility.
- Pipes, redirects, globbing, arithmetic, loops, conditionals.
- Persistent variable stores.
- Provider-native namespace passthrough (`opencode ...`, `codex ...`) until represented as a provider-neutral shared command.
- Workflow logs command unless needed to replace the TUI-only workflow terminal pane.
