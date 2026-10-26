#include "chariox/app.hpp"
#include <cstdio>
#include <initializer_list>
#include <type_traits>
extern "C" {
#include "abi.h"
}

static unsigned starts, steps, destroys, frees;
static app_subtask_status_t import_status;
static uint64_t waited_for, waited_until;
static char **pending_result;
struct guard { ~guard() { destroys++; } };
struct payload { char *text; };
static void free_payload(payload *p) { ::free(p->text); frees++; }
static bool register_app(chariox_app_app_types_app_error_t *) noexcept { return true; }
static app_subtask_status_t start(payload &p) noexcept {
  starts++; pending_result = &p.text; return import_status;
}
app_subtask_status_t wasi_clocks_0_3_0_monotonic_clock_wait_for(uint64_t ns) {
  starts++; waited_for = ns; return import_status;
}
app_subtask_status_t wasi_clocks_0_3_0_monotonic_clock_wait_until(uint64_t mark) {
  starts++; waited_until = mark; return import_status;
}

static chariox::call run(chariox::context ctx) noexcept {
  guard local;
  auto name = ctx.request().name;
  if (cx_string_is(name, "yield")) {
    for (unsigned i = 0; i < 3; i++) { co_await ctx.yield(); steps++; }
  } else if (cx_string_is(name, "clock")) {
    assert(ctx.deadline_ms() == 123456);
    co_await ctx.wait_for(17);
    co_await ctx.wait_until(29);
    steps++;
  } else if (cx_string_is(name, "stream")) {
    auto original = ctx.stream(8, 16);
    auto stream = std::move(original);
    auto data = co_await stream.next();
    assert(!data.end && data.size == 3 && !memcmp(data.data, "abc", 3)); steps++;
    auto end = co_await stream.next();
    assert(end.end && end.size == 0 && end.data == nullptr); steps++;
    stream.close(); stream.close();
    assert((co_await stream.next()).end);
  } else if (cx_string_is(name, "stream-moved")) {
    auto original = ctx.stream(8, 16);
    auto stream = std::move(original);
    co_await original.next(); steps++;
  } else if (cx_string_is(name, "stream-close")) {
    auto stream = ctx.stream(8, 16);
    co_await stream.next(); steps++;
    stream.close(); stream.close();
  } else if (cx_string_is(name, "result")) {
    chariox::owned<payload, free_payload> data;
    data.value.text = static_cast<char *>(::malloc(5));
    assert(data.value.text); memcpy(data.value.text, "true", 5);
    co_return ctx.ok(data.value.text, 4);
  } else if (cx_string_is(name, "error")) {
    chariox::owned<payload, free_payload> data;
    data.value.text = static_cast<char *>(::malloc(6));
    assert(data.value.text); memcpy(data.value.text, "FIRST", 6);
    chariox_app_app_types_app_error_t err{};
    err.code = cx_string(data.value.text); err.message = cx_string(data.value.text);
    err.retryable = {true, true};
    co_return ctx.error(err);
  } else {
    chariox::owned<payload, free_payload> data;
    if (cx_string_is(name, "first-error")) ctx.fail("FIRST", "keep the original error");
    auto future = ctx.import([&]() noexcept { return start(data.value); });
    static_assert(!std::is_copy_constructible_v<decltype(future)>);
    auto moved = std::move(future);
    static_assert(!std::is_copy_constructible_v<decltype(std::move(moved).operator co_await())>);
    if (cx_string_is(name, "awaiter-twice")) {
      auto waiter = std::move(moved).operator co_await();
      co_await waiter;
      co_await waiter;
      steps++;
      co_return ctx.ok("true", 4);
    }
    if (cx_string_is(name, "moved-from")) co_await std::move(future);
    else if (!cx_string_is(name, "lazy")) co_await std::move(moved);
    if (cx_string_is(name, "twice")) co_await std::move(moved);
    steps++;
  }
  co_return ctx.ok("true", 4);
}
extern "C" const cx_app cx_application = chariox::application<register_app, run>();

