# Binary Room exec drill (MP-08 / MP-09 / MP-10 / MP-11)

Build the matching kernel, relay, native `chariox-cli`, kernel-client and CLI.
The kernel must advertise `room_exec_v1` (local 632 / peer 235). Run with a
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
