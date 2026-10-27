# MP-07 / MP-09 / MP-10 / MP-11 managed shutdown acceptance

This procedure prepares a coordinator-owned isolated Cloud run. The b214 run
changes Node observation fixtures only: no VM creation, deployment, staging,
provider login, or live billing/deletion claim. All four MP items remain open.
Use the canonical Browser/Computer plan and its frozen 2026-10-01 outage decision.
The retained managed8 F handoff is historical evidence, never G2 evidence.

## MP-07 / MP-09 / MP-10 / MP-11 prerequisites

The coordinator must supply a reviewed paired Cloud commit. The available
builder2 checkout `f6cfd0066d75844dbab795cdf715789ec5fa37d6` is unpaired;
its warning-only no-ACK tests are insufficient for the D/G2 outage policy.
Do not run it as the acceptance control plane or silently substitute staging.
The OSS starting source is G2 main `358491d662330aff538bbd6d6450391332f62ac2`
(protocol 411 / relay 70). Record the final integrated source separately from
this base and the source commit inside each signed release. Never relabel a
base fixture run, G2 artifact, Apps aggregate, or signed F observation.

Before live work, retain independent exact-head reviews, final signed release
and image digests, signer public fingerprints, kernel binary hash, Cloud source
and deployed image hash, and effective service policy. No owner private key or
provider API credential belongs on builder2. Provider logins remain human-only;
use product-linked/materialized profiles. An empty-context shutdown target does
not prove selected-context transfer or provider login acceptance.

Provide an isolated reviewed Cloud database/API/Web/reconciliation manager and
relay, an owner-authenticated local kernel using an absolute lane-owned
CHARIOX_HOME, and a reviewed product VM bridge with an external watchdog.
The bridge must have an independently reviewed public source/hash receipt.
The Cloud endpoint must identify the isolated deployment. Never contact the
reserved Apps/relay machines or active staging. Runtime stays kernel-authoritative;
Cloud owns lifecycle/billing and relay carries encrypted transport only.

Run one target per driver and at most four coordinator-owned VMs overall.
Use agent-small, explicit region, operation-bound names/IDs and a deletion
watchdog before CREATE. Reserve the real 3-hour and 60-minute windows; no fake
clock or shortened substitute qualifies them. Keep at least 16 GiB MemAvailable
and 10 GiB disk free on builder2. Rust builds, if needed, use the coordinator's
compile-1 flock, four jobs and `/root/work/cargo-target-b214`.

## MP-09 / MP-10 / MP-11 complete trigger ledger

Each row starts OPEN. Fill a fresh external receipt, including skipped/blocked
rows. The normal driver supplies projection observations, not an acceptance
verdict or independent provider/Cloud/relay absence.

