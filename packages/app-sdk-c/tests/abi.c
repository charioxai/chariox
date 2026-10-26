/* Native ABI simulator, for allocation/order regressions only. Not a worker. */
#include "abi.h"
#include <stdio.h>
void *test_context;
unsigned test_returns, test_destroys, test_cancel_calls, test_import_drops, test_stream_drops;
bool test_return_error, test_cancel_immediate, test_read_immediate, test_cancel_reason_race;
app_subtask_status_t test_cancel_status;
app_waitable_status_t test_read_status;
app_waitable_set_t test_joined[TEST_HANDLES];
bool test_future_active[TEST_HANDLES], test_stream_active[TEST_HANDLES];
app_string_t *test_future_buffers[TEST_HANDLES];
uint8_t *test_stream_buffer;
char test_result[1024], test_error[128], test_message[256];
static unsigned sets;
app_string_t test_string(const char *text) {
  size_t len = strlen(text); char *copy = malloc(len+1); assert(copy); memcpy(copy, text, len+1);
  return (app_string_t){(uint8_t *)copy, len};
}
void app_string_free(app_string_t *s) { free(s->ptr); *s = (app_string_t){0}; }
void app_context_set_0(void *p) { test_context = p; }
void *app_context_get_0(void) { return test_context; }
app_waitable_set_t app_waitable_set_new(void) { return ++sets; }
void app_waitable_set_drop(app_waitable_set_t set) {
  for (size_t i = 0; i < TEST_HANDLES; i++) assert(test_joined[i] != set);
}
void app_waitable_join(uint32_t handle, app_waitable_set_t set) { assert(handle && handle < TEST_HANDLES); test_joined[handle] = set; }
app_subtask_status_t app_subtask_cancel(app_subtask_t h) {
  assert(!test_joined[h]); test_cancel_calls++; return test_cancel_status;
}
void app_subtask_drop(app_subtask_t h) { assert(!test_joined[h]); test_import_drops++; }
app_waitable_status_t exports_chariox_app_handlers_future_string_read(uint32_t h, app_string_t *buf) {
  test_future_buffers[h] = buf;
  if (test_cancel_immediate) { *buf = test_string("deadline"); return APP_WAITABLE_COMPLETED; }
  test_future_active[h] = true; return APP_WAITABLE_STATUS_BLOCKED;
}
app_waitable_status_t exports_chariox_app_handlers_future_string_cancel_read(uint32_t h) {
  assert(!test_joined[h]); assert(test_future_active[h]); test_future_active[h] = false;
  if (test_cancel_reason_race) { *test_future_buffers[h] = test_string("racing cancellation reason"); return APP_WAITABLE_COMPLETED; }
  return APP_WAITABLE_CANCELLED;
}
void exports_chariox_app_handlers_future_string_drop_readable(uint32_t h) {
  assert(!test_joined[h]); assert(!test_future_active[h]); test_future_buffers[h] = NULL;
}
app_waitable_status_t chariox_app_files_stream_u8_read(uint32_t h, uint8_t *buf, size_t size) {
  assert(size >= 3); test_stream_buffer = buf;
  if (test_read_immediate) { memcpy(buf, "abc", 3); return test_read_status; }
  test_stream_active[h] = true; return APP_WAITABLE_STATUS_BLOCKED;
}
app_waitable_status_t chariox_app_files_stream_u8_cancel_read(uint32_t h) {
  assert(!test_joined[h]); assert(test_stream_active[h]); test_stream_active[h] = false; return APP_WAITABLE_CANCELLED;
}
void chariox_app_files_stream_u8_drop_readable(uint32_t h) { assert(!test_joined[h]); assert(!test_stream_active[h]); test_stream_drops++; }
static void copy_text(char *out, size_t capacity, app_string_t text) {
  assert(text.len < capacity); if (text.len) memcpy(out, text.ptr, text.len); out[text.len] = 0;
}
static void result(bool error, const chariox_app_app_types_app_error_t *err, const app_string_t *json) {
  test_returns++; test_return_error = error;
  if (error) { copy_text(test_error, sizeof test_error, err->code); copy_text(test_message, sizeof test_message, err->message); }
  else if (json) copy_text(test_result, sizeof test_result, *json);
}
void exports_chariox_app_handlers_call_tool_return(exports_chariox_app_handlers_result_json_app_error_t r) { result(r.is_err, &r.val.err, r.is_err ? NULL : &r.val.ok); }
void exports_chariox_app_handlers_deliver_event_return(exports_chariox_app_handlers_result_json_app_error_t r) { result(r.is_err, &r.val.err, r.is_err ? NULL : &r.val.ok); }
void exports_chariox_app_handlers_lifecycle_return(exports_chariox_app_handlers_result_option_json_app_error_t r) { result(r.is_err, &r.val.err, !r.is_err && r.val.ok.is_some ? &r.val.ok.val : NULL); }
void exports_chariox_app_handlers_deliver_wake_return(exports_chariox_app_handlers_result_void_app_error_t r) { result(r.is_err, &r.val.err, NULL); }
static void option_string_free(app_option_string_t *s) { if (s->is_some) app_string_free(&s->val); }
void chariox_app_app_types_invocation_context_free(chariox_app_app_types_invocation_context_t *r) {
  option_string_free(&r->installation_id); option_string_free(&r->room_id); option_string_free(&r->operation_id);
  option_string_free(&r->agent_id); option_string_free(&r->task_id); option_string_free(&r->turn_id);
  if (r->actor.is_some) app_string_free(&r->actor.val.id);
}
void chariox_app_app_types_wake_free(chariox_app_app_types_wake_t *r) { app_string_free(&r->id); option_string_free(&r->revision); }
app_callback_code_t test_call(const char *name, const char *input, uint32_t cancel) {
  app_string_t n = test_string(name), in = test_string(input);
  chariox_app_app_types_invocation_context_t context = {.room_id={true,test_string("room")}, .actor={.is_some=true,.val={.id=test_string("actor")}}};
  return exports_chariox_app_handlers_call_tool(&n, &in, &context, cancel);
}
app_callback_code_t test_event(app_event_code_t event, uint32_t handle, uint32_t status) {
  app_event_t e = {event,handle,status};
  if (event == APP_EVENT_FUTURE_READ) {
    test_future_active[handle] = false;
    if (APP_WAITABLE_STATE(status) == APP_WAITABLE_COMPLETED) *test_future_buffers[handle] = test_string("cancelled by test");
  }
  if (event == APP_EVENT_STREAM_READ) test_stream_active[handle] = false;
  return exports_chariox_app_handlers_call_tool_callback(&e);
}
void test_reset(void) {
  assert(!test_context);
  for (size_t i = 0; i < TEST_HANDLES; i++) { assert(!test_joined[i]); assert(!test_future_active[i]); assert(!test_stream_active[i]); }
  test_returns = test_destroys = test_cancel_calls = test_import_drops = test_stream_drops = 0;
  test_return_error = test_cancel_immediate = test_read_immediate = test_cancel_reason_race = false;
  test_cancel_status = APP_SUBTASK_RETURNED_CANCELLED;
  test_read_status = (3 << 4) | APP_WAITABLE_COMPLETED;
  test_result[0] = test_error[0] = test_message[0] = 0;
}
