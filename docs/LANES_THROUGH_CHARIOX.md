# Lanes through Chariox

MP-08 / MP-09 / MP-10 / MP-11. This runbook keeps the coordinator's policy outside
while replacing worker transport/control with the public CLI. Its adapters are
based on `dogfood/adapters` at `4cebf9f0f32ff933db0a6bbb52e7020871c0b200`.
The r6 runtime pin is `extagent/v2-events` at
`0d31a2ee3bd5943765f13c47c0e35f65816c5c9b`. No protocol allocation or kernel change is made
by the coordinator scripts.

## Ported loop

Use [the command/configuration reference](../tools/lanes/coordinator/README.md).
Keep the long-lived coordinator process alive after its owner web popup grant;
run its child commands with the same local kernel home/socket. Launch all lanes,
then block on one grant-owned §14 inbox. React to events with a one-shot status,
fetch, bundle or stop. Do not put a status request in a timer loop. Each batch is
saved before acknowledgement; timeouts simply wake the coordinator.

| Mac step | Chariox operation now | Remaining Mac work/gap | Agent-model piece |
| --- | --- | --- | --- |
| brief.sh | `coordinator/brief.mjs`: lane-launch + files put; original launch checkpoint and kernel deadline | Choose lane, source, linked profile and brief. Restart means a new round identity, not extending the deadline. | A01 Room delegation/admission and A02 durable inbox; source `amint/c2` @4bea22d44. |
| watch.sh | `coordinator/watch.mjs`: one WaitEvents/AckEvents inbox across workers; archive every batch | GitHub review lookup stays on Mac. File-only HOST_REQUEST/OWNER_SESSION changes need an event bridge; working state alone is not progress. | A02 ledger/wakes and A03 process/timer watchers are producers; EA2 §14 exposes outside subscriptions. |
| deliver.sh | `coordinator/deliver.mjs`: read supplied review, status once, compare-and-steer the active prompt | Fetch review text on Mac; a finished turn needs a new round, never an implicit prompt submission. | A01 send_agent_message correlation / steer; no duplicate delivery core. |
| lanefin.sh | `coordinator/lanefin.mjs`: status + fetch notes/head + SHA-verified bundle/import | Publication, independent review and central acceptance merge stay on Mac. Final message must be written into fetched notes; no transcript scrape. | A02 explicit done/waiting/blocked; A03 process-exit wake for bundle commands. |
| lanehealth.sh | `coordinator/lanehealth.mjs`: state, last activity and deadline with LONG-ROUND/SILENT flags | Decide whether a silent worker is stuck; no claim that last activity proves useful progress. | A02 /goal completion assessor and no-progress check-ins; A03 watchers can improve evidence. |
| stop | Kernel SetAgentDeadline/GetAgentDeadline, then `coordinator/stop.mjs` | Exact owned-resource cleanup audit remains necessary; agent destruction alone is not a complete host cleanup receipt. | A04 sudo is permission duration, not a hard stop. EA2 durable deadline supplies the missing unconditional wall-clock bound. |
| Usage tracking | Product profile refresh/status for admission | Mac fleet usage accounting and quota policy by design | Account/profile source is separate from agent-model. |
| Host rental | Approved process supervision can report exit through a Room terminal | Mac cost decision/rental by design; no automated paid host creation | A03 process watcher can supervise an already approved rental command. |
| Acceptance merge | Fetch validates full acceptance row shape | Mac coordinator merges central acceptance by design | A02 ledger can carry readiness, not acceptance authority. |
| Review lookup | Receive already supplied review text | Mac GitHub lookup by design | A03 could supervise lookup; no built-in GitHub policy is claimed. |

## Two shapes

**A — outside coordinator.** The coordinator remains in its official provider
harness. An owner-approved until-exit grant admits the public CLI on its local
kernel; hosted WSS routes to the executor. The six scripts replace launch,
steer, event wait, health, collect and stop without SSH. This gets lanes running
through Chariox first because it preserves existing policy/accounting and the
publication boundary. The r5 grant/inbox drift is fixed upstream in the r6
runtime pin; watcher receipts additionally refuse a changed subscription id.

**B — Room coordinator and leased workers.** Chariox agents coordinate in a Room,
with workers leased onto b3 and woken by the ledger/watchers. The source map is
`amint/c2` @`4bea22d440d792ba9e866020bcfb8408a6deb7d6`,
`AGENT_MODEL_OVERHAUL_PLAN.md`: A01 Room delegation, A02 inbox/wakes and goal
assessment, A03 process/terminal/timer producers, A04 elevation and A10 leases.
These pieces are built in that source; this dogfood round does not validate its
separate A01–A10 matrix or combine the branches. B still needs the coordinator
policy/brief/accounting bridge and owned-resource cleanup. Both shapes can
combine: an outside coordinator observes and steers the same Room workers through
§14. Do not implement a parallel ledger, watcher service or delegation engine.

## Evidence and acceptance boundary

MP-08 / MP-09 / MP-10 / MP-11. The r6 cell is three concurrent real read-only PR
review workers (OSS #1081, Cloud #443, OSS #1044), a mid-round steer, one short
durable deadline, two normal completions, artifact/bundle collection and explicit
stops over hosted WSS. The round's evidence and final per-condition result live
outside git at `dogfood/r6/`; the acceptance update uses a full tracker id.
Source tests and prior rounds alone do not establish r6 acceptance or broad
Path-1 parity. Provider model ids, exact commits, one-inbox events and cleanup
must be included before recording a live PASS.
