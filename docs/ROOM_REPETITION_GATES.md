# MP-07/MP-08/MP-10/MP-11: release-B local repetition gates

`apps/cli/scripts/live-room-repetition-drill.mjs` runs the ordinary signed B
kernel and relay with a B client, headed image, and retained Cloud View build.
It requires absolute `LOOPS_CLIENT_ROOT`, `LOOPS_RELEASE_ROOT`,
`LOOPS_CLOUD_ROOT`, `LOOPS_STATE_ROOT`, and `LOOPS_EVIDENCE_ROOT` paths. State
and evidence must be outside repositories. The Cloud checkout must already
contain its compiled Web assets and Playwright dependency; nothing is built,
published, or deployed by this harness.

Defaults are 50 reconnects, 10 save/full recreates, and 50 headed create/deletes.
`LOOPS_RECONNECT_CYCLES`, `LOOPS_SAVE_CYCLES`, and `LOOPS_CREATE_CYCLES` select
smoke counts. `LOOPS_PHASE=reconnect|persistence|create-delete` selects an
isolated phase; skipped gates are explicitly `NOT_RUN`.
`LOOPS_FORCE_RESTORE_PORT_RACE=1` occupies the first restore's old display port
with an owned test listener (or records an existing natural collision), then
requires the normal bounded slice recreation retry. Run the comparison
regressions first:

```sh
node --test apps/cli/scripts/lib/room-repetition-gates.test.mjs
node apps/cli/scripts/live-room-repetition-drill.mjs
```

The harness verifies the exact B binary hashes and headed-image runtime label.
It records runtime, built-client, Cloud, and harness identities separately.
It uses a private synthetic local Cloud bootstrap and the production View
pages. Browser keys are generated through the normal browser client and only
public pairing pins cross into the kernel. Relay fixture signers remain in
process memory. Provider accounts and external infrastructure are unused.

MP-08/MP-10 reconnect checks restart both real TUI processes and reload one
of two real Web View pages against the same Room and active browser. Both
canvases must retain decoded frames for a two-second stability window matching
the Room viewport; active-browser pixels must advance on each viewer. Each cycle
compares Room, environment, runtime generation, tabs, agents, history, and
action ledger. This is client detach/reattach and Web reload evidence, not a
relay/kernel crash, in-flight provider reconciliation, or offline event replay
gate. No real provider turns are submitted.

MP-08/MP-10 persistence checks save/shutdown, verify a nonempty home archive,
remove the original container and home volume, restore, and assert cookie,
localStorage, IndexedDB, Cache Storage, service-worker registration, machine
identity, durable display ports, Room environment identity, and a saved file.
Same-slice restarts require unchanged machine identity and ports. If a port
collision requires deleting/recreating the slice, the saved state is reused,
the new slice receives new ports and its own stable machine identity, and all
browser/file/Room assertions still run. These replacements are recorded rather
than counted as same-slice identity preservation.
This focused repetition gate complements M20; it does not repeat M20's real
editor, login, download, named-backup, or offline-service-worker cases.

MP-10 create/delete captures full host process, container, volume, image,
listener, disk and service-resource inventories after every cycle. Owned
resources are compared separately from unrelated shared-builder activity.
Kernel/relay FD drift above eight, RSS drift above 128 MiB, or owned disk drift
above 64 MiB fails the cycle; raw deltas remain available below these limits.
Worker process metrics include the browser/controller and worker services.
Owned worker PID reuse is fenced by process start time. A listener on a
retired port is attributed to a foreign container only when its published-port
mapping positively establishes that owner. Orphan listeners without such a
mapping still fail. Docker proxy PIDs are observed while the owned container is
alive and fenced by process birth time. Foreign host listeners require process
ancestry evidence outside the owned runner/services and cannot be previously
observed worker/proxy PIDs. Foreign port reuse remains in the per-cycle inventory.
Port collisions cause
slice deletion/recreation and are recorded, with at most three retries. No
shared slice-port lock is used.

No MP item closes from these local provider-free results. Full Drill H also
requires the approved maximum slice count, a normal workflow, slow-viewer
injection, and their full cleanup comparison. Fresh-machine Path-1 parity,
hosted transport/auth, official-provider thread continuity, and the eight-hour
and twenty-four-hour soaks remain independent gates.

## MP-08/MP-10/MP-11: OCR discovery finding

The release-B live probe advertises both `slice_ocr` and `chariox.slice_ocr`
for a Room-bound native `dev-stub` run, in both Starting and Running states.
A second discovery-only probe using the original `default` model confirms
both names remain available during twelve Running samples. After explicit
`EndSession`, that same run becomes Ended and its old MCP binding returns an
empty tools list.

