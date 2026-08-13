# Running Chariox Locally

## Status

Current local runtime guide for the daemon and CLI baseline.

## 1. Purpose

This document explains how to run the local Chariox processes that exist today:

- `chariox-kernel`
- `chariox-cli`
- the direct TypeScript CLI development path

It also summarizes the current env vars and local configuration knobs that affect those processes.

## 2. Current Local Runtime Shape

Today the local baseline is two main processes:

1. `chariox-kernel`
2. `chariox-cli`

`chariox-cli` is currently a Rust launcher that builds and starts the primary TypeScript CLI from `apps/cli`.

For normal local startup, use `pnpm run start:kernel` and `pnpm run start:cli`.
Those wrappers launch the cached debug binaries directly and only rebuild when
the Rust sources are newer than the binary. Use `cargo run ...` only when you
are explicitly debugging the Rust layer and want Cargo to stay in the loop.

## 3. Prerequisites

Required for the current local path:

- Rust stable with `cargo`
- Node.js 22+
- `pnpm`
- Bun 1.2+ for the TypeScript CLI
- `opencode` installed locally, or `CHARIOX_OPENCODE_BIN` set
- `codex` installed locally, or `CHARIOX_CODEX_BIN` set, if you want to use the Codex backend

Install workspace dependencies first:

```bash
pnpm install
```

## 4. Required Runtime Configuration

The current structured provider paths require explicit local ports:

```bash
export CHARIOX_OPENCODE_PORT=43111
export CHARIOX_CODEX_PORT=43112
```

Without that, the daemon will reject the corresponding managed launch path.

## 5. Running The Daemon

Start the daemon from the repository root:

```bash
pnpm run start:kernel
```

What it does:

- boots the daemon runtime
- binds the kernel WebSocket listener
- manages sessions and provider runs
- launches and supervises the local OpenCode and Codex structured server paths

## 6. Running The Primary CLI

In another terminal:

```bash
pnpm run start:cli
```

What it does:

- ensures the TypeScript CLI is built
- launches the `apps/cli` OpenTUI client through Bun
- connects that client to the daemon over the kernel WebSocket transport

Common direct CLI options:

- `--kernel-url URL`
  - connect to a specific kernel WebSocket URL instead of the default derived one

- `--session ID`
  - attach to a specific session ref by full id, unique id prefix, alias, or unique alias prefix

- `--socket PATH`
  - legacy compatibility path for the older local socket transport

- `--create-session`
  - force creation of a new session instead of auto-attach

- `--alias NAME`
  - set the alias for `--create-session`

- `--delete-session REF`
  - delete a session by id or alias and exit without entering the TUI

- `--client-id ID`
  - override the attachment client id; default is `chariox-cli-<pid>`

- `--model MODEL`
  - select the provider model name used when the CLI launches a provider run; default is `default`

- `--account-profile PROFILE`
  - select the provider account profile used when the CLI launches a provider run; default is `default`

- `--workspace PATH`
  - override the logical workspace id; default is the current working directory

- `--worktree PATH`
  - override the logical worktree id; default is the workspace path

Interactive placement commands:

- `/session new [DIR]`
  - create and attach a new session in `DIR`; if omitted, uses the directory where the CLI was launched

- `/session new --dir DIR`
  - explicit form for creating the new session in an existing local directory

- `/session new --worktree DIR --branch BRANCH [--from REF]`
  - create a local git worktree before creating the session, then launch the session in that worktree
  - `DIR` is resolved relative to the current session/CLI worktree; if omitted, Chariox creates a sibling worktree path from the repo name and branch

- `/agent spawn [ALIAS] [MODEL] --dir DIR`
  - spawn a local agent in an existing local directory

- `/agent spawn [ALIAS] [MODEL] --worktree DIR --branch BRANCH [--from REF]`
  - create a local git worktree before spawning the agent, then launch that agent's provider process in the new worktree

- `/agent spawn [ALIAS] [MODEL] --machine MACHINE --dir REMOTE_DIR`
  - spawn a remote agent in an existing remote directory

- `/agent spawn [ALIAS] [MODEL] --machine MACHINE --worktree REMOTE_DIR --branch BRANCH [--from REF]`
  - ask the worker kernel to create the git worktree on the remote machine, then spawn the remote agent there
  - the worker must already have `git` installed and configured, and the worker kernel must be running from the source repo; Chariox surfaces git/repo/configuration failures directly

Example:

```bash
pnpm run start:cli -- \
  --workspace /path/to/repo \
  --worktree /path/to/repo \
  --model claude-sonnet-4 \
  --account-profile default
```

