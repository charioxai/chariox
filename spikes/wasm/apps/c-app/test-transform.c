// Run the actual tool entry point natively so ASan can detect reads beyond a
// malloc allocation; a Wasm linear-memory bound does not catch those reads.
#include "app.c"
#include <assert.h>

static char returned[512];
static void *context_value;
static char *counter_value;
static bool adjacent_digit;

void app_string_set(app_string_t *ret, const char *s) {
  ret->ptr = (uint8_t *) s;
  ret->len = strlen(s);
}
void app_string_dup(app_string_t *ret, const char *s) { app_string_set(ret, strdup(s)); }
void app_string_free(app_string_t *s) { free(s->ptr); }
void app_context_set_0(void *value) { context_value = value; }
void *app_context_get_0(void) { return context_value; }
app_waitable_set_t app_waitable_set_new(void) { return 1; }
void app_waitable_join(uint32_t waitable, app_waitable_set_t set) { (void) waitable; (void) set; }
void app_waitable_set_drop(app_waitable_set_t set) { (void) set; }
void chariox_app_types_invocation_context_free(chariox_app_types_invocation_context_t *ptr) { (void) ptr; }
app_waitable_status_t exports_chariox_app_handlers_future_string_read(exports_chariox_app_handlers_future_string_t reader, app_string_t *buf) {
  (void) reader; (void) buf;
  return APP_WAITABLE_STATUS_BLOCKED;
}
app_waitable_status_t exports_chariox_app_handlers_future_string_cancel_read(exports_chariox_app_handlers_future_string_t reader) {
  (void) reader;
  return APP_WAITABLE_CANCELLED;
}
void exports_chariox_app_handlers_future_string_drop_readable(exports_chariox_app_handlers_future_string_t reader) { (void) reader; }
void exports_chariox_app_handlers_call_tool_return(exports_chariox_app_handlers_result_json_app_error_t ret) {
  assert(!ret.is_err);
  assert(ret.val.ok.len < sizeof returned);
  memcpy(returned, ret.val.ok.ptr, ret.val.ok.len);
  returned[ret.val.ok.len] = '\0';
}

// Transform must not call any broker. Keep other tools linkable and fail if
// dispatch accidentally reaches one of their imports.
#define UNUSED_IMPORT(result, name, args) result name args { abort(); }
UNUSED_IMPORT(app_subtask_status_t, app_subtask_cancel, (app_subtask_t subtask))
UNUSED_IMPORT(void, app_subtask_drop, (app_subtask_t subtask))
UNUSED_IMPORT(app_subtask_status_t, wasi_clocks_monotonic_clock_wait_for, (wasi_clocks_monotonic_clock_duration_t duration))
UNUSED_IMPORT(void, chariox_app_registry_tool, (app_string_t *name))
app_subtask_status_t chariox_app_state_get(app_string_t key, chariox_app_state_result_option_state_record_app_error_t *result) {
  assert(key.len == 5 && memcmp(key.ptr, "count", 5) == 0);
  counter_value = malloc(adjacent_digit ? 4 : 2);
  memcpy(counter_value, adjacent_digit ? "419" : "41", adjacent_digit ? 4 : 2);
  result->is_err = false;
  result->val.ok.is_some = true;
  result->val.ok.val.value.ptr = (uint8_t *) counter_value;
  result->val.ok.val.value.len = 2;
  result->val.ok.val.version = 7;
  return APP_SUBTASK_RETURNED;
}
app_subtask_status_t chariox_app_state_transact(chariox_app_state_transaction_t *tx, chariox_app_state_result_committed_app_error_t *result) {
  assert(tx->checks.len == 1 && tx->checks.ptr[0].version.val == 7);
  assert(tx->writes.len == 1);
  app_string_t value = tx->writes.ptr[0].val.put.f1;
  assert(value.len == 2 && memcmp(value.ptr, "42", 2) == 0);
  result->is_err = false;
  result->val.ok.revision = 8;
  return APP_SUBTASK_RETURNED;
}
UNUSED_IMPORT(app_subtask_status_t, chariox_app_http_send, (chariox_app_http_request_t *request, chariox_app_http_result_response_app_error_t *result))
UNUSED_IMPORT(app_subtask_status_t, chariox_app_http_fetch, (chariox_app_http_request_t *request, chariox_app_http_result_tuple2_response_head_stream_u8_app_error_t *result))
UNUSED_IMPORT(app_waitable_status_t, chariox_app_http_stream_u8_read, (chariox_app_http_stream_u8_t reader, uint8_t *buf, size_t amt))
UNUSED_IMPORT(app_waitable_status_t, chariox_app_http_stream_u8_cancel_read, (chariox_app_http_stream_u8_t reader))
UNUSED_IMPORT(void, chariox_app_http_stream_u8_drop_readable, (chariox_app_http_stream_u8_t reader))
UNUSED_IMPORT(void, chariox_app_log_write, (chariox_app_log_level_t level, app_string_t *message, chariox_app_log_json_t *fields))
UNUSED_IMPORT(void, chariox_app_types_app_error_free, (chariox_app_types_app_error_t *error))

static void check_tool(const char *tool, const char *input, const char *expected) {
  app_string_t name, json;
  app_string_dup(&name, tool);
  app_string_dup(&json, input);
  exports_chariox_app_handlers_invocation_context_t context = {0};
  returned[0] = '\0';
  assert(exports_chariox_app_handlers_call_tool(&name, &json, &context, 2) == APP_CALLBACK_CODE_EXIT);
  assert(strcmp(returned, expected) == 0);
}

int main(void) {
  for (unsigned round = 0; round < 3; round++) {
    check_tool("transform", "{\"n\":0}", "{\"sum\":0,\"inBytes\":2,\"outBytes\":2}");
    check_tool("transform", "{\"n\":1}", "{\"sum\":0,\"inBytes\":57,\"outBytes\":2}");
    check_tool("transform", "{\"n\":10,\"rounds\":3}", "{\"sum\":351,\"inBytes\":570,\"outBytes\":126}");
    check_tool("transform", "{\"n\":1000,\"rounds\":3}", "{\"sum\":36750,\"inBytes\":60681,\"outBytes\":14103}");
  }
  check_tool("counter", "{}", "{\"count\":42,\"revision\":8}");
  adjacent_digit = true;
  check_tool("counter", "{}", "{\"count\":42,\"revision\":8}");
  puts("transform and counter: empty, single, filtered and repeated inputs passed under ASan/UBSan");
}
