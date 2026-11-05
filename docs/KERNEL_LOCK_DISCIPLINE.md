# Kernel lock discipline

MP-08 / MP-09 / MP-10 / MP-11. Round 1, incident 2026-11-05.

## Observed failure

At base 96f1c682ac the dependency was provider service STORE -> process-wide
`env_lock` -> `CODEX_ACCOUNT_ENDPOINTS` -> unchecked process-tree termination ->
unbounded `Child::wait`. The helper stayed alive. A provider launch waiting for
the environment lock retained STORE, blocking MCP authentication and completed
run publication. All executor runtime workers eventually stopped progressing.
The independent ROWS cache retained its mutex across unbounded `ps.output()`;
eleven samplers waited behind one process-table read.

## Design and required order

Shared locks protect in-memory state transitions only. Copy or reserve input,
release the guard, perform launch/connect/filesystem/network/process work, then
reacquire to publish the result. A request must not retain a store guard while
waiting for an owner response. Admission tokens may span a launch, but the
admission registry's mutex may not.

A dedicated account-endpoint supervisor owns child handles and serializes
ensure/invalidate/shutdown commands. Request threads exchange bounded replies;
no shared endpoint map or process-wide environment lock participates. Children
that ignore TERM get KILL after a short grace, followed by a bounded reap.
Unreaped children remain owned by the supervisor for later polling and produce
an explicit diagnostic. Both Codex and OpenCode use this ownership path.

Production launch planning reads stable startup configuration and constructs
per-child environments with `Command::envs` / `env_remove`. The process-wide
environment guard is available only in tests. Production never serializes
provider launches on a global environment mutex.

One sampler thread owns bounded `ps` execution and publishes immutable snapshots.
Readers clone an Arc and never start or wait for `ps`; a failed refresh preserves
the last good snapshot and logs the failure once until recovery.

Provider launch preparation and adapter connect execute outside STORE. The store
reserves the run identity and snapshots the registry, then publishes the completed
run under a short lock. MCP auth lookup has no dependency on a provider child.

Touched shared locks use checked guards: acquisition and holding are registered
with call sites and thread names; an independent watchdog reports threshold
violations even if the holder never releases. Test guards enforce a larger bound.
The watchdog also records progress of runtime and relay work. Bounded subprocess
operations are the only admitted child wait/output path, enforced by source lint.

## Inventory at the incident base

The attached [site inventory](KERNEL_LOCK_SITES.tsv) enumerates 3,382 declaration,
acquisition, environment-lock, and read/write candidates at incident base
`96f1c682ac`, with the enclosing function, approximate guard scope and blocking
operations in that scope. The [global inventory](KERNEL_LOCK_GLOBALS.tsv) lists
110 global ownership/lock declarations. Line numbers refer to that base, before
this change. Test-named paths and explicit test-only items are excluded from the
site table; the global table deliberately retains a few test statics and immutable
OnceLocks as candidates. A read/write candidate is not necessarily an RwLock.
These tables are a complete lexical inventory, not a claim of a full call-graph
or semantic proof. The verified lock classes and exceptions below distinguish
confirmed hazards from the remaining audit work.

### Lock classes and users

