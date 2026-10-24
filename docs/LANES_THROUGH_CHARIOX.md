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
| deliver.sh | `coordinator/deliver.mjs`: read supplied review, compare-and-steer the active prompt, then confirm through a capability-gated §14 event or acknowledged-response/status fallback | Fetch review text on Mac; new instructions for a finished turn need a new round. The runtime pin lacks the event capability; extagent supplies its producer/projection and advertisement. | A01 send_agent_message correlation / steer and A02 admission records exist; r7 prepares their outside steer-receipt consumer. No duplicate delivery core. |
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

## R6 result and required scenario

MP-08 / MP-09 / MP-10 / MP-11. Required new scenario
`external-agents-v2/EA2-16C` is the complete coordinator round: three concurrent
real frozen-PR review workers, one grant/inbox, accepted and distinctly observable
mid-round steer, one active worker stopped by its kernel deadline, two ordinary
finishes, fetch/bundle verification and observable explicit stop for all three.

On 2026-10-24 the exact runtime pin and adapters `ccd6239cfb` completed the control
flow over hosted WSS: 15.727 seconds of three-worker overlap, 21 strictly ordered
events, all launch/removal events, all three fetched outputs and verified bundles.
The 20-second deadline reached at 269 ms after its due time and recorded physical
stop at 354 ms after due. Those are observations on b3, not performance gates.
The worker's partial notes remained explicitly incomplete; the other two workers
finished normally. All selected models were the exact id `gpt-6-luna`, effort
`low`. Acknowledgements followed durable local batch archival. No SSH or state
polling controlled the workers.

**EA2-16C is FAIL.** The mid-round steer command succeeded and the worker's fetched
notes explicitly acknowledge it, but no distinct steer event appeared in the
complete §14 inbox. The schema exposes agent state, interaction, terminal,
session, gap and subscription-end values; task publication suppresses an
unchanged state/reason. A01/A02 can supply admission evidence, but extagent owns
its outside producer/projection (and any coordinated protocol change). A failed
live assertion and exact event/command evidence are under `dogfood/r6/`.
Do not substitute a locally manufactured event or mark the full round accepted.

The first attempt is retained as FAIL: its short worker finished 3.331 seconds
before its 35-second deadline. A later reached receipt on an already idle worker
did not prove active cancellation. The second attempt used the measured shorter
deadline and exercised real active cancellation. Neither attempt regressed the
inherited grant/inbox binding: one stable subscription id was used per round.

A subsequent input-compatibility follow-up accepts the Mac watcher's maximum
minutes and the all-lane health invocation; its fake-adapter tests supplement the
live default path above. It does not change the pinned kernel/client results or
close the missing steer-event requirement. Source checks: 44 adapter/coordinator
tests, 2,682 compiled TypeScript CLI tests and process-signal lint pass. The
coordinator handoff carries exact source identities, cleanup and tracker update;
no broad MP parity or real-Mac workday acceptance is claimed.

## R7 adapter preparation

MP-08 / MP-09 / MP-10 / MP-11. Resume now compares both file and piped briefs
against the original launch. WaitEvents exit 3 is parsed so the subscription-end
batch and its reason are archived before `event_stream_incomplete`, without ack.
Both fixes have fail-first real-child regressions.

Deliver now expects `outside_steer_events_v1` to advertise the distinct-event
contract: `type: "steer"`, `value.steer_id` matching the returned
`ActivePromptSteered.prompt.id`, with the exact kernel/session/agent target.
It archives batches and waits for that event, retaining confirmation before ack.
Without that capability it reports `acknowledged-steer-status` after the accepted
response and one status read. Neither fallback nor fake-adapter coverage closes
EA2-16C. Upstream advertisement/schema agreement and a new full real hosted round
remain required; r7 starts no kernels or stacks and changes no kernel code.
