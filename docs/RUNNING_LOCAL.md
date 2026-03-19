# Running Arroba Locally

## Status

Current local runtime guide for the daemon and CLI baseline.

## 1. Purpose

This document explains how to run the local Arroba processes that exist today:

- `arroba-daemon`
- `arroba-cli`
- the direct TypeScript CLI development path
- the phased-out Rust CLI fallback

It also summarizes the current env vars and local configuration knobs that affect those processes.

## 2. Current Local Runtime Shape

Today the local baseline is two main processes:

1. `arroba-daemon`
2. `arroba-cli`

`arroba-cli` is currently a Rust launcher that builds and starts the primary TypeScript CLI from `apps/cli`.

The old Rust-only CLI still exists as `arroba-cli-rust`, but it is phased out and should be treated only as a fallback/debugging path.

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

## 7. Running The TypeScript CLI Directly

For direct CLI development:

```bash
pnpm --filter @arroba/cli run dev
```

That path still expects the daemon to already be running.

## 8. Running The Phased-Out Rust CLI

Fallback/debug-only path:

```bash
cargo run --manifest-path apps/daemon/Cargo.toml --bin arroba-cli-rust
```

Use this only when comparing behavior or debugging a daemon-contract issue. New client work should target `apps/cli`.

## 9. Typical Local Startup Flow

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

## 10. Current CLI Controls

Inside the CLI:

- `/stop` requests cancellation of the active provider turn
- `/exit` detaches or ends the local session and exits

Outside the TUI:

- `arroba-cli logs ...` inspects the shared log root

Example:

```bash
cargo run --manifest-path apps/daemon/Cargo.toml --bin arroba-cli -- logs --follow
```

## 11. Current Local Configuration

### 11.1 OpenCode

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

### 11.2 Bun / CLI Launcher

- `BUN_BIN`
  - optional
  - overrides the Bun executable used by the Rust `arroba-cli` launcher

Example:

```bash
export BUN_BIN=/absolute/path/to/bun
```

### 11.3 Daemon Socket

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

### 11.4 Logging

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
- `arroba-cli-rust` is still present, but it is no longer the primary client path.
- The current local runtime is still single-agent focused; slash-command capability work and broader provider support remain in M3.