| Row / driver | Exact policy and required evidence |
| --- | --- |
| Final agent / `shutdown_agents_done` | 0/900. Real provider busy→idle, final finish transition, full delay, unique STOP. This is distinct from immediate agents-done mode. |
| Immediate agents-done / bridge | 0/0. Preserve the 30-second warning floor; prove the final real provider transition and kernel fence, not silence. |
| Default idle / `shutdown_idle_15m` | 0/900. Warning = deadline−30s; deadline measured from last finish. |
| Idle 30m / `shutdown_idle_30m` | 0/1800. Full real window, no early queued STOP. |
| Minimum runtime / `shutdown_minimum_3h` | 10800/900. Deadline is max(runtime start+10800s, last finish+900s, accepted report+30s). Observer stays alive through the 3-hour boundary. |
| Custom / `shutdown_custom` | 0/600. Preserve actual selected custom policy and clocks. |
| Disconnected / `shutdown_all_clients_disconnected` | 0/900. Independent client census proves every interactive client disconnected before deadline; owner-IPC observer stays alive. Enter alone is not proof. |
| Disabled / `shutdown_disabled` | 0/null. No deadline/warning/STOP for the declared bounded negative window. Never call this indefinite keep-alive proof. |
| Keep running / `shutdown_keep_running` | 0/900. Genuine Web click, unchanged finish identity, cleared warning/deadline, no STOP through original deadline+120s. Minimum driver budget is 1800s. |
| Default-policy manual / `shutdown_manual` | 0/null. Always-available normal Web Stop control; unique STOP receipt. A warning control with another policy cannot qualify this row. |
| Warning Stop now / bridge | 0/600. Wait for actual warning and click its Stop now control; separate screenshot/action/operation receipt from default-policy manual. |
| Cloud restart / `shutdown_restart_reconciliation` | 0/900. Restart only the owned isolated reconciliation deployment. Original finish/deadline/reservation cutoff survive; eventually the same bound STOP succeeds. |
| Explicit lifecycle / `shutdown_explicit_lifecycle_reconciliation` | 0/null. Normal owner request and exact successful STOP/revision. Does not prove Web UI. |
| Activity fence / bridge | Fresh work or Keep running cancels obsolete reservation. Re-admission while a real acknowledged fence is held follows kernel policy. Record challenge/ACK/release tuples; require explicit kernel receipt. |
| No-ACK outage / bridge | Due deadline, stale heartbeats, durable reservation and latest busy ACK bound the 3600s cutoff. No early STOP; restart cannot renew cutoff; silence is never a fence. Fresh heartbeat/activity, keep-running, disabled policy, changed account/Machine/kernel/generation/revision or pending operations cancel/block obsolete STOP. Require durable reason and normal non-destructive provider STOP. |
| Signed deployment / `shutdown_deployment_reconciliation` | 0/900. Real Cloud-authorized signed A→B activation, exact update ID/from/to digests, original shutdown clocks preserved. A same-release no-op is insufficient. MP-07 rollback/recovery must qualify first. |
| Retry once / bridge + normal driver | Inject one owned provider STOP failure, then one manager retry of the same operation/idempotency/revision (attempts 1→2). Also exercise DELETE once. No second lifecycle request or second allocation. Retain both failure and success observations. |

For every row retain UTC plus process monotonic observation times, observer PID
and lifetime, request latency/gaps, target environment/account/owner/Machine/kernel,
Cloud generation and activity sequence, requested/effective policy, runtime start,
last busy/final finish/report, warning/deadline, challenge creation/idle tuple,
latest busy ACK, fence ACK/release or explicit absence, operation ID/kind/revision,
attempts/status/created/completed times, physical power and storage receipts,
cleanup and exact source/release bindings. Missing facts remain unknown.

The driver's `snapshotCoverage` counts successful projection reads separately
for workflow and cleanup; it retains first/last UTC, monotonic bounds, maximum
gap and request latency even when summaries are deduplicated. It is not proof
of a fresh kernel heartbeat. `operationObservations` retains projected changes
rather than overwriting a failed attempt with final success. `acknowledgedAt`
records the operator's reply, not proof of a genuine UI click. Screenshots and
independent receipts must bind the actual action. A dead/expired observer or
an unexplained sampling gap cannot qualify a continuous timing window.

## MP-07 / MP-09 / MP-10 / MP-11 fixture commands

From the lane OSS checkout:

```sh
node --test apps/cli/scripts/live-managed-shutdown-trigger-drill.test.mjs
node --test --test-concurrency=1 \
  deploy/managed-kernel/managed-release-activation.test.mjs \
  deploy/managed-kernel/managed-update-recovery.test.mjs \
  scripts/managed-kernel-upgrade.test.mjs \
  deploy/managed-kernel/path1-service-policy.test.mjs
```

After the coordinator supplies and records the paired Cloud source, build in
its own worktree and run the focused control-plane suites, with logs external:

```sh
pnpm --filter @chariox-cloud/api... run build
node --test --test-concurrency=1 \
  apps/api/dist/managed-environments/auto-stop-deadline.test.js \
  apps/api/dist/managed-environments/auto-stop-reconciliation.test.js \
  apps/api/dist/managed-environments/auto-stop-quiescence-contract.test.js \
  apps/api/dist/managed-environments/auto-stop-quiescence-lifecycle.test.js \
  apps/api/dist/managed-environments/auto-stop-quiescence-release-policy.test.js \
  apps/api/dist/managed-environments/auto-stop-quiescence-timeout.test.js \
  apps/api/dist/server-managed-auto-stop-timer.test.js
```

Include the paired source's no-ACK cutoff/restart/binding-race and normal
manager STOP/DELETE retry suites. Inventory their exact filenames first; the
stale baseline's timeout-warning suite does not cover them. UI Stop-now and
Keep-running controller fixtures are a separate focused Web check and do not
replace real clicks. Fixture clocks establish source behavior only.