| # | Class | Declared at | Guards | Main acquirers | Held across blocking? | Verdict |
|---|---|---|---|---|---|---|
| 1 | `ProviderProcessServiceStore.inner` (`Arc<Mutex<ProviderProcessService>>`) | `provider/service/store.rs:45-53` (approx), accessors `:115-120` | run table, runs, runtime slots, next run id, registry | ~80 sites in `store.rs`, app layer, `mcp_server/catalog` snapshot | yes (§2) | **BLOCKING, critical** |
| 2 | `ManagedKernelQuiescenceGate.admission_lock` | `managed_kernel_quiescence_state.rs:118` | admission serialization with reservation | 3 (see §4) | yes, whole launch | **BLOCKING, critical** |
| 3 | `ManagedKernelQuiescenceGate.inner` | `:119` | quiescence state, fence, restore error | `ensure_admission_open` `:214-236`, reservations | no | OK |
| 4 | `ManagedKernelQuiescenceGate.mutation_lock` (`Arc<Mutex<()>>`) | `:116` | durable mutation serialization | reservation paths | to-verify (durable I/O under it?) | to-verify |
| 5 | `AgentMoveStore.records` (`MoveRecords`) | `agent_move/store.rs:114` | move homes/sources/targets, fences | ~22 sites in `agent_move/store.rs` | yes via `with_admission` | **BLOCKING, critical** |
| 6 | `env_lock` (owner-thread reentrant, Condvar) | `env_lock.rs:27-78` (test bypass `:32-37`) | nothing in production (§8) | 9 production sites (§8) | yes: planning + endpoint ensure | **BLOCKING, critical** (remove) |
| 7 | `CODEX_ACCOUNT_ENDPOINTS` | `provider/codex/catalog_endpoint.rs:17` | per-account app-server helpers | `ensure` (`:37`), `invalidate` (`:134`), `shutdown` (`:~150-160`) | yes: `wait` (`:144`), spawn/poll under env_lock | **BLOCKING, critical** (your fix) |
| 8 | `OPENCODE_ACCOUNT_ENDPOINTS` | `provider/opencode/catalog_endpoint.rs:17` | same for OpenCode | `ensure`/`invalidate` (`:21` env_lock) | to-verify (same pattern) | to-verify |
| 9 | Codex/OpenCode managed catalog port (`CODEX_MANAGED_CATALOG_PORT`, `OPENCODE_…`) | `provider/codex/ports.rs:9`, `provider/opencode/ports.rs:9` | port reservation | catalog | to-verify | to-verify |
| 10 | Sampler `ROWS` (`OnceLock<Mutex<CachedRows>>`) | `runtime/process_resources/sampling.rs:772-782` | cached `ps` table + timestamp | `process_rows` `:784-791`, `ensure_available` `:827` | yes: `ps` `.output()` `:835-843` | **BLOCKING, critical** |
| 11 | Sampler `ERRNO` | `sampling.rs:849` (`#[cfg(test)]`) | test fault injection | tests | n/a | test-only |
| 12 | process-resource `REGISTRY` | `runtime/process_resources.rs:745` | watch-domain registry (weak refs) | `hand_over_released` `handover.rs:50-64`, `ask` `control.rs:` (first) | cgroup file reads only | OK (bounded) |
| 13 | watch-domain `Observation` (per domain `Arc<Mutex<…>>`) | `runtime/process_resources.rs:~283-300` (`state`) | owned tree, rates, control sender | watch thread (not held across `sample()`), `ask`, `hand_over_released` | no (sampling is outside it) | OK |
| 14 | `control.rs` `LIVE`, `OWNER_ENDS` | `runtime/process_resources/control.rs:118-119` | live domain set, owner-end records | `domain_started/dropped`, `owner_ended`, `end_owner` (`:~170-180`, order registry→LIVE→OWNER_ENDS) | no | OK (order documented) |
| 15 | pool budget slot | `runtime/process_resources/pool.rs:105-112` | cgroup budget `Weak` | `budget()` | brief fs (cgroup create) | OK, low |
| 16 | process-resource `ADOPTED` | `runtime/process_resources/admission.rs:188` | adopted command table | admission | to-verify | to-verify |
| 17 | MCP stdio `SUPERVISOR` | `provider/mcp_proxy/stdio.rs:11` | process table, closed run/session sets | `process()` (spawns the child under the lock), `release_*`, `stop_all` | spawn (fork/exec, bounded) | MEDIUM (short term) |
| 18 | MCP stdio per-process `Mutex<StdioMcpProcess>` | `stdio.rs:23` (`Arc<Mutex<…>>`) | one child's stdin/stdout state | `mcp_proxy.rs:150-158` | yes: `write_frame` `:385-393` no deadline | **BLOCKING, medium-high** |
| 19 | `LAUNCH_TARGET_CACHE` | `runtime/waiting_room_public_projection.rs:33`, held `:625-643` | launch target per cwd | `cached_launch_target` | yes: git subprocesses `:650-675` | **BLOCKING, high** |
| 20 | `GIT_BRANCH_CACHE`, `WORKTREE_LABEL_CACHE`, `WORKSPACE_LABEL_CACHE` | `waiting_room_public_projection.rs:34-38` | label caches | `cached_git_branch` `:1194-1203`, `:1172`, `:1218` | to-verify: is the guard alive across `detect_git_branch` (`:1203`)? | to-verify |
| 21 | `workflow_provider_launch_lock` | `runtime/state/workflow.rs:112-114` | admission + workflow-tool activation as one section | `workflow_ensure_provider_run` | yes, whole launch (documented) | MEDIUM, intentional; inherits §2 |
| 22 | `Link.writer` (display link) | `runtime/browser_controller_process/display_frames.rs` (field), `Link::send` `:338-341` | per-link writer | display frames | yes: `write_all` `:341`, no deadline | **BLOCKING, medium** |
| 23 | controller `process.stdin` lock | `runtime/browser_controller_process.rs` ~`:624-628` | per-process stdin | request path | yes: `write_line` | **BLOCKING, medium** |
| 24 | browser controller supervisor lock | `runtime/browser_controller_process/reconciliation.rs:205-212` | supervisor ownership | `browser.wait` path | released before the wait (`prepare_unlocked_request`) | OK (good pattern) |
| 25 | kernel browser backend lock (`browser`) | `runtime/kernel_browser_host.rs:545-547` | backend operation serialization | browser ops | held through the backend operation | to-verify deadline |
| 26 | shutdown lock (`ownership`) | `runtime/app_lifecycle/ownership.rs:194-197`, `join` `:209` | shutdown serialization | shutdown | yes: `entry.join()` unbounded | **BLOCKING, medium** (shutdown only) |
| 27 | durable writer worker lock | `durable_state/writer_fence.rs:24-31` | writer `JoinHandle` | `fence_writer` | yes: `worker.join()` | **BLOCKING, low-medium** |
| 28 | `hot_reload` statics (`RECEIVED_*`, `ADOPTING`, `LISTENERS`, `PREPARED`) | `hot_reload.rs:127-133`, `:246-247`, `:623` | handover state | claim/adopt/receive | no (short) | OK |
| 29 | secret / vault caches and `VAULT_FILE_LOCK` (RwLock) | `secret/vault.rs:1713`, `:1722`, `:1769`, `secret.rs:1372` | vault plaintext cache, file lock | vault | to-verify: file I/O under `VAULT_FILE_LOCK` | to-verify |
| 30 | `INVENTORY_CACHE` (SCM credential) | `managed_context/scm.rs:29` | cache entry | `:214-220` | no (probe after guard drops) | OK |
| 31 | `CONNECTED_ADDRESSES` | `outbound_dial.rs:40`, `:64-82` | connected-address memory | dial path | no | OK |
| 32 | provider runtime slots map and per-run slots | `provider/run_actor/runtime_slots.rs` (71 sites) | run runtime state; take-out pattern | run actor | no (state taken out before IO/sleep; `command_execution.rs:43-502`) | OK (partly verified); order map→slot |
| 33 | relay reconnect gate (`RelayReconnectGate.state`, std Mutex) | `transport/relay_client/incoming_envelopes.rs:18-80` | in-flight count, pending reconnect | request begin/finish | no (`try_send` with spawn fallback, `:571-582`) | OK |
| 34 | relay client state (`RwLock<RelayClientState>`, tokio) | `transport/relay_client/connection_state.rs` | connection state | connect/disconnect/peer | no (guards block-scoped, no await inside) | OK |
| 35 | tokio async locks (~135 `.lock().await`) | e.g. `runtime_transport.rs:1100-1946`, `relay_client/subscriptions.rs:54-68` | async state | async tasks | async by design; not audited per site | to-verify (sample) |

