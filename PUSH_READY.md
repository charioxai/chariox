# Push readiness — NOT READY

Do not push or merge this lane yet. Code is committed locally with `[skip ci]`;
real acceptance is incomplete. See `LANE_STATUS.md` for exact live evidence and
blockers. New receiving-machine login and resumed queued work require the human
login response. OpenCode needs a linked account; Claude is BLOCKED(owner).

Compatibility: local protocol 455, relay peer 74, unchanged relay transport.
Managed publication producers need explicit original machine/kernel/account
provenance. Direct slice import requires an upgraded receiver with the pinned
home key. Retired raw/startup provider imports fail with an actionable command.

Focused local checks:

- Kernel/relay and CLI builds: passed, including the exact final kernel source.
- Rust protocol snapshots/hashes: 184 passed on the final test build.
- Runtime recovery regressions: 4 passed (supplementary fake OAuth fixtures).
- Auth failure classifier: 3 passed.
- Managed-context package: 15 passed.
- Publication account materialization: 6 passed.
- Peer transport: 10 passed.
- Account materialization: 10 passed.
- CLI provider-account handlers/details: 25 passed.
- Slice identity/protocol contract: 3 passed.
- Kernel-client request contracts: 85 passed.
- Managed Docker broker: 32 passed, 4 environment-dependent skips.
- Copy/default-remap fixtures: 3 passed; lease policy/receipt fixtures: 8 passed.
- Slice owner-pin rejection and receiving account paths: 1 passed each.

No GitHub CI, push, PR, merge or deployment was performed.
