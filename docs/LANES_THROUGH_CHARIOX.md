# Lanes through Chariox

MP-08 / MP-09 / MP-10 / MP-11. This runbook keeps the coordinator's policy outside
while replacing worker transport/control with the public CLI. Its adapters are
based on `dogfood/adapters` at `39a01d95f5` (the r8 adapter fixes).
The r8 runtime pin is `extagent/v2-events` at
`f2220d938749fcac2a92a9aa3ce32af1a8c0f6b5` (local591/peer197). No protocol allocation or kernel change is made
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
| deliver.sh | `coordinator/deliver.mjs`: read supplied review, compare-and-steer the active prompt, then confirm through a capability-gated §14 event or acknowledged-response/status fallback | Fetch review text on Mac; new instructions for a finished turn need a new round. The r8 runtime passes event confirmation; older kernels report the explicit status fallback. | A01 send_agent_message correlation / steer and A02 admission records exist; r7 prepares their outside steer-receipt consumer. No duplicate delivery core. |
| lanefin.sh | `coordinator/lanefin.mjs`: status + fetch notes/head + SHA-verified bundle/import | Publication, independent review and central acceptance merge stay on Mac. Final message must be written into fetched notes; no transcript scrape. | A02 explicit done/waiting/blocked; A03 process-exit wake for bundle commands. |
| lanehealth.sh | `coordinator/lanehealth.mjs`: state, last activity and deadline with LONG-ROUND/SILENT flags | Decide whether a silent worker is stuck; no claim that last activity proves useful progress. | A02 /goal completion assessor and no-progress check-ins; A03 watchers can improve evidence. |
| stop | Kernel SetAgentDeadline/GetAgentDeadline, then `coordinator/stop.mjs` | Exact owned-resource cleanup audit remains necessary; agent destruction alone is not a complete host cleanup receipt. | A04 sudo is permission duration, not a hard stop. EA2 durable deadline supplies the missing unconditional wall-clock bound. |
| Usage tracking | Product profile refresh/status for admission | Mac fleet usage accounting and quota policy by design | Account/profile source is separate from agent-model. |
| Host rental | Approved process supervision can report exit through a Room terminal | Mac cost decision/rental by design; no automated paid host creation | A03 process watcher can supervise an already approved rental command. |
| Acceptance merge | Fetch validates a present acceptance update; verified bundle HEAD is independent | Mac coordinator merges central acceptance by design | A02 ledger can carry readiness, not acceptance authority. |
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

**The r6 EA2-16C cell is FAIL.** The mid-round steer command succeeded and the worker's fetched
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

## R8 hosted event-path acceptance

MP-08 / MP-09 / MP-10. `external-agents-v2/EA2-16C` passes on the immutable
r8 runtime pin above: three real official Codex workers (`gpt-6-luna`, `low`),
22.346 seconds of concurrent work, one grant-owned subscription, 22 ordered
and gap-free events, exact accepted-steer event confirmation, two ordinary
completions before their deadlines, one active 30-second deadline, all three
verified bundles and explicit removal events. The independent receipt audit
and the inherited `assert-steer-event.py` both pass. The owner grant used the
real web popup at DPR2 with a disposable product Vault; video inspection found
a masked passphrase and ordinary approval/return to usable controls. This is
grant-flow evidence, not cold-frame or general Browser acceptance.

Evidence lives outside git under `dogfood/r8/attempt3/`, with the independent
`dogfood/r8/EA2_LIVE_AUDIT.json`. The exact kernel/native CLI/relay build and
hashes are in `BUILD_PROVENANCE.json`. Adapter launch/steer used `4610bd77bd`;
collection spans that commit and `a8448698c8`, which moves remote bundle files
beside external handoffs. This is an explicit source transition, not a claim
that every command ran from one final adapter checkout. The changed collection
path also has fail-first fake-adapter/real-Git regressions.

The first two r8 attempts remain FAIL. Workers successfully wrote their JSON
handoffs, but the real coordinator acceptance merger consumed them before
collection. Fetch now reports only typed `file_not_found` for that optional
receipt; required Markdown handoffs, other transfer errors and invalid rows
still fail. Bundle HEAD comes from the verified artifact, independently of
acceptance merge, with strict expected-head and base-ancestry checks. A worker
also chose to complete its real review instead of waiting artificially; the
passing attempt uses a short deadline during actual review work. No synthetic
delay or manufactured event establishes its result. No kernel defect was
attributed to these failures.

## Required real implementation-lane round