### Baseline ordering and blocking calls

Edges verified by reading code (A → B means B is taken while A is held):

- Admission → store: `gate.admission_lock` → `records` → `store.inner` (`store.rs:263-279`; `agent_move/store.rs:481-488`; `managed_kernel_quiescence_state.rs:206-212`).
- Store → env_lock: `store.inner` → `env_lock` (`run_lifecycle.rs:79` → `claude.rs:79`, `codex.rs:69`, `opencode.rs:72`).
- env_lock → endpoints: `env_lock` → `CODEX_ACCOUNT_ENDPOINTS` (`codex/catalog_endpoint.rs:37`, your file); same pattern for OpenCode (to-verify).
- Store → ROWS: `store.inner` → `ROWS` (`with_current_live_launch` → `PreparedDomain::start` → `ensure_available` → `process_rows`; `process.rs:110`, `process_resources.rs:245`).
- Process resources: `REGISTRY` → domain `Observation` (`handover.rs:51-56`); `REGISTRY` → `LIVE` → `OWNER_ENDS` (`control.rs` `end_owner`); `REGISTRY` → `OWNER_ENDS` (`owner_ended`, documented "under the registry lock").
- Workflow → store: `workflow_provider_launch_lock` → store chain (`workflow.rs:112-114` → `provider_store.*`).
- Run actor: runs map → per-run slot (`run_actor/runtime_slots.rs` ~`:545-600`).
- Stdio MCP: supervisor → (released) → per-process mutex (sequential, not nested; `mcp_proxy.rs:150-158`).
- Shutdown: `ownership.shutdown` → entry joins (writes under lock; `ownership.rs:209`).

