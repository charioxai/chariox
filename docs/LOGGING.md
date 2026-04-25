# Arroba Logging Guide

## Status

Current local logging guide for the daemon and CLI runtime.

## 1. Purpose

This document explains how Arroba logging works today:

- where logs are written
- how to enable or quiet them
- how to configure them
- how to inspect them
- what is and is not logged by default

## 2. Current Coverage

The shared logging system currently covers:

- `arroba-kernel`
- the Rust `arroba-cli` launcher
- the primary TypeScript CLI process in `apps/cli`
- the Fastify server process in `apps/server`

Future work will extend the same logging model to provider-side helper processes.

## 3. Log Format

Arroba uses `NDJSON`:

- one JSON object per line
- one file per process
- one shared machine-local log root

This makes it easy to:

- follow logs with `tail -f`
- filter logs with `jq`
- merge logs across daemon and client processes
- build future log-collection/debug-bundle tooling

## 4. Default Log Root

Arroba resolves the log root in this order:

1. `ARROBA_LOG_DIR`
2. `XDG_STATE_HOME/arroba/logs`
3. `~/.local/state/arroba/logs`
4. `./.arroba/logs`

Each process writes its own `.ndjson` file under that root.

## 5. Default Logging Behavior

By default:

- logging is enabled
- level defaults to `info`
- metadata, lifecycle, warnings, and errors are logged
- prompt text and provider output content are not logged by default

Current retention defaults:

- about 7 days
- about 200 MB total per log root

## 6. Configuration

### 6.1 Log Root

Set a custom log directory:

```bash
export ARROBA_LOG_DIR=/absolute/path/to/arroba-logs
```

### 6.2 Log Level

Supported values:

- `debug`
- `info`
- `warn`
- `error`
- `off`

Examples:

```bash
export ARROBA_LOG_LEVEL=debug
```

```bash
export ARROBA_LOG_LEVEL=warn
```

```bash
export ARROBA_LOG_LEVEL=off
```

Notes:

- `debug` is the most verbose supported mode today.
- `off` disables record emission for the covered processes.

## 7. Activating And Deactivating Logs

### Enable normal logs

Normal logging is already on by default.

If you want to be explicit:

```bash
export ARROBA_LOG_LEVEL=info
```

### Enable verbose debugging

```bash
export ARROBA_LOG_LEVEL=debug
```

### Reduce noise

```bash
export ARROBA_LOG_LEVEL=warn
```

or:

```bash
export ARROBA_LOG_LEVEL=error
```

### Disable log emission

```bash
export ARROBA_LOG_LEVEL=off
```

If you want disabled logging plus an isolated runtime directory during debugging experiments:

```bash
export ARROBA_LOG_LEVEL=off
export ARROBA_LOG_DIR="$(pwd)/.arroba/logs"
```

## 8. Inspecting Logs

### 8.1 Built-In Viewer

The primary inspection command is:

```bash
arroba-cli logs
```

Common examples:

```bash
arroba-cli logs --follow
```

```bash
pnpm run start:cli -- logs
```

```bash
pnpm run start:cli -- logs --follow
```

```bash
pnpm run start:cli -- logs --process-kind daemon
```

```bash
pnpm run start:cli -- logs --level error
```

```bash
pnpm run start:cli -- logs --session session-1
```

```bash
arroba-cli logs --process-kind daemon --level error
```

```bash
arroba-cli logs --provider-run provider-run-3 --follow
```

Supported filters today:

- `--follow`
- `--process-kind`
- `--component`
- `--session`
- `--provider-run`
- `--client-id`
- `--level`
- `--limit`

If you are launching through Cargo:

```bash
cargo run --manifest-path apps/kernel/Cargo.toml --bin arroba-cli -- logs --follow
```

If you are running the TypeScript CLI directly:

```bash
pnpm --filter @arroba/cli run dev -- logs --follow
```

### 8.2 Standard Shell Tools

Follow all local Arroba logs:

```bash
tail -f ~/.local/state/arroba/logs/*.ndjson
```

Pretty-print records:

```bash
jq . ~/.local/state/arroba/logs/*.ndjson
```

Filter one session:

```bash
jq 'select(.session_id=="session-1")' ~/.local/state/arroba/logs/*.ndjson
```

Show only errors:

```bash
jq 'select(.level=="error")' ~/.local/state/arroba/logs/*.ndjson
```

Render a compact table:

```bash
jq -r '[.timestamp_ms, .process_kind, .component, .level, .message] | @tsv' ~/.local/state/arroba/logs/*.ndjson
```

## 9. Record Shape

Current records include fields such as:

- `timestamp_ms`
- `level`
- `process_kind`
- `pid`
- `component`
- `message`
- `log_path`
- `session_id` when known
- `provider_run_id` when known
- `attachment_id` or `client_id` when known
- request/trace fields when present

Not every record will include every correlation field.

## 10. Logging Policy

The shared logger is the only supported committed logging mechanism for Arroba runtime diagnostics.

Do not add:

- one-off debug env vars for separate log files
- temporary `appendFileSync` loggers
- committed `eprintln!` or `console.log` debug paths as a parallel mechanism

If more visibility is needed, extend the shared logger and document the new fields or commands.

## 11. Current Limitations

- The built-in log viewer is intentionally simple and local-first.
- There is not yet a session-scoped debug-bundle export command.
- Server and future provider-helper processes are not yet on the shared logger.
- Prompt/provider content capture is intentionally conservative and not yet exposed as a separate opt-in mode.
