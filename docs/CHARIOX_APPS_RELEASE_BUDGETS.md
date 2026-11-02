# App release resource budgets

MP-08 / MP-09 / MP-10 / MP-11. Apps v2 resource enforcement lives in
`packages/app-wasm-worker/src/limits.rs`, `scope.rs`, `worker.rs` and
`containment.rs`, plus the supervisor monitor in `packages/app-runtime`.

The component's linear memory is bounded to 128 MiB. Native address space is
bounded to 1 GiB, file size to 8 MiB and descriptors to 64. Calls use fuel and
epoch deadlines; cancellation retains a bounded cleanup grace. Host operations
remain capability-brokered and separately admitted. Native limits and a passing
component test do not establish aggregate-pressure acceptance.

Measure representative Excalidraw/Univer edit/save workloads and concurrent
App/agent/browser use on the real kernel, relay and clients before publishing
release performance claims. Report source/binary identities, admission refusals,
private data preservation, cancellation, process cleanup and resource samples.
