# Live workflow-gate matrices (G-04, G-05, G-07, G-11)

Each `gNN.mjs` starts its own kernel (own `CHARIOX_HOME` under `R`), talks raw local IPC through
`packages/kernel-client/dist`, and prints PASS/FAIL per case. Evidence is appended as JSON lines.

- `matrix-gate/`: import-free gate component; the manual prompt (`run`, `drop`, `defer:N`, `forever`, `late`, `past`, `trap`, `spin`, `badnode`, `batch:S:key`) selects its decision. Build with `build-gate.sh` (through the build-slot wrapper) and the `chariox-gate-component` encoder.
- `shim/clockshim.c`: test-only `LD_PRELOAD` that adds `FAKE_CLOCK_OFFSET_S` to `CLOCK_REALTIME`, used to move a restarted kernel past the 7-day entry deadline.
- Needs a debug `chariox-kernel` plus a release `chariox-app-wasm-worker` mode 0555 beside it, `pnpm build:kernel-client`, and `WFGATE_SCRATCH` (kernel, worker, gate and run homes; default `/root/.chariox/dev/wfgate-r1`) and `WFGATE_EVIDENCE` (JSON lines). The kernel-client is taken from this checkout.
