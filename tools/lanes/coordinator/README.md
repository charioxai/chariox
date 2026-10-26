# Coordinator loop (MP-08 / MP-09 / MP-10 / MP-11)

Set `CHARIOX_BIN` to the real native CLI and `CHARIOX_COORDINATOR_HOME` to a
private directory outside git. Obtain an until-exit access grant for the
long-lived coordinator process through the owner web popup. All commands must
remain descendants of that process. The selected local kernel routes to the
executor over its normal hosted WSS peer connection.

Create `$CHARIOX_COORDINATOR_HOME/config.json` with this non-secret policy:

```json
{
  "kernel": "executor-kernel-id",
  "providers": {
    "codex": {
      "profile": "product-linked-profile-id",
      "model": "gpt-6-luna",
      "effort": "low",
      "permission": "yolo"
    }
  },
  "deadlineSeconds": 7200,
  "repository": "/absolute/local/git/repository",
  "laneDirectories": { "review-a": "/absolute/lane-state/review-a" },
  "bases": { "review-a": "FULL_BASE_COMMIT" }
}
```

`laneProfiles` (optional) overrides a provider profile's model, effort, permission or `deadlineSeconds` for one lane.
`laneDirectories` keeps uploaded briefs, remote bundles and fetched handoffs in the lane's
state directory outside its checkout. Omit a lane to retain the worktree-based
layout. The launch receipt retains that location across resume; bundle commands
always run in the worktree. Create the lane directory before launch and tell
the worker to write its handoffs there.

MP-08 / MP-10: acceptance merge may consume `ACCEPTANCE_UPDATE.json` before
collection. Fetch explicitly reports that file as missing on `file_not_found`;
both Markdown handoffs remain required. Other transfer errors and invalid
acceptance rows still refuse collection. Bundle tips and base ancestry are
verified independently, so a consumed update cannot hide or manufacture a head.

| Replacement command | Inputs and behavior |
| --- | --- |
| `node tools/lanes/coordinator/brief.mjs LANE WORKTREE codex [BRIEF_FILE]` | Reads the brief file, or stdin like `brief.sh`; uploads it and launches through `lane-launch`. Reusing identical inputs resumes the original launch and deadline. Changed brief text, including stdin, raises `launch_conflict`. |
| `node tools/lanes/coordinator/watch.mjs [MINUTES]` | One blocking `WaitEvents` on the grant's merged inbox (default 15, max 240 minutes; longer waits re-arm WaitEvents only). Archives the whole batch before AckEvents. Timeout is a normal wake. |
| `node tools/lanes/coordinator/deliver.mjs LANE REVIEW_FILE` | Reads the review text already obtained on the Mac and steers the current prompt with a compare fence. Confirms via a matching §14 event when advertised, otherwise via the acknowledged response and one status read. Identical review/current-prompt pairs reuse their receipt. New instructions for a settled worker are refused. |
| `node tools/lanes/coordinator/lanefin.mjs LANE...` | Checks status once, fetches the worker's handoffs, then verifies/imports its bundle. The bundle supplies the actual head; `bases[LANE]` supplies its base. A present acceptance update must identify one matching head. Review notes and the final message belong in `LANE_STATUS.md`/`PUSH_READY.md`. |
| `node tools/lanes/coordinator/lanehealth.mjs [LANE]` | One status/deadline snapshot per selected lane (all recorded lanes when omitted), with LONG-ROUND and SILENT flags; no recurring poller or automatic destruction. |
| `node tools/lanes/coordinator/stop.mjs LANE` | Unconditionally stops the recorded worker, then archives/acknowledges its exact removal event before returning. The coordinator timer requires its process, network and grant to remain available. |

The launch adapter always returns a required coordinator timer; run its foreground
job as documented in the parent README. It is not durable across orchestrator
failure. Never treat the health script as a deadline replacement.

The original Mac scripts remain read-only. GitHub review lookup, usage accounting,
host rental and central acceptance merge remain there. These replacements take
lane/worktree/brief/review inputs; they do not run SSH or query GitHub. Keep one
inbox consumer at a time: finish the watch command before running deliver, then
resume watching. Commands create private receipts, transferred files
and an `events/` journal under the coordinator home; retain them as evidence.
The r8 runtime `f2220d9387` advertises and produces the accepted-steer event.
Older runtime `0d31a2ee3` uses the status fallback and cannot establish the
distinct-event acceptance row. A worker acknowledgement in fetched notes is
supplementary.

