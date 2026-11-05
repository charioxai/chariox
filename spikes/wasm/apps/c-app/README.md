# C spike graduated to the C SDK

The maintained C path is [packages/app-sdk-c](../../../../packages/app-sdk-c/README.md).
`build.sh <out.wasm>` delegates to its R1 template; `test-transform.sh` delegates
to the SDK's native ASan/UBSan suite, including the original 12 transform calls.
Set `CHARIOX_C_TOOLCHAIN` to a checksum-pinned installation as documented there.
The old hand-written task runtime and loose JSON scans were removed.

R0 proved the callback ABI, per-call state, waitable sets, stream/future reads,
and the requirement to detach waitables before cancelling or dropping them.
Those mechanics are now in the SDK. The transform scan is retained only as a
bounded regression fixture over text that the fixture itself builds, not as a
JSON API. All actual SDK/template input uses the strict JSON helpers.

The 2026-10-22 measurements recorded in APPS_V2_SHAPE.md used the old spike
world and worker, before R1. They are historical measurements, not results for
this template. Do not load the R1 template into the old spike harness's `wasm-c`
arm; use the Stage 1 R3/R4/R6 worker/install/broker path when integrated.