static void reset() {
  test_reset(); starts = steps = destroys = frees = 0;
  pending_result = nullptr; waited_for = waited_until = 0;
  import_status = (7 << 4) | APP_SUBTASK_STARTED;
}
static void done(const char *error = nullptr) {
  assert(test_returns == 1 && !test_context && destroys == 1);
  assert(test_return_error == (error != nullptr));
  if (error) assert(!strcmp(test_error, error));
  else assert(!strcmp(test_result, "true"));
}
static void cancel_before_start() {
  reset(); test_cancel_immediate = true;
  assert(test_call("import", "{}", 2) == APP_CALLBACK_CODE_EXIT);
  assert(!starts && !steps && !destroys && !frees && test_returns == 1 && !test_context);
  assert(test_return_error && !strcmp(test_error, "CANCELLED") && !strcmp(test_message, "deadline"));
  for (unsigned handle : {0u, 7u}) {
    reset(); import_status = (handle << 4) | APP_SUBTASK_STARTED_CANCELLED;
    assert(test_call("import", "{}", 2) == APP_CALLBACK_CODE_EXIT);
    done("CANCELLED"); assert(!steps && frees == 1 && test_import_drops == (handle != 0));
  }
  for (auto event : {APP_EVENT_CANCEL, APP_EVENT_FUTURE_READ}) {
    reset(); import_status = (7 << 4) | APP_SUBTASK_STARTING;
    test_cancel_status = APP_SUBTASK_STARTED_CANCELLED;
    assert(test_call("import", "{}", 2) != APP_CALLBACK_CODE_EXIT);
    assert(test_event(event, 2, APP_WAITABLE_COMPLETED) == APP_CALLBACK_CODE_EXIT);
    done("CANCELLED"); assert(!steps && frees == 1 && test_import_drops == 1 && test_cancel_calls == 1);
  }
  reset(); assert(test_call("import", "{}", 2) != APP_CALLBACK_CODE_EXIT);
  assert(test_event(APP_EVENT_SUBTASK, 7, APP_SUBTASK_STARTED_CANCELLED) == APP_CALLBACK_CODE_EXIT);
  done("CANCELLED"); assert(!steps && frees == 1 && test_import_drops == 1);
}
static void yield_cancel() {
  reset(); assert(test_call("yield", "{}", 2) == APP_CALLBACK_CODE_YIELD);
  test_queue_event(APP_EVENT_FUTURE_READ, 2, APP_WAITABLE_COMPLETED);
  assert(test_event(APP_EVENT_NONE, 0, 0) == APP_CALLBACK_CODE_EXIT);
  done("CANCELLED"); assert(steps == 0);
  reset(); assert(test_call("yield", "{}", 2) == APP_CALLBACK_CODE_YIELD);
  test_queue_event(APP_EVENT_FUTURE_READ, 2, APP_WAITABLE_DROPPED);
  for (unsigned i = 1; i <= 3; i++) {
    assert(test_event(APP_EVENT_NONE, 0, 0) == (i < 3 ? APP_CALLBACK_CODE_YIELD : APP_CALLBACK_CODE_EXIT));
    assert(steps == i);
  }
  done();
  reset(); assert(test_call("yield", "{}", 2) == APP_CALLBACK_CODE_YIELD);
  assert(test_event(APP_EVENT_CANCEL, 0, 0) == APP_CALLBACK_CODE_EXIT);
  done("CANCELLED"); assert(!steps);
}
static void stream_eof() {
  reset(); test_read_immediate = true; test_read_status = (3 << 4) | APP_WAITABLE_DROPPED;
  assert(test_call("stream", "{}", 2) == APP_CALLBACK_CODE_EXIT);
  done(); assert(steps == 2 && test_stream_drops == 1);
  reset(); assert(test_call("stream", "{}", 2) != APP_CALLBACK_CODE_EXIT);
  memcpy(test_stream_buffer, "abc", 3);
  assert(test_event(APP_EVENT_STREAM_READ, 8, (3 << 4) | APP_WAITABLE_DROPPED) == APP_CALLBACK_CODE_EXIT);
  done(); assert(steps == 2 && test_stream_drops == 1);
  reset(); test_read_immediate = true;
  assert(test_call("stream-close", "{}", 2) == APP_CALLBACK_CODE_EXIT);
  done(); assert(steps == 1 && test_stream_drops == 1);
  for (auto status : {unsigned(APP_WAITABLE_CANCELLED), (17u << 4) | APP_WAITABLE_DROPPED}) {
    reset(); assert(test_call("stream", "{}", 2) != APP_CALLBACK_CODE_EXIT);
    assert(test_event(APP_EVENT_STREAM_READ, 8, status) == APP_CALLBACK_CODE_EXIT);
    done(status == APP_WAITABLE_CANCELLED ? "CANCELLED" : "SDK_STATE");
    assert(!steps && test_stream_drops == 1);
  }
  reset(); assert(test_call("stream", "{}", 2) != APP_CALLBACK_CODE_EXIT);
  assert(test_event(APP_EVENT_FUTURE_READ, 2, APP_WAITABLE_COMPLETED) == APP_CALLBACK_CODE_EXIT);
  done("CANCELLED"); assert(!steps && test_stream_drops == 1);
}
static void lifetime() {
  for (auto terminal : {APP_SUBTASK_RETURNED, APP_SUBTASK_RETURNED_CANCELLED}) {
    reset(); test_cancel_status = APP_SUBTASK_STARTED;
    assert(test_call("import", "{}", 2) != APP_CALLBACK_CODE_EXIT);
    assert(test_event(APP_EVENT_FUTURE_READ, 2, APP_WAITABLE_COMPLETED) != APP_CALLBACK_CODE_EXIT);
    assert(!destroys && !frees && !test_returns && pending_result);
    *pending_result = static_cast<char *>(::malloc(5)); assert(*pending_result);
    memcpy(*pending_result, "late", 5);
    assert(test_event(APP_EVENT_SUBTASK, 7, terminal) == APP_CALLBACK_CODE_EXIT);
    done("CANCELLED"); assert(!steps && frees == 1 && test_import_drops == 1 && test_cancel_calls == 1);
  }
  reset(); test_cancel_reason_race = true;
  assert(test_call("result", "{}", 2) == APP_CALLBACK_CODE_EXIT);
  done(); assert(frees == 1);
  reset(); assert(test_call("error", "{}", 2) == APP_CALLBACK_CODE_EXIT);
  done("FIRST"); assert(frees == 1);
}
static void first_error() {
  reset(); test_cancel_status = APP_SUBTASK_STARTED;
  assert(test_call("first-error", "{}", 2) != APP_CALLBACK_CODE_EXIT);
  assert(test_cancel_calls == 1 && !destroys && !frees && !test_returns);
  assert(test_event(APP_EVENT_FUTURE_READ, 2, APP_WAITABLE_COMPLETED) != APP_CALLBACK_CODE_EXIT);
  *pending_result = static_cast<char *>(::malloc(1)); assert(*pending_result);
  assert(test_event(APP_EVENT_SUBTASK, 7, APP_SUBTASK_RETURNED_CANCELLED) == APP_CALLBACK_CODE_EXIT);
  done("FIRST"); assert(frees == 1 && !steps && test_import_drops == 1 && test_cancel_calls == 1);
}
static void future_clock() {
  reset(); assert(test_call("lazy", "{}", 2) == APP_CALLBACK_CODE_EXIT);
  done(); assert(!starts && steps == 1 && frees == 1);
  reset(); assert(test_call("moved-from", "{}", 2) == APP_CALLBACK_CODE_EXIT);
  done("SDK_STATE"); assert(!starts && !steps && frees == 1);
  reset(); import_status = APP_SUBTASK_RETURNED;
  assert(test_call("twice", "{}", 2) == APP_CALLBACK_CODE_EXIT);
  done("SDK_STATE"); assert(starts == 1 && !steps && frees == 1);
  reset(); assert(test_call("import", "{}", 2) != APP_CALLBACK_CODE_EXIT);
  assert(test_event(APP_EVENT_SUBTASK, 7, APP_SUBTASK_STARTED) != APP_CALLBACK_CODE_EXIT);
  assert(!steps && !destroys);
  assert(test_event(APP_EVENT_SUBTASK, 7, APP_SUBTASK_RETURNED) == APP_CALLBACK_CODE_EXIT);
  done(); assert(starts == 1 && steps == 1 && frees == 1 && test_import_drops == 1);
  reset();
  auto name = test_string("clock"), input = test_string("{}");
  chariox_app_app_types_invocation_context_t context{}; context.deadline_ms = 123456;
  assert(exports_chariox_app_handlers_call_tool(&name, &input, &context, 2) != APP_CALLBACK_CODE_EXIT);
  assert(waited_for == 17 && !waited_until && !steps);
  assert(test_event(APP_EVENT_SUBTASK, 7, APP_SUBTASK_RETURNED) != APP_CALLBACK_CODE_EXIT);
  assert(waited_until == 29 && !steps);
  assert(test_event(APP_EVENT_SUBTASK, 7, APP_SUBTASK_RETURNED) == APP_CALLBACK_CODE_EXIT);
  done(); assert(steps == 1 && starts == 2 && test_import_drops == 2);
}

