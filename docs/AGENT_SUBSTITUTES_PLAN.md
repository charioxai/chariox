# Agent Substitutes

## Goal

An agent can have one or more substitute provider profiles. If the active provider becomes unavailable, Chariox can replace the provider run for the same agent without changing the agent id, pane, workflow ownership, worktree, grants, permissions, or thread. Manual activation must also be available from the TUI slash commands and from `chariox-shell`.

## Model

Substitutes are profiles on `AgentInstance`, not separate agents:

- `provider`
- `model`
- `variant`

Agent-level metadata tracks:

- configured substitute list
- active substitute index
- last substitution reason and timestamp
- optional per-agent timeout override

The default timeout is `60s`.

## Commands

TUI and shell use the same kernel requests:

- `/agent substitute list [agent]`
- `/agent substitute add <provider> <model> [--variant <variant>] [--agent <agent>]`
- `/agent substitute remove <index> [--agent <agent>]`
- `/agent substitute clear [agent]`
- `/agent substitute timeout <duration> [--agent <agent>]`
- `/agent substitute activate <index> [--agent <agent>]`
- `/agent substitute primary [--agent <agent>]`

Shell equivalents omit the leading slash:

- `agent substitute list`
- `agent substitute add codex gpt-5.4 --variant medium`
- `agent substitute activate 0`

## Runtime Policy

Manual activation launches the selected profile for the same agent and marks the profile active. Automatic activation will use the same kernel path, so manual and automatic substitution have identical provider launch semantics.

Automatic activation is eligible for:

- quota/no credits
- rate/run limit

Automatic activation is not eligible for:

- invalid model
- auth required
- permission denied
- provider unreachable or timeout
- provider process exit without an explicit resource-limit diagnostic
- explicit user cancellation

When a substitute provider run starts, Chariox reuses the inter-provider context handoff packet. If a workflow node prompt is the next provider-bound prompt, it receives the handoff too.

## UI

The agent pane footer should show substitute availability and active substitute state:

- primary active: `opencode • gpt-5.4 • high • 2 subs`
- substitute active: `codex • gpt-5.4 • medium • sub 1/2`
- manual substitute active: `codex • gpt-5.4 • medium • manual sub`
- automatic substitute active: `codex • gpt-5.4 • medium • sub: quota`

## Phases

1. Add durable agent substitute fields and service mutations.
2. Add kernel/local API requests for substitute management and manual activation.
3. Add TUI slash commands and footer rendering.
4. Add shell request helpers and shell commands.
5. Add automatic substitution hooks for eligible provider failures.
   Implemented scope: explicit quota, billing, rate-limit, and run-limit diagnostics activate and launch the next configured substitute. Generic launch failures, network errors, and unexpected provider exits do not activate substitutes automatically. The failed turn is not silently replayed; Chariox settles the failed provider turn first and the new run receives the normal inter-provider context handoff for subsequent prompts.
6. Add timeout watchdog and retry active prompt without duplicate history.
   Deferred by policy: unavailability can mean machine or relay disconnection, so substitute activation remains manual for timeout/unreachable cases.
7. Run drills for manual activation, shell activation, quota fallback, workflow fallback, and exhausted substitutes.
