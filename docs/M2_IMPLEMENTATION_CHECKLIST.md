# M2 Implementation Checklist

## Status

Execution checklist for **M2 - End-to-End Local OpenCode Baseline**.

M2 is now the highest-priority milestone.

This checklist translates the updated roadmap into concrete implementation work for the repository state after M1. The current runtime already has in-memory session lifecycle, PTY management, prompt submission, terminal output fan-out, a local in-process API, and a `dev-stub` provider. M2 is about turning that runtime into a real local daemon + CLI path with a real first provider.

## 1. Target M2 outcomes

From `docs/ROADMAP.md`, M2 outcomes are:

- OpenCode provider adapter wired through the daemon
- local CLI client attached to daemon-managed sessions
- single-agent prompt submission from an input field
- live terminal/output streaming from the active OpenCode run back into the CLI as output appears
- stable session creation/attach/launch/prompt/output loop for one local user on one machine
- no workflow mode, no remote relay, no web app, and no provider switching in this milestone

Exit criteria:

- local CLI can create or attach to a session, launch OpenCode, submit a prompt, and stream output in real time
- daemon remains the authority for session and PTY/provider-run lifecycle during that flow

## 2. M2 implementation principles

- Prefer one real end-to-end path over broader but partial feature coverage.
- Reuse the existing daemon runtime and local request/response surface wherever possible instead of redesigning prompt/output flow.
- Keep the first provider integration PTY-first and wrapper-style.
- Defer slash-command UX, broader capability work, workflows, and relay/web concerns until the local OpenCode path is solid.
- Treat the existing local harness as a prototype to replace with real daemon transport and a real CLI app.

## 3. Concrete repository target for M2

At the end of M2, the repo should have something close to:

```text
apps/
  daemon/
    src/
      local/
        api.rs          # existing local request/response contract
        ipc.rs          # daemon-side local transport / framing
      provider/
        registry.rs     # includes a real opencode adapter
        opencode.rs     # launch details for the opencode CLI
  cli/
    src/
      main.rs           # local CLI entrypoint
      client.rs         # request/response wrapper over daemon transport
      ui.rs             # minimal terminal input/output loop
```

Exact filenames may evolve, but M2 should converge on a shape close to that.

## 4. Workstreams

## 4.1 Daemon Local Transport

- [ ] Add a real local transport for the daemon instead of relying only on in-process calls.
- [ ] Prefer Unix domain sockets on Unix-like systems for the first cut.
- [ ] Use a simple framed request/response protocol suitable for local CLI use.
- [ ] Expose the existing local daemon request/response types through that transport.
- [ ] Keep the transport single-user and local-first for M2.
- [ ] Add tests for:
  - daemon boot and socket availability
  - request/response round-trip
  - malformed request handling
  - client disconnect handling

## 4.2 Local CLI App

- [ ] Add a real CLI client app under `apps/` that connects to the daemon transport.
- [ ] Support create-or-attach session flow for one local user.
- [ ] Add a prompt input field or line-input loop.
- [ ] Submit prompts through the daemon rather than writing directly to provider stdin.
- [ ] Continuously poll or stream terminal output and render it live.
- [ ] Handle terminal resize and clean shutdown.
- [ ] Keep the first UI intentionally minimal:
  - one active session
  - one active provider run
  - one input field
  - one output pane/stream

## 4.3 OpenCode Provider Adapter

- [ ] Add a real `opencode` provider adapter to the daemon provider registry.
- [ ] Resolve the `opencode` executable from the local machine environment.
- [ ] Launch OpenCode in a PTY through the existing provider-run lifecycle.
- [ ] Keep the first iteration PTY-only:
  - no login flow
  - no command discovery
  - no provider control operations
  - no provider-specific extension projection
- [ ] Add clear errors for:
  - executable not found
  - launch failure
  - immediate process exit
- [ ] Add adapter tests using a fixture or controllable subprocess where needed.

## 4.4 Prompt and Output Flow Hardening

- [ ] Reuse the existing prompt submission path instead of bypassing session state.
- [ ] Ensure the local CLI path exercises:
  - `session.create`
  - `session.attach`
  - `provider_run.launch`
  - `prompt.submit`
  - `terminal.output.poll`
- [ ] Ensure prompt submission failure does not leave session state inconsistent.
- [ ] Ensure streamed output remains live and incremental rather than only showing a final snapshot.
- [ ] Verify resize events reach the PTY while output is active.

## 4.5 Replace Harness-Only Assumptions

- [ ] Keep the current local harness for smoke coverage, but stop treating it as the primary user path.
- [ ] Add a real end-to-end smoke flow using the daemon transport and CLI app.
- [ ] Update docs and scripts so the recommended local test path references the actual CLI when available.

## 4.6 Deferred M2-old capability work

These items are no longer part of the immediate M2 critical path and should remain deferred until M3:

- shell command capability expansion
- directory tree and file capability expansion
- screenshot and transfer UX completion
- git integration expansion
- schedule execution baseline
- slash-command UX beyond what is needed to keep the local CLI usable

Existing implementation work in these areas should be preserved, but it should not drive milestone completion ahead of the end-to-end OpenCode path.

## 5. Testing and Verification

- [ ] Add unit tests for the OpenCode adapter and daemon local transport.
- [ ] Add integration tests for the real local daemon request/response path.
- [ ] Add an end-to-end smoke test covering:
  - daemon startup
  - CLI connection
  - session creation or attachment
  - provider launch
  - prompt submission
  - live output capture
- [ ] Keep existing daemon tests passing while adding the new path.
- [ ] Keep formatting and linting clean across Rust and JS workspace checks.

## 6. Documentation updates required in the same PR set

- [ ] Update `README.md` when the CLI app exists and can be run locally.
- [ ] Update `docs/PROTOCOL.md` if the daemon transport framing or local command surface becomes more concrete.
- [ ] Update `docs/ARCHITECTURE.md` once the real daemon transport and CLI app land.
- [ ] Update `docs/ops/TASKS.md` and `docs/ops/PROGRESS_LOG.md` as M2 work advances.

## 7. Suggested execution order

1. daemon local transport
2. minimal CLI app shell
3. OpenCode adapter
4. end-to-end prompt/output flow through real transport
5. smoke and integration tests
6. doc cleanup around the new default local path

## 8. Verification commands for claiming meaningful M2 progress

Run and pass locally before claiming meaningful M2 progress:

```bash
pnpm lint
pnpm build
pnpm test
cargo test --manifest-path apps/daemon/Cargo.toml
```

Recommended additional Rust checks:

```bash
cargo fmt --manifest-path apps/daemon/Cargo.toml --check
cargo clippy --manifest-path apps/daemon/Cargo.toml --all-targets --all-features -- -D warnings
```