## 7. Running The TypeScript CLI Directly

For direct CLI development:

```bash
pnpm --filter @chariox/cli run dev
```

That path still expects the daemon to already be running.

## 8. Typical Local Startup Flow

In terminal 1:

```bash
export CHARIOX_OPENCODE_PORT=43111
pnpm run start:kernel
```

In terminal 2:

```bash
export CHARIOX_OPENCODE_PORT=43111
pnpm run start:cli
```

## 9. Current CLI Controls

Inside the CLI:

- `/stop` requests cancellation of the active provider turn
- `/exit` currently detaches and exits the CLI
- in-session temporary commands:
  - `/session create [alias]`
  - `/session attach <ref>`
  - `/session delete [ref]`
- manual multi-agent session commands:
  - `/agent spawn [alias] [model]`
  - `/agent spawn [alias] [model] --dir <directory>`
  - `/agent spawn [alias] [model] --machine <machine-ref> --dir <remote-directory>`
  - `/agent spawn <number_of_agents>`
  - `/agent delete [name-or-alias]`
  - `/agent focus <id>`
  - `/agent list`
  - `/agent cycle`
  - `Tab` cycles focus to the next session agent
- `/agent spawn --dir` sets the provider working directory at spawn time. Without `--machine`, the directory is resolved on the local/home machine. With `--machine`, the directory is interpreted on the worker machine and is required; Chariox does not currently change an existing agent's directory after spawn.
- `/view <split|individual>` switches between the focused transcript view and the current split-pane response layout
- deleting the currently attached session keeps the CLI process alive, clears the transcript/session chrome, renders a Chariox ASCII-art no-session landing state, returns the user to an unattached shell, and removes that session from future attach/list resolution

Current agent-command note:

- the daemon and TypeScript CLI now provide a real manual multi-agent session path for the local runtime
- the focused agent is the direct prompt target
- provider runs are tracked per top-level agent and can be parked/resumed as focus changes or the session goes idle
- session history and streamed output records now carry `agent_id`, which the CLI uses for per-agent transcript views
- `/view individual` and `/view split` switch between a single focused transcript and the current split-pane response layout
- the current split-pane UI is still an initial slice centered on the primary transcript plus up to two auxiliary panes
- daemon-scheduled workflow execution is still not implemented, and the OpenCode-backed multi-agent path still needs stabilization work

Outside the TUI:

- `pnpm run start:cli -- logs ...` inspects the shared log root

Current log viewer options:

- `--follow`
  - keep tailing appended records

- `--process-kind KIND`
  - filter by process kind such as `daemon`, `cli`, or `cli-launcher`

- `--component NAME`
  - filter by logger component name

- `--session ID`
  - filter by a specific session id

- `--provider-run ID`
  - filter by a specific provider run id

- `--client-id ID`
  - filter by a specific client id

- `--level LEVEL`
  - filter by `debug`, `info`, `warn`, `error`, or `off`

- `--limit N`
  - show only the newest `N` matching records before follow mode starts; default is `200`

Example:

```bash
pnpm run start:cli -- logs --follow
```

## 10. Session Selection Behavior

By default, the CLI tries to reattach before it creates anything new.

- If you pass `--session REF`, the CLI resolves that session ref directly.
- If you pass `--create-session`, the CLI always creates a new session before attaching.
- If you pass `--delete-session REF`, the CLI deletes that session and exits without entering the TUI.
- If you do not pass `--session`, the CLI lists sessions from the daemon and filters to sessions whose `workspace_id` and `worktree_id` match the effective CLI `--workspace` and `--worktree` values.
- Ended sessions are excluded from auto-attach.
- If multiple matching non-ended sessions exist, the CLI sorts them by `created_at_ms` and picks the newest one.
- If no matching non-ended session exists, the CLI creates a new session.

Important consequences:

- Running the CLI from different directories changes the default `workspace` and can change which session is selected.
- Overriding `--workspace` or `--worktree` changes the session pool considered attachable.
- Session ids are 16-character lowercase hexadecimal values.
- Sessions may also have an optional alias.
- Users can refer to sessions by:
  - full id
  - unique id prefix
  - alias
  - unique alias prefix
- Alias matching is workspace-scoped and normalized to lowercase.
- Ambiguous references fail with an explicit error instead of picking one arbitrarily.

Examples:

```bash
# Reattach to the newest open session for the current directory.
pnpm run start:cli

# Force attachment to a specific existing session.
pnpm run start:cli -- --session 7f9c2a1b

# Create a new named session explicitly.
pnpm run start:cli -- --create-session --alias main

# Delete a session by alias without opening the TUI.
pnpm run start:cli -- --delete-session main

# Use a custom workspace/worktree identity, which changes auto-attach behavior.
pnpm run start:cli -- \
  --workspace /tmp/demo-workspace \
  --worktree /tmp/demo-workspace
```

## 11. Current Local Configuration

### 12.1 Transcript Highlighting

The TypeScript CLI now renders assistant/reasoning markdown more richly and syntax-highlights fenced code blocks in the terminal.

Notes:

- this uses OpenTUI's markdown/code rendering path, not LSP
- some language parsers are downloaded on first use when a matching fenced language is displayed
- if a parser is unavailable, the code block still renders as plain text

### 12.2 OpenCode

- `CHARIOX_OPENCODE_PORT`
  - required for the managed OpenCode launch path
  - local port used for `opencode serve`

- `CHARIOX_OPENCODE_BIN`
  - optional
  - overrides the `opencode` executable path

Examples:

```bash
export CHARIOX_OPENCODE_PORT=43111
export CHARIOX_OPENCODE_BIN=/absolute/path/to/opencode
```

### 12.3 Codex

- `CHARIOX_CODEX_PORT`
  - required for the managed Codex launch path
  - local port used for `codex app-server --listen ws://127.0.0.1:<port>`

- `CHARIOX_CODEX_BIN`
  - optional
  - overrides the `codex` executable path

Examples:

```bash
export CHARIOX_CODEX_PORT=43112
export CHARIOX_CODEX_BIN=/absolute/path/to/codex
```

Codex login is available through the CLI:

```text
/provider codex
/provider status
/provider login
/provider logout
/provider reauth
```

`/provider login` returns the provider-native device-login URL and code.
`/provider logout` clears the stored Codex login on the host machine.
`/provider reauth` clears the stored login, then starts a fresh device-login flow.

### 12.4 Bun / CLI Launcher

- `BUN_BIN`
  - optional
  - overrides the Bun executable used by the Rust `chariox-cli` launcher

Example:

```bash
export BUN_BIN=/absolute/path/to/bun
```

### 12.5 Kernel Transport

- `CHARIOX_DAEMON_SOCKET`
  - optional
  - legacy compatibility override for the older local IPC socket path

- `CHARIOX_DAEMON_ID`
  - optional
  - affects the default socket filename when no explicit socket path is set

- `CHARIOX_KERNEL_URL`
  - optional
  - explicit kernel WebSocket URL for the CLI, for example `ws://127.0.0.1:43118/kernel`

- `CHARIOX_KERNEL_HOST`
  - optional
  - host bound by the daemon kernel WebSocket listener when `CHARIOX_KERNEL_URL` is not used on the CLI

- `CHARIOX_KERNEL_PORT`
  - optional
  - port bound by the daemon kernel WebSocket listener and used by the CLI default URL derivation

If you override the kernel host/port, both processes must use the same values or the CLI must be launched with a matching `CHARIOX_KERNEL_URL`.

Example:

```bash
export CHARIOX_DAEMON_SOCKET=/tmp/chariox-demo.sock
```

Then run both the daemon and the CLI in shells that share that env var.

For the default kernel WebSocket path:

```bash
export CHARIOX_KERNEL_HOST=127.0.0.1
export CHARIOX_KERNEL_PORT=43118
```

### 12.4 Logging

- `CHARIOX_LOG_DIR`
  - optional
  - overrides the shared log root

- `CHARIOX_LOG_LEVEL`
  - optional
  - one of `debug`, `info`, `warn`, `error`, `off`

See [LOGGING.md](/Users/miguel/chariox/docs/LOGGING.md) for the full logging guide.

## 12. Troubleshooting

### The CLI cannot connect to the daemon

Check:

- the daemon is already running
- both processes are using the same `CHARIOX_DAEMON_SOCKET` if overridden
- the socket path is reachable by the current OS user

### OpenCode launch fails immediately

Check:

- `CHARIOX_OPENCODE_PORT` is set
- `opencode` is on `PATH` or `CHARIOX_OPENCODE_BIN` is set correctly
- the configured port is free before launch

### The Rust launcher says Bun is missing

Either install Bun or set:

```bash
export BUN_BIN=/absolute/path/to/bun
```

### You want to inspect what happened across daemon and CLI

Use the built-in viewer:

```bash
pnpm run start:cli -- logs --follow
```

or inspect the shared NDJSON files directly:

```bash
tail -f ~/.local/state/chariox/logs/*.ndjson
```

## 13. Workspace live sync

