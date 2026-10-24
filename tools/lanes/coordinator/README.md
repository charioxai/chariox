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
      "permission": "full"
    }
  },
  "deadlineSeconds": 7200,
  "repository": "/absolute/local/git/repository",
  "bases": { "review-a": "FULL_BASE_COMMIT" }
}
```

| Replacement command | Inputs and behavior |
| --- | --- |
| `node tools/lanes/coordinator/brief.mjs LANE WORKTREE codex [BRIEF_FILE]` | Reads the brief file, or stdin like `brief.sh`; uploads it and launches through `lane-launch`. Reusing inputs resumes the original launch and deadline. |
| `node tools/lanes/coordinator/watch.mjs [MINUTES]` | One blocking `WaitEvents` on the grant's merged inbox (default/max 15 minutes). Archives the whole batch before AckEvents. Timeout is a normal wake. |
| `node tools/lanes/coordinator/deliver.mjs LANE REVIEW_FILE` | Reads the review text already obtained on the Mac and steers the current prompt with a compare fence. Identical review/current-prompt pairs reuse their command id. A settled worker is refused. |
| `node tools/lanes/coordinator/lanefin.mjs LANE` | Checks status once, fetches the worker's notes/update, then verifies/imports its bundle. The update must identify one full head; `bases[LANE]` supplies its base. Review notes and the final message belong in `LANE_STATUS.md`/`PUSH_READY.md`. |
| `node tools/lanes/coordinator/lanehealth.mjs LANE` | One status/deadline snapshot, with LONG-ROUND and SILENT flags; no recurring poller or automatic destruction. |
| `node tools/lanes/coordinator/stop.mjs LANE` | Explicit unconditional adapter stop of the recorded worker. The kernel's durable deadline operates even while the coordinator is disconnected. |

The launch adapter reports a coordinator timer when an older kernel lacks
`agent_deadline_v1`; run its returned foreground job as documented in the parent
README. That mode does not establish a durable deadline. This round requires the
kernel capability. Never treat the health script as a deadline replacement.

The original Mac scripts remain read-only. GitHub review lookup, usage accounting,
host rental and central acceptance merge remain there. These replacements take
lane/worktree/brief/review inputs; they do not run SSH or query GitHub. Keep one
watcher per coordinator inbox. Commands create private receipts, transferred files
and an `events/` journal under the coordinator home; retain them as evidence.
A changed grant, event gap, missing followed target or ended subscription refuses
the wake without acknowledging lost data. Restore authority or reconcile the gap
explicitly. File-only HOST_REQUEST/OWNER_SESSION changes are not live events yet;
workers must include an ask in their reply as well.

Run `pnpm test:lanes` and `pnpm lint:process-signals`. Fake-adapter tests supplement
real providers, clients, kernels and hosted relay drills; they do not establish
live acceptance.