MP-08 / MP-09 / MP-10. `external-agents-v2/EA2-16L` requires one actual coordinator
lane through these adapters: launch the real brief using its requested provider,
observe its first commit through the same event inbox, deliver the supplied
coordinator note with distinct steer confirmation, detect finish or durable
hard stop, collect its Markdown handoffs and verified Git bundle/head, then stop
its worker. Control uses one WaitEvents subscription, with no SSH, recurring
state polling or Mac scripts. Worker commits remain local for coordinator
publication. This row establishes orchestration; the worker's own source/UI
acceptance remains a separate obligation.

## R8 real layering round

MP-08 / MP-09 / MP-10. `external-agents-v2/EA2-16L` passes its orchestration
scenario on the same immutable runtime. The real `layering` r1 brief ran with
the existing product-linked Claude profile, resolved model `claude-sonnet-5-5`,
effort `high`. One subscription recorded 163 ordered events without a gap or
subscription-end event. The worker announced its first commit through a Room
terminal exit; that exact worker event triggered delivery of the supplied
`NOTE-1.md`, confirmed by its exact accepted-steer event. The worker completed
normally before its durable deadline. Fetch collected its handoffs and bundle,
and stop archived the matching removal event before returning.

The preserved Cloud worktree is `/root/work/agent-layering`, branch
`webterm/layering`, base `4ac4da2341745f0263d4abbc3e2992ddfa9da24a`, head
`beb0bc73e25dd783823ac9281a68c5247cc7c64a` (eight local commits). Its verified
bundle SHA-256 is
`fb9574104efa78bca679933c0c7866a6a4f13a9f584f63c38bdc8f6a3e80ff94`.
Nothing was published. Evidence is under `dogfood/r8/collected/layering/` and
`dogfood/r8/LAYERING_LIVE_AUDIT.json`, outside git.

Worker source checks passed: root build/lint, 10 boundary tests, 104 domain
tests and 7,515 web tests. Independent replay of the two new lint regressions
fails against the exact base script and passes against the final script.
The API-availability copy check was omitted. Both 1440×900/DPR1 screenshot
pairs are identical, but the new shell uses FakeKernel and the classic view
shows the login page. These are supplementary regression checks; signed-in
runtime/UI acceptance for the Cloud change is not established by this row.

| Step | What this real round used | Mac work or remaining gap |
| --- | --- | --- |
| Bootstrap | Exact-head kernels/native CLI/relay, own hosted WSS stack, product Vault and owner popup; reused the lane's certified hostname | Bootstrap harness remains outside the six loop scripts. Repository and profile selection remain coordinator policy. |
| First commit | Brief convention: Room terminal exits 73 only after HEAD differs from base; its exact event triggers NOTE-1 | No native Git-commit event is claimed. The brief must emit this marker. |
| Claude permissions | Required permission mode, 77 event-triggered one-shot state reads and kernel-owned interaction responses | Driver supplies response policy; no interactive owner input or recurring state polling was needed. |
| Finish and stop | Required Markdown, independently verified bundle HEAD/ancestry, matching removal event archived before acknowledgement | Agent removal remains distinct from a complete host-resource cleanup receipt. Audit owned subprocesses and scratch after the round. |
| Acceptance merge | Real merger consumed an earlier review's JSON before fetch; typed optional missing receipt and independent bundle HEAD now work live | Central acceptance merge stays with the Mac coordinator by design. Transport metadata does not establish acceptance. |
| Usage and rental | Existing approved account/model; no rented host | Mac fleet accounting, reserves and paid-host approval stay there by design. |
| GitHub | Supplied frozen source heads and brief; no writes | Lookups, publication and reviewer scheduling stay on the Mac by design. |

The real lane used no SSH, recurring state polling or Mac control script.
Its own build/test scripts are worker work, not coordinator transport. Launch
and note delivery used adapters `a8448698c8`; the final stop used
`39a01d95f5`, which waits for the exact removal event. The runtime stayed fixed.
Adapter checks pass: 68 tests, 2,653 compiled TypeScript CLI tests and
process-signal lint. Four focused groups fail against their recorded pre-fix
revisions. The initial CLI test run raced a separate typecheck rebuilding
generated dependencies; its failed log and successful sequential rerun are
both retained. No kernel defect or protocol change was introduced here.

## R9 two real rounds on the lasting kernel

MP-08 / MP-09 / MP-10 / MP-11. Two real lane rounds ran at the same time through
`tools/lanes/coordinator/round.mjs` on the [lasting lanes kernel](LANES_KERNEL.md)
(`f2220d9387`, protocol 591/197), both with Claude through the product-created
`lanes-claude` profile and one `WaitEvents` subscription (1,230 ordered events,
no cursor gap):

