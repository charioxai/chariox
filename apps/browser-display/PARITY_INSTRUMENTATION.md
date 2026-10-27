# MP-08 / MP-10 / MP-11 paired latency instrumentation

`parity-cadence.mjs` reproduces the r15 click/type schedule: uniform 1200–1700 ms
gaps, seeded by repeat/site/action. Both arms use the same seed and retain every
gap. Fixed mode reproduces the earlier 1400 ms clicks and 1000 ms keys.

The timing mux runs behind a lane-owned TLS front on the quiet measurement host:

```sh
node apps/browser-display/parity-timing-mux.mjs LISTEN_PORT RELAY_PORT BASELINE_PORT /absolute/external/evidence/mux.jsonl
```

All listeners/upstreams bind loopback. `/api/websockets` routes to the comparison
baseline; other upgrades route to the ordinary relay. HTTP routes to the baseline.
The TLS front owns admission. This harness adds no runtime authority.
Receipts contain timestamps, connection numbers, direction, byte count and
browser/native class. URLs, headers and payloads are forwarded without logging.
Run `node --test apps/browser-display/parity-instrumentation.test.mjs` for the
supplementary metadata/cadence checks.

Join receipts to viewer input/presentation timestamps and allowlisted kernel
and controller timing receipts on the same host clock. Chunk matching is a
timing estimate: pipelined credits can share or overlap chunks, and serialization
can precede controller input completion. Do not add marginal P95s or report a
negative stage difference as a physical hop duration. Keep ambiguous matches
visible, and use the controller stage receipts to assess dispatch costs.

For CPU validity, retain host CPU and per-unit cgroup usage at 100 ms intervals.
Report both host CPU and background CPU (host minus the measured arm's source,
viewer, TLS front, mux and relay). A background P95 over 40% invalidates a run;
the measured arm's own load counts toward its cost. Keep pipeline cores separate
from this validity calculation. Strict parity requires click/type P95 no worse
than the baseline both pooled and per repeat, scroll FPS at least the baseline,
and pipeline cores at most 1.0 and at most the baseline. Missing repeats are
incomplete evidence. Source tests and lab measurements do not establish live
acceptance.
