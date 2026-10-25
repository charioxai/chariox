# The lasting lanes kernel

MP-08 / MP-09 / MP-10 / MP-11. A dev kernel on the b3 builder that outlives a lane round, so the
[ported coordinator loop](LANES_THROUGH_CHARIOX.md) can run real lane rounds every day without
rebuilding a rig. Everything lives under `/root/.chariox/dev/lanes-kernel/` (outside git, owner-only).

## What runs

| Part | Role |
| --- | --- |
| **executor kernel** | Runs the lane workers (Room agents). Own Vault, own product-created Claude profile `lanes-claude`. |
| **coordinator kernel** | The kernel the loop's CLI talks to (the Mac's local kernel in production). It holds the access grant and routes `--kernel <executor>` over the hosted relay. |
| **holder** | A long-lived process the `until-exit` access grant binds to. Loop commands run beneath it with `exec`. There are two seats, each with its own grant and so its own event inbox: `loopd` (the round service) and `coordinator` (the coordinator's own `lanewait`, `steer` and `answer`, run with `exec --seat coordinator`). |
| **hosted stack** | `pr1080-dogfoodr6` (own Cloud web + own relay, WSS). Kept running (`KEEP` file is empty). |

Both kernels are the exact OSS #1080 build `af52049173` (upgraded from `b53e3f4704` on 2026-10-25 with six coordinator rounds
running; the previous binaries stay in `bin.prev/`); hashes and worktree are in `PROVENANCE.json`. The Rust binaries were built through `agents/cargo-slot.sh` and
copied out of the slot into `bin/`.

## Upgrading the runtime

A kernel restart ends the `until-exit` grant of the old holder, kills the workers' Claude processes and any decision nobody
answers within 300 s is denied. From `b53e3f4704` the restored kernel starts a new `claude --resume` for every running worker
(same Room session, same agent, same prompt), so rounds can be carried across **one** restart; the in-flight tool call of each
worker is lost (its process is orphaned, the worker reruns the command). **A worker can be restored only once**: a second
restart strands it (agent `working`, no provider process, steers refused with `no_active_provider_run`; found in r12,
reported to extagent). Restart the lanes kernel only for an upgrade, and treat a worker that has been through one restart as
due to finish before the next one. The binaries can be built at any time, because the running kernels use copies in `bin/`.

1. In `/root/work/agent-dogfood/lanes-kernel` (detached checkout of the new head): `touch` the sources, then
   `agents/cargo-slot.sh cargo build -p chariox-kernel --bin chariox-kernel --bin chariox-cli -p chariox-relay` (dev profile, `CARGO_BUILD_JOBS=4`,
   `TMPDIR` under the state dir). Copy `chariox-kernel`, `chariox-cli` (installed as `chariox`) and `chariox-relay` out of the slot's `target/debug`
   into `bin.next/`, check with `strings` that they embed that worktree, and write their sha256 to `PROVENANCE.json`.
2. Prove it on a **drill pair** first: `LANES_KERNEL_STATE=/root/.chariox/dev/lanes-kernel-drill<N>` with its own `config.json` (copy the
   real one, change `ports` and set `agents` to a directory of its own: the holder passes it to loopd as `LOOPD_AGENTS`, so the drill loopd
   reads its own queue and never the coordinator's), a copy of the current `bin/`, `loopd install`, `start`. Run a long drill round, swap the
   drill's `bin` for the new binaries with `stop`, `mv`, `start` in the middle of a turn, and check that the same session and agent keep committing.
3. Real kernels: `lanes-kernel.mjs stop`, then `mv bin bin.prev && mv bin.next bin`, then `lanes-kernel.mjs start` (about 25 s). loopd stops with
   `stillRunning` naming every lane, and the new loopd logs `picked_up` for each one within seconds.
4. Check `status`, `chariox access status` through `exec`, that every lane in `stopped.stillRunning` is `picked_up` again with the same
   session and agent ids and `launchedAtMs` in its `LOOP_RUN.json`, and that new events and permission answers keep arriving.

From `c76466e0be` the kernel offers Claude `allow_session` on a routine permission when it can name the exact command (single-line Bash,
an exact file path); the loop answers it by itself and records what was offered with each answer (`permissions[].offered`). The rule holds
for that Claude run only and matches only the same command again, so it removes repeats, not the stream of new commands.

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
  holder.pid  holder.sock  holder.log                          the loopd seat
  coordinator-holder.pid  coordinator-holder.sock  coordinator-holder.log   the coordinator seat
  watch/                 lanewait state: state.json, events/ archive, emitted.jsonl (LANEWAIT_HOME of the coordinator seat)
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
node tools/lanes/kernel/lanes-kernel.mjs grant [coordinator]   # only an access grant (headless owner popup), default the loopd seat
node tools/lanes/kernel/lanes-kernel.mjs loopd install|start|stop   # the round service (see "loopd")
node tools/lanes/kernel/lanes-kernel.mjs set-claude-credential [--from FILE]   # re-store the lanes-claude credential in place (see "Switching the Claude account")
node tools/lanes/kernel/lanes-kernel.mjs exec [--seat coordinator] [--detach --log FILE] -- CMD...   # run beneath a holder
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
(`allow_session` when offered, else `allow_once`) through the public CLI, except critical, credential-related and question decisions, which stay pending for the coordinator (see "The coordinator's seat").

## Switching the Claude account

`set-claude-credential [--from FILE]` re-stores the running executor's `lanes-claude` credential in place
(`SetProviderAccountCredential` with overwrite, read inside the process, never printed; default file is
`config.claudeToken`). No kernel restart, no worker stopped. It prints the profile, the auth state and the
organization id suffix of the stored account, nothing else. The kernel verifies a new credential with a real
single Claude turn before it replaces the stored one, so a revoked token (`expired or revoked`) or an account
over its limit (`HTTP=429`, `verification was inconclusive`) is refused and the old credential stays.

`tools/lanes/coordinator/mac/switch-claude-account.sh chariox|old` is the whole move: it probes the target
token on b3 (HTTP 200 and limit status `allowed` or `allowed_warning`; only the org suffix is printed) and
refuses if it is not usable yet, then runs `set-claude-credential`, then copies the token to
`/root/.claude-lanes/oauth-token` so classic launches use it. The kernel step comes first: if it refuses,
nothing has changed. Run it with `B3=local` on b3 itself or through the Mac's `b3.sh` helper.

**When a running worker moves.** A Claude process gets `CLAUDE_CODE_OAUTH_TOKEN` in its environment when its
provider run is launched, and the kernel keeps that value for the run's life (an in-place restart of the
process for a config change reuses it too: `claude_runtime.rs` `restart_claude_runtime` passes
`state.provider_credential_env`). A process cannot change its own environment, so **a worker already
running keeps the account it started with until its provider run is launched again**: the next loopd round
(a new Room and agent), or a restore after a kernel restart. Every new launch after the command reads the new
credential. Nothing is stopped, but no running worker moves by itself.

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

