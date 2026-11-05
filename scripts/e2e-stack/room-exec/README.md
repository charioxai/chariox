# Binary Room exec drill (MP-08 / MP-09 / MP-10 / MP-11)

Build the matching kernel, relay, native `chariox-cli`, kernel-client and CLI.
The kernel must advertise `room_exec_v1` (local 995 / peer 434). Run with a
memory bound and a finite timeout, using a new state directory outside the repo:

```sh
node scripts/e2e-stack/room-exec/live.mjs \
  --kernel-bin /absolute/path/chariox-kernel \
  --relay-bin /absolute/path/chariox-relay \
  --cli-bin /absolute/path/chariox-cli \
  --home /absolute/path/to/new/disposable-state \
  --evidence /absolute/path/to/evidence
```

The drill creates two real kernels and a private scoped relay, pairs their
normal gateway, and uses a disposable Vault passphrase to approve its own
process-bound external grant. It never uses an existing kernel, login or
provider account. It runs the native CLI for the actual command streams.
The exact scenarios are:

- (a) 200 MiB random binary stdin through `cat`, streamed to a local file and
  checked against the original SHA-256;
- (b) output delivered before a sleeping command exits;
- (c) repeated stdin/response exchanges while both pipes remain open;
- (d) separate stderr, exit 23 and signal 15 propagated as exit 143;
- (e) a real incremental Git bundle streamed from the target kernel, verified
  by Git and SHA-256, then streamed in a tar archive to the target and checked;
- (f) SIGINT during an unbounded output stream, returning exit 130;
- (g) absence of a live grant refuses both terminal open and exec;
- (h) a paused output reader backpressures a 64 MiB duplex `cat`, with kernel
  RSS samples during a 30-second pause (including a final five-second plateau
  check), then completes with every byte.

Whole-kernel grants have no separate terminal-rights bit. Case (g) tests the
same actual grant boundary for both interfaces. Router regressions additionally
refuse invited Room members and cross-Room exec IDs. Managed-work regressions
check busy/idle reporting and the existing automatic-stop admission fence.

`results.json` records individual results. `cleanup.json` records the exact
owned PIDs stopped and state directory removed. Kernel/relay logs remain only
in the evidence directory. The ordinary/managed global parity matrix, hosted
network and provider acceptance are separate; this drill changes no provider or
Cloud shutdown policy.

The MP-08 / MP-09 / MP-10 / MP-11 recovery drill uses the same arguments:

```sh
node scripts/e2e-stack/room-exec/recovery.mjs \
  --kernel-bin /absolute/path/chariox-kernel \
  --relay-bin /absolute/path/chariox-relay \
  --cli-bin /absolute/path/chariox-cli \
  --home /absolute/path/to/new/disposable-state \
  --evidence /absolute/path/to/evidence
```

It SIGKILLs a native exec client, lists its orphan through the native request
command, closes it and verifies that its process was reaped. It also checks
SIGHUP cleanup, an already-exited producer whose canceled pipe data is
reported incomplete, the actual CLI failure after an origin-kernel crash,
and the real five-minute reader lease for both blocked stdin
and more than 1 MiB of unread output. Polling `exec_list` must not keep either
orphan alive. The kernel managed-work/stop-reservation tests cover busy-to-idle
reporting after expiry; this local relay drill alone does not prove Cloud's
managed shutdown delivery.

For a focused before/after run, add `--only sigkill-list-reclaim`,
`--only sighup-cancels`, `--only cancel-truncated-output` or
`--only real-kernel-connection-loss`.

The MP-08 / MP-09 / MP-10 / MP-11 slow-consumer regression uses
`slow-consumer.mjs` with the same binary, home and evidence arguments. It pipes
the real CLI into a downstream Python process that reads one byte, stalls for
330 seconds (past the production 300-second reader lease), then drains the
pipe. It requires every byte of 8 MiB stdout and 1 MiB stderr to match their
SHA-256 hashes and the CLI to return the producer's exit 23. The CLI renews
the lease every minute while local writes are pending, acknowledging only
bytes already consumed locally. No lease is shortened for this drill.

The MP-08 / MP-09 / MP-10 / MP-11 pre-spawn regression uses `pre-spawn.mjs`
with the same arguments. It seeds the ordinary learned-peak cache with an
impossible prediction before the real kernel starts. The old kernel executes
the marker immediately; the corrected kernel holds the original argv before
spawn. The native CLI supplies binary input while queued. Cancel leaves no
marker; Start now executes once and preserves every stdin/stdout byte, stderr
and exit23; revoking the original grant prevents queued execution. This is a
focused pre-spawn regression. It also checks a second kernel selected through
native `--kernel` before submission, peer cancellation, original grant revocation
and target gateway revocation. It does not move a queued launch to another kernel.
Post-hold placement, paid-VM placement, hosted Web DPR1/2 and durable remote claim
recovery remain separate acceptance work.

The MP-08 / MP-09 / MP-10 / MP-11 `reservation-peaks.mjs` drill takes the same
arguments. Run it on Linux with permission to create a private cgroup below
`/sys/fs/cgroup`. It bounds the kernels' actual delegated workload parent to
6 GiB and removes that cgroup after stopping its processes. The native CLI
starts a 1536 MiB binary with a 2048 MiB prediction, then requires a second
2048 MiB launch to start while the first remains resident. A separate binary
learns a real 640 MiB peak. After kernel restart, that same argv/cwd must queue
under a 4608 MiB reservation and start automatically once the reservation ends.
`--only-reservations` and `--only-peaks` isolate the fail-first cases. These are
focused binary-admission checks, not the full managed/ordinary parity matrix.

The MP-08 / MP-09 / MP-10 / MP-11 `target-restart.mjs` contract drill uses the
same arguments. It starts a binary on the selected peer, observes its marker,
SIGKILLs that owned target kernel and restarts it with the same state. Reading the
original exec ID must report an explicit unknown/incomplete outcome and must not
replay the program. This drill is known-red on the base and the r2 foundation:
both lose the receipt and return `exec_not_found`. It is separate from the passing
origin-connection-loss drill and is not a claim of durable recovery.

The MP-08 / MP-09 / MP-10 / MP-11 `reservation-domains.mjs` drill takes
those same binary/home/evidence arguments. It requires a private cgroup and
uses a 10 GiB budget. A binary with a learned 4 GiB peak starts a separately
learned 4 GiB descendant after the launch window; a second 4 GiB binary must
start while both remain alive. Add `--disk` for the real filesystem case:
the fixture creates and mounts its own sparse 32 GiB ext4 image, starts a
12 GiB predicted writer, waits for 10 GiB of actual output and fsync, then
requires an 8 GiB predicted binary to start before the writer exits. Mount,
image, private cgroups and fixture processes are removed after each run.
These are focused native accounting regressions; they do not claim the hosted
or ordinary-versus-managed acceptance matrix.
