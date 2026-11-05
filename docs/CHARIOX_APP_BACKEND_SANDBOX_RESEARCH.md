# App backend containment

MP-08 / MP-09 / MP-10 / MP-11. Apps use WASM components hosted by Wasmtime in
`packages/app-wasm-worker`. The worker admits only the versioned App WIT world.
All host operations are capability-brokered; no ambient filesystem, network,
process or environment access is available to a guest.

Linux enters seccomp and Landlock before starting runtime threads or executing
a guest. macOS applies default-deny Seatbelt and checks the installed hardened
worker's signing entitlements. The worker uses fuel, epoch deadlines, component
memory limits and native resource ceilings. Failure to enter containment exits
before a ready record, and the kernel refuses activation.

The old engine comparison and native launcher experiment are retired. Current
contracts and drills live in `spikes/wasm/APPS_V2_PLAN.md`,
`packages/app-wasm-worker/src/containment/` and
`packages/app-wasm-worker/tests/containment.rs`. Component tests supplement the
real install/open/edit/save and containment-refusal acceptance drills.