Workspace Live Sync keeps selected Chariox session worktrees aligned at provider turn boundaries. Managed mode enforces Chariox runtime/MCP write tools and performs collision checks before syncing writes. Tracked mode watches file changes made during Chariox-managed provider turns and syncs those changes without write enforcement. Changes made outside Chariox-managed turns are ignored.

Workspace live sync defaults are read from the Chariox user config TOML:

```text
$XDG_CONFIG_HOME/chariox/config.toml
```

If `XDG_CONFIG_HOME` is unset, the fallback path is:

```text
~/.chariox/config.toml
```

The default policy is Off:

```toml
version = 1

[providers]
default = "opencode"
model = "default"
account_profile = "default"
workspace_live_sync = "off"
```

Use `managed` to enforce Chariox runtime/MCP writes for protected roots, or `tracked` to allow provider-native writes and sync them at turn end. Session commands switch the current attached session only:

```text
/workspace sync off
/workspace sync managed
/workspace sync tracked
/workspace sync status
```

Set the default for new sessions without changing the attached session:

```text
/workspace sync default off
/workspace sync default managed
/workspace sync default tracked
```

Workspace Live Sync protects and syncs only the selected worktree Git root plus explicitly attached workspace-link roots. Sibling and unrelated repositories outside those roots stay writable and unsynced. Workspace Live Sync uses `.charioxignore` for per-workspace exclusions. New ignore files are initialized from `.gitignore` when one exists, otherwise they start empty. Runtime/private paths such as `.git/` and Chariox state are always excluded.

You can inspect or modify the same default policy through the lower-level config commands:

```text
/config show
/config path
/config workspace-live-sync off
/config workspace-live-sync managed
/config workspace-live-sync tracked
/config unset providers.workspace_live_sync
```

To disable the policy for all managed providers:

```toml
[providers]
workspace_live_sync = "off"
```

Remote leased agents use the same workspace live sync policy as local agents.

Agent-facing tools:

- `chariox.read_artifact`
- `chariox.write_artifact`
- `chariox.edit_artifact`
- `chariox.apply_patch`
- `chariox.move_artifact`
- `chariox.delete_artifact`
- `chariox.list_extensions`
- `chariox.request_extension`

The same operations are also exposed as short aliases: `read_artifact`, `write_artifact`, `edit_artifact`, `apply_patch`, `move_artifact`, `delete_artifact`, `list_extensions`, and `request_extension`. Codex may display these as provider-qualified tool names such as `mcp__chariox__read_artifact`.

`list_extensions` lets an agent discover Chariox-managed MCPs, skills, and scripts available in the current workspace. `request_extension` accepts `kind` (`mcp`, `skill`, or `script`) and `name`; script requests also require an `environment`. V1 grants valid requests automatically. A requested MCP is rendered into provider-native MCP config by Chariox-managed provider conversation activation; if the agent requested it mid-turn, Chariox reloads after that turn and sends an automatic continuation prompt. A requested skill returns the full `SKILL.md` body by default and is usable immediately in the current turn. A requested script becomes an agent-facing runtime MCP tool backed by the registered local script and external environment.

Text artifacts use snapshot-aware fine-grained coordination. If a stale edit overlaps an external or concurrent managed change, the tool rejects it and the agent should reread the artifact before retrying. Non-text artifacts use `domain: "opaque"` with base64 payloads and whole-file coordination in v1.

Remote leased agents that are working in the same repo/branch as the home session, or in worktrees explicitly attached to the same session workspace link, forward workspace live sync through the home kernel. If workspace identity changes while a managed run is active, workspace live sync rejects the request until the run rejoins a valid coordinated workspace.

## 14. Workflow Queues

Workflow prompt queue limits are owned by the kernel config and read from the Chariox user config TOML. If unset, the kernel defaults to `1024` queues per workflow.

```toml
[workflow]
max_queues_per_workflow = 10
```

The kernel rejects workflow queue creation if this setting is zero or if a workflow already has the configured number of queues.

## 15. Current Limitations

- There is no single combined launcher yet; daemon and CLI are still separate processes.
- OpenCode currently requires explicit `CHARIOX_OPENCODE_PORT`.
- the previous Rust-only CLI has been removed; the supported local client paths are `chariox-cli` and direct `apps/cli` development
- The OpenCode-backed multi-agent runtime path still needs more stabilization work, but the current daemon integration suite is green again.
- The current split-pane UI is still limited to the primary transcript plus up to two auxiliary panes even though the runtime model now tracks more session agents than that.
- Slash-command capability work, broader provider support, and daemon-scheduled workflow execution remain open beyond the current manual multi-agent slice.
