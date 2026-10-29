#include "abi.h"
#include <stdio.h>

typedef struct { cx_reader *reader; app_string_t imported; char *allocation; } state;
static state *active;
static void destroy(void *ptr) {
  state *s = ptr; test_destroys++; free(s->imported.ptr);
  assert(s->allocation == NULL || s->allocation[0] == 'x');
}
static bool register_app(chariox_app_app_types_app_error_t *error) { (void)error; return true; }
static cx_poll resume(cx_task *t) {
  state *s = cx_task_state(t); active = s;
  if (cx_string_is(cx_task_request(t)->name, "invalid-result")) return cx_task_ok(t, "{\"a\":1,\"a\":2}", 13);
  if (cx_string_is(cx_task_request(t)->name, "yield")) {
    if (cx_task_step(t) == 0) { cx_task_set_step(t,1); return CX_YIELD; }
    return cx_task_ok(t,"true",4);
  }
  if (cx_task_step(t) == 0) {
    s->allocation = cx_task_alloc(t, 1024, NULL);
    if (!s->allocation) return CX_DONE;
    s->allocation[0] = 'x';
    if (cx_string_is(cx_task_request(t)->name, "stream")) {
      s->reader = cx_task_reader(t, 8, 4096);
      if (!s->reader) return CX_DONE;
      cx_task_set_step(t, 1);
    } else {
      if (cx_string_is(cx_task_request(t)->name, "early-error")) {
        assert(cx_task_await(t, (7 << 4) | APP_SUBTASK_STARTED) == CX_WAIT);
        return cx_task_fail(t,"APP_ERROR","application failed");
      }
      CX_AWAIT(t, (7 << 4) | APP_SUBTASK_STARTED, 2);
    }
  }
  if (cx_task_step(t) == 1) {
    const uint8_t *bytes; size_t count;
    cx_read_status result = cx_reader_next(s->reader, &bytes, &count);
    if (result == CX_READ_WAIT) return CX_WAIT;
    if (result == CX_READ_ERROR) return CX_DONE;
    if (result == CX_READ_DATA) { assert(count == 3 && !memcmp(bytes,"abc",3)); }
    cx_reader_close(s->reader);
  }
  return cx_task_ok(t, "true", 4);
}
const cx_app cx_application = {sizeof(state),register_app,resume,destroy};
int main(void) {
  test_reset();
  assert(test_call("pending", "{}", 2) != APP_CALLBACK_CODE_EXIT);
  void *task = test_context; state *s = active;
  test_cancel_status = APP_SUBTASK_STARTED;
  assert(test_event(APP_EVENT_FUTURE_READ, 2, APP_WAITABLE_COMPLETED) != APP_CALLBACK_CODE_EXIT);
  assert(test_destroys == 0 && test_returns == 0 && test_import_drops == 0 && test_context == task);
  assert(s->allocation[0] == 'x');
  /* A late import writes into task-owned storage even after cancellation. */
  s->imported = test_string("late result allocation");
  assert(test_event(APP_EVENT_SUBTASK, 7, APP_SUBTASK_STARTED) != APP_CALLBACK_CODE_EXIT);
  assert(test_destroys == 0);
  app_event_t repeated = {APP_EVENT_CANCEL,0,0};
  assert(exports_chariox_app_handlers_call_tool_callback(&repeated) != APP_CALLBACK_CODE_EXIT);
  assert(test_cancel_calls == 1);
  assert(test_event(APP_EVENT_SUBTASK, 7, APP_SUBTASK_RETURNED) == APP_CALLBACK_CODE_EXIT);
  assert(test_destroys == 1 && test_returns == 1 && test_import_drops == 1 && !test_context);
  assert(test_return_error && !strcmp(test_error, "CANCELLED"));
  test_reset();
  assert(test_call("pending", "{}", 2) != APP_CALLBACK_CODE_EXIT);
  assert(test_event(APP_EVENT_FUTURE_READ, 2, APP_WAITABLE_COMPLETED) == APP_CALLBACK_CODE_EXIT);
  assert(test_destroys == 1 && test_cancel_calls == 1 && test_import_drops == 1);
  test_reset(); test_cancel_immediate = true;
  assert(test_call("pending", "{}", 2) == APP_CALLBACK_CODE_EXIT);
  assert(test_return_error && test_cancel_calls == 0 && !strcmp(test_message,"deadline"));
  test_reset(); test_cancel_reason_race = true;
  assert(test_call("pending", "{}", 2) != APP_CALLBACK_CODE_EXIT);
  assert(test_event(APP_EVENT_SUBTASK,7,APP_SUBTASK_RETURNED)==APP_CALLBACK_CODE_EXIT);
  assert(!test_return_error && test_returns==1);
  test_reset();
  assert(test_call("pending", "{}", 2) != APP_CALLBACK_CODE_EXIT);
  assert(test_event(APP_EVENT_FUTURE_READ, 2, APP_WAITABLE_DROPPED) != APP_CALLBACK_CODE_EXIT);
  assert(test_event(APP_EVENT_SUBTASK, 7, APP_SUBTASK_RETURNED) == APP_CALLBACK_CODE_EXIT);
  assert(!test_return_error && test_returns == 1);
  test_reset();
  assert(test_call("stream", "{}", 2) != APP_CALLBACK_CODE_EXIT);
  assert(test_event(APP_EVENT_FUTURE_READ, 2, APP_WAITABLE_COMPLETED) == APP_CALLBACK_CODE_EXIT);
  assert(test_stream_drops == 1 && test_destroys == 1);
  test_reset();
  assert(test_call("stream", "{}", 2) != APP_CALLBACK_CODE_EXIT);
  memcpy(test_stream_buffer,"abc",3);
  assert(test_event(APP_EVENT_STREAM_READ, 8, (3 << 4) | APP_WAITABLE_COMPLETED) == APP_CALLBACK_CODE_EXIT);
  assert(!test_return_error && test_stream_drops == 1);
  test_reset(); test_read_immediate = true;
  assert(test_call("stream", "{}", 2) == APP_CALLBACK_CODE_EXIT);
  assert(!test_return_error && test_stream_drops == 1);
  test_reset();
  assert(test_call("invalid-result", "{}", 2) == APP_CALLBACK_CODE_EXIT);
  assert(test_return_error && !strcmp(test_error,"INVALID_JSON"));
  test_reset();
  assert(test_call("pending", "{\"a\":1,\"\\u0061\":2}", 2) == APP_CALLBACK_CODE_EXIT);
  assert(test_return_error && !strcmp(test_error,"INVALID_JSON") && test_cancel_calls == 0);
  test_reset();
  assert(test_call("yield", "{}", 2) == APP_CALLBACK_CODE_YIELD);
  assert(test_event(APP_EVENT_NONE,0,0) == APP_CALLBACK_CODE_EXIT && !test_return_error);
  test_reset();
  assert(test_call("yield", "{}", 2) == APP_CALLBACK_CODE_YIELD);
  test_queue_event(APP_EVENT_FUTURE_READ,2,APP_WAITABLE_COMPLETED);
  assert(test_event(APP_EVENT_NONE,0,0) == APP_CALLBACK_CODE_EXIT && test_return_error);
  test_reset(); test_cancel_status = APP_SUBTASK_STARTED;
  assert(test_call("early-error", "{}", 2) != APP_CALLBACK_CODE_EXIT);
  assert(test_destroys == 0 && test_returns == 0);
  assert(test_event(APP_EVENT_SUBTASK,7,APP_SUBTASK_RETURNED_CANCELLED) == APP_CALLBACK_CODE_EXIT);
  assert(test_return_error && !strcmp(test_error,"APP_ERROR"));
  test_reset();
  puts("tasks: pending cancellation retention, late results, stream release, normal close and bounded JSON passed");
}
