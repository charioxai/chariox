# M4 Multi-Agent Session Plan

## Goal

Turn the current session-agent metadata plumbing into a real manual multi-agent session runtime:

- one session can host multiple top-level Arroba agents
- each agent has its own provider-run context and history
- `Ctrl+A` and `/agent cycle` change the active agent, not just the footer label
- the CLI response area is visibly split into one sub-area per agent

## Current Baseline

Already implemented:

- session agent records and focused-agent state in daemon session state
- `/agent spawn`, `/agent delete`, `/agent focus`, `/agent list`, `/agent cycle`
- `Ctrl+A` cycles session agent focus in the TypeScript CLI

Still missing:

- provider-run ownership per agent
- prompt routing that reliably follows the focused agent
- agent-scoped session history and live output routing
- split-pane CLI rendering with one area per agent

## Runtime Model

### Top-level agent ownership

- each top-level agent keeps a stable `agent_id`
- each provider run is associated with exactly one top-level agent
- the session-level `active_provider_run_id` remains useful, but it represents the focused agent's currently active run

### Focus transitions

When focus changes:

- the daemon updates `focused_agent_id`
- if the newly focused agent already has a parked provider run, Arroba resumes it
- if the previous focused agent had the active provider run, Arroba parks it
- if the newly focused agent has no provider run yet, the session may temporarily have no active run until a launch/recovery path creates one

### Prompt routing

- direct user prompt submission always targets `focused_agent_id`
- before dispatch, the daemon ensures the focused agent has an active provider run
- if a parked run exists for that agent, it is resumed
- if no run exists yet, Arroba launches one for that agent using the agent's provider/model metadata plus the current default account profile behavior

## History and Output Model

### Agent-scoped history

- session history entries carry `agent_id`
- user prompts are recorded against the prompt's `target_agent_id`
- provider output is recorded against the `agent_id` owning the emitting provider run
- notices may also carry `agent_id` when they are clearly tied to one agent/provider run

### Live output routing

- terminal output records sent to the CLI carry `agent_id`
- the CLI appends focused-agent output to the rich active pane immediately
- the CLI appends non-focused-agent output to that agent's read-only pane preview and fetches the full agent history again when the user focuses that pane

## CLI Pane Model

### Layout

- the response area is split into one sub-area per active agent
- the focused agent owns the primary rich transcript pane
- non-focused agents render compact read-only panes with recent history previews
- pane headers always show agent ref, alias if present, and focus/working state

### Focus behavior

- `Ctrl+A` and `/agent cycle` update the focused pane header immediately
- after focus changes, the CLI refreshes provider metadata and loads the full history for the newly focused agent into the rich transcript pane
- prompt submission always uses the focused pane/agent

## Implementation Order

1. attach provider runs and history/output records to `agent_id`
2. make focus changes park/resume or recover the correct provider run
3. make prompt submit/recovery paths launch runs for the focused agent
4. split the CLI response area into per-agent panes
5. keep the focused pane rich and load agent-specific history on focus switches

## Out of Scope For This Slice

- daemon-scheduled workflow routing
- structured node handoffs/barriers
- hierarchical or circular workflow automation
- perfect multi-pane parity for every transcript interaction feature on day one
