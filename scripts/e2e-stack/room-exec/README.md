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
  RSS samples during the pause, then completes with every byte.

Whole-kernel grants have no separate terminal-rights bit. Case (g) tests the
same actual grant boundary for both interfaces. Router regressions additionally
refuse invited Room members and cross-Room exec IDs. Managed-work regressions
check busy/idle reporting and the existing automatic-stop admission fence.

`results.json` records individual results. `cleanup.json` records the exact
owned PIDs stopped and state directory removed. Kernel/relay logs remain only
in the evidence directory. The ordinary/managed global parity matrix, hosted
network and provider acceptance are separate; this drill changes no provider or
Cloud shutdown policy.
