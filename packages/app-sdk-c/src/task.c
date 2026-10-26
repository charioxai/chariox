#include "task-internal.h"
#include <stdlib.h>
#include <string.h>

app_string_t cx_string(const char *s) { return (app_string_t){(uint8_t *)s, strlen(s)}; }
bool cx_string_is(app_string_t s, const char *expected) { return s.len == strlen(expected) && !memcmp(s.ptr, expected, s.len); }
const cx_request *cx_task_request(const cx_task *t) { return &t->request; }
void *cx_task_state(cx_task *t) { return t->state; }
const cx_json_doc *cx_task_json(const cx_task *t) { return t->json; }
unsigned cx_task_step(const cx_task *t) { return t->step; }
void cx_task_set_step(cx_task *t, unsigned step) { t->step = step; }

cx_poll cx_task_fail(cx_task *t, const char *code, const char *message) {
  if (t->failed) return CX_DONE;
  t->failed = true;
  t->error = (chariox_app_app_types_app_error_t){.code=cx_string(code), .message=cx_string(message)};
  return CX_DONE;
}
cx_poll cx_task_error(cx_task *t, const chariox_app_app_types_app_error_t *error) {
  if (!t->failed) { t->failed = true; t->error = *error; }
  return CX_DONE;
}
cx_poll cx_task_ok(cx_task *t, const char *json, size_t len) {
  if (json) {
    cx_json_doc *doc = NULL;
    cx_json_status status = cx_json_parse(json, len, &doc);
    cx_json_free(doc);
    if (status != CX_JSON_OK) return cx_task_fail(t, status == CX_JSON_MEMORY ? "APP_MEMORY_LIMIT" : "INVALID_JSON", "result must be bounded interoperable JSON");
  }
  t->has_result = json != NULL; t->result = (app_string_t){(uint8_t *)json, len}; return CX_DONE;
}
void *cx_task_alloc(cx_task *t, size_t size, void (*destroy)(void *)) {
  if (size > SIZE_MAX - sizeof(cx_allocation)) { cx_task_fail(t, "APP_MEMORY_LIMIT", "task allocation is too large"); return NULL; }
  cx_allocation *a = calloc(1, sizeof *a + size);
  if (!a) { cx_task_fail(t, "APP_MEMORY_LIMIT", "task allocation failed"); return NULL; }
  a->destroy = destroy; a->next = t->allocations; t->allocations = a; return a->data;
}
bool cx_subtask_terminal(app_subtask_status_t s) {
  return APP_SUBTASK_STATE(s) == APP_SUBTASK_RETURNED || APP_SUBTASK_STATE(s) == APP_SUBTASK_RETURNED_CANCELLED;
}
cx_poll cx_task_await(cx_task *t, app_subtask_status_t status) {
  if (t->pending) return cx_task_fail(t, "SDK_STATE", "await one import at a time");
  uint32_t handle = APP_SUBTASK_HANDLE(status);
  if (cx_subtask_terminal(status)) {
    if (handle) app_subtask_drop(handle);
    return APP_SUBTASK_STATE(status) == APP_SUBTASK_RETURNED ? CX_READY : cx_task_fail(t, "CANCELLED", "import was cancelled");
  }
  if (!handle) return cx_task_fail(t, "SDK_STATE", "pending import has no handle");
  t->pending = handle; app_waitable_join(handle, t->set); return CX_WAIT;
}
static void close_reader(void *ptr) { cx_reader_close(ptr); }
cx_reader *cx_task_reader(cx_task *t, uint32_t handle, size_t capacity) {
  if (!handle || !capacity || capacity > CX_JSON_MAX_BYTES) {
    if (handle) chariox_app_files_stream_u8_drop_readable(handle);
    cx_task_fail(t, "SDK_STATE", "stream needs a handle and a 1..1MiB buffer"); return NULL;
  }
  cx_reader *r = cx_task_alloc(t, sizeof *r + capacity, close_reader);
  if (!r) { chariox_app_files_stream_u8_drop_readable(handle); return NULL; }
  r->task = t; r->handle = handle; r->capacity = capacity; r->next = t->readers; t->readers = r; return r;
}
void cx_reader_close(cx_reader *r) {
  if (!r->handle) return;
  if (r->blocked) {
    app_waitable_join(r->handle, 0);
    chariox_app_files_stream_u8_cancel_read(r->handle);
    r->blocked = false;
  }
  chariox_app_files_stream_u8_drop_readable(r->handle); r->handle = 0;
}
cx_read_status cx_reader_next(cx_reader *r, const uint8_t **bytes, size_t *count) {
  *bytes = NULL; *count = 0;
  if (!r->handle) return r->failed ? CX_READ_ERROR : CX_READ_END;
  if (r->blocked) return CX_READ_WAIT;
  app_waitable_status_t s = r->ready ? r->status : chariox_app_files_stream_u8_read(r->handle, r->bytes, r->capacity);
  r->ready = false;
  if (s == APP_WAITABLE_STATUS_BLOCKED) { r->blocked = true; app_waitable_join(r->handle, r->task->set); return CX_READ_WAIT; }
  bool ended = APP_WAITABLE_STATE(s) == APP_WAITABLE_DROPPED;
  if (APP_WAITABLE_STATE(s) == APP_WAITABLE_COMPLETED || ended) {
    size_t n = APP_WAITABLE_COUNT(s);
    if (n > r->capacity) {
      r->failed = true;
      cx_task_fail(r->task, "SDK_STATE", "stream returned more bytes than requested"); cx_reader_close(r); return CX_READ_ERROR;
    }
    if (ended) cx_reader_close(r);
    if (!n && ended) return CX_READ_END;
    *count = n; *bytes = r->bytes; return CX_READ_DATA;
  }
  cx_reader_close(r);
  r->failed = true;
  cx_task_fail(r->task, "CANCELLED", "stream read was cancelled");
  return CX_READ_ERROR;
}