**Inverse orders found:** none verified across functions in the paths above. Two suspicious cases to check with a call graph:
- `records` → `store.inner` (commit) versus any path that takes `store.inner` and then calls into agent-move code. I found none in `provider/service` (`grep` of `agent_move` for provider store calls is empty); other crates' call graph to be confirmed.
- `REGISTRY` → domain `state` (hand-over, ask) versus domain-state → `REGISTRY`. None seen; the watch thread does not take `REGISTRY`.

## 7. Guards held across blocking work: verified list

### 7.1 Verified true positives (read in code)

| ID | Where | Held | Blocking call | Bound? | Fix |
|---|---|---|---|---|---|
| B1 | `provider/codex/catalog_endpoint.rs:134-146` | `CODEX_ACCOUNT_ENDPOINTS` | `Child::wait` (`:144`); terminate result ignored (`:143`) | no | your owner design |
| B2 | `runtime/process_resources/sampling.rs:784-791`, `:835-843` | `ROWS` | `ps` `Command::output` | no | sampler owner: snapshot, deadline, kill |
| B3 | `provider/service/store.rs:158-170` + `app/provider_processes.rs:45-52` + `provider/claude_runtime/process.rs:110` | `store.inner` (read) | spawn; `ensure_available` → `ps` (B2) | `ps` no | spawn outside; sampler snapshot |
| B4 | `provider/service/run_lifecycle.rs:79` (`connect`) + `claude.rs:79` / `codex.rs:69` / `opencode.rs:72` | `store.inner` + env_lock | `plan_*` (env_lock wait); `preflight` (fs) | env_lock unbounded (waits on B1) | §3 |
| B5 | `provider/executable_resolution.rs:37-38` | store + env_lock (via B4) | `thread::sleep` up to 5 s | bounded 5 s | move out of request path |
| B6 | `managed_isolation.rs:1144` → `:2248-2253` | store (via B4) | `git rev-parse` `.output()` | no | bounded helper, or plan-cache per workspace |
| B7 | `workspace_write_fence.rs:207-262`, `:148-170` (macOS) | store (via B4) | `read_dir` up to 20,000 dirs; file writes | cap 20,000 | cache; move out (plan) |
| B8 | `runtime/kernel_browser_host` / `runtime/state/workflow.rs:112-114` | `workflow_provider_launch_lock` | whole provider launch | inherits B3/B4 | keep (documented), or per-agent lock |
| B9 | `runtime/app_lifecycle/ownership.rs:194-212` | shutdown lock | `entry.join()` `:209` | no | join outside lock; bounded join |
| B10 | `durable_state/writer_fence.rs:24-31` | worker lock | `worker.join()` | no | take handle out, join outside |
| B11 | `runtime/waiting_room_public_projection.rs:625-643` | `LAUNCH_TARGET_CACHE` | `git rev-parse` ×2 `.output()` (`:650-675`) and `detect_git_branch` | no | compute outside lock, then insert |
| B12 | `provider/mcp_proxy/stdio.rs:385-393` (`write_frame`), reached from `:305-330` | per-process mutex (`mcp_proxy.rs:150-158`) | `write_all` to child stdin | **no deadline**; kill path skipped | writer thread + deadline, or kill at deadline |
| B13 | `runtime/browser_controller_process/display_frames.rs:338-341` | `Link.writer` | `write_all` | no | writer thread or deadline |
| B14 | `runtime/browser_controller_process.rs:624-628` | controller stdin lock | `write_line` | no | writer thread or deadline |
| B15 | `provider/codex_client/auth.rs:491-500` | none | `codex --version` `.output()` | no | bounded helper |
| B16 | `provider/codex.rs:151` | none | `codex logout` `.status()` | no | bounded helper |

