# Running Arroba Locally

## Status

Current local runtime guide for the daemon and CLI baseline.

## 1. Purpose

This document explains how to run the local Arroba processes that exist today:

- `arroba-daemon`
- `arroba-cli`
- the direct TypeScript CLI development path

It also summarizes the current env vars and local configuration knobs that affect those processes.

## 2. Current Local Runtime Shape

Today the local baseline is two main processes:

1. `arroba-daemon`
2. `arroba-cli`

`arroba-cli` is currently a Rust launcher that builds and starts the primary TypeScript CLI from `apps/cli`.

## 3. Prerequisites

Required for the current local path:

- Rust stable with `cargo`
- Node.js 22+
- `pnpm`
- Bun 1.2+ for the TypeScript CLI
- `opencode` installed locally, or `ARROBA_OPENCODE_BIN` set

Install workspace dependencies first:

```bash
pnpm install
```

## 4. Required Runtime Configuration

The current OpenCode path requires an explicit local port:

```bash
export ARROBA_OPENCODE_PORT=43111
```

Without that, the daemon will reject the OpenCode launch path.

## 5. Running The Daemon

Start the daemon from the repository root:

```bash
cargo run --manifest-path apps/daemon/Cargo.toml --bin arroba-daemon
```

What it does:

- boots the daemon runtime
- resolves the local IPC socket path
- manages sessions and provider runs
- launches and supervises the local OpenCode server path

## 6. Running The Primary CLI

In another terminal:

```bash
cargo run --manifest-path apps/daemon/Cargo.toml --bin arroba-cli
```

What it does:

- ensures the TypeScript CLI is built
- launches the `apps/cli` OpenTUI client through Bun
- connects that client to the daemon over local IPC

Common direct CLI options:

- `--session ID`
  - attach to a specific session ref by full id, unique id prefix, alias, or unique alias prefix

- `--socket PATH`
  - connect to a specific daemon socket instead of the default derived path

- `--create-session`
  - force creation of a new session instead of auto-attach

- `--alias NAME`
  - set the alias for `--create-session`

- `--delete-session REF`
  - delete a session by id or alias and exit without entering the TUI

- `--client-id ID`
  - override the attachment client id; default is `arroba-cli-<pid>`

- `--model MODEL`
  - select the provider model name used when the CLI launches a provider run; default is `default`

- `--account-profile PROFILE`
  - select the provider account profile used when the CLI launches a provider run; default is `default`

- `--workspace PATH`
  - override the logical workspace id; default is the current working directory

- `--worktree PATH`
  - override the logical worktree id; default is the workspace path

Example:

```bash
cargo run --manifest-path apps/daemon/Cargo.toml --bin arroba-cli -- \
  --workspace /path/to/repo \
  --worktree /path/to/repo \
  --model claude-sonnet-4 \
  --account-profile default
```

## 7. Running The TypeScript CLI Directly

For direct CLI development:

```bash
pnpm --filter @arroba/cli run dev
```

That path still expects the daemon to already be running.

## 8. Typical Local Startup Flow

In terminal 1:

```bash
export ARROBA_OPENCODE_PORT=43111
cargo run --manifest-path apps/daemon/Cargo.toml --bin arroba-daemon
```

In terminal 2:

```bash
export ARROBA_OPENCODE_PORT=43111
cargo run --manifest-path apps/daemon/Cargo.toml --bin arroba-cli
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
  - `/agent delete [name-or-alias]`
  - `/agent focus <id>`
  - `/agent list`
  - `/agent cycle`
  - `Ctrl+A` cycles focus to the next session agent
- `/view <split|individual>` switches between the focused transcript view and the current split-pane response layout
- deleting the currently attached session keeps the CLI process alive, clears the transcript/session chrome, renders an Arroba ASCII-art no-session landing state, returns the user to an unattached shell, and removes that session from future attach/list resolution

Current agent-command note:

- the daemon and TypeScript CLI now provide a real manual multi-agent session path for the local runtime
- the focused agent is the direct prompt target
- provider runs are tracked per top-level agent and can be parked/resumed as focus changes or the session goes idle
- session history and streamed output records now carry `agent_id`, which the CLI uses for per-agent transcript views
- `/view individual` and `/view split` switch between a single focused transcript and the current split-pane response layout
- the current split-pane UI is still an initial slice centered on the primary transcript plus up to two auxiliary panes
- daemon-scheduled workflow execution is still not implemented, and the OpenCode-backed multi-agent path still needs stabilization work

Outside the TUI:

- `arroba-cli logs ...` inspects the shared log root

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
cargo run --manifest-path apps/daemon/Cargo.toml --bin arroba-cli -- logs --follow
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
cargo run --manifest-path apps/daemon/Cargo.toml --bin arroba-cli

# Force attachment to a specific existing session.
cargo run --manifest-path apps/daemon/Cargo.toml --bin arroba-cli -- --session 7f9c2a1b

# Create a new named session explicitly.
cargo run --manifest-path apps/daemon/Cargo.toml --bin arroba-cli -- --create-session --alias main

# Delete a session by alias without opening the TUI.
cargo run --manifest-path apps/daemon/Cargo.toml --bin arroba-cli -- --delete-session main

# Use a custom workspace/worktree identity, which changes auto-attach behavior.
cargo run --manifest-path apps/daemon/Cargo.toml --bin arroba-cli -- \
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

- `ARROBA_OPENCODE_PORT`
  - required today
  - local port used for `opencode serve`

- `ARROBA_OPENCODE_BIN`
  - optional
  - overrides the `opencode` executable path

Examples:

```bash
export ARROBA_OPENCODE_PORT=43111
export ARROBA_OPENCODE_BIN=/absolute/path/to/opencode
```

### 12.2 Bun / CLI Launcher

- `BUN_BIN`
  - optional
  - overrides the Bun executable used by the Rust `arroba-cli` launcher

Example:

```bash
export BUN_BIN=/absolute/path/to/bun
```

### 12.3 Daemon Socket

- `ARROBA_DAEMON_SOCKET`
  - optional
  - overrides the local IPC socket path used by both daemon and CLI

- `ARROBA_DAEMON_ID`
  - optional
  - affects the default socket filename when no explicit socket path is set

If you override the socket path, both processes must use the same value.

Example:

```bash
export ARROBA_DAEMON_SOCKET=/tmp/arroba-demo.sock
```

Then run both the daemon and the CLI in shells that share that env var.

### 12.4 Logging

- `ARROBA_LOG_DIR`
  - optional
  - overrides the shared log root

- `ARROBA_LOG_LEVEL`
  - optional
  - one of `debug`, `info`, `warn`, `error`, `off`

See [LOGGING.md](/Users/miguel/arroba/docs/LOGGING.md) for the full logging guide.

## 12. Troubleshooting

### The CLI cannot connect to the daemon

Check:

- the daemon is already running
- both processes are using the same `ARROBA_DAEMON_SOCKET` if overridden
- the socket path is reachable by the current OS user

### OpenCode launch fails immediately

Check:

- `ARROBA_OPENCODE_PORT` is set
- `opencode` is on `PATH` or `ARROBA_OPENCODE_BIN` is set correctly
- the configured port is free before launch

### The Rust launcher says Bun is missing

Either install Bun or set:

```bash
export BUN_BIN=/absolute/path/to/bun
```

### You want to inspect what happened across daemon and CLI

Use the built-in viewer:

```bash
cargo run --manifest-path apps/daemon/Cargo.toml --bin arroba-cli -- logs --follow
```

or inspect the shared NDJSON files directly:

```bash
tail -f ~/.local/state/arroba/logs/*.ndjson
```

## 13. Current Limitations

- There is no single combined launcher yet; daemon and CLI are still separate processes.
- OpenCode currently requires explicit `ARROBA_OPENCODE_PORT`.
- the previous Rust-only CLI has been removed; the supported local client paths are `arroba-cli` and direct `apps/cli` development
- The OpenCode-backed multi-agent runtime path is not fully stable yet; the daemon integration suite still has failing multi-agent adapter scenarios.
- The current split-pane UI is still limited to the primary transcript plus up to two auxiliary panes even though the runtime model now tracks more session agents than that.
- Slash-command capability work, broader provider support, and daemon-scheduled workflow execution remain open beyond the current manual multi-agent slice.