The reproduced missing-tool seam is run lifetime, not a missing OCR spec:
`ProviderService::get_runs_by_runtime_mcp_auth_token` excludes Ended runs
(`apps/kernel/src/provider/service/run_lifecycle.rs`), and
`runtime_tool_specs_for_auth_token` advertises slice tools only for the active
run/binding (`apps/kernel/src/runtime/state/tool_dispatch.rs`). GetProviderRun
can retain old MCP binding metadata after termination; checking only for its
presence does not establish an active invocation context.

Bench2's original `ad7e2fa6808e8912823bd3138e5525dcc18d13a9` result reported
missing OCR without retaining the run state or tools list at failure. Its
later `e8b641a424ddde2875c7b27e298db6cda33d8d4a` fixture uses
`native-tui-idle` and retains state/tool names; its fresh Running probe has OCR.
The default stub runs `cat`, whereas the idle fixture explicitly keeps reading
until its bounded one-hour shutdown. Ended-run discovery is a confirmed
failure mode and a plausible explanation of the original result. The original
run's exact cause remains unproven without its missing lifecycle evidence.
A discovery harness should require an active run and retain the run state and
tool names before declaring a tool missing. No runtime change is justified by
this finding alone.

Retained evidence is under
`/root/.codex/evidence/browser-resume-20260930/loops/`, including
`ocr-default-lifetime.json`, `smoke4/ocr-tools-list-running.json`, fail-first
and passing focused-test logs, per-cycle inventories, source identities,
commands/exit codes, initial REDs, and cleanup.

## MP-08/MP-10: retained initial REDs and first seams

All failures remain under their original runtime/harness identities in external
evidence. Later harness corrections do not relabel them:

- `full1`: after nine reconnects, kernel TUI attachment cleanup won a redundant
  explicit-detach race. The harness now accepts only the exact already-removed
  attachment error; all other detach errors still fail.
- `full2`: ten reconnects passed before both Web canvases disappeared. The
  bootstrap fixture reused one client ID across distinct reload-generated keys.
  The corrected fixture uses a public-key-specific client ID. This is a fixture
  identity collision explanation, not proof of a production reconnect defect.
  Public-pin and decoded-canvas evidence are retained in the rerun.
- `full2`: five save/full restores passed before a host-port collision. The
  original harness lacked saved-state-preserving recreation; retries now reuse
  the saved state, delete only the owned failed slice, and allocate new ports.
- `full2`: nine create/delete comparisons passed; the tenth observer flagged
  retired ports now mapped to other live containers. Supplemental attribution
  resolves all 90 final flags to foreign owners; no owned process/container/
  volume/image survived. The original RED remains unchanged.
- `persistence3`: the fault injector itself got EADDRINUSE because another
  container already owned the old display port. That natural collision now
  drives the normal retry rather than failing injection setup. Its 45 final
  listener flags were positively attributed to foreign containers.
- `persistence4`: the injected collision was recovered, then the harness rejected
  the new slice's machine ID as if it were the old slice. The provisioner hashes
  `CHARIOX_SLICE_MACHINE_ID`, supplied as `slice:<slice-id>` by
  `apps/kernel/src/slice/local_docker.rs`; it deliberately refreshes machine ID
  during provisioning. Same-slice stability remains strict; replacement identity
  is checked against the new slice's hash. Cleanup was GREEN.

Fail-first regressions cover resource leak classes, Room continuity, exact
attachment-race handling, published-port ownership, and replacement identity.
`create3` also retained a RED at its fourth cycle: the concurrent persistence
run's owned collision-injector process reused a retired port. Its PID is proven
by that run's retained identity. Supplemental cleanup resolves the flag and
removes only the finished create run's scratch root. The observer now tracks
active owned Docker proxies by PID/birth and checks host process ancestry;
foreign host reuse is excluded without exempting orphan proxies.

These observer/fixture fixes do not change B runtime or protocol shapes.

## MP-08/MP-10: reconnect visual-audit correction

`reconnect3` completed 50 transient readiness assertions and wrote an original
GREEN result. Independent visual inspection of `web-reconnect-50.png` found
Screen unavailable / Room display disconnected after that assertion. Its
`VISUAL_AUDIT.json` records acceptance RED without editing the original result.
The per-key fixture correction did not establish sustained display recovery or
prove the cause of the earlier two-viewer failure. The corrected harness requires
two seconds of uninterrupted decoded display on both pages plus advancing
active-browser pixel hashes after each reload, and retains failure socket-close
codes, readiness snapshots, and container CPU-throttling counters. The original
50 assertions establish transient reconnection only; they do not close this gate.

## MP-07/MP-08/MP-10/MP-11: final exact-B lane results

