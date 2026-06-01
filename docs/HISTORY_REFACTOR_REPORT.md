# History Refactor Report

## Baseline

Captured before replacing the current `GetSessionHistory` hydration path.

Command:

```sh
cargo test --manifest-path apps/kernel/Cargo.toml performance_drill_session_history_current_baseline -- --nocapture
```

Fixture:

- agents: 6
- turns per agent: 24
- tool blobs per turn: 4
- tool blob payload: 4096 bytes
- seeded events: 864

Current behavior:

- request count: 6
- total attach/history time: 151.04 ms
- decoded entries: 5184
- returned entries: 420
- response bytes: 1331538

Per-agent latencies:

- agent-0: 29.60 ms, 221923 bytes
- agent-1: 22.76 ms, 221923 bytes
- agent-2: 26.58 ms, 221923 bytes
- agent-3: 25.63 ms, 221923 bytes
- agent-4: 24.65 ms, 221923 bytes
- agent-5: 21.08 ms, 221923 bytes

## Hierarchical History

Pending implementation.
