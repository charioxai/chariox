# M2 Implementation Checklist

## Status

Execution checklist for **M2 - Capability Surface**.

M2 is now in progress.

This checklist translates the M2 roadmap milestone into concrete implementation steps for the repository state after M1.

## 1. Target M2 outcomes

From `docs/ROADMAP.md`, M2 outcomes are:

- shell command capability
- directory tree + file view/edit capabilities
- screenshot capture capability
- git/worktree inspection capability
- file transfer + attach-transferred workflow
- schedule metadata + daemon execution baseline
- workflow-aware capability design so structured node handoffs, aggregation artifacts, and isolated branch outputs can reuse the same daemon-owned capability surfaces later

Exit criteria:

- core capabilities are callable through a local-first daemon API surface
- capability failures are isolated from the terminal lane
- capability results are structured and inspectable by current clients and future workflow nodes

## 2. M2 implementation principles

- Keep capability execution daemon-owned and separate from provider PTY traffic.
- Prefer structured request/response results over ad hoc transcript scraping.
- Keep capability APIs local-first and additive.
- Preserve compatibility with future workflow-mode node execution and explicit worktree assignment.
- Treat shell/file/git/screenshot/transfer features as reusable runtime services, not UI-only affordances.

## 3. Suggested module responsibilities

The exact filenames may evolve, but M2 should converge on something close to:

```text
apps/daemon/src/
  capability/
    mod.rs
    shell.rs          # shell command capability runtime
    tree.rs           # directory snapshot capability
    file.rs           # file view/edit capability runtime
    git.rs            # git/worktree inspection capability
    screenshot.rs     # screenshot capture capability
    transfer.rs       # file transfer + attachment handoff runtime
    schedule.rs       # schedule execution baseline
```

## 4. Capability workstreams

## 4.1 Shell command capability

- [x] Add a daemon-owned shell command service with structured request/response types.
- [x] Capture stdout, stderr, exit status, and execution metadata.
- [x] Support explicit working directory selection compatible with current session/worktree state.
- [x] Isolate shell-command failure from provider PTY lifecycle.
- [x] Enforce timeout bounds for shell commands.
- [x] Scope shell execution to the session worktree and validate the requesting attachment.
- [x] Add tests for:
  - successful command execution
  - non-zero exit status
  - working-directory scoping
  - timeout handling

## 4.2 Directory tree capability

- [ ] Add a structured directory tree/snapshot capability.
- [ ] Keep output deterministic and suitable for terminal clients and future workflow handoffs.
- [ ] Add tests for scoped tree generation and ignored-path behavior when introduced.

## 4.3 File view/edit capabilities

- [ ] Add read-only file view capability with structured text output.
- [ ] Add daemon-owned file edit capability with change reporting.
- [ ] Add tests for large-file chunking or bounded output once behavior is concrete.

## 4.4 Git/worktree inspection capability

- [ ] Add git/worktree status inspection capability.
- [ ] Surface branch, dirty state, and relevant worktree metadata through structured responses.
- [ ] Keep the runtime compatible with future isolated workflow branches and worktree assignments.

## 4.5 Screenshot capability

- [ ] Add a screenshot capture capability contract and local runtime baseline.
- [ ] Ensure produced artifacts are session-associated and discoverable.

## 4.6 File transfer and attach-transferred workflow

- [ ] Add daemon-owned file transfer metadata and storage baseline.
- [ ] Add an attach-transferred workflow that can reuse future provider control-lane operations.
- [ ] Keep degradation behavior explicit when provider-side attach support is absent.

## 4.7 Schedule baseline

- [ ] Add daemon-owned schedule metadata handling and execution baseline.
- [ ] Ensure scheduled prompt execution reuses the same queue/scheduler semantics as interactive prompts.
- [ ] Add tests for schedule registration and prompt-queue interaction once behavior is concrete.

## 4.8 Local API and contract alignment

- [x] Extend the local daemon API for implemented capabilities.
- [x] Keep protocol docs aligned with every new capability request/response shape.
- [x] Ensure capability APIs remain future-compatible with workflow-node execution.

## 4.9 Testing and verification

- [x] Add Rust unit tests for each implemented capability service.
- [x] Add integration tests covering capability execution through the daemon API.
- [x] Ensure JS workspace verification still passes after M2 changes.
- [x] Keep daemon formatting, tests, and clippy clean.

## 4.10 Documentation updates required in the same PR set

- [x] Update `docs/PROTOCOL.md` for each new capability contract.
- [x] Update `docs/ARCHITECTURE.md` if capability ownership/responsibilities become more concrete.
- [x] Update `docs/CONTRIBUTING.md` with any new verification commands.
- [x] Update `agents/AGENTS.md`, `README.md`, `docs/ops/TASKS.md`, and `docs/ops/PROGRESS_LOG.md` as M2 work lands.

## 5. Suggested execution order

1. shell command capability
2. local API exposure for shell command
3. directory tree capability
4. file view/edit capabilities
5. git/worktree inspection capability
6. screenshot capability
7. file transfer baseline
8. schedule baseline
9. docs/protocol alignment

## 6. Verification commands for claiming M2 progress

Run and pass locally before claiming meaningful M2 progress:

```bash
pnpm lint
pnpm build
pnpm test
pnpm smoke:daemon
cargo test --manifest-path apps/daemon/Cargo.toml
```

Recommended additional daemon checks:

```bash
cargo fmt --manifest-path apps/daemon/Cargo.toml --check
cargo clippy --manifest-path apps/daemon/Cargo.toml --all-targets --all-features -- -D warnings
```