## The coordinator's seat: events and steers without loopd

The coordinator (a Claude or Codex session on the Mac, reaching b3 over one multiplexed SSH connection) works beneath its
own holder, so its inbox is independent of loopd's. The Mac scripts in `tools/lanes/coordinator/mac/` wrap these:

```sh
lanewait.sh [MINUTES] [--json] | --replay N        # blocks; one line per event: HH:MM:SSZ lane round kind summary; exit 0 lines printed, 3 none
deliver.sh <lane> <file|->                        # steers the running worker (or queues the next prompt), prints the kernel's answer, then appends to REVIEW_INBOX.md
laneanswer.sh <lane> <decision-id> <choice>       # answers a decision lanewait listed
```

`lanewait` kinds: `turn_end`, `finished` (head and bundle state), `error`, `cancelled`, `decision` (with its reason and the
choices), `silent`, `long_round`, `attention` (events lost, grant ended). A routine permission is never listed: loopd answers it
(`allow_session` when offered, else `allow_once`). The decision reasons are `critical`, `owner_only`, `credential_related`,
`question` (including `AskUserQuestion`), `not_a_claude_permission`, `no_allow_choice` and `unclassified`.

After a kernel restart the coordinator grant is new too: `start` requests both grants (loopd's first, one popup at a time), and
the next `lanewait` sets the old state aside and follows every running lane again. Workers get `/usr/sbin` on PATH and the shared
`scout` agent (copied into the lanes Claude profile at `start`).

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
order, and the waiting entry starts only when it has finished. A *new* launch that fails 5 times (30 s apart) ends as `finishReason: launch_failed`. A round whose launch receipt shows a submitted prompt (a worker that may be running) is never abandoned: after the fifth failure loopd sets `driverError` in its `LOOP_RUN.json` (the Mac shows LOOP-DOWN with the reason), keeps its queue entry and lane, and keeps replaying every 30 s until the kernel answers; the next round of the lane waits. The kernel's armed deadline still stops the worker. loopd also starts while the kernel is down (`inbox_unavailable` in its log) and serves once it answers.

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
(`allow_session` when the kernel offers it for that call, i.e. from `c76466e0be`; else `allow_once`; Claude as root cannot run `yolo`, Codex does; critical, passkey and credential decisions stay with the owner); delivers each new `## ` section of
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

## Planned Claude YOLO restart (dogfood r15)

MP-08 / MP-09 / MP-10 / MP-11. When the **running executor kernel** has exactly
`IS_SANDBOX=1`, new loopd Claude workers use `permission: yolo`, which the kernel
launches as `--permission-mode bypassPermissions --allow-dangerously-skip-permissions`.
Without that environment, Claude keeps `required`; Codex and OpenCode behavior and
loopd's routine-decision handling are unchanged. The root behavior was audited in
`agents/rootaudit/AUDIT.md` sections 1 and 4 (outside git). This is the owner's approved
lanes setting, not a change to the product's default permissions.

After the account switch, the coordinator runs once on b3:

```sh
B3=local bash tools/lanes/coordinator/mac/lanes-kernel-restart.sh
```

The Mac wrapper also supports the usual `B3`, `LANES_STATE` and `ADAPTERS` overrides.
It calls `restart-yolo`, which verifies the af52049173 carry-over runtime, the installed
binary hash and `claude-acct.sh probe` before stopping anything. It drains loopd's
current step, saves every running round's session/agent/launch time in
`restart-rounds.json`, writes the non-secret launcher setting
`kernel-environment.json` (`{"IS_SANDBOX":"1"}`), and stops/starts the pair once.
It requires a fresh `picked_up` for every round, unchanged identities, no driver error
and a readable worker provider/permission, then prints each worker's provider,
permission and Claude account name/organization suffix. Any failed check exits nonzero;
do not repeat a failed restart blindly. The operation has a ten-minute outer timeout.

**Existing rounds keep their launch permission**, including after restore. They pick up
YOLO on their next round, which creates a new agent; no active work is relaunched just
to change permission. The planned kernel restart still interrupts an in-flight tool
call and resumes the existing Claude conversation, as with other af52049173 restarts.

`switch-claude-account.sh chariox --probe` is a read-only usability check. A failed
classic-file copy, rename or switch-marker update after kernel success returns nonzero
and explicitly reports that the kernel credential has already changed.
