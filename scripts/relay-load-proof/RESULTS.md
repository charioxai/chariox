# Relay load proof: results of round 1 (2026-11-09)

Builds are debug builds. Base = OSS `e9a16037ca` (relay + kernel). After-fix = a throwaway merge of the two fix
branches, `relayfix/r` `642b8615ee` and `relayfix/k` `fa52d2ca24` (merge `61a2f28f44`, no conflicts).
RTT 60 ms (jitter 6 ms). Several instances of the harness ran at the same time on the build host (each has its own
relay, kernels and shaped link; the host load average was 6 to 18), so the numbers are for comparison, not for
absolute timing. Full tables with p50/p95/max, log lines and RSS: the evidence directory
(`/root/.codex/evidence/relayproof/r1/report-base.md`, `report-merge2.md`, one directory per variant).

## Baseline and after-fix, bar = the one in README.md

| variant | link | base | after-fix |
|---|---|---|---|
| single, 100 MiB | 8m | FAIL: 259 s (ideal 105 s), downlink use 73 %, small request p50/p95/max 239/286/523 ms, kernel list 244/292/1052 ms (idle 72 ms); no failures, no disconnects | FAIL: 254 s, 74 %, small 236/283/325 ms, list 236/285/430 ms |
| 3 concurrent, 100 MiB each | 8m | FAIL: 570 s each (fair, ratio 1.0), link 100 % used, small 496/877/1131 ms, list 498/880/1131 ms; relay RSS 13.8 -> 21.6 MiB | FAIL: 570 s each, 99 %, small 498/888/1207 ms; RSS 13.6 -> 21.4 MiB |
| frozen reader, 30 MiB, client kernel SIGSTOP 20 s + 3000 queued small requests | 8m | PASS: serving kernel connected all 20 s (slowest local answer 45 ms), relay RSS 13.5 -> 17.9 MiB, no relay log line | PASS: same (8 ms), RSS 13.6 -> 18.3 MiB |
| single, 12 MiB | 1m | FAIL: 193 s (ideal 101 s), downlink use 94 %, small 1914/2083/2466 ms, list 1917/2096/2479 ms | FAIL: 192 s, 94 %, small 1931/2079/2123 ms |
| single, 6 MiB, 16 KiB bottleneck buffer | 1m-bdp | FAIL: small 1828/2069/2114 ms | FAIL: 1813/2066/2087 ms |
| single, 30 MiB, 64 KiB buffer | 8m-bdp | FAIL: 75 s, 75 %, small 254/293/404 ms | FAIL: 75 s, 75 %, small 218/287/311 ms |
| 3 concurrent, 6 MiB each, deep buffer (incident link) | 600k | FAIL: 487 s, 651 of 1948 small requests ran out of the probe's 15 s, p50 8.7 s, p95 12.4 s; list probe 5.0 s throughout, no failure | FAIL: 481 s, 654 of 1923 small requests timed out, p50 8.7 s; list 5.0 s |
| single, 6 MiB, 16 KiB chunks (experiment) | 8m-bdp | latency PASS (small 71/77/82 ms), link use 34 % FAIL (34 s, ideal 6 s) | same: 70/78/83 ms, 34 % |

The experiment row patched the CLI chunk size to 16 KiB in a throwaway `dist` copy; nothing of it is in the harness.

## What it shows