## MP-09 / MP-10 / MP-11 live commands for the coordinator

The following standard driver creates and deletes exactly one target through
normal owner IPC. Execute only after isolated Cloud/VM/watchdog approval and
normal owner login. Build the matching released kernel client first. The region
and loopback port below are examples to replace with approved values.

```sh
export CHARIOX_HOME=/root/.chariox/dev/browser-resume-20260930/agents/b214/live-home
node apps/cli/scripts/live-managed-shutdown-trigger-drill.mjs \
  --scenario shutdown_minimum_3h \
  --kernel-url ws://127.0.0.1:4488/kernel \
  --region hel1 --compute-class agent-small \
  --max-billable-seconds 14400 \
  --confirm-one-target CREATE-AND-DELETE-ONE-MANAGED-TARGET \
  --output /root/.codex/evidence/browser-resume-20260930/b214/MP09_minimum_3h.json
```

For other normal driver rows use the scenario in the ledger and a new output
path. Use 1800s for final-agent/default idle/custom/disconnected/Keep running/
restart/deployment, 3000s for 30m, and 900s for disabled/manual/explicit lifecycle.
These are starting budgets: measure provisioning, human-action and observation
needs before CREATE. At most 14400s total; the last 300s are cleanup reserve.
Never force-stop the observer with an outer timeout before cleanup can settle.
Keep-running now fails if it cannot observe through the old deadline+120s.

The standalone CLI deletes promptly after its projection observation. It does
not inspect provider storage or prove STOP/start persistence. For full evidence
use the exported driver with the coordinator's reviewed bridge. The bridge must
implement `createManagedShutdownDependencies` (returns official local IPC
client/requests plus real owner actions and `observeBeforeCleanup`), and
`runManagedShutdownFaultScenario` for the additional ledger rows. These exports
are a handoff contract; no live bridge is bundled or claimed validated here.
Constructor calls must be side-effect free; allocation occurs only through the
normal product CREATE. The coordinator must pin/review the actual module before
using this exact wrapper, never accept a module's own success assertion.

Set `B214_BRIDGE`, its independently reviewed `B214_BRIDGE_SHA256`,
`B214_KERNEL_URL`, `B214_REGION`, `B214_SCENARIO`, `B214_SECONDS`, `B214_CASE`,
`B214_CLOUD_SHA`, and a new absolute `B214_OUTPUT` outside repositories. Set
`B214_CASE=normal` for a normal row; fault row names are `activity_fence`,
`no_ack_grace`, `agents_done_immediate`, `warning_stop_now`, `retry_once`.
The reviewed bridge must reject unsupported rows and non-isolated identities.

```sh
node --input-type=module <<'JS'
import assert from 'node:assert/strict'
import { createHash } from 'node:crypto'
import { lstat, readFile } from 'node:fs/promises'
import { execFileSync } from 'node:child_process'
import { pathToFileURL } from 'node:url'
import { isAbsolute } from 'node:path'
import { parseArguments, runManagedShutdownTrigger } from './apps/cli/scripts/live-managed-shutdown-trigger-drill.mjs'
const env = process.env
assert.ok(isAbsolute(env.B214_BRIDGE))
assert.ok((await lstat(env.B214_BRIDGE)).isFile())
assert.match(env.B214_BRIDGE_SHA256, /^[a-f0-9]{64}$/)
assert.equal(createHash('sha256').update(await readFile(env.B214_BRIDGE)).digest('hex'), env.B214_BRIDGE_SHA256)
assert.match(env.B214_CLOUD_SHA, /^[a-f0-9]{40}$/)
const bridge = await import(pathToFileURL(env.B214_BRIDGE).href)
const options = parseArguments([
  '--scenario', env.B214_SCENARIO, '--kernel-url', env.B214_KERNEL_URL,
  '--region', env.B214_REGION, '--compute-class', 'agent-small',
  '--max-billable-seconds', env.B214_SECONDS,
  '--confirm-one-target', 'CREATE-AND-DELETE-ONE-MANAGED-TARGET', '--output', env.B214_OUTPUT,
])
const signalController = new AbortController()
process.once('SIGINT', () => signalController.abort())
process.once('SIGTERM', () => signalController.abort())
const binding = {
  ossSha: execFileSync('git', ['rev-parse', 'HEAD'], { encoding: 'utf8' }).trim(),
  cloudSha: env.B214_CLOUD_SHA, options, signal: signalController.signal,
  maximumConcurrentTargets: 1,
}
try {
  if (env.B214_CASE === 'normal') {
    const deps = await bridge.createManagedShutdownDependencies(binding)
    assert.equal(typeof deps.observeBeforeCleanup, 'function')
    await runManagedShutdownTrigger(options, { ...deps, signal: signalController.signal })
  } else {
    await bridge.runManagedShutdownFaultScenario({ ...binding, case: env.B214_CASE })
  }
} catch {
  process.stderr.write('MP-07/MP-09/MP-10/MP-11 capture incomplete; inspect safe external receipts.\n')
  process.exitCode = 1
}
JS
```

