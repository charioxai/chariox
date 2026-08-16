# Agent terminal

`chariox-agent-terminal` is a stdio MCP peer backed by the ordinary Chariox kernel client. It is deliberately stateless: every execution carries its filesystem workspace/worktree paths plus any required kernel workspace/worktree IDs, session, attachment, agent, and workflow context. Filesystem paths and kernel resource IDs are separate fields; the peer never infers one from the other. It never changes human focus or selects an implicit focused agent. This is the agent-terminal client surface, not a provider-external prompt channel.

An MCP host learns the interface through the normal handshake:

1. `tools/list` returns exactly `chariox_status`, `chariox_search`, `chariox_describe`, `chariox_execute`, and `chariox_wait`.
2. `chariox_search` returns a bounded set of matching kernel operation contracts (20 by default, at most 50).
3. `chariox_describe` returns one stable operation contract, including required context, targets, schema, projections, and parity variants.
4. `chariox_execute` accepts either the same command language as `chariox-shell`, or an `operation_id` plus structured `input` from `chariox_describe`; parity operations are sent to the kernel using their native request variant. It returns output plus the next explicit context.
5. `chariox_wait` pumps the shared session state for one explicitly targeted attachment/agent. It is bounded to two minutes, treats queued/dispatching/settling work as incomplete, and can be cancelled with the standard MCP `notifications/cancelled` notification.
6. `chariox_status` reports connection and registry state plus the explicitly targeted session snapshot.

Hosts that do not implement MCP can run `chariox-shell agent-terminal --jsonl`. The JSONL adapter accepts one request per line with `op` equal to `status`, `search`, `describe`, `execute`, or `wait`, and returns `{ "id", "ok", "result" }` (or `{ "id", "ok": false, "error" }`) per line.

`source` and `run` retain the shell language's worktree-relative script resolution, but every nested command is revalidated and the resolved script must remain inside the selected worktree (including symlink targets). Nested source lines are not echoed into MCP output. Presentation-only focus/view commands and prompts without the explicit session, attachment, and agent tuple are rejected even when they come from a sourced script.

Local use:

```json
{
  "command": "chariox-agent-terminal",
  "env": { "CHARIOX_KERNEL_URL": "ws://127.0.0.1:43118/kernel" }
}
```

Remote use uses the existing relay transport. Set `CHARIOX_KERNEL_URL` to the relay websocket endpoint, `CHARIOX_RELAY_AUTH_TOKEN` to the scoped relay token, and optionally `CHARIOX_RELAY_TARGET_DAEMON_ID` or `CHARIOX_RELAY_TARGET_DAEMON_ALIAS`. No harness identity is required or trusted.

For a one-shot remote target bootstrap through the same home-kernel path as the TUI, set `CHARIOX_AGENT_TERMINAL_HOME_KERNEL_URL` to the home kernel websocket endpoint and `CHARIOX_AGENT_TERMINAL_KERNEL_REF` to the selected remote kernel. `CHARIOX_AGENT_TERMINAL_MACHINE_REF` and `CHARIOX_AGENT_TERMINAL_SESSION_ID` are optional scopes. The peer asks the home kernel for a target-scoped relay connection, then uses the ordinary relay transport; the returned token is held only in process memory. This is a bounded bootstrap connection. A launcher must re-bootstrap and restart/reattach before token expiry or after authentication failure; static MCP environment tokens are not a durable renewal mechanism.

Meta mode stays on the normal prompt path: use `prompt <agent-id> "/meta <task>"` with the explicit session and attachment context. The kernel performs the same mode transition and durable projection as a human `/meta` prompt.

Agent-terminal prompts remain Chariox-owned (`prompt_origin: chariox`) and carry the orthogonal `prompt_source: agent_terminal` tag through prompt state, active-turn projection, and history. `prompt_origin: external` remains reserved for provider-native turns observed from another app, which use `prompt_source: provider_external`. Provider-external prompting is not part of this runtime iteration; the distinct tag is retained only for the existing provider-observation boundary.

The provider-install step is intentionally separate from this runtime contract. A future installer may register this stdio command with Codex, OpenCode, or Claude, but registration must not create a second kernel authority or provider-specific permission model.
