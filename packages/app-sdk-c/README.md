# C App SDK

A C11 callback/state-machine SDK for `chariox:app/app@1.0.0`, generated from
[`app-wit`](../app-wit/README.md). It adds no host imports or protocol shapes.
It replaces the C spike's task runtime and preserves its native transform
regression. Code is MIT, under the repository licence.

## Build the backend template

Requires Python 3.12+, a Linux x86_64 or macOS arm64/x86_64 build host, and no
system-installed WASI tools. Choose a tool directory **outside the checkout**:

```sh
python3 packages/app-sdk-c/scripts/toolchain.py "$CHARIOX_HOME/c-toolchain"
export CHARIOX_C_TOOLCHAIN="$CHARIOX_HOME/c-toolchain"
bash packages/app-sdk-c/build.sh "$CHARIOX_HOME/template.wasm"
# For another App source using this SDK:
bash packages/app-sdk-c/build.sh "$CHARIOX_HOME/my-app.wasm" /path/to/app.c
cargo run -p chariox-app-wit --example check-component -- "$CHARIOX_HOME/template.wasm"
```

Set `CHARIOX_HOME` to your own state directory first. `TMPDIR` selects the build
scratch directory; generated C bindings/objects are removed on exit. The output
is a portable component, not a native/AOT executable. The build never downloads
implicitly, uses no tool from PATH, and does not replace an output on a failed
compile. Install only into an empty directory; remove a failed installation
before retrying. `toolchain.lock.json` pins the official release assets:

- wit-bindgen **0.63.0**, callback ABI;
- wasi-sdk **33.0**, C11, Preview 2 reactor model.