Runtime remains OSS `b37f4504e4ce040a2d6c35dc56475315defbc861`, kernel
`0695a7cb3a266546e51634f6fcda8b33b5d8f6f8e4f35430f96d4b2f4e2d229d`,
image `sha256:a282bac1efb1be1d34037ca068b7a9fe14575fac25381bd1ec956c9cadb299fc`,
protocol370/peer64, B client, and compiled Cloud
`94fda0ec7af67ce37c3da19e0dd6588c1131ae8e`. Harness identities below are
separate from runtime identity.

| MP-08/MP-10 gate | Retained run / harness | Result and scope |
| --- | --- | --- |
| Reconnect Web/local/remote TUI | `reconnect3` / `2a661b5a9` | 50 transient readiness/Room continuity assertions completed; visual audit makes acceptance **RED**. Stronger direct runs `reconnect4/5/6/8/10` fail two-viewer stability during initial attachment. |
| Save/full recreate/restore | `persistence5` / `aff78e9d0` | **GREEN10/10**. First cycle deliberately forced a port collision and restored saved state into a replacement slice; next nine fully removed/recreated the same slice container and home volume. Storage/SW/file/Room assertions passed every cycle. |
| Headed create/delete, comparison every cycle | `create4` / `72f61a1a7` | **GREEN50/50**, plus50 Unix/UDP socket audits. Three port-race retries recorded; no lock used. |
| One browser,2 Web viewers, local and remote TUI | Direct stable-view probes above | **RED**. Both TUIs attach, Room stays ready, but decoded Web display is lost. |

MP-10 create/delete maximum kernel/relay FD drift is+1/0. Maximum positive
kernel RSS drift is180,224bytes; relay RSS never exceeded baseline. Owned
scratch growth max5,835,451bytes includes retained runtime/log/ledger data.
Controller/worker metrics and full global/scoped inventories exist each cycle.
Minimum observed reserves are5.939GiB available memory and76.309GiB disk free.
Per-run final inventories and independent PID/birth/listener audits are clean;
all owned runtime scratch roots, containers, volumes and snapshot images are
removed. Other lanes and shared infrastructure were left alone.

## MP-08/MP-10: confirmed display seam; underlying cause open

`reconnect8` records both display sockets closing within one millisecond, code
1005, with no browser-requested close. Both canvases disappear while the Room
remains ready. `reconnect4` uses a one-CPU Web runner; `reconnect5/6/8/10` use
three CPUs and also fail, so the one-CPU Web cap is not the sole explanation.
The direct existing private-adapter probe in `reconnect6` keeps both viewers
alive for ten seconds, receives301/291 video records, and closes both with
exit0. It isolates a functioning private adapter case; it does not prove the
adapter cannot fail under the encrypted path's controls or backpressure.

`reconnect10/private-relay-close-codes.json` captures the actual private
loopback control envelope `daemon_display_tunnel_close` with
`display_stream_closed`, without changing forwarding or retaining packet,
auth, or video payloads. This identifies the encrypted forward-task completion
seam in `apps/kernel/src/transport/relay_client/display_tunnel.rs`,
`proxy_selkies_websocket`. Its `result = &mut forward` branch maps both a
forwarder error and a task join error to the same fixed public code. The exact
record/cipher/control/output/lease error is unavailable in signed B. A
backpressure root cause has **not** been proven.

The next useful runtime diagnostic is fixed failure classification from
`forward_selkies_stream` through that existing error surface, followed by this
same direct stable-view gate on a separately identified candidate. Keep each
record/cipher/control/output/lease category free of provider or credential
payloads. No runtime fix or protocol change is included in this lane.

`LOOPS_WEB_CPUS` selects the browser-runner CPU cap (default1).
`LOOPS_PRIVATE_STREAM_PROBE=1` runs the bounded direct two-adapter probe.
`LOOPS_PRIVATE_RELAY_PROBE=1` starts a scoped loopback close-code observer in
the owned slice; it needs root's existing packet-socket capability. Its parser
has a portable masked/unmasked/auth-exclusion regression:

```sh
PYTHONDONTWRITEBYTECODE=1 python3 apps/cli/scripts/lib/room-repetition-private-relay-probe.test.py
```

The ineffective home-relay diagnostic tap from `reconnect7` was removed after
confirming that local display sockets use the slice's private relay. That
run's RED and identity remain in evidence. `reconnect9` also retains its
observer setup RED (missing explicit exec port, empty diagnostic parse); the
input correction is in `ec3292a85`. Final focused checks pass7 Node tests and
the Python packet-observer contract. All cleanup passes, including failed
probes. No MP-01 through MP-11 item closes; full Drill H and managed parity
remain open.
