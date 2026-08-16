# Agent-terminal validation battery

The agent peer must be validated as another terminal, not as a special provider integration. The battery is organized by the shared kernel authority:

| Area | Required evidence |
| --- | --- |
| MCP/JSONL contract | `tools/list`, bounded search, operation describe/schema, shell and structured-kernel execute, structured errors, bounded wait |
| Local kernel | create/attach/list/end sessions, explicit attachment and agent context, focus isolation, reconnect |
| Shared state | agent creates a workflow; another TUI/web/client reads it; TUI/web mutations are visible to the agent |
| Parallel agents | two agent peers address different `(session_id, attachment_id, agent_id)` tuples concurrently; no cross-target output |
| Workspaces | files, git status/commit/PR, worktree/live-sync changes are observed by every attached surface |
| Extensions and vault | MCP/skill/connector grants, vault lock/unlock and secret metadata stay kernel-owned and redacted |
| Slices and remote kernels | local, relay, remote-machine, and slice placement use the same peer with relay reconnect and stale-target checks |
| Workflows/publication | graph edits, runs, schedules, event bindings, publication and deployment controls are searchable and executable |
| Provider discovery | Codex, OpenCode, and Claude can register/discover the five-tool peer without a model turn |
| Resilience | reconnect, duplicate request replay, kernel restart, relay restart, stale snapshots, and bounded-result/500-agent scale tests |

Current runnable evidence:

```sh
pnpm --filter @chariox/kernel-client test
pnpm --filter @chariox/shell test
pnpm --filter @chariox/cli run agent-terminal:mcp-drill
```

The MCP drill starts a disposable kernel, performs the handshake, searches and describes the canonical registry, executes a native shell operation and a direct kernel parity operation, creates a session and workflow, runs a second `chariox-shell` process against that same session, verifies both terminals observe each other's workflow mutations, and proves that a focus mutation is rejected. The shell unit battery also exercises the equivalent `chariox-shell agent-terminal --jsonl` adapter and request cancellation. Existing relay, browser, workflow, provider, slice, vault, and scale drills remain the source of truth for those surfaces; they should be run with the same MCP peer in the mixed-surface matrix before PR merge.

## Full parity battery

Run the base peer checks first, then reuse the same explicit context tuple in each existing drill:

```sh
# shared state and mixed clients
pnpm --filter @chariox/cli run multi-user-cli-workflow:drill
pnpm --filter @chariox/cli run multi-user-workflow:drill
pnpm --filter @chariox/cli run tui-web-parity:drill

# relay, remote kernels, restart, and slices
pnpm --filter @chariox/cli run relay-identity:drill
pnpm --filter @chariox/cli run cloud-relay:drill
pnpm --filter @chariox/cli run remote-restart:drill
pnpm --filter @chariox/cli run slice:lifecycle-drill

# vault and extension surfaces
pnpm --filter @chariox/cli run agent-vault-credential:drill
pnpm --filter @chariox/cli run script-extension-agent:drill
pnpm --filter @chariox/cli run connector-extension-agent:drill
pnpm --filter @chariox/cli run runtime-register-mcp-use:drill

# workflows, publication, and deployment
pnpm --filter @chariox/cli run graph:drills
pnpm --filter @chariox/cli run workflow-code:artifact-drill
pnpm --filter @chariox/cli run publication:drill
pnpm --filter @chariox/cli run workflow-to-workflow-publication:drill

# provider MCP registration/discovery only (no model turn)
node apps/cli/scripts/provider-agent-terminal-mcp-discovery-drill.mjs

# resilience and durable observation
pnpm --filter @chariox/cli run local-restart:drill
pnpm --filter @chariox/cli run runtime-resilience:deterministic-chaos-drill
pnpm --filter @chariox/cli run runtime-resilience:chaos-matrix-drill
```

Each drill is complete only when a mutation made from one surface is read back from the agent peer and at least one independent TUI, web, or kernel client. Record the context tuple, request id, catalog revision, resulting session revision, and relay target id with the drill artifact. Provider-install registration is a separate client packaging test; it must not replace these runtime checks.

Provider-native execution, external provider prompts, and model-turn parity are intentionally deferred for this iteration. Do not run the provider live-turn or external-prompt drills as part of the AgentTerminal gate; the provider check above is limited to MCP registration/discovery and must not invoke a model.