static void ownership() {
  reset();
  auto name = test_string("stream-moved");
  chariox_app_app_types_invocation_context_t context{};
  assert(exports_chariox_app_handlers_lifecycle(&name, nullptr, &context, 2) == APP_CALLBACK_CODE_EXIT);
  done("SDK_STATE"); assert(!steps && test_stream_drops == 1);
}
static void single_awaiter() {
  reset(); import_status = APP_SUBTASK_RETURNED;
  assert(test_call("awaiter-twice", "{}", 2) == APP_CALLBACK_CODE_EXIT);
  assert(starts == 1); done("SDK_STATE"); assert(!steps && frees == 1);
}

static void allocations() {
  bool frame_failure = false, success = false;
  for (long i = 0; i < 32; i++) {
    reset(); cx_test_fail_after = i;
    assert(test_call("lazy", "{}", 2) == APP_CALLBACK_CODE_EXIT);
    cx_test_fail_after = -1;
    assert(test_returns == 1 && !test_context);
    if (test_return_error) {
      assert(!strcmp(test_error, "APP_MEMORY_LIMIT"));
      if (!strcmp(test_message, "coroutine allocation failed")) frame_failure = true;
    } else {
      success = true; assert(destroys == 1 && frees == 1 && !strcmp(test_result, "true"));
    }
  }
  assert(frame_failure && success);
}

int main(int argc, char **argv) {
  struct suite { const char *name; void (*run)(); };
  const suite suites[] = {{"cancel-before-start", cancel_before_start}, {"yield-cancel", yield_cancel},
    {"stream-eof", stream_eof}, {"lifetime", lifetime}, {"first-error", first_error}, {"future-clock", future_clock},
    {"ownership", ownership}, {"single-awaiter", single_awaiter}, {"allocations", allocations}};
  assert(argc <= 2);
  bool ran = false;
  for (auto test : suites) if (argc == 1 || !strcmp(argv[1], test.name)) {
    ran = true;
    test.run(); test_reset(); std::printf("%s: passed\n", test.name);
  }
  assert(ran);
}
