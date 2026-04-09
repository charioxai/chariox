## Multi-Agent Runtime Reliability

### Problem

Multi-agent sessions currently mix two incompatible models:

- daemon execution is mostly per-agent for prompts and provider runs
- CLI state and kernel snapshots are still largely session-global and focus-projected

That mismatch causes:

- pane badges showing `IDLE` while an agent is still working
- the prompt-area badge mirroring session-global activity instead of the focused agent
- focus cycling interfering with provider-run ownership and making agents appear stuck

### Invariants

1. Focus is a UI concern only.
2. Prompt execution is per-agent.
3. Provider-run liveness is per-agent.
4. The prompt-area badge mirrors the focused agent only.
5. Split-pane badges and prompt-area badge must derive from the same per-agent state.
6. Cycling focus must never park, resume, terminate, or otherwise disturb another live agent run in a multi-agent session.

### Refactor

#### Daemon

- Keep prompt truth in `prompt_states`.
- Treat `active_prompt`, `queued_prompts`, and `active_provider_run_id` as compatibility projections only.
- In multi-agent sessions, focus changes must only update the projected focused run for UI metadata.
- Do not park or resume provider runs on focus changes when more than one agent exists in the session.
- Session snapshots must project the focused agent's latest provider run for prompt metadata without mutating other active runs.

#### CLI

- Treat `prompt_states` as the source of truth for multi-agent prompt work.
- Derive `working`, queue depth, footer hint, and focused badge from focused-agent state instead of session-global projections.
- Preserve agent activity labels while that agent still has prompt work or provider output in flight.
- Make split-pane footer badges and prompt-area badge use the same per-agent status derivation.

### Tests

#### Daemon

- focus cycling in a multi-agent session does not park or replace another agent's active run
- session snapshots project the focused agent's provider run without tearing down other runs
- concurrent prompts on multiple agents retain independent prompt state across focus changes

#### CLI

- `sessionHasPromptWork()` returns true when any agent has active or queued work in `prompt_states`
- split-pane badges stay non-idle while the agent still has prompt work
- focused badge mirrors the focused agent rather than unrelated agent activity
- focused footer hint and queue depth come from the focused agent

### Acceptance

- a busy agent remains busy while cycling to another agent
- the prompt-area badge always matches the focused agent
- pane badges never regress to `IDLE` until that agent is actually settled
- multi-agent sessions survive repeated focus cycling without losing live runs
