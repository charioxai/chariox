# Relay load proof (MP-08 / MP-09 / MP-10 / MP-11)

A repeatable end-to-end harness for the relay under bulk transfer. It reproduces the incident where one
large `chariox files get` through the kernel gateway starved the kernel list renewal of the whole fleet,
and measures any build against the bar: a large transfer takes long, and nothing else is affected.

Test tool only: no product code. It never touches a live relay or kernel. Every process, home directory,
port and 100 MB file is created for the run and removed at its end.

## What runs

- a throwaway local relay (`chariox-relay`, scoped-token auth with a generated key);
- two real kernels in throwaway homes on loopback ports: the serving kernel (`worker`, next to the relay)
  and the client kernel (`home`), paired through a real pairing link;
- `throttle-proxy.mjs` between `home` and the relay: a token bucket per direction, a one-way delay with
  jitter and a bounded queue (a full queue stops reading, so the sender's TCP window closes like on a real
  bottleneck). Default profiles: `8m` = 8 Mbit/s down / 2 Mbit/s up, `1m` = 1 Mbit/s both ways; 60 ms RTT
  (30 ms each way) and 6 ms jitter;
- a random file on the serving kernel and `chariox files get --kernel <serving kernel>` (the real TypeScript
  CLI, `apps/cli/dist`, through the real kernel gateway);
- probes every 250 ms for the length of the transfer, from the harness through `home`'s gateway:
  - **small request**: `ListSessions` forwarded to the serving kernel (a new relay round trip);
  - **kernel list**: `ListGatewayKernels` (the renewal path of `chariox kernels`);
  - the serving kernel's own `RelayStatus` (is its relay link still up);
  - the relay process RSS (every 500 ms).

Variants (`--variant`, comma separated):

- `single`: one transfer;
- `three`: three concurrent transfers by one client (fairness);
- `stalled`: one transfer, and after 10 s the client kernel is frozen with `SIGSTOP` (exact PID) for 20 s while the
  serving kernel is probed on its own socket every second; then `SIGCONT`.

Packet timing (`--packet-timing 1`, a diagnostic: never in measured cells): the kernels and the relay log
`MP-10-PACKET-TIMING` lines for each request, joined by a digest of its ciphertext: `kernel_gateway_enqueue` and
`kernel_gateway_written` on the client kernel's gateway link, `relay_read` (a whole frame) or `relay_reassembled`
(a chunked upload) on the relay, `kernel_receive` and `kernel_write_start` on the serving kernel. Round 3 used them to
find where a small request waited during `files put`: on the gateway link, behind every upload request queued ahead of it.

Heartbeat gaps: the serving kernel logs `kernel cloud presence visible` for each presence publish; the harness
lists their timestamps (the relay does not log heartbeats). At a 35 s cadence the 8m runs show a few lines.

Profiles (`--profile`): `8m` (8 Mbit/s down, 2 up, 256 KiB bottleneck buffer), `1m` (1 Mbit/s both ways), `1m-bloat` (`1m` with a 1 MiB
buffer), `600k` (600 kbit/s, 1 MiB buffer: the incident link), `8m-bdp` and `1m-bdp` (small buffers, so bulk waits in the sender's
socket). The frozen-reader variant moves `--stalled-size-mb` (30) MiB and queues `--stall-burst` (3000) small requests for the frozen
client. Results of round 1: `RESULTS.md`.

## Run it

```sh
pnpm install --frozen-lockfile && pnpm build:kernel-client && (cd apps/cli && node scripts/build.mjs)
# kernel and relay debug builds, through the slot (never two kernel builds at once on a machine):
/root/.chariox/dev/browser-resume-20260930/agents/cargo-slot.sh bash -c 'cargo build -p chariox-relay -p chariox-kernel && cp $CARGO_TARGET_DIR/debug/chariox-relay $CARGO_TARGET_DIR/debug/chariox-kernel /var/tmp/rp/bin-x/'

NODE_OPTIONS=--max-old-space-size=3072 node scripts/relay-load-proof/run.mjs \
  --kernel-binary /var/tmp/rp/bin-x/chariox-kernel --relay-binary /var/tmp/rp/bin-x/chariox-relay \
  --label base --profile 8m --variant single,three,stalled --size-mb 100 --evidence-dir <dir outside the repo>
node scripts/relay-load-proof/run.mjs ... --label base --profile 1m --variant single --size-mb 12 ...
node scripts/relay-load-proof/report.mjs <dir> base      # markdown tables and PASS/FAIL
```

State goes under `/root/.chariox/dev/relayproof/runs` (`--state-root`, must stay under `/root/.chariox/dev`).
Each variant writes `<label>-<profile>-<variant>/result.json`, the relay, proxy and kernel logs and
`probes.json` (every probe sample) to the evidence directory. The `1m` profile uses 12 MiB so the run lasts
about two minutes; the ideal time is still size / rate.

Check: `node --check scripts/relay-load-proof/*.mjs` and `node --test scripts/relay-load-proof/throttle-proxy.test.mjs`
(the proxy holds 800 kbit/s to within 20 % and delays the first byte; a 4 Mbit/s downlink delivers about 500 KB/s
and a full queue pauses the sender).

## The bar

- every transfer completes with the right hash and uses at least 90 % of the downlink (bytes through the proxy / window / link rate; the file
  travels as base64 inside JSON, so payload reaches at most about 73 % of the link, and the table shows both);
- small-request p95 <= RTT + 100 ms while the bulk runs, and the kernel-list probe never fails or times out (15 s);
- no kernel or client disconnected: both kernels report `connected` after the run, the serving kernel's link
  stayed up all run;
- stalled reader: the serving kernel stays connected for the whole freeze, and the relay RSS stays within
  baseline + 16 MiB;
- three concurrent transfers: none is starved (slowest <= 1.5 x fastest) and the probes stay inside the bar.

Notes on what the model cannot show: the proxy's queue (256 KiB per direction) plays the bottleneck buffer, so the
small-request latency during bulk contains the time a small frame waits behind queued bulk bytes. That head-of-line
wait is the effect a TCP-level bound on unsent bytes is meant to remove.

## File bytes over wire bytes (relay8)

The bar "verified file bytes / bytes the relay put on the shaped link for them >= 90 %" is measured by every
`run.mjs` run (`fileShare` in `result.json`, a line in `report.mjs`) and repeated by `file-share.mjs`:

```sh
node scripts/relay-load-proof/file-share.mjs --kernel-binary K --relay-binary R --evidence-dir DIR --label base --repeat 3 --size-mb 30
```

It runs `files get` (8m profile) and `files put` (8m-sym) `--repeat` times each, one at a time, prints each share and the median
per direction, and writes `<label>-file-share.md` and `.json`. The share is a ratio of byte counts: a loaded host slows the run but
does not change it.
