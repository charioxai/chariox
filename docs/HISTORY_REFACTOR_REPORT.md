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

Captured after replacing full transcript hydration with outline plus lazy blob
content loading.

Command:

```sh
cargo test --manifest-path apps/kernel/Cargo.toml performance_drill_session_history_outline -- --nocapture
```

Fixture:

- agents: 6
- turns per agent: 24
- tool blobs per turn: 4
- tool blob payload: 4096 bytes
- seeded events: 864

Outline behavior:

- request count: 1
- outline attach/history time: 9.93 ms
- outline turns returned: 24
- lazy blob placeholders returned: 96
- outline response bytes: 42488
- blob expand time: 0.43 ms
- blob response bytes: 4652
- blob entries returned: 1

Comparison:

- baseline_attach_ms: 151.04
- outline_attach_ms: 9.93
- attach improvement: 93.42%
- baseline_bytes: 1331538
- outline_bytes: 42488
- response byte improvement before blob expansion: 96.81%
- baseline_request_count: 6
- outline_request_count: 1
- request count improvement: 83.33%
- blob_expand_ms: 0.43

## Implementation

- Replaced `GetSessionHistory`/`SessionHistory` with `GetSessionHistoryOutline`,
  `SessionHistoryOutline`, `GetSessionHistoryBlobContent`, and
  `SessionHistoryBlobContent`.
- Bumped `LOCAL_DAEMON_PROTOCOL_VERSION` to 86 and updated protocol shape and
  TypeScript conformance checks.
- Kernel outline loading now reads latest prompt turns per requested active
  agent from operational SQLite and returns full prompts, final summaries, and
  blob metadata.
- TUI attach/resume now primes panes from outline data and expands placeholders
  through blob content requests.
- Browser terminal bootstrap now uses one outline request across active agents
  and expands lazy blobs on demand.
- Deleted the deprecated full transcript hydration path, cursor pagination,
  stale TUI autoload controller, and web older-history pagination controls.
- Kept operational/durable storage, prompt input history, recall/search, and
  append/read paths used outside resume hydration.

## Validation

OSS:

```sh
cargo test --manifest-path apps/kernel/Cargo.toml local_daemon_protocol_session_history_outline_shape_is_versioned -- --nocapture
cargo test --manifest-path apps/kernel/Cargo.toml local_daemon_protocol_version_matches_typescript_kernel_client -- --nocapture
cargo test --manifest-path apps/kernel/Cargo.toml session_history_entries_read_operational_history -- --nocapture
cargo test --manifest-path apps/kernel/Cargo.toml performance_drill_session_history_outline -- --nocapture
pnpm --filter @chariox/cli run lint
pnpm --filter @chariox/cli run build
node --test apps/cli/dist/attached-session-prime-controller.test.js apps/cli/dist/deferred-bootstrap-controller.test.js apps/cli/dist/agent-pane-refresh-controller.test.js apps/cli/dist/session-bootstrap.test.js apps/cli/dist/ipc-requests.test.js packages/kernel-client/dist/shell-executor.test.js
pnpm --filter @chariox/cli run history-outline:tui-drill
pnpm --filter @chariox/cli run history-outline:tui-e2e-drill
```

TUI history drill output:

```json
{
  "drill": "history-outline-tui",
  "outlineEntries": 4,
  "placeholderId": 2,
  "expandedEntries": 4
}
```

Live TUI e2e drill output:

```json
{
  "drill": "history-outline-tui-e2e",
  "sessionId": "18b4eb7a459a8628",
  "agentId": "agent-1",
  "outlinePromptCount": 1,
  "outlineBlobCount": 1,
  "initialEntryCount": 8,
  "initialPlaceholder": {
    "id": 3,
    "historyBlobId": "history:2:2",
    "historyBlobLoaded": false,
    "blobTitle": "tool",
    "blobSummary": "TUI live lazy blob detail from provider tool"
  },
  "initialHadPrompt": true,
  "initialHadSummary": true,
  "initialHadToolDetail": false,
  "expandedEntryCount": 8,
  "expandedHadToolDetail": true,
  "expandedPlaceholderRemoved": true
}
```

Observation: the first live TUI drill showed the kernel outline request
completed, but deferred bootstrap history was not applied into the live
transcript. The fix made deferred bootstrap application idempotent and scheduled
it immediately as well as on mount. The passing drill then observed prompt and
summary entries, an unloaded lazy blob placeholder, and independently fetched
tool content after expanding that blob through the TUI automation toggle path.

Current workspace note: a later `pnpm --filter @chariox/cli run lint` in the
dirty shared worktree is blocked by unrelated `slice-command-handlers.ts`
type errors from concurrent slice/auth edits. The history automation and
deferred-bootstrap tests still pass, and the live TUI e2e drill passes.

Cloud/web:

```sh
pnpm --filter @chariox-cloud/web run build
pnpm --filter @chariox-cloud/web run lint
node --test apps/web/dist/terminal-history.test.js apps/web/dist/terminal/history-hydration-controller.test.js apps/web/dist/terminal/terminal-runtime-hydration-coordinator.test.js apps/web/dist/terminal/freeform-pane-chrome-dom-controller.test.js apps/web/dist/freeform-footer-controls.test.js apps/web/dist/terminal/terminal-session-orchestration-composition.test.js
node --test apps/web/dist/terminal/history-hydration-controller.test.js apps/web/dist/terminal/terminal-output-composition.test.js apps/web/dist/terminal/freeform-pane-chrome-dom-controller.test.js apps/web/dist/terminal/freeform-transcript-renderer.test.js
pnpm --filter @chariox-cloud/web run history-outline:web-e2e-drill
pnpm --filter @chariox-cloud/web run drill:browser
pnpm run smoke:browser-relay-kernel
```

Web terminal DOM e2e drill output:

```json
{
  "drill": "history-outline-web-e2e",
  "outlineRequests": 1,
  "blobRequestsBeforeClick": 0,
  "blobRequestsAfterClick": 1,
  "initialHadPrompt": true,
  "initialHadSummary": true,
  "initialHadBlobSummary": true,
  "initialHadToolDetail": false,
  "expandedHadToolDetail": true,
  "expandedEntryCount": 3
}
```

Observation: the web drill rendered an attached terminal pane from outline data,
verified prompt, summary, and blob summary were present before expansion, and
verified no blob content request was sent until the blob header was clicked.
The first pass also exposed that fetched blob content replaced the placeholder
as a collapsed tool entry; the fix now transfers expansion to the inserted
entry so opening a lazy blob leaves the fetched content open.

Browser drill result: passed through the marketing shell, waiting-room shell, and
terminal `/test` route controls.

Browser relay/kernel smoke result: passed with a local relay, kernel, browser
transport, dev-stub provider turn, terminal output delivery, and completion.
