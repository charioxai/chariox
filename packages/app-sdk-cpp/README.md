# C++ App SDK

A header-only C++20 coroutine layer over the [C SDK](../app-sdk-c/README.md)
for `chariox:app/app@1.0.0`. It uses the same C dispatcher, exports, cancellation
future, waitable set and owned byte-stream readers. There is no scheduler,
executor, nested task API or second runtime. Code is MIT under the repository
licence.

## Build the template

Install the C SDK's checksum-pinned tools into your own state directory, outside
the checkout. The existing wasi-sdk **33.0** includes `wasm32-wasip2-clang++` and
the C++20 coroutine headers; no additional download or pin is needed.
wit-bindgen remains **0.63.0** with the callback ABI.

```sh
export CHARIOX_HOME=/path/to/your/chariox-state
python3 packages/app-sdk-c/scripts/toolchain.py "$CHARIOX_HOME/c-toolchain"
export CHARIOX_C_TOOLCHAIN="$CHARIOX_HOME/c-toolchain"
bash packages/app-sdk-cpp/build.sh "$CHARIOX_HOME/template.wasm"
# Or build your own backend:
bash packages/app-sdk-cpp/build.sh "$CHARIOX_HOME/my-app.wasm" /path/to/app.cpp
cargo run -p chariox-app-wit --example check-component -- "$CHARIOX_HOME/template.wasm"
```

The supported build hosts, artifact rules, toolchain verification and libc
import restrictions are the C SDK's. The builder compiles the C runtime as C11
and your source as C++20, then links one portable component. It never selects
WASI tools from PATH or downloads implicitly. Set `TMPDIR` outside the checkout
for scratch; generated bindings/objects are removed on exit. A failed build
leaves an existing output intact. Use broker logging and the JSON helpers;
libc stdio can introduce imports that R1 correctly rejects.

Every App translation unit must use **`-fno-exceptions`**. The headers reject
exception-enabled compilation; the builder supplies the flag. All export adapter
functions are `noexcept`. Allocation errors become normal `APP_MEMORY_LIMIT`
results. Do not link exception-enabled App code. Guest traps, undefined behavior
and aborts still require worker attribution/containment; the SDK does not recover
them by unwinding into the host.

`template/app.cpp` has the C template's `echo`, transactional `counter` and
cancellable `wait` tools, schema version 1 and key `count`. Register those same
names in your package declarations. The template returns conflicts without
replaying writes. As with C, this is a backend template: views, declarations,
signing and the real `.cxapp` install flow need the other Stage 1 lanes.

## Invocation coroutines and ownership

```cpp
#include "chariox/app.hpp"

static bool register_app(chariox_app_app_types_app_error_t *) noexcept {
  auto name = cx_string("lookup");
  chariox_app_registry_tool(&name);
  return true;
}

static chariox::call run(chariox::context ctx) noexcept {
  chariox::owned<chariox_app_state_result_option_state_record_app_error_t,
                 chariox_app_state_result_option_state_record_app_error_free> got;
  auto pending = ctx.import([&]() noexcept {
    return chariox_app_state_get(cx_string("key"), &got.value);
  });
  co_await std::move(pending);
  if (got.value.is_err) co_return ctx.error(got.value.val.err);
  co_return ctx.ok("true", 4);
}

extern "C" const cx_app cx_application = chariox::application<register_app, run>();
```

Define the table in exactly one translation unit. Each C invocation owns one
initially suspended `call` frame. The adapter resumes it only through C's
`resume` callback and destroys it through C's state destructor. Do not resume or
destroy frames yourself. Use only this SDK's awaitables inside the invocation.
There are no child tasks or `co_await call` operations.

`ctx.import(start)` returns a lazy, move-only, single-awaiter `future`: creating,
moving or abandoning it starts no operation. `co_await` invokes its callable
once and passes its status to `cx_task_await`. Only one import is pending per
invocation. An immediate returned result continues inline; pending results resume
through C. Awaiting a consumed/moved-from future or awaiter returns `SDK_STATE`
without starting another import. Keep each future and its captures in its own
invocation. Pass borrowed arguments and result pointers into the import from
coroutine locals, which survive suspension. Do not borrow a temporary inside the
start callable: its stack ends when the callable returns.

