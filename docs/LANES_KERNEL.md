# The lasting lanes kernel

MP-08 / MP-09 / MP-10 / MP-11. A dev kernel on the b3 builder that outlives a lane round, so the
[ported coordinator loop](LANES_THROUGH_CHARIOX.md) can run real lane rounds every day without
rebuilding a rig. Everything lives under `/root/.chariox/dev/lanes-kernel/` (outside git, owner-only).

## What runs

| Part | Role |
| --- | --- |
| **executor kernel** | Runs the lane workers (Room agents). Own Vault, own product-created Claude profile `lanes-claude`. |
| **coordinator kernel** | The kernel the loop's CLI talks to (the Mac's local kernel in production). It holds the access grant and routes `--kernel <executor>` over the hosted relay. |
| **holder** | A long-lived process the `until-exit` access grant binds to. Loop commands run beneath it with `exec`. |
| **hosted stack** | `pr1080-dogfoodr6` (own Cloud web + own relay, WSS). Kept running (`KEEP` file is empty). |

Both kernels are the exact OSS #1080 build `e46dcc0ce6` (upgraded from `f2220d9387` between rounds; the previous
binaries stay in `bin.prev/`); hashes and worktree are in `PROVENANCE.json`. The Rust binaries were built through `agents/cargo-slot.sh` and
copied out of the slot into `bin/`.

## State layout

```
lanes-kernel/
  config.json            non-secret: runtime worktree, stack, ports, toolchain, token file path
  PROVENANCE.json        build head, worktree and sha256 of the three binaries
  bin/                   chariox-kernel, chariox (native CLI), chariox-relay
  executor/, coordinator/
    chariox/             CHARIOX_HOME (vault.json, config.toml, identities) — never copy it
    passphrase           disposable Vault passphrase, mode 600, never printed or logged
    kernel.pid  ids.json  logs/  home/  tmp/
  holder.pid  holder.sock  holder.log
  loop/                  CHARIOX_COORDINATOR_HOME: config.json, receipts, events/ journal, round.json
  run/                   frozen copy of tools/lanes used by a live round or by loopd (see "Rounds", "loopd")
  loopd.pid  loopd.log   the standing round service
```

## Commands

```sh
export TMPDIR=/root/.chariox/dev/lanes-kernel/tmp
node tools/lanes/kernel/lanes-kernel.mjs status   # JSON: pids, relay, vault, holder, stack readiness
node tools/lanes/kernel/lanes-kernel.mjs start    # idempotent: kernels, Vault unlock, relay, Claude profile, grant
node tools/lanes/kernel/lanes-kernel.mjs stop     # holder and both kernels, by exact verified pid
node tools/lanes/kernel/lanes-kernel.mjs grant    # only the access grant (headless owner popup)
node tools/lanes/kernel/lanes-kernel.mjs loopd install|start|stop   # the round service (see "loopd")
node tools/lanes/kernel/lanes-kernel.mjs exec [--detach --log FILE] -- CMD...   # run beneath the holder
```

**After a restart** (`stop` then `start`, about 15 s): the Vault relocks, and `start` unlocks it from the
passphrase file by answering the kernel's own `vault-unlock` decision in a fresh owner session. Kernel
and machine ids persist and the relay reconnects by itself. The `until-exit` grant does **not** survive
a kernel restart, so `start` ends by requesting a new one and approving it in the real web popup
(headless Chromium at DPR 2, typing the coordinator passphrase). A new grant has a new inbox. loopd notices
the change by itself, sets the old `subscription.json` aside, follows every running lane's Room and
operations session again and re-reads each lane (see "After a lanes-kernel restart" below).

**Healthy** means `status` shows both kernels with `relay_connected: true` and `vault.unlocked: true`,
a `holder` pid, `stack_ready: true`, and `chariox access status --holder-pid <holder>` (through `exec`)
prints `{"status":"granted"}`. The Claude profile is verified by the official CLI when `start` runs.

**Stop** with `stop`. The hosted stack is separate: `/w/valenv/pr-stack stop pr1080-dogfoodr6`
(then remove its `KEEP` file). If the stack image is gone, rebuild the tag
`chariox-valenv/pr1080-dogfoodr6:<cloud commit>` with the same `buildx` command `pr-stack` uses,
copy `bin/chariox-relay` into `/w/valenv/prs/pr1080-dogfoodr6/bin/` and run `pr-stack start`.

## Claude through the lanes profile

`start` creates the product profile `lanes-claude` (once) and stores the existing lanes setup token
through `SetProviderAccountCredential`, read from its file inside the process and never printed. The
profile never logs in or out of the shared account. Claude as root cannot run `yolo`, so workers use
`permission: required` and the round driver answers each `Approve Claude Code …?` decision
(`allow_once`) through the public CLI.

## Rounds

`tools/lanes/coordinator/round.mjs` drives several lanes with one subscription: launch with the
brief, answer permission decisions, deliver new `REVIEW_INBOX.md` sections once, detect finish from an
event-triggered status read, collect handoffs plus verified bundles (including a second repository),
stop at the hard stop, and write `agents/<lane>/LOOP_RUN.json`. Rerunning it with the same spec
resumes from the recorded receipts. Run it beneath the holder from a frozen copy of the adapters so
later edits cannot change a live round:

```sh
git archive HEAD tools/lanes | tar -x -C /root/.chariox/dev/lanes-kernel/run
node tools/lanes/kernel/lanes-kernel.mjs exec --detach --log loop/round.log -- \
  /usr/bin/node /root/.chariox/dev/lanes-kernel/run/tools/lanes/coordinator/round.mjs loop/round.json
```

## loopd: the standing round service

