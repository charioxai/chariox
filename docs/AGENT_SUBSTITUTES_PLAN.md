# Agent Substitutes

## Goal

An agent can have an ordered list of substitute provider profiles. When a provider fails a turn, Chariox reruns that same turn on the next substitute, without changing the agent id, pane, workflow ownership, worktree, grants or permissions. Substitutes are the first defense against provider errors; they never become the agent's profile.

## Model

Substitutes are profiles on `AgentInstance`, not separate agents:

- `provider`
- `model`
- `variant`
- `account_profile` (a stable account id, resolved when the substitute is added)

The agent keeps only the ordered list and an optional per-agent timeout. There is no active-substitute state: the agent's configured profile is the only profile any new turn starts on.

## Commands

TUI and shell use the same kernel requests:

- `/agent substitute list [agent]`
- `/agent substitute add <provider> <model> [--variant <variant>] [--account <alias>] [--agent <agent>]`
- `/agent substitute remove <index> [--agent <agent>]`
- `/agent substitute move <from> <to> [--agent <agent>]`
- `/agent substitute clear [agent]`
- `/agent substitute timeout <duration> [--agent <agent>]`

Shell equivalents omit the leading slash, for example `agent substitute add codex gpt-5.4 --variant medium`.

## Runtime Policy (protocol 388)

Every provider failure of a turn reruns it on the next substitute:

- an error result or a structured error code (Codex `codexErrorInfo` such as `server_overloaded` or `usage_limit_exceeded`, Claude `StopFailure` kinds, OpenCode session errors);
- the provider process exiting mid-turn;
- a provider timeout the kernel already detects.

Not a provider failure: a user cancel or interrupt, and a turn the agent completes, whatever its text says.

The rerun keeps the same active prompt. The failed attempt stays visible, its provider run is retired, and a provider run for the substitute is launched through the normal (workflow-aware) launch path and receives the turn. Substitutes are tried in order; one whose saved account is missing or confirmed exhausted is skipped with a notice. If the substitute also fails, the next one gets the turn. When none is left, the turn fails with its provider error.

Each rerun records a notice: `This turn runs on claude-opus-5-5 because gpt-6.1-sol failed: model at capacity (server_overloaded).`

The substitute's provider run serves only that turn. It is retired when the turn completes, and any later turn that would reuse it moves to a fresh run of the agent's configured profile. The substitute run never writes its profile or resume state into the agent.

The substitute always starts a new provider session; it never resumes the agent's saved sessions, which belong to the configured profile. For a conversational turn, the substitute receives the conversation as a context handoff, and once it completes the turn, the configured profile's next turn (submitted or queued) receives a bounded handoff with the substitute's answer. This holds even when the substitute differs from the primary only in account or effort. Workflow turns carry their own context and get no handoff.

There are no retries, backoff, parked states or automatic return-to-primary state in the kernel: the next turn simply starts on the primary.

Not covered: remote (leased) agents, provider-native TUI turns, and failures before a turn reaches its provider (a prompt dispatch or provider launch that fails).

## Migration

An agent persisted on a substitute by an older kernel (`active_substitute_index` with a `primary_*` snapshot) loads on its primary profile; the retired fields are dropped.