### 7.2 Heuristic hits dismissed after reading the code

- `terminal/stream/store.rs:67-79` (`release.wait()` under shard lock): `#[cfg(test)]` hook (`hold_session_shard_for_test`).
- `runtime/state/agent_process_watch.rs:172-179` (`thread::sleep` in `shutdown`): the `live.lock()` is in a `while` condition, a temporary dropped before the body.
- `runtime/browser_controller_process/pending_mutation.rs:263-266`: `try_lock` loop; sleeps with no guard held.
- `runtime/app_http/streams/ports.rs:118-125`: `try_lock`, then `select!` (async).
- `room_terminal/service.rs:238-357`: guard dropped (`drop(creation)` at `:356`) before `pending.wait()` (`:357`).
- `managed_bootstrap/supervisor.rs:518-522`: `if lease.lock()…is_some()` temporary.
- `managed_context/credential_free/history.rs:76-80`, `:184-189`: guard is per loop iteration (`let mut owned` in loop body); `kill` is fast. OK.
- `runtime/prompt_state.rs`, `runtime/state/provider_login_state.rs` `.status()` hits: state accessor, not `Command::status`. Not blocking.
- `history.rs:1113`, `runtime/kernel_gateway/link.rs:433`, `app_worker/startup.rs:249`: channel `.send` (non-blocking or bounded by design).
- `runtime/state/workflow.rs:116-219`: `request(` / `launch_` hits are the launch in B8 (kept).

### 7.3 Still to verify (round 2)

`managed_context/outbound_operation_retention.rs:100-108`, `:184-222` (fs under let-bound lock, long scope); `runtime/state/workflow_prompt_queue_owned_state.rs:19`, `:395` (fs under lock); `runtime/kernel_browser_host.rs:546-580` (backend request scope); `secret/vault.rs` `VAULT_FILE_LOCK` (file I/O); `runtime/app_http/streams.rs:388-437` (`.await` under `inner`, may be async-safe).


## Round 1 changes and enforced order

The incident endpoint maps and ROWS lock no longer exist. Codex and OpenCode
supervisors own their children behind bounded command/reply channels. The
sampler alone runs bounded `ps`; its checked snapshot mutex protects only an Arc
and an error. Last-good samples retain their original timestamp after failures.
`ensure_available` checks executable availability without starting or waiting
for a process. Control paths recheck process identities before signals.

Provider launch uses a per-agent reservation. Reservation reads the registry and
allocates an id under STORE, planning and preparation run with no store or
admission guard, then commit validates the admission fences and previous run
again. Failed or refused plans release their reservation. Handover identities
are checked for collision and claimed only during commit, after all fallible
planning, so a fence refusal cannot consume a live handover. Existing runs may
be replaced when the agent's previous run is unchanged; simultaneous starts for
one agent return an explicit retry error. Waiting-room target computation also
runs outside its cache lock.