The bounded `observeBeforeCleanup({target, scenario, signal, remainingMs})`
barrier follows successful projection observation and precedes DELETE. Its
return value is discarded. It must write separate allowlisted receipts, obey
abort/deadline, and bind exactly the target given. A timeout/failure makes the
workflow incomplete and still reserves normal DELETE. It cannot turn a helper
return into acceptance or inspect foreign targets.

## MP-07 / MP-09 / MP-10 / MP-11 STOP, persistence, deletion and cost

Before STOP, create public fixture files and installed-package/context/history
markers through a real provider turn. Record hashes, provider thread/turn IDs,
workspace and storage identities without credential/profile content. At STOP,
independently observe provider OFF and unchanged root/volume identities before
DELETE. The bridge then requests normal START, observes a fresh heartbeat for
the same Machine/kernel and reads the public markers/history through product
paths. STOP must preserve disk/context and never reimage. Compare exact hashes,
package version and a real next provider turn. Request normal STOP again and
observe OFF, then allow the driver's normal DELETE. Record each operation and
policy deadline anew after START; do not confuse it with the original timing
row. Allocate enough action budget for these checks or leave persistence OPEN.

For no-ACK, fault only the owned kernel/heartbeat route after a durable challenge
exists. Keep the observer/Cloud manager alive. Retain the challenge creation,
idle deadline, last busy ACK and restart identities; derive the cutoff from the
paired implementation's durable max anchors +3600s. Independently observe no
early operation/power change and final normal STOP with missing-ACK reason.
Restore through the product path before persistence checks. Never infer a
kernel fence from offline state. Repeat fresh-heartbeat/activity/keep-running
and obsolete-binding cancellation negatives before qualifying this exception.

For MP-07 reconciliation, retain the actual Cloud update request, effective
release link, binary hash and health-admission receipt. Crash/rollback/reboot
proof is separate from a timer fixture. Do not reuse the failed F rollback or
call a G2 same-release request a real signed deployment transition.

After normal DELETE, retain the exact operation/revision/completion and
independent provider per-ID GET-not-found plus account LIST census for the owned
server, every root/data volume, primary IPv4/IPv6 and any separately allocated
network resource. Cloud/relay inspection must independently show the environment
DELETED at that revision, Machine revoked, no active machine credentials,
unrevoked grants/tokens or non-revoked targets, and matching retirement
tombstones. Retained revoked heartbeat rows are historical data, not online
presence. Distinguish authoritative absence from permission errors/timeouts;
never infer deletion from one missing UI card. Check watchdog/process absence
by exact run-bound identity. Inspect only public projections, not raw secret rows.

Keep a cost receipt per resource: allocation/deletion UTC, provider resource ID,
price quote date/currency/tax basis/unit/rate, charged-unit rounding, included
versus additional storage/IP, reservation/watchdog cap, STOP/start durations,
estimated ceiling and actual provider invoice line (or `actual: unobserved`).
Do not copy F's prices or EUR1.922564 estimate into a G2 bill. Compute the estimate
from approved current quote inputs; STOP/OFF alone does not prove billing ends.
Track intentionally retained shared snapshots separately by owner authorization;
they are neither deleted evidence nor lane cleanup targets.

Finally remove only run-owned disposable state/identities after exact inventory,
excluding durable key/backup/profile stores and shared reviewer state. Remove
owned build outputs when settled. Retain source commits, small public receipts,
clock/resource/exit logs, screenshots and cleanup records outside repositories.
Never prune Docker/buildx caches or foreign images/containers. Every ledger row
needs independent final-release review and live receipts before MP closure.