The template uses integer JSON formatting helpers instead of libc stdio.
With wasi-sdk 33, pulling in libc stdio can add WASI 0.2.6 imports (including
stdin), which R1 correctly refuses. Use the brokered log interface for logging
and the JSON helpers for output; any additional libc imports must pass R1's
unchanged explicit allowlist. Async App imports, exports, futures and streams
come directly from R1. Dependencies retain their upstream licences:
[wit-bindgen](https://github.com/bytecodealliance/wit-bindgen/tree/v0.63.0),
[wasi-sdk](https://github.com/WebAssembly/wasi-sdk/tree/wasi-sdk-33).
Download/extract/build scripts use only Python's standard library.

`template/app.c` registers `echo`, a transactional `counter`, and cancellable
`wait`. It supplies all required handler exports through the SDK. It is a
backend onboarding template; view/declarations/signing and the real local
`.cxapp` install flow require R4 and the development tooling. Register the same
tool names in the package declarations. Counter state uses schema version 1,
key `count`, and returns conflicts to the caller rather than replaying writes.

## Tasks and ownership

Include `chariox/task.h` and define the `cx_application` table. The SDK owns
all four R1 handler entry points and callbacks. `register_app` runs once before
readiness; `resume` runs initially and when an awaited import or stream is ready.
For bounded CPU work, save progress in task state and return `CX_YIELD` without
a pending import; the next callback resumes it and can deliver cancellation.
`resume` returns only `CX_WAIT`, `CX_YIELD` or `CX_DONE`; `CX_READY` is an internal
await result. Invalid poll transitions return `SDK_STATE` and drain pending work
before freeing anything. A ready stream is retained while an import finishes. Every call has its own zeroed state, parsed input, context, cancellation future,
waitable set and allocations. Shared application state must be updated without
yielding or through version-checked state transactions.

```c
if (cx_task_step(task) == 0) {
  CX_AWAIT(task, chariox_app_state_get(cx_string("key"), &state->got), 1);
}
if (state->got.is_err) return cx_task_error(task, &state->got.val.err);
/* Use the result, then return bounded JSON stored in task state. */
```

`CX_AWAIT` advances the step before evaluating the import. It handles both an
immediate return and a pending subtask; at most one import may be awaited at a
time. All arguments, results, transaction arrays and buffers that cross an await
must live in task state or `cx_task_alloc`, **never in a stack local**. Borrowed
arguments must remain valid until the import terminates. `cx_string` borrows a
NUL-terminated C string; do not pass it to generated `_free` helpers.

`cx_task_alloc` returns zeroed aligned storage; NULL is an allocation error and
must be checked before use. An optional destructor releases nested resources.
On completion, streams close first, the application's state destructor runs,
then task allocations/destructors run in reverse order, followed by inputs,
context and waitables. Task results/errors are borrowed until the callback
returns them to the host; store dynamic text in task state/allocations.
Do not free the task yourself. Generated result `_free` functions release
strings/lists but **do not drop stream/future handles**. Explicitly drop any
owned handle in the state destructor or transfer it into an SDK reader.

The SDK reads the cancellation future before calling application code.
Cancellation detaches waitables before cancel/drop, cancels blocked stream
reads, requests subtask cancellation once, and retains all task memory until a
`RETURNED`, `STARTED_CANCELLED` or `RETURNED_CANCELLED` result.
`STARTED_CANCELLED` means cancellation before the import began; no later event
or result write follows it.
A late result must still be released by the state destructor. Closing the
cancellation writer without a value allows normal completion. Cancellation
never retries a transaction or promises that an already accepted effect was
rolled back. A cancelled transaction's effect outcome can be unknown to the
caller. If a host import never terminates, the task stays owned; R3's grace
period and worker termination bound that case.

`cx_task_reader` takes ownership of an R1 broker byte-stream reader and a bounded
buffer, including on failure. `cx_reader_next` returns DATA, END, WAIT or ERROR;
return `CX_WAIT` when blocked and `CX_DONE` on ERROR. Process data in task state,
then read again. Closing a reader is idempotent. Generated imports for state,
HTTP, schedules, events, files, host and validation remain available in `app.h`;
they retain the kernel's existing grants and approval rules.

C errors return WIT `app-error` values; the SDK has no exceptions, longjmp or
abort paths. It cannot recover from application undefined behavior or a guest
trap. The worker must attribute/contain those failures.

## JSON helpers

`cx_json_parse` owns a copy of its input and returns a document. Nodes, decoded
strings and embedded NUL bytes are length-delimited and live until
`cx_json_free`. Use `cx_json_get`, `cx_json_at`, and `cx_json_u64` to read values;
`cx_json_first`/`cx_json_next` iterate array or object children in linear time;
`cx_json_at` scans to an individual index. `cx_json_quote` produces an escaped
JSON string for arbitrary valid UTF-8,
and `cx_json_write_u64` writes a safe integer into a caller-provided buffer.
Object keys are compared after escape decoding, including surrogate pairs.

The SDK enforces R0's conservative package limits (root depth 0, maximum depth
32, 32,768 value nodes), a 1 MiB JSON text limit, Unicode scalar strings, unique
object keys, finite numbers, and integers in ±9,007,199,254,740,991. These limits
fit within the wire's 64-level rule. Invalid syntax, trailing text, invalid UTF-8
and unpaired surrogates fail explicitly. Parsing uses the C numeric locale;
Apps must keep `LC_NUMERIC=C`. JSON text is not RFC 8785 canonicalized and must
not be used as a package signing subject. The host still checks full wire
frames and their envelopes; a 1 MiB JSON value alone can exceed a frame budget.

Input is checked before `resume`; `cx_task_json` exposes that parsed document.
`cx_task_ok` checks output JSON. `cx_task_error` preserves a broker's retryable
flag. Kernel errors, cancellation, missing handlers, invalid input/state and
allocation failure all return normal WIT errors through the SDK exports. The
first error is preserved through cleanup and cancellation.

## Checks and integration requirements

```sh
python3 packages/app-sdk-c/tests/test-toolchain.py
bash packages/app-sdk-c/test-native.sh  # native clang + ASan/UBSan
cargo test -p chariox-app-wit --test c_sdk -- --ignored --nocapture
```

The second test builds the real template with the pinned tools and applies
R1's actual install admission policy. `test-native.sh` uses native ABI stubs to
call the SDK/template's actual exports. It checks allocation failures, pending
cancellation and late result storage, stream cleanup, all export kinds, strict
JSON, transaction conflicts, 64 concurrent task states, and R0's transform
terminator cases. These stubs are regression checks, not a real worker.
macOS asset pins are recorded; native/compiler execution on macOS needs its own
platform evidence.

Live acceptance requires R3 (actual async engine, deadlines, cancellation grace,
traps and resource limits), R4 (signed package install/native worker launch) and
R6 (broker-backed WASI, HTTP and data/tmp scopes). Repeat S1-01 and S1-06 on
ordinary/managed Linux and macOS with the real clients, relay and provider.
Source builds, admission and sanitizer results do not close those acceptance
rows or the MP-08 / MP-09 / MP-10 / MP-11 behavioural matrix.


## Browser UI handlers

This SDK dispatches backend WIT handlers. Browser UI actions receive
`(input, caller)` and use `await caller.call(method, input)` for backend work
belonging to that invocation. Ambient `chariox.call` always acts as the human.
Register async forms with `chariox.form(element, async (input, caller) => {...})`;
annotated forms without registration are refused. See the
[view caller and form contract](../app-wit/README.md#view-callers-and-forms).
