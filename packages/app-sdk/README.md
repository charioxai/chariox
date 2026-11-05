# App transport conformance fixtures

MP-08 / MP-09 / MP-10 / MP-11. This private package supplies JavaScript peers
for tests of the native SDK wire protocol and the kernel capability brokers.
It is not shipped in an App runtime or an installed release. The fixed libc
worker fixtures also use that protocol to exercise kernel lifecycle tests.

New App backends use `packages/app-sdk-js` (`@chariox/app-sdk-wasm`) or
`packages/app-sdk-rust` and compile to WASM components with
`packages/app-sdk-tools`. Excalidraw and Univer are the current examples.
The contract-1 examples exercise this fixture protocol only; they cannot be
installed or launched by the current kernel.

Run `node --test --test-concurrency=1 packages/app-sdk/test/*.test.js` for wire
and broker-adapter regression checks. These tests do not establish native
containment, component admission or end-to-end App acceptance.