These futures wrap async imports; they are **not general WIT future readers**.
The WIT `future<string>` for cancellation remains exclusively C-owned. Per the
L3 scope decision, the frozen C API has no new future hook, shared future or
multi-awaiter future. Never read/drop the invocation's cancellation future
yourself.

`owned<T, Free>` zero-initializes a generated result and calls its generated free
function on every exit, including cancellation and late completion. It is
noncopyable. Generated free functions release lists/strings but do **not** drop
stream/future handles: transfer those handles into their SDK owner or drop them
explicitly. Initialize a guard before starting the import that writes to it.

Coroutine locals are destroyed at `co_return`, before the C export returns to
the host. `ctx.ok(json, size)`, `ctx.fail(code, message)` and `ctx.error(error)`
therefore copy their text to C task allocations. They preserve the broker error's
retryable flag. `co_return` their `completion` value to finish the invocation;
`ctx.ok()` supplies an absent lifecycle result or void wake result. Tool/event
calls still require JSON. The C SDK validates input/output JSON and preserves
the first error through later failures and cancellation.

## Cancellation, deadlines, yield and streams

C reads the cancellation future before starting the coroutine and between
cooperative CPU slices. `co_await ctx.yield()` ends a bounded slice; the next
callback polls C's waitable set before resuming application code. It does not
create a new execution thread. Cancellation prevents further coroutine resumes,
closes readers and requests subtask cancellation once. `STARTED_CANCELLED` is
terminal; no invented later completion is needed. If a started import can still
write a late result, C keeps the coroutine frame and all its locals alive until
the import terminates, then destroys them exactly once. Cancelling an accepted
effect does not promise rollback or trigger a retry.

`ctx.deadline_ms()` exposes the invocation's wall-clock deadline, passed by the
supervisor. The supervisor resolves cancellation when it expires; there is no
competing SDK timer or local deadline authority. `co_await ctx.wait_for(ns)` and
`co_await ctx.wait_until(mark)` use the world's async WASI 0.3 monotonic clock
imports and the same cancellation path. A monotonic mark is not a wall-clock
millisecond deadline. Actual deadline enforcement, cancellation grace and CPU
preemption belong to R3.

`ctx.stream(handle, capacity)` transfers a broker byte-stream reader to C's
bounded buffer, even on allocation failure. The wrapper is move-only and must
stay in its own invocation. `co_await stream.next()` returns `{data, size, end}`;
use the borrowed bytes before the next read. `end` is true only after all data
has been delivered, including bytes returned together with EOF. Stream errors
finish the invocation without resuming application code. `stream.close()` and
destruction are idempotent; cancellation closes the reader before destroying
the frame. Reads through a moved-from wrapper fail explicitly.

## Checks and integration limits

```sh
bash packages/app-sdk-c/test-native.sh
bash packages/app-sdk-cpp/test-native.sh
cargo test -p chariox-app-wit --test cpp_sdk -- --ignored --nocapture
```

Native C++ checks use the C suite's ABI simulator, real SDK exports and the
same ASan/UBSan flags and leak detection. They cover cancel before start,
`STARTED_CANCELLED`, cancellation during yield, final stream bytes at EOF,
exactly-once handle drop/destruction, late host writes, preserved earlier errors,
future ownership, monotonic waits and refusal of exception-enabled builds. The
unchanged C template suite also runs against the C++ template: transactions,
conflicts, 64 calls, allocation failures and every handler export.

The explicit Rust test builds the real pinned C++ template and applies R1's
unchanged admission policy. Native stubs and admission are **not real-engine
execution or S1-15 acceptance**. S1-15 repeats S1-01 and cancellation/state
checks on R3's actual async engine, with R4 install/launch, R5 containment and R6
brokered resources. Real ordinary/managed Linux and macOS clients, hosted relay,
providers and signed packages remain those lanes' integration gates. No
MP-08 / MP-09 / MP-10 / MP-11 item closes from these native checks.


## Browser UI handlers

This SDK dispatches backend WIT handlers. Browser UI actions receive
`(input, caller)` and use `await caller.call(method, input)` for backend work
belonging to that invocation. Ambient `chariox.call` always acts as the human.
Register async forms with `chariox.form(element, async (input, caller) => {...})`;
annotated forms without registration are refused. See the
[view caller and form contract](../app-wit/README.md#view-callers-and-forms).
