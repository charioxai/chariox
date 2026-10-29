# AGENTS.md

Chariox's open-source runtime: kernel (`apps/kernel`, Rust), relay (`apps/relay`), terminal client (`apps/cli`), iOS client (`apps/ios`) and shared packages (`packages/`). Cloud is the separate `chariox-cloud` repo.

## Commands

```sh
pnpm install --frozen-lockfile
pnpm build:kernel-client          # once per fresh worktree
pnpm lint && pnpm build && pnpm test
cargo fmt --all --check
node scripts/clippy-changed-lines-gate.mjs origin/main
RUST_MIN_STACK=16777216 RUST_TEST_THREADS=1 cargo test --workspace
pnpm protocol:conformance
pnpm lint:process-signals         # when spawning or signalling processes
```

One kernel test: `RUST_MIN_STACK=16777216 RUST_TEST_THREADS=1 cargo test -p chariox-kernel --lib <filter>`. Run locally: `pnpm start:kernel`, then `pnpm start:cli`. A kernel build needs 8–12 GB of RAM; never run two at once on one machine.

## Protocol changes

When a serialized shape a client depends on changes:

1. Increase `LOCAL_DAEMON_PROTOCOL_VERSION` (`apps/kernel/src/local/api/types.rs`), and `RELAY_PEER_PROTOCOL_VERSION` (`apps/kernel/src/transport/relay_peer.rs`) if peers are affected.
2. Update the protocol snapshot and hash tests.
3. Raise a client's minimum version only if it needs the new behaviour.
4. Add or update a focused drill for the change.

Development stage: there is no backward compatibility. Managed upgrade and rollback admit the current and the previous release only; older predecessors are dropped when that simplifies the code. Do not report the missing upgrade or rollback path from an older release as a finding.

## Runtime rules

1. The kernel is the authority for sessions, agents, provider runs, workspaces, history and state. Put behaviour in the kernel and the shared protocol, not in one client.
2. Providers run only through their official tools: their CLIs, local servers and documented hooks. No provider SDKs or third-party agent services. Provider tools own credential formats and refresh; Chariox keeps credentials only in the kernel Vault or the provider profile and moves them only through account transfer (chariox `docs/PROVIDER_ACCOUNTS.md`).
3. The relay only carries encrypted traffic. It never reads prompts, outputs or data, and never decides session state.
4. Cloud signs users in, issues relay tokens and picks relay targets with fresh heartbeats. It never carries runtime traffic and never changes kernel behaviour.
5. Provider permission prompts resolve through one kernel-owned interaction shown to every client in the session.
6. Hosted connections use `wss://` behind Caddy. Hosted settings that break these rules must fail loudly.

## Writing clean code

- Solve the actual problem with the simplest design. No options, layers or abstractions nobody asked for.
- Delete dead code, old paths and temporary scaffolding in the same change.
- One job per module, function and type. Coordinating files only wire parts together; split a file before it mixes responsibilities.
- Model what can happen with types so wrong states are impossible. Check input once at the boundary, then trust it.
- Fail clearly, never silently: no hidden fallbacks, no half-finished states. Every operation ends with a result and a reason, and errors say what to do next.
- Name things for what they are, in the project's own words.
- Comments explain why, not what. No commented-out code.
- Match the style and structure of the code around you.
- Prove each change with a test that fails before it and passes after it, through real entry points.

## Done means tested for real

A change is done when it works on the real path a user takes: real kernel, relay, client and provider. Keep screenshots, recordings and logs outside the repository.

## State, secrets and safety

- Keep local state in `~/.chariox/dev/<task>/` with `CHARIOX_HOME` set to it, and checkouts outside `~/.chariox`. Never put state, logs or test data in a repository.
- Never print, log or commit credentials, tokens or keys, and move them only through the product's account transfer.
- Stop processes by exact PID, never by name.
- Don't touch the reviewer's services and state (`~/.chariox-reviewer`).
- Clean up what you started.

## Secrets and agents

UX comes first. Agents are benign and act for the user. Chariox gives them tools to finish secret workflows, such as Vault fill by handle and the secret-capture helper, and does not gatekeep them.

- Guard one thing: the product must not hand a Vault value to the model on its own. Keep the Vault-filled field masked in what the model sees (frames, accessibility and DOM text), and never echo Vault values in tool results, history, logs or traces.
- Don't guard secrets that are visible on screens, terminals or pages, anything an agent does with a value itself (copy, paste, move, re-display), or what a person types.
- A guard that blocks or slows an agent or the user's view without protecting what the model sees is a bug.

## Commits and pull requests

- A commit message is a short summary of the change: one subject line, and a few short lines only if needed. No Co-Authored-By lines, no agent or model names, no logs.
- Work on a branch. Changes reach `main` through a squash merge with a short summary.
- Run CI only when the work is ready to merge; until then use focused checks and `[skip ci]`.

## Read more

Architecture: `docs/ARCHITECTURE.md`. Protocol, including native provider terminals (section 3.3.2): `docs/PROTOCOL.md`. Provider accounts: `docs/PROVIDER_ACCOUNTS.md`.