| Lane | Model | Ran | Workers' heads (each bundle verified, base ancestry checked) |
| --- | --- | --- | --- |
| amint r19 | Opus 5.5 high | 17:21Z to 19:45Z | OSS `608f771fb9`, Cloud `ccde43c209` |
| dictation r2 | Sonnet 5.5 high | 17:21Z to 19:37Z | Cloud `a7f57943ec`, OSS `1d473d81b3` |

Both workers finished on their own, long before the hard stops. The driver
launched them with their briefs, answered 250 and 358 kernel-owned
`Approve Claude Code …?` decisions with `allow_once` (no failures), read state
once per event-triggered check, and stopped each worker. No ssh, polling or Mac
script was used. Hashes and paths are in each lane's `LOOP_RUN.json`.

| Step | What the loop did | Still needed from the Mac or broke |
| --- | --- | --- |
| Start | `lanes-kernel.mjs start`: kernels, Vault unlock from the state-dir file, relay, Claude profile, grant through the real owner popup. A stop/start cycle takes about 13 s. | The grant does not survive a kernel restart and the new grant has a new inbox, so the old `subscription.json` has to be removed once lanes are finished. |
| Permissions | Claude as root has no non-prompting mode, so the driver answers each decision (`respond` adapter, new). | About 600 decisions in a 2 h round. The response policy (allow everything) is coordinator policy, not kernel behaviour. |
| Liveness | Event `at_ms` is recorded per lane as `lastEventAtMs`. | `last_activity_at_ms` moves only at turn boundaries, so the old `SILENT` flag was a false positive on working lanes. |
| Notes | Delivery of new `REVIEW_INBOX.md` sections once each is implemented and tested with fake adapters. | No coordinator note arrived in these rounds, so live delivery was not exercised here (R8 proved it). |
| Finish and collect | Event-triggered status read detects the idle worker; the driver stops it. | **Broke:** both workers wrote `ACCEPTANCE_UPDATE.json` rows with free-form heads (`OSS 608f771fb9 / Cloud ccde43c209`) because each lane changed two repositories. `lanefin` correctly refuses a head that is not one full commit id, so both rounds ended `finished_collect_failed`. The bundles were then made by hand with the same `bundle` adapter and the full ids read from the worktrees (recorded under `manualCollect` in `LOOP_RUN.json`). A Mac script would have refused the same file. |
| Two repositories | `extraBundles` (new) bundles the second repository's tip. | The brief must tell a two-repository lane to put one full commit id in every row head, and which repository that id belongs to, or to leave the update out. The old driver snapshot did not run `extraBundles` after the failure, so the extra bundles were also manual. |
| Error clarity | `invalid_input` from `lanefin` said only "chariox refused the operation", which hid the cause for several minutes. The acceptance check now reports `invalid_acceptance` with its own remediation text (fail-first test). | |
| Cleanup | Workers stopped by the loop. | Agent removal is not a host-resource receipt; scratch and stacks that workers started were cleaned by the workers themselves. |

Observations about kernel behaviour (grant lost on restart, a restored session
that cannot answer the Vault-unlock decision, `last_activity_at_ms` semantics,
the permission volume) are in `agents/extagent/REVIEW_INBOX.md`. None has a failing
kernel test here and no kernel code was changed.

## R10 the standing round service (loopd) on OSS #1080

