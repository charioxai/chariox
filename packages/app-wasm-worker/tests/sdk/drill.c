#include "chariox/task.h"
#include <string.h>

typedef struct {
  chariox_app_http_result_tuple2_response_head_stream_u8_app_error_t response;
  chariox_app_state_result_option_state_record_app_error_t held;
  chariox_app_http_fetch_args_t fetch;
  cx_reader *reader;
  size_t bytes;
  const char *name;
} state;

static void note(const char *message, const char *name) {
  app_string_t msg=cx_string(message), fields=cx_string(name);
  chariox_app_log_write(CHARIOX_APP_LOG_LEVEL_INFO, &msg, &fields);
}
static bool register_app(chariox_app_app_types_app_error_t *error) {
  (void)error;
  const char *names[] = {"sdk-stream", "sdk-yield", "sdk-cancel-start", "sdk-held", "sdk-before-start"};
  for (size_t i=0; i<5; ++i) { app_string_t name=cx_string(names[i]); chariox_app_registry_tool(&name); }
  return true;
}
static void destroy(void *value) {
  state *s=value;
  chariox_app_http_result_tuple2_response_head_stream_u8_app_error_free(&s->response);
  chariox_app_state_result_option_state_record_app_error_free(&s->held);
  if (s->name) note("sdk.destroy", s->name);
}
static cx_poll resume(cx_task *task) {
  state *s=cx_task_state(task);
  app_string_t name=cx_task_request(task)->name;
  if (!s->name) {
    if (cx_string_is(name,"sdk-yield")) s->name="\"sdk-yield\"";
    else if (cx_string_is(name,"sdk-cancel-start")) s->name="\"sdk-cancel-start\"";
    else if (cx_string_is(name,"sdk-held")) s->name="\"sdk-held\"";
    else s->name="\"sdk-stream\"";
    note("sdk.start",s->name);
  }
  if (cx_string_is(name,"sdk-yield")) return CX_YIELD;
  if (cx_string_is(name,"sdk-cancel-start")) {
    cx_task_await(task,chariox_app_state_get(cx_string("must-not-start"),&s->held));
    return cx_task_fail(task,"EXPECTED","cancel import before yielding");
  }
  if (cx_string_is(name,"sdk-held")) {
    if (!cx_task_step(task)) CX_AWAIT(task,chariox_app_state_get(cx_string("held"),&s->held),1);
    return cx_task_ok(task,"null",4);
  }
  if (!cx_task_step(task)) {
    s->fetch.req.url=cx_string("https://example.com/");
    s->fetch.req.method=cx_string("GET");
    CX_AWAIT(task,chariox_app_http_fetch(&s->fetch,&s->response),1);
  }
  if (s->response.is_err) return cx_task_error(task,&s->response.val.err);
  if (cx_task_step(task)==1) {
    s->reader=cx_task_reader(task,s->response.val.ok.f1,4096);
    cx_task_set_step(task,2);
  }
  for (;;) {
    const uint8_t *bytes; size_t count;
    cx_read_status status=cx_reader_next(s->reader,&bytes,&count);
    if (status==CX_READ_WAIT) return CX_WAIT;
    if (status==CX_READ_ERROR) return CX_DONE;
    if (status==CX_READ_END) break;
    if (count!=3 || memcmp(bytes,"abc",3)) return cx_task_fail(task,"BYTES","wrong final bytes");
    s->bytes+=count;
  }
  cx_reader_close(s->reader); cx_reader_close(s->reader);
  if (s->bytes!=3) return cx_task_fail(task,"BYTES","lost or duplicated final bytes");
  return cx_task_ok(task,"{\"bytes\":3}",11);
}
const cx_app cx_application={sizeof(state),register_app,resume,destroy};
