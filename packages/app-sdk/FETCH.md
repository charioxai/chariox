# Fetch transport fixtures

MP-08 / MP-09 / MP-10 / MP-11. This package tests the JSON broker protocol with
Node-based fixture clients. The adapter preserves request/response streaming,
cancellation, limits and error classes for conformance tests; it is not a
production App execution environment.

The portable App Fetch API is in `packages/app-sdk-js` and
`packages/app-sdk-rust`. Its component imports are serviced by the WASM worker
and the kernel capability broker. Use their tests and real component drills for
runtime acceptance. `scripts/test-app-fetch-clients.mjs` is supplementary
transport compatibility testing only.