MP-08 / MP-09 / MP-10 / MP-11. The [lasting lanes kernel](LANES_KERNEL.md) now runs `e46dcc0ce6`
(OSS #1080) and keeps a standing round service, `loopd` (`round.mjs serve`), beneath the holder. The Mac drops
one queue entry per round with `mac/loopbrief.sh`; loopd does the rest and writes the same
`LOOP_RUN.json` / `last-message.md` files the Mac's `lanes.sh` and `watch.sh` read. `prompt_steer_event_unrecorded`
from a steer is treated as delivered (the kernel took the steer; resending would repeat it).

**The collect failure from R9 is fixed.** `lanefin` no longer refuses a round because of a free-form
`ACCEPTANCE_UPDATE.json`: `fetch` returns `acceptance: { valid, issues: [{ field, problem }] }`, the bundle is
guarded by the acceptance head only when the file is valid and names one commit, and otherwise the worktree HEAD is
bundled and `LOOP_RUN.json` carries `acceptanceIssues` (for example `rows[0].head: must be one full 40-character git
commit id`). Fail-first tests use the two real worker files from R9 (`tools/lanes/fixtures/r9-*`). A handoff file
a stopped worker never wrote is reported as `missing` and no longer costs the round its commits.

### Drill (workers on claude-haiku-5-5 and gpt-6-luna, scratch repositories; no product work)

| Check | Result |
| --- | --- |
| Two entries at different times | `loopdrilla-r4` and `loopdrillb-r4` were picked up 17 s apart through `loopbrief.sh` (pickup in about 2 s each). |
| One finishes, one stops at a short hard stop | A finished by itself (40 poems, commit `0eff181276`, head and bundle verified, `last-message.md` written). B hit its 7 min hard stop after 100 poems: stopped first, then collected, head `c8da7c1be6` equal to the worktree HEAD. |
| Lanes-kernel restart mid-way | `lanes-kernel.mjs stop` then `start` (16 s) while both ran; loopd restarted with the kernel, set the old inbox aside, followed both Room sessions again and re-read both lanes. Same session and agent ids before and after (`restart4-before.txt`), no second launch, both lanes continued and ended as above. |
| Codex entry | `loopdrillc-r2` ran `gpt-6-luna` low through the product-created `lanes-codex` profile (linked in place from the lanes' Codex credential home; never logged in or out): committed `a0c9744ad6`, wrote its handoff files, final message captured, head and bundle collected. Codex runs as root with `permission: yolo` (approval never, sandbox danger-full-access); there is no root refusal as there is for Claude. |

### Gaps found (all but the kernel items are fixed here, with fail-first tests)

| Gap | What happened | State |
| --- | --- | --- |
| A removed agent poisons the whole event stream | An agent-level target that outlives its agent makes the kernel report `events_refused` for the entire subscription, so every other lane went silent (one Codex lane finished unnoticed). | Worked around: unsubscribe before destroying a worker, prune stopped workers' targets at open and at loopd start, look again after an unsubscribe because the refusal clears a moment later; every lane is also read in its first minute and every 5 minutes. Reported to the kernel owner in `agents/extagent/REVIEW_INBOX.md`. |
| New grant after a restart has an empty inbox | The old targets are gone; loopd failed with `subscription_target_missing` on every wake. | Fixed: loopd follows each running lane's sessions again (`events subscribe --session`). |
| Decision raised during that gap | A permission raised while nothing followed the worker has no event; it times out (`default_on_timeout: deny` after 300 s) and the worker stops with a denial. | Fixed on this side: the lane status now lists `active_interactions` and the engine answers them on its next read. A typed replay from the kernel would be better (reported). |
| Hard stop lost the head | A stop that could not confirm removal aborted the collection. | Fixed: `stop_confirmation_timeout` is soft, the final message is saved before the stop, a hard stop stops first and collects regardless of handoff files. |
| Empty-wake archives | 1,183 empty batches were archived in one evening. | Fixed: empty wakes are not archived. |
| Naming | The Mac's `LOOP-DOWN` check looks for a running `round.mjs`. | loopd is `round.mjs serve`, so the existing check keeps working. |
| `notes` file | `loopbrief.sh` has no notes parameter; the optional entry field `notes` names the coordinator's REVIEW_INBOX file if it is not `agents/<lane>/REVIEW_INBOX.md`. | Documented in LANES_KERNEL.md. |

### The five #1079 review findings, fixed where the code ended up

Each has a fail-first test (revert the fix and the test is red).

| Finding | Fix |
| --- | --- |
| Failed notes were skipped | A section that fails to deliver stays pending (`noteFailures` in `LOOP_RUN.json`). The next wake sends the same content, which resumes the same steer receipt, also after a driver restart. A settled worker (`no_active_prompt`) or ten failures in a row end the attempts and record `delivered: false` with the reason. |
| Failed permission replies were marked handled | Only a reply with a `choice` counts. A failed one keeps its entry (`attempts`, `error`), the lane is read again at the next wake, and the decision (listed by the lane status) is answered with the same command identity. |
| Other lanes' completion wakes lost while a worker is stopping | Any non-decision event marks its lane for a status read (`needsRead`), whichever call consumed the batch (stop confirmation, note delivery), so the read happens in the same or next wake. |
| Completion wake in a lane's first minute dropped | Same `needsRead` mark; the read waits until the first minute is over instead of being forgotten. |
| `exec --detach` always exited 1 | The holder acknowledges a detached command once it has really started (`{pid, exit: 0}`); a failed spawn answers with its error and exit 127. Tests start a real holder in a temporary state directory (`kernel/lanes-kernel.test.mjs`). A running holder picks this up at its next restart. |

### What the next kernel changes for loopd

extagent r27 (`89c4e35ef6`, not pushed, protocol unchanged) keeps an `until-exit` grant across a kernel restart as long as its holder process lives, with the same inbox and `subscription.json`; a restored session answers its own decisions; `last_activity_at_ms` moves with each recorded entry. loopd needs no change for that: the new-grant path (set the stale inbox aside, follow the lanes again, answer decisions from status) stays for the case where the holder is gone, and `lanes-kernel.mjs start` should then stop re-requesting a grant that is still granted. The `events_refused` poisoning by a removed agent target and the replay of a decision raised during a restart are still open with the kernel owner.

### Real queue rounds: not exercised in this round

The coordinator dropped no entry into `agents/_loop/queue/` between 20:13Z and the end of the round (00:46Z), so
integ2 r35 and webterm2 r15 were not run by loopd. The drill above used the same path (`loopbrief.sh` over the
`b3.sh` helper, queue entry, loopd, kernel, real Claude and Codex workers), but with scratch repositories and cheap
models. What a real entry still has to show: a multi-hour lane, `REVIEW_INBOX.md` sections delivered live, a second
repository through `extraBundles`, and the Mac's `lanes.sh` / `watch.sh` reading `LOOP_RUN.json`. loopd stays running
for the first one.

## dogfood r11: loopd's restart P1 and two P2s, while it served the coordinator's rounds

The incremental review of `b1361dd3bf` had one P1 and two P2s in loopd. Each is fixed with a fail-first test (fake adapters); the live drills below ran through the installed loopd while it kept serving `amint-r21` and `webterm2-r15`.

| Finding | Fix | Live proof |
| --- | --- | --- |
| P1: a restart admitted a queued round before restoring the running one (`alpha-r10` sorts before `alpha-r9`) | `scan` reads every queue entry first and admits the ones whose `LOOP_RUN.json` shows a launched, unfinished round before any waiting entry; the one-round-per-lane guard then holds the rest. | `loopdrillr11-r9` running and `-r10` queued; loopd stopped and started: `picked_up loopdrillr11-r9` only, r10 `picked_up` 2 s after r9 `finished` (`p1-restart.txt`, `loopd-log-drill.txt`). |
| P2: the next round read the previous round's handoff files | When a round is first admitted (before its worker exists) `PUSH_READY.md`, `LANE_STATUS.md` and `ACCEPTANCE_UPDATE.json` of the lane are renamed `<name>.<previous round>.<ext>`. Admission is keyed on the round's spec file, so a resume moves nothing. The live file for the current round stays at `agents/<lane>/PUSH_READY.md`. | r9 left a valid `ACCEPTANCE_UPDATE.json` (head `45e280a2be`); r10 wrote none and was hard-stopped: `hard_stop`, head `d8183333d1` equals the worktree HEAD, no `acceptanceIssues`, no `collectError`, r9's files only under `*.r9.*`. |
| P2: events consumed while a stop waited for a removal that never came were lost | `confirmRemoval` puts the archives it consumed on the `stop_confirmation_timeout` error; the engine handles them before it softens the error. An empty wake is no longer listed as an archive (it is never written). | Unit regression only: alpha's removal never arrives, beta's pending permission and completion still reach the engine. |

Also in this round:

- **Preamble.** A queue entry may list `preamble` files (absolute paths, up to 8). loopd writes `rounds/<key>.brief.md` once at admission (brief, then each file, separated by a blank line) and launches with that, so the wrapper no longer pastes `common.md` itself. `loopbrief.sh` sends `PREAMBLE` (space-separated paths) or, if it is unset and `agents/common.md` exists, that file. A missing file ends the entry as `launch_failed` with `preamble_unavailable` naming it. Live: the drill rounds ran with a one-line preamble (`rounds/loopdrillr11-r9.brief.md`).
- **Permissions.** The lane status lists each decision's choice ids. The engine answers a routine `Approve Claude Code …` decision with `allow_session` when it is offered, else `allow_once`; events carry no choices, so the status is read first. Critical, passkey and credential decisions are not touched. The running kernel (`e46dcc0ce6`) offers no `allow_session` yet, so the live drill still recorded 80 `allow_once` answers.
- **Install and carry-over.** `loopd install` (HEAD `aa3a21a729`) and a plain `loopd stop/start` restarted loopd twice with `amint-r21` and `webterm2-r15` running: the log shows `stopped … stillRunning` naming both, then `picked_up` for both within a second; agent and session ids and `launchedAtMs` are unchanged in their `LOOP_RUN.json` files, and their permission counts kept rising (`install-before.txt`, `install-after.txt`).
