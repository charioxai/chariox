#include "chariox/task.h"
#include <string.h>

/* Each call gets zeroed state. Nothing that survives an await is on the stack. */
typedef struct {
  chariox_app_state_result_option_state_record_app_error_t got;
  chariox_app_state_result_committed_app_error_t committed;
  chariox_app_state_check_t check;
  chariox_app_state_write_t write;
  chariox_app_state_transaction_t tx;
  uint64_t next;
  char value[32], result[128];
} counter;

static void destroy(void *state) {
  counter *s = state;
  chariox_app_state_result_option_state_record_app_error_free(&s->got);
  chariox_app_state_result_committed_app_error_free(&s->committed);
}
static bool register_app(chariox_app_app_types_app_error_t *error) {
  (void)error;
  const char *tools[] = {"echo", "counter", "wait"};
  for (size_t i = 0; i < sizeof tools / sizeof tools[0]; i++) {
    app_string_t name = cx_string(tools[i]); chariox_app_registry_tool(&name);
  }
  return true;
}
static cx_poll resume(cx_task *task) {
  const cx_request *r = cx_task_request(task);
  counter *s = cx_task_state(task);
  if (r->kind == CX_LIFECYCLE) return cx_task_ok(task, NULL, 0);
  if (r->kind != CX_TOOL) return cx_task_fail(task, "NOT_REGISTERED", "no event or wake handler registered");
  if (cx_string_is(r->name, "echo")) return cx_task_ok(task, (const char *)r->input.ptr, r->input.len);
  if (cx_string_is(r->name, "wait")) {
    if (cx_task_step(task) == 0) {
      uint64_t ms;
      if (!cx_json_u64(cx_json_get(cx_task_json(task), cx_json_root(cx_task_json(task)), "ms"), &ms) || ms > 30000)
        return cx_task_fail(task, "INVALID_INPUT", "ms must be an integer from 0 through 30000");
      CX_AWAIT(task, wasi_clocks_0_3_0_monotonic_clock_wait_for(ms * UINT64_C(1000000)), 1);
    }
    return cx_task_ok(task, "{\"waited\":true}", 15);
  }
  if (!cx_string_is(r->name, "counter")) return cx_task_fail(task, "TOOL_NOT_FOUND", "unknown tool");
  if (cx_task_step(task) == 0) CX_AWAIT(task, chariox_app_state_get(cx_string("count"), &s->got), 1);
  if (cx_task_step(task) == 1) {
    if (s->got.is_err) return cx_task_error(task, &s->got.val.err);
    uint64_t count = 0;
    if (s->got.val.ok.is_some) {
      cx_json_doc *stored = NULL;
      app_string_t v = s->got.val.ok.val.value;
      cx_json_status parsed = cx_json_parse((const char *)v.ptr, v.len, &stored);
      bool valid = parsed == CX_JSON_OK && cx_json_u64(cx_json_root(stored), &count);
      cx_json_free(stored);
      if (!valid) return cx_task_fail(task, "INVALID_STATE", "count must be an interoperable nonnegative integer");
    }
    if (count == CX_JSON_MAX_SAFE_INTEGER) return cx_task_fail(task, "INVALID_STATE", "counter reached the JSON integer limit");
    s->next = count + 1;
    cx_json_write_u64(s->value, sizeof s->value, s->next);
    s->check = (chariox_app_state_check_t){.key=cx_string("count"), .version={.is_some=s->got.val.ok.is_some, .val=s->got.val.ok.val.version}};
    s->write.tag = CHARIOX_APP_STATE_WRITE_PUT;
    s->write.val.put.f0 = cx_string("count"); s->write.val.put.f1 = cx_string(s->value);
    s->tx = (chariox_app_state_transaction_t){.schema_version=1, .checks={&s->check,1}, .writes={&s->write,1}};
    /* A conflict is returned to the caller; this SDK never replays a write. */
    CX_AWAIT(task, chariox_app_state_transact(&s->tx, &s->committed), 2);
  }
  if (s->committed.is_err) return cx_task_error(task, &s->committed.val.err);
  if (s->committed.val.ok.revision > CX_JSON_MAX_SAFE_INTEGER)
    return cx_task_fail(task, "INVALID_STATE", "revision exceeds JSON integer range");
  memcpy(s->result, "{\"count\":", 9);
  size_t len = 9 + cx_json_write_u64(s->result + 9, sizeof s->result - 9, s->next);
  memcpy(s->result + len, ",\"revision\":", 12); len += 12;
  len += cx_json_write_u64(s->result + len, sizeof s->result - len, s->committed.val.ok.revision);
  s->result[len++] = '}';
  return cx_task_ok(task, s->result, len);
}
const cx_app cx_application = {sizeof(counter), register_app, resume, destroy};