The checked-lock ranks below are executable in tests. A reverse edge or recursive
acquisition panics before waiting. Production records holder/waiter call sites,
thread names, and durations. The independent watchdog logs one notice per stall
with all checked holders; its registry never acquires a product lock. Runtime
pump progress is watched at 10 seconds, and relay read progress at 90 seconds.
Healthy relay pings occur every 30 seconds. A read-idle connection or payload
handler exceeding that deadline is dropped and reconnected.

| Lock | Rank | Work allowed while held |
|---|---:|---|
| `waiting_room_launch_target` | 5 | clone/publish cache entry |
| `provider_managed_admission` | 10 | clone gate reference |
| `provider_move_admission` | 20 | clone move-store reference |
| `provider_service` | 30 | reserve/read/commit run state |
| `process_rows_snapshot` | 40 | clone/publish immutable table |

The gate admission and agent-move records guards wrap only reservation and commit,
using their existing order gate -> move records -> STORE. Those admission guards
are not held during planning. STORE -> hot-reload handover tables remains a short
in-memory edge. No production edge reaches `env_lock`. The guard module is
compiled only with `cfg(test)`; its remaining call sites are also test-only.

Subprocess output and synchronous child reaps use `runtime::bounded_process`.
General command output has a 60-second deadline and 64-MiB combined output budget;
authentication probes retain their stricter one-MiB budget and specific deadline.
Sampler output has a one-second deadline. Shutdown uses TERM, 250-ms grace, KILL,
and a one-second reap deadline. All shutdown errors are logged; account owners
retain unreaped children and continue polling without blocking request threads.
Deliberately long-lived managed-kernel/async process owners poll through the same
helper, under the caller's lifecycle/cancellation policy. The source gate rejects
raw child wait/output calls outside the helper, production environment locks and
the previous STORE-held planning expression; domain output projection and the
bounded Docker broker facade are explicitly distinguished from subprocess APIs.
The gate is part of `pnpm lint`, and its tests are part of the existing script suite.

## Explicit bootstrap environment exception

Production launch planning only reads stable process configuration and constructs
child environments explicitly. Bootstrap still scrubs consumed transport auth,
Docker broker descriptors, cloud enrollment settings and the handover-file locator
from the environment to prevent accidental inheritance or later reloading of
bootstrap-only credentials. These existing one-time initializers do not use an
environment mutex. Main now loads/scrubs configuration and receives handover before
starting the Tokio runtime or process logger. Removing these scrubs without first
replacing every inherited-environment/bootstrap reader would change credential
handling; that migration is deliberately left for round 2. This round therefore
removes every production environment **lock**, but does not claim zero bootstrap
environment mutation.

## Round 2 remaining work

1. Replace the lexical candidate classifications with a cross-function order
   graph and verify every remaining RwLock/tokio guard scope; migrate remaining
   lock classes to checked guards. The tables deliberately mark unresolved cases.
2. Finish removing STORE from runtime binding/spawn, park/resume/termination and
   other closures outside the planning/commit path; preserve stale-run cleanup
   and retirement races. The sampler owner removes the original STORE -> ps edge,
   but process creation and filesystem preflight still need extraction there.
3. Give MCP stdio, browser/display writers, filesystem persistence and joins owned
   workers and explicit deadlines. Per-run serialization alone does not bound I/O.
4. Replace bootstrap environment scrubs with explicit startup context and inherited
   environment filtering, then forbid production environment mutation entirely.
5. Consolidate remaining command status, pipe-reader joins, platform-specific
   process supervision and spawn handshakes into the same deadline/ownership
   contract. A subprocess deadline cannot interrupt arbitrary OS filesystem I/O
   or a pre-exec/exec handshake before the child handle becomes available.
6. Add full live multi-hour hosted relay/provider acceptance under the deployment
   matrix. Source regressions and synthetic failure injection prove these seams;
   they do not establish the complete MP ordinary/managed acceptance matrix.