MP-08 / MP-09 / MP-10 / MP-11: the r7 adapter contract expects the executing
kernel to advertise `outside_steer_events_v1` in `RelayStatus.status.capabilities`.
The capability means it emits `type: "steer"` with
`value.steer_id` equal to `ActivePromptSteered.prompt.id`. The event's kernel,
session and agent must also match the lane. This is the returned provider steer
id, distinct from the adapter's retry command id. The r8 frozen runtime advertises it; older kernels continue to use the fallback.

Deliver reports `confirmation.mode: "event"` after a matching event, or
`"acknowledged-steer-status"` after the older path's status read. The fallback
does not establish distinct-event acceptance. Event confirmation blocks on
WaitEvents for up to 120 seconds, archiving every batch before acknowledgement;
the returned `confirmation.archives` includes batches for other lanes too, which
the coordinator must process. A gap/end is archived and refused without ack;
timeout raises `steer_confirmation_timeout`. Resume the same review to confirm
its retained accepted steer, including after its original turn settles. A
confirmed receipt is saved before ack, so an interrupted acknowledgement cannot
cause a retry to wait for an already consumed event. A pending event confirmation
never silently changes to the fallback if its capability disappears.

A changed grant, event gap, missing followed target or ended subscription refuses
the wake without acknowledging lost data. Restore authority or reconcile the gap
explicitly. File-only HOST_REQUEST/OWNER_SESSION changes are not live events yet;
workers must include an ask in their reply as well.

Run `pnpm test:lanes` and `pnpm lint:process-signals`. Fake-adapter tests supplement
real providers, clients, kernels and hosted relay drills; they do not establish
live acceptance.

MP-08 / MP-09 / MP-10: coordinator stop retains the physical stop receipt before
waiting up to 120 seconds for the exact kernel/session/agent removal event. Its
`removal.archives` also includes intervening bundle-terminal events. Confirmation
is saved before acknowledgement and reused on retry. Gap/end/timeout refuses
confirmation without repeating destruction; a stopped receipt is not proof that
the event was observed. Keep the holder alive until this command returns.

Flat org (dogfood r13): the coordinator gets worker events straight from the lanes kernel with `lanewait.mjs`
(beneath the `coordinator` holder seat, its own inbox), steers a worker with `steer.mjs` (`mac/deliver.sh`), and answers
decisions loopd leaves pending with `answer.mjs` (`mac/laneanswer.sh`). `decisions.mjs` is the single routine/other
split. See docs/LANES_KERNEL.md "The coordinator's seat".

MP-08 / MP-09 / MP-10 / MP-11: lane questions use the existing `request_popup`
runtime tool. It creates an agent-owned `RuntimeInteraction`; a Room owner sees
the normal card, and an admitted outside follower sees its `interaction` event
with `value.state:"pending"` and `refs.interaction_id`. `lanewait` reads that
pending card, prints its message and labelled fixed choices, and retains their
ids in its JSON line. Loopd retains the same details in `LOOP_RUN.json.unanswered`
and leaves choice questions pending for the coordinator.

Answer with `node tools/lanes/coordinator/answer.mjs LANE INTERACTION_ID CHOICE_ID`
(or `mac/laneanswer.sh` with those arguments). It checks that the choice remains
offered, then uses `RespondToInteraction`. The provider tool resumes with the
choice's reply. This requires the normal grant and the target gateway capability;
protected owner interactions retain their existing target checks. No lane-specific
product request or event is needed. Use fixed choices with meaningful labels and
exact replies; use `COORDINATION_REQUEST.md` as a fallback when the runtime tool
is unavailable or a question times out. Do not put credentials in either channel.

Source anchors: `runtime/state/tool_dispatch/credential.rs` (popup creation and
awaited resolution), `runtime/state/runtime_interaction_owned_state.rs` (reply),
`docs/PROTOCOL.md` (runtime interactions and gateway target authorization), and
`runtime/kernel_access` (outside interaction events and grants).
