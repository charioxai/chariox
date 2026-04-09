# CLI Commands

This page tracks the current user-facing slash commands and keyboard shortcuts in the Arroba CLI.

The source of truth is the current TypeScript CLI implementation in:

- `apps/cli/src/index.tsx`
- `apps/cli/src/command-center.ts`
- `apps/cli/src/commands.ts`
- `apps/cli/src/command-actions.ts`

## Core Commands

### `/stop`

Request cancellation of the active provider turn.

### `/exit`

Exit the CLI.

### `/waiting`

Leave the current workspace/session view and return to the waiting room.

## Provider Selection

### `/provider <name>`

Select the active provider backend.

Supported values:

- `opencode`
- `codex`

Additional provider commands:

- `/provider status [name]`
  Show auth status for the current or named provider.
- `/provider login [name]`
  Start provider-native login for the current or named provider. For Codex this returns a device-login URL and one-time code.
- `/provider logout [name]`
  Clear the current or named provider login. For Codex this runs the local `codex logout` flow on the host machine.
- `/provider reauth [name]`
  Log out the current or named provider, then start a fresh provider-native login flow.

### `/model <id>`

Select the active model.

### `/variant <name>`

Select the active model variant.

### `/view <mode>`

Set the multi-agent response layout.

Supported values:

- `split`
- `individual`

## Session Commands

### `/session new [alias]`

Create and attach to a new session.

### `/session create [alias]`

Alias for `/session new`.

### `/session attach <ref>`

Attach to a session by:

- full session id
- unique id prefix
- alias
- unique alias prefix

### `/session list`

List available sessions in the current workspace.

### `/session delete [ref]`

Delete the current session, or the referenced session if a ref is provided.

## Agent Commands

### `/agent spawn [alias] [model]`

### `/agent spawn <number_of_agents>`

Spawn a new agent in the current session.

### `/agent delete [ref]`

Delete the focused agent, or a referenced agent.

### `/agent destroy [ref]`

Alias for `/agent delete`.

### `/agent focus <id>`

Focus a specific agent.

### `/agent list`

List agents in the current session.

### `/agent cycle`

Cycle focus to the next agent.

## Workflow Commands

Current status:

- workflow definition commands and basic workflow runtime commands are available in the CLI
- the daemon executes endpoint-triggered workflow runs with daemon-owned downstream handoffs and explicit `output.message` payloads, plus optional artifact refs when a workflow-owned artifact is produced
- join nodes now buffer inbound workflow messages on the target side and, by default, wait for all upstream parents before starting one aggregated node run
- the workflow outline now shows graph structure for every node, and expands the selected node with run status and other non-graph attributes
- there is still no `/workflow schedule` command

### `/workflow`

Open the workflow outline. If already on the workflow screen, nothing changes.

### `/workflow list`

List workflows in the current session/workspace.

### `/workflow show <workflow-ref>`

Resolve and show a workflow by id or alias.

### `/workflow new [alias]`

Create a new workflow with an optional alias.

### `/workflow run <workflow-ref> <endpoint-ref> [prompt]`

Invoke a workflow endpoint, optionally with an invocation prompt.

### `/workflow runs [workflow-ref]`

List workflow runs in the current session, optionally filtered to one workflow.

### `/workflow cancel <run-ref>`

Cancel a workflow run by id or unique prefix.

### `/workflow <workflow-ref> <alias>`

Assign or update the alias of an existing workflow.

### `/workflow <workflow-ref> <from-node-or-agent-ref> <to-node-or-agent-ref>`

Shorthand for creating a workflow edge. Each endpoint can be:

- a workflow node id
- an agent reference (agent id, hash ref, or alias) that maps to exactly one node in that workflow

### `/workflow node add <workflow-ref> <agent-id>`

Add a workflow node bound to an existing agent.

### `/workflow node remove <workflow-ref> <node-id>`

Remove a workflow node. Connected edges and endpoints targeting that node are also removed by the kernel.

### `/workflow edge add <workflow-ref> <from-node-or-agent-ref> <to-node-or-agent-ref>`

Add a directed edge between two existing workflow nodes.

### `/workflow edge remove <workflow-ref> <edge-id>`

Remove a workflow edge.

### `/workflow endpoint new <workflow-ref> <entry-node-id> [alias]`

Create a workflow endpoint targeting one entry node.

### `/workflow endpoint alias <workflow-ref> <endpoint-ref> <alias>`

Assign or update the alias of a workflow endpoint.

### `/workflow endpoint bind <workflow-ref> <endpoint-ref> <entry-node-id>`

Rebind an existing workflow endpoint to a different entry node.

## Keyboard Shortcuts

### `Tab`

Cycle to the next agent.

### `Ctrl+Tab`

Switch between the agent screens and the workflow outline.

### `Ctrl+T`

Open the hotkeys/help overlay.

### `Ctrl+E`

Exit the CLI with the same behavior as `/exit`.
