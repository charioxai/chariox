#include "abi.h"
#include <stdio.h>

typedef struct { cx_reader *reader; } state;
static unsigned resumes;
static bool done;
static cx_poll resume(cx_task *t) {
  resumes++;
  if (cx_string_is(cx_task_request(t)->name, "stream")) {
    state *s = cx_task_state(t);
    if (!s->reader) s->reader = cx_task_reader(t, 8, 3);
    assert(s->reader);
    const uint8_t *bytes; size_t count;
    cx_read_status status = cx_reader_next(s->reader, &bytes, &count);
    if (status == CX_READ_WAIT) return CX_YIELD;
    assert(status == CX_READ_DATA && count == 3 && !memcmp(bytes, "abc", 3));
    return cx_task_ok(t, "true", 4);
  }
  return done ? cx_task_ok(t, "true", 4) : CX_YIELD;
}
const cx_app cx_application = {.state_size=sizeof(state), .resume=resume};
int main(void) {
  test_reset();
  assert(test_call("spin", "{}", 2) == APP_CALLBACK_CODE_YIELD);
  for (unsigned i = 0; i < 3; i++) assert(test_event(APP_EVENT_NONE, 0, 0) == APP_CALLBACK_CODE_YIELD);
  assert(resumes == 4 && test_returns == 0);
  test_queue_event(APP_EVENT_FUTURE_READ, 2, APP_WAITABLE_COMPLETED);
  assert(test_event(APP_EVENT_NONE, 0, 0) == APP_CALLBACK_CODE_EXIT);
  assert(resumes == 4 && test_returns == 1 && test_return_error && !strcmp(test_error, "CANCELLED"));
  assert(!strcmp(test_message, "cancelled by test"));
  test_reset(); resumes = 0;
  assert(test_call("spin", "{}", 2) == APP_CALLBACK_CODE_YIELD);
  assert(test_event(APP_EVENT_CANCEL, 0, 0) == APP_CALLBACK_CODE_EXIT);
  assert(resumes == 1 && test_return_error);
  test_reset(); resumes = 0;
  assert(test_call("spin", "{}", 2) == APP_CALLBACK_CODE_YIELD);
  test_queue_event(APP_EVENT_FUTURE_READ, 2, APP_WAITABLE_DROPPED);
  assert(test_event(APP_EVENT_NONE, 0, 0) == APP_CALLBACK_CODE_YIELD);
  done = true;
  assert(test_event(APP_EVENT_NONE, 0, 0) == APP_CALLBACK_CODE_EXIT);
  assert(resumes == 3 && !test_return_error);
  test_reset(); resumes = 0;
  assert(test_call("stream", "{}", 2) == APP_CALLBACK_CODE_YIELD);
  memcpy(test_stream_buffer, "abc", 3);
  test_queue_event(APP_EVENT_STREAM_READ, 8, (3 << 4) | APP_WAITABLE_DROPPED);
  assert(test_event(APP_EVENT_NONE, 0, 0) == APP_CALLBACK_CODE_EXIT);
  assert(resumes == 2 && !test_return_error && test_stream_drops == 1);
  test_reset();
  puts("yield: NONE callbacks poll cancellation, closed futures and stream readiness passed");
}
