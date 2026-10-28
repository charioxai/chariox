#include "abi.h"
#include <stdio.h>

static app_subtask_status_t import_status;
static unsigned resumes;
static void destroy(void *state) { assert(state); test_destroys++; }
static cx_poll resume(cx_task *t) {
  resumes++;
  if (!cx_task_step(t)) {
    cx_task_set_step(t, 1);
    cx_poll poll = cx_task_await(t, import_status);
    if (cx_string_is(cx_task_request(t)->name, "error")) return cx_task_fail(t, "FIRST", "app failed");
    if (poll != CX_READY) return poll;
  }
  return cx_task_ok(t, "true", 4);
}
const cx_app cx_application = {.state_size=1, .resume=resume, .destroy=destroy};
static void finished(const char *error, unsigned drops) {
  assert(!test_context && test_returns == 1 && test_destroys == 1);
  assert(test_import_drops == drops && test_return_error && !strcmp(test_error, error));
  assert(resumes == 1);
}
int main(void) {
  for (unsigned handle = 0; handle < 2; handle++) {
    test_reset(); resumes = 0;
    import_status = ((handle ? 7u : 0u) << 4) | APP_SUBTASK_STARTED_CANCELLED;
    assert(test_call("await", "{}", 2) == APP_CALLBACK_CODE_EXIT);
    finished("CANCELLED", handle);
    assert(test_cancel_calls == 0);
  }
  for (unsigned canonical = 0; canonical < 2; canonical++) {
    test_reset(); resumes = 0;
    import_status = (7 << 4) | APP_SUBTASK_STARTING;
    assert(test_call("pending", "{}", 2) != APP_CALLBACK_CODE_EXIT);
    test_cancel_status = APP_SUBTASK_STARTED_CANCELLED;
    assert(test_event(canonical ? APP_EVENT_CANCEL : APP_EVENT_FUTURE_READ,
                      canonical ? 0 : 2, APP_WAITABLE_COMPLETED) == APP_CALLBACK_CODE_EXIT);
    finished("CANCELLED", 1);
    assert(test_cancel_calls == 1);
  }
  test_reset(); resumes = 0;
  assert(test_call("pending", "{}", 2) != APP_CALLBACK_CODE_EXIT);
  assert(test_event(APP_EVENT_SUBTASK, 7, APP_SUBTASK_STARTED_CANCELLED) == APP_CALLBACK_CODE_EXIT);
  finished("CANCELLED", 1);
  assert(test_cancel_calls == 0);
  test_reset(); resumes = 0; test_cancel_status = APP_SUBTASK_STARTED_CANCELLED;
  assert(test_call("error", "{}", 2) == APP_CALLBACK_CODE_EXIT);
  finished("FIRST", 1);
  assert(test_cancel_calls == 1);
  test_reset();
  puts("cancellation: before-start await, cancel and callback terminate without a later event");
}
