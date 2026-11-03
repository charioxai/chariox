# Chariox Apps implementation status

MP-08 / MP-09 / MP-10 / MP-11. Apps v2 is the current implementation.

## Execution and packaging

An App is a signed `.cxapp` archive with a contract-2 manifest, a WASM component,
verified UI assets and integrity metadata. `chariox-app-package` and the shared
package verifier own package validation. The kernel owns installation, publisher
trust, capability approval, generations, updates, private files and snapshots.

`packages/app-wasm-worker` is the production worker. It runs Wasmtime with the
versioned App WIT world, no ambient capabilities, fuel and epoch deadlines,
bounded linear memory, and Linux/macOS native containment entered before guest
instantiation. `packages/app-runtime` owns the worker transport, broker and
lifecycle. The worker ships beside the kernel and its exact bytes identify the
AOT cache. Release signatures cover that binary with the rest of the release.

The replaced C launcher, JavaScript bootstrap, dynamic engine library, runtime
inventory enrollment, root storage helper and their fixtures are retired.
The small libc transport fixture is test-only; it never executes package code.

JavaScript/TypeScript App authors compile through the component build tools.
Node remains a build/test tool for JavaScript assets and clients; deployed App
backends execute as WASM components. Workflow-code's separate JavaScript
compiler and browser-controller tooling also use Node independently of Apps.

## Views and examples

Room App Tabs expose only verified UI assets on installation-scoped origins.
The kernel binds bridge calls to installation/generation/document and checks
capabilities. Agent view calls and direct input preserve the agent caller.
Excalidraw and Univer are the Stage 1 examples. Their edit/save/reopen drills
must include the actual new Web UI at DPR1/DPR2, the built kernel/relay and
containment refusals. F9 inspection and file-save controls consume kernel state.

## Validation

Run focused App package/runtime/worker tests, the kernel App tests, protocol
shape/hash checks, client conformance, and the worker component/containment
scripts in `packages/app-wasm-worker/scripts/`. Live acceptance is separate:
record the exact source heads, binary hashes, screenshots, logs and cleanup
outside the repository. Do not reuse earlier acceptance as proof of a new head.

See `spikes/wasm/APPS_V2_PLAN.md`, `spikes/wasm/APPS_V2_SHAPE.md`,
`packages/app-wasm-worker/README.md` and `docs/PROTOCOL.md` for the contracts.
