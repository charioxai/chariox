# C App on the async world (appshape r1, 2026-10-22)

Answers Miguel's question "can we have C from the get go?" with a measured
test. `app.c` implements the same tools as `apps/rust-async` against
`wit/app.wit` (async imports, a streamed body, the `cancel` future, the
WASI 0.3 timer) using only `wit-bindgen c` (callback ABI) and wasi-sdk 33.
No C SDK layer exists yet: the file shows what one would wrap.

Build (0.2 s): `build.sh <out.wasm>` with `wit-bindgen` 0.63.0 and
`wasm32-wasip2-clang` (wasi-sdk 33, `-mexec-model=reactor`) on PATH. Run the
harness with variant `wasm-c` (added to the async list in
`harness/src/process.rs`).

Measured on b3 against the r2 async worker (evidence
`appshape/r1/c-*.json`): every cell passed. Cold start 1.7 ms (7–8 ms under
bubblewrap), 13 MiB RSS, 302 KB component (222 KB precompiled in 16 ms),
tool call p50 0.03 ms; 16 and 64 concurrent callers on one instance with no
lost update; 50 MiB stream at 231 MiB/s with 4 chunks read ahead; cancel
released the pending import and stream in 12.5 ms (as Rust); malloc returns
NULL at the memory cap so the App handles its own out-of-memory and the 8
other calls survive (like QuickJS/Python, unlike Rust); the runaway loop is
stopped at 1.0 s with the culprit named.

Toolchain findings: `wit-bindgen c` supports the async callback ABI fully
(subtasks, waitable sets, stream/future reads, `context.set/get`); sync
builtins (`cancel-read`, `drop`) trap on a waitable still joined to a set,
so a C SDK must remove it first. wasi-libc still imports WASI 0.2 (linked by
the worker as today). No JSON library, no `fetch`, no async/await: a C SDK
is a state-machine helper plus the broker calls.

Memory-safety regression: `bash test-transform.sh` with `wit-bindgen` 0.63 and
native `clang` with ASan/UBSan on PATH. It calls the actual `transform` entry
point with empty, single-record, filtered and repeated inputs. Broker imports
are test stubs; this checks C allocation bounds, not the Wasm runtime or sandbox.
Nonzero allocator fill makes an unterminated scan fail deterministically.
