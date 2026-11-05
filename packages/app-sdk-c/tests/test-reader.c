#include "abi.h"
#include <stdio.h>

typedef struct { cx_reader *reader; } state;
static app_waitable_status_t status;
static unsigned chunks;
static cx_poll resume(cx_task *t) {
  state *s = cx_task_state(t);
  if (!s->reader) s->reader = cx_task_reader(t, 8, 3);
  assert(s->reader);
  const uint8_t *bytes; size_t count;
  cx_read_status result = cx_reader_next(s->reader, &bytes, &count);
  if (result == CX_READ_WAIT) return CX_WAIT;
  if (APP_WAITABLE_COUNT(status) > 3) {
    assert(result == CX_READ_ERROR);
    assert(cx_reader_next(s->reader, &bytes, &count) == CX_READ_ERROR);
    return CX_DONE;
  }
  if (APP_WAITABLE_COUNT(status)) {
    assert(result == CX_READ_DATA && count == 3 && !memcmp(bytes, "abc", 3));
    chunks++;
    assert(cx_reader_next(s->reader, &bytes, &count) == CX_READ_END);
  } else assert(result == CX_READ_END);
  assert(cx_reader_next(s->reader, &bytes, &count) == CX_READ_END);
  assert(bytes == NULL && count == 0);
  return cx_task_ok(t, "true", 4);
}
const cx_app cx_application = {.state_size=sizeof(state), .resume=resume};
int main(void) {
  const app_waitable_status_t statuses[] = {
    (3 << 4) | APP_WAITABLE_DROPPED, APP_WAITABLE_DROPPED,
    (4 << 4) | APP_WAITABLE_DROPPED, (4 << 4) | APP_WAITABLE_COMPLETED,
  };
  for (unsigned immediate = 0; immediate < 2; immediate++) {
    for (size_t i = 0; i < sizeof statuses / sizeof statuses[0]; i++) {
      test_reset(); chunks = 0; status = statuses[i];
      test_read_immediate = immediate; test_read_status = status;
      app_callback_code_t code = test_call("stream", "{}", 2);
      if (!immediate) {
        assert(code != APP_CALLBACK_CODE_EXIT);
        memcpy(test_stream_buffer, "abc", 3);
        code = test_event(APP_EVENT_STREAM_READ, 8, status);
      }
      assert(code == APP_CALLBACK_CODE_EXIT && !test_context && test_returns == 1);
      assert(test_stream_drops == 1);
      assert(chunks == (APP_WAITABLE_COUNT(status) == 3 ? 1u : 0u));
      assert(test_return_error == (APP_WAITABLE_COUNT(status) > 3));
      if (test_return_error) assert(!strcmp(test_error, "SDK_STATE"));
    }
  }
  test_reset();
  puts("readers: final bytes with EOF, empty EOF and oversized counts sync/callback passed");
}
