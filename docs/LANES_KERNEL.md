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

Both kernels are the exact `extagent/v2-events` build `f2220d9387` (protocol 591/197); hashes and
worktree are in `PROVENANCE.json`. The Rust binaries were built through `agents/cargo-slot.sh` and
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
  run/                   frozen copy of tools/lanes used by a live round (see "Rounds")
```

## Commands

```sh
export TMPDIR=/root/.chariox/dev/lanes-kernel/tmp
node tools/lanes/kernel/lanes-kernel.mjs status   # JSON: pids, relay, vault, holder, stack readiness
node tools/lanes/kernel/lanes-kernel.mjs start    # idempotent: kernels, Vault unlock, relay, Claude profile, grant
node tools/lanes/kernel/lanes-kernel.mjs stop     # holder and both kernels, by exact verified pid
node tools/lanes/kernel/lanes-kernel.mjs grant    # only the access grant (headless owner popup)
node tools/lanes/kernel/lanes-kernel.mjs exec [--detach --log FILE] -- CMD...   # run beneath the holder
```

**After a restart** (`stop` then `start`, about 15 s): the Vault relocks, and `start` unlocks it from the
passphrase file by answering the kernel's own `vault-unlock` decision in a fresh owner session. Kernel
and machine ids persist and the relay reconnects by itself. The `until-exit` grant does **not** survive
a kernel restart, so `start` ends by requesting a new one and approving it in the real web popup
(headless Chromium at DPR 2, typing the coordinator passphrase). A new grant has a new inbox: if a
loop home still holds the old `subscription.json`, finish or stop its lanes first, then remove that file.

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