1. The two fix branches change none of these numbers. Small-request latency under bulk equals about half the
   transmit time of one bulk reply on the link (0.5 x 0.46 s at 8 Mbit/s = 237 ms; 0.5 x 3.7 s at 1 Mbit/s = 1.9 s;
   three in flight at 600 kbit/s = 8.7 s median, past the probe's 15 s for a third of the requests).
2. The cause is the size of one indivisible message, not the queue discipline: with 16 KiB chunks the small request
   is at 71 ms (idle 70 ms) on both builds. A priority order or a byte budget can only reorder messages that have
   not started to leave; a 256 KiB chunk (about 460 KB on the wire) that has started blocks everything behind it.
   Where: `KERNEL_FILE_MAX_CHUNK_BYTES` (`apps/kernel/src/local/api/types/kernel_files.rs:7`), `CHUNK_BYTES`
   (`apps/cli/src/kernel-files.ts:9`), the relay writes a whole message per frame (`apps/relay/src/server/connection/writer.rs`).
   Meeting both "small p95 <= RTT + 100 ms" and "link >= 90 %" needs either small chunks with several in flight (the
   download loop `apps/cli/src/kernel-files.ts:96-101` keeps one request in flight), or a relay and kernel that
   interleave fragments of a large message with small ones.
3. Link use of a single transfer at 8 Mbit/s is 73 %: one chunk in flight, so the link idles for an RTT plus kernel CPU
   (debug builds) between chunks. Three concurrent transfers fill the link (99 to 100 %) and share it fairly.
4. The downlink carries 1.79 x the file size (a bit more than base64 of the payload twice). Likely cause: a kernel's
   answer to another kernel (the gateway) travels as `DaemonIncomingPeerResponse` with a base64 ciphertext inside a JSON
   text frame (`apps/relay/src/protocol.rs:303`); the binary form (`apps/relay/src/binary_response.rs`) is only defined
   for `DaemonResponse` and `ClientResponse`. Not verified further.
5. Not reproduced end to end, on base or after-fix: the `kernel list did not refresh in time` error (the kernel-list probe
   answered in 5.0 s at 600 kbit/s for 8 minutes, longer than the 5-minute usability of a list, and never failed),
   the relay dropping the serving kernel for a frozen client (the gateway lets a client hold too few requests in flight
   to fill the relay's 1024-message queue; only the in-process RED test reaches it), and relay memory growth (+8 MiB at most
   with three transfers). The frozen-reader variant's CLI transfer ended with `kernel_connection_lost` on both builds when the
   client kernel was thawed, and none of the 3000 queued small requests was answered within 30 s after the thaw on either build.
6. Not run: release builds, a hosted relay path (TLS), a transfer with a longer-lived frozen reader.

# Round 2 (2026-11-09): relay window + chunking and the transfer window, combined

Debug builds, RTT 60 ms unless stated, several harness instances at once on a shared host (load average 8 to 12).
Head = relaychunk `91022ec905` + the filepipe commits + a fix of the transfer window (burst of replies opened it to 64
requests, past the 32 a kernel admits per connection) + a 30 s wait for an upload chunk that lands behind a later one.
`--direction put` runs `files put` (profile `8m-sym` = 8 Mbit/s both ways). Wire = bytes through the shaped link / link rate;
payload = file bytes / link rate: the wire carries 1.79 x the payload (base64 of the chunk, then base64 of the sealed
message), so 99 % of the wire is 55 % of the payload.

| run | result |
|---|---|
| get 100 MiB, 8m | 191 s (ideal 105), wire 99 %, payload 55 %; small 137/147/296 ms (bar 160); list p95 148 ms |
| put 100 MiB, 8m-sym | 189 s, wire 99 %, payload 55 %; small 403/457/476 ms: FAIL (the small request waits behind the upload window) |
| get 3 x 8 MiB, 1m | 376 s (ideal 201), wire 97 %; small 181/217/249 ms: FAIL (bar 160); list p95 223 ms, none timed out |
| put 3 x 8 MiB, 1m | 366 s, hash ok; small p50 10.6 s, 502 of 1500 ran out of 15 s; list probe 5.0 s: FAIL |
| get 3 x 6 MiB, 600k | 483 s, wire 94 %; small 320/378/463 ms: FAIL (was p50 8.7 s, 654 of 1923 timed out in round 1) |
| put 3 x 4 MiB, 600k | all three `kernel_request_timeout` (30 s): FAIL |
| get / put 50 MiB, 8m, RTT 150 | get 97 s small 318/331 ms (bar 250); put 95 s small 521/573 ms |
| frozen reader, 30 MiB, 8m | serving kernel connected for the whole 20 s freeze (slowest answer 9 ms), relay RSS 13.9 -> 16.2 MiB: PASS |
