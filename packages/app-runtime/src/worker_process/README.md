# App worker process ownership

MP-08 / MP-09 / MP-10 / MP-11. `PreparedWorker::prepare_wasm_update` is the
production preparation path. `wasm::WasmRuntime` verifies the installed worker
beside the kernel, compiles verified components into the identity-keyed AOT
cache, and retains runtime/release/data leases through process reap and broker
drain. The worker enters native containment before its readiness handshake.

`spawn`, `monitor` and `private_data` own blocking process lifetime, bounded
logs, resource checks and private file operations. Cancellation is nonblocking;
releasing admission waits for physical exit and broker settlement. macOS adds
native resource monitoring; Linux worker containment denies process creation.

`record` carries the WASM identity handshake. The fixed libc fixture codec is
compiled only for tests or the guarded `test-fixtures` feature. Its C sources
live in `fixture/`; it exercises transport/lifecycle without executing App code.