`start` also starts loopd (`lanes-kernel.mjs loopd start|stop|install` do it alone; `stop` stops it). It is
the Mac's round loop for any number of lanes, kept running: it picks up queue entries as the coordinator
drops them, runs each lane through the same engine and one shared inbox as `round.mjs`, and writes the
files the Mac's `lanes.sh` / `watch.sh` read. It runs beneath the holder from the frozen copy in `run/`;
`loopd install` refreshes the copy from the checkout's `HEAD` (only between rounds: it restarts loopd, which
resumes from receipts). Process: `node run/tools/lanes/coordinator/round.mjs serve` (`LOOP-DOWN` checks look for that name).

**Queue.** `agents/_loop/queue/<lane>-<round>.json` (the file name must match the fields):

```json
{ "lane": "integ2", "round": "r35", "provider": "claude", "model": "claude-sonnet-5-5", "effort": "high",
  "hardStopMinutes": 240, "brief": "/abs/agents/integ2/BRIEF-r35.md",
  "worktrees": ["/abs/primary", { "path": "/abs/second", "base": "<40-hex commit>" }],
  "preamble": ["/abs/agents/common.md"], "notes": "/abs/REVIEW_INBOX.md" }
```

`provider` is `claude` or `codex`; `effort` is `low|medium|high|xhigh|max`; `worktrees` lists 1..4 repositories
(the first is where the worker starts; every other one is bundled too); `notes` is optional and defaults to
`agents/<lane>/REVIEW_INBOX.md`. `preamble` is an optional list (up to 8) of absolute files that loopd appends to the
brief, in order, so the worker gets the shared rules without the wrapper pasting them: the composed brief is written
once, at first admission, and a resumed round keeps it even if a shared file changes. A missing file ends the entry
as `launch_failed` (`preamble_unavailable`, naming the file). The coordinator side is `tools/lanes/coordinator/mac/loopbrief.sh <lane> <worktree>
<claude|codex> < brief.md` (env `MODEL`, `EFFORT`, `HARD_STOP_MIN`, `ROUND`, `EXTRA_WORKTREES`, `PREAMBLE`; the preamble defaults to `agents/common.md` when it exists): it copies the
brief to `agents/<lane>/BRIEF-<round>.md` over the `b3.sh` helper, writes the entry atomically and prints the
queue file and `LOOP_RUN.json` path once loopd has picked it up. An invalid entry moves to `queue/rejected/`
with `<name>.reason.txt` naming the exact field. One round per lane runs at a time; a second entry for the
same lane waits in the queue; after a loopd restart the lane's unfinished round is resumed first, whatever the file
order, and the waiting entry starts only when it has finished. A launch that fails 5 times (30 s apart) ends as `finishReason: launch_failed`.

**Per lane** loopd writes `agents/<lane>/LOOP_RUN.json` (`launchedAtMs`, `events`, `permissions`, `notes`, `head`,
`bundle`, `bundleSha256`, `files`, `finishReason`, `finishedAtMs`, `driverError`, `lastMessage`), and
`last-message.md` (the worker's final message, from the session history). A finished entry moves to
`queue/done/` and its run file is also kept as `LOOP_RUN.<round>.json`; a file left by another round is set
aside as `LOOP_RUN.<round>.json` before the new round starts. The previous round's handoff files
(`PUSH_READY.md`, `LANE_STATUS.md`, `ACCEPTANCE_UPDATE.json`) are set aside the same way (`PUSH_READY.<round>.md`, ...)
when the next round is first admitted, so a round that writes none is not collected with the old ones; the live
file for the current round stays at `agents/<lane>/PUSH_READY.md`, and a resume does not move it. `finishReason` is `finished`, `hard_stop`
or one of those with `_collect_failed` (then `collectError` names the code; the stop still happened).
Handoff files the worker did not write are listed as `missing` and do not stop the collection of its commits.

**What it does each wake** (15 s): waits on the one inbox; answers Claude permission decisions
(`allow_once`, because Claude as root cannot run `yolo`; Codex runs `yolo`); delivers each new `## ` section of
`REVIEW_INBOX.md` once (a steer whose event the kernel could not record counts as delivered, never resent);
reads a lane's status on its events, in its first minute, and every 5 minutes anyway (events can be missed);
collects a lane that went idle (handoffs, verified bundle, extra bundles, final message, then the stop);
at the hard stop it stops the worker first and collects its commits regardless of its handoff files.

**Receipts, no double launch.** Every lane keeps `loop/<lane>-<round>.json` plus launch, delivery and finish
receipts, and `loop/rounds/<key>.spec.json` fixes the base commits at the first sighting. Restarting loopd (or
the whole lanes kernel) reuses them: the same Room session and agent are followed, nothing is launched again.

**After a lanes-kernel restart** the Vault relocks, the access grant is new and its inbox is empty. loopd
sets the old subscription aside, follows each running lane's Room and operations sessions again, answers any
decision that was raised while nothing followed the worker (it reads `active_interactions` in the lane's
status) and re-reads every lane. A worker's provider process can outlive the kernel restart; a permission
decision that nobody answered within its 300 s default is denied by the kernel and the worker reports it,
so restart between rounds when you can.

**Codex.** `start` links the lanes' existing Codex credential home into the kernel as a product profile
(`LinkProviderAccountProfile`, in place, no copy, no login or logout; the home is `codexHome` in `config.json`)
and loopd runs `provider: codex` entries with `permission: yolo` (Codex has no root refusal). The model
and effort come from the entry (`gpt-6.1-sol` by default in `loopbrief.sh`).

**Tests:** `node --test tools/lanes/coordinator/loopd.test.mjs tools/lanes/coordinator/mac/loopbrief.test.mjs` (fake adapters,
a real `round.mjs serve` process, a stand-in `b3.sh`).
