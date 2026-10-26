#include "task-internal.h"
#include <stdlib.h>

static void release(cx_task *t) {
  for (cx_reader *r = t->readers; r; r = r->next) cx_reader_close(r);
  if (t->cancel) {
    app_waitable_join(t->cancel, 0);
    if (t->cancel_reading) {
      /* A racing read can still deliver an allocated cancellation reason. */
      exports_chariox_app_handlers_future_string_cancel_read(t->cancel);
    }
    exports_chariox_app_handlers_future_string_drop_readable(t->cancel);
  }
  app_string_free(&t->cancel_reason);
  if (cx_application.destroy && t->state) cx_application.destroy(t->state);
  while (t->allocations) {
    cx_allocation *a = t->allocations; t->allocations = a->next;
    if (a->destroy) a->destroy(a->data);
    free(a);
  }
  cx_json_free(t->json);
  app_string_free(&t->request.name); app_string_free(&t->request.input); app_string_free(&t->request.occurrence_id);
  chariox_app_app_types_invocation_context_free(&t->request.context);
  chariox_app_app_types_wake_free(&t->request.wake);
  if (t->set) app_waitable_set_drop(t->set);
  app_context_set_0(NULL);
}
static app_callback_code_t finish(cx_task *t) {
  if (!t->failed && (t->request.kind == CX_TOOL || t->request.kind == CX_EVENT) && !t->has_result)
    cx_task_fail(t, "SDK_STATE", "tool/event requires a JSON result");
  switch (t->request.kind) {
  case CX_TOOL: case CX_EVENT: {
    exports_chariox_app_handlers_result_json_app_error_t result = {.is_err=t->failed};
    if (t->failed) result.val.err = t->error; else result.val.ok = t->result;
    if (t->request.kind == CX_TOOL) exports_chariox_app_handlers_call_tool_return(result);
    else exports_chariox_app_handlers_deliver_event_return(result);
    break;
  }
  case CX_LIFECYCLE: {
    exports_chariox_app_handlers_result_option_json_app_error_t result = {.is_err=t->failed};
    if (t->failed) result.val.err = t->error;
    else { result.val.ok.is_some = t->has_result; result.val.ok.val = t->result; }
    exports_chariox_app_handlers_lifecycle_return(result); break;
  }
  case CX_WAKE: {
    exports_chariox_app_handlers_result_void_app_error_t result = {.is_err=t->failed};
    if (t->failed) result.val.err = t->error;
    exports_chariox_app_handlers_deliver_wake_return(result); break;
  }
  }
  release(t); return APP_CALLBACK_CODE_EXIT;
}
static app_callback_code_t cancel(cx_task *t);
static app_callback_code_t drive(cx_task *t) {
  if (t->cancelled) {
    if (t->pending) return APP_CALLBACK_CODE_WAIT(t->set);
    if (!t->failed) {
      cx_task_fail(t, "CANCELLED", "call cancelled");
      if (t->cancel_reason.len) t->error.message = t->cancel_reason;
    }
    app_callback_code_t code = finish(t); free(t); return code;
  }
  /* A ready stream must not advance an application's import continuation. */
  if (t->pending) return APP_CALLBACK_CODE_WAIT(t->set);
  cx_poll poll = t->failed ? CX_DONE : cx_application.resume(t);
  if (poll == CX_READY) cx_task_fail(t, "SDK_STATE", "resume must return WAIT, YIELD or DONE");
  if (poll == CX_YIELD && !t->failed) {
    if (!t->pending) return APP_CALLBACK_CODE_YIELD;
    cx_task_fail(t, "SDK_STATE", "cannot yield while awaiting an import");
  }
  if (poll == CX_WAIT && !t->failed) {
    bool waiting = t->pending != 0;
    for (cx_reader *r = t->readers; r; r = r->next) waiting |= r->blocked;
    if (waiting) return APP_CALLBACK_CODE_WAIT(t->set);
    cx_task_fail(t, "SDK_STATE", "waiting task has no pending work");
  }
  if (t->pending) {
    /* Even an App error cannot free an import's argument/result storage early. */
    cx_task_fail(t, "SDK_STATE", "resume completed while awaiting an import");
    return cancel(t);
  }
  app_callback_code_t code = finish(t); free(t); return code;
}
static app_callback_code_t cancel(cx_task *t) {
  if (t->cancelled) return drive(t);
  t->cancelled = true;
  for (cx_reader *r = t->readers; r; r = r->next) cx_reader_close(r);
  if (t->pending) {
    app_waitable_join(t->pending, 0);
    app_subtask_status_t s = app_subtask_cancel(t->pending);
    if (cx_subtask_terminal(s)) { app_subtask_drop(t->pending); t->pending = 0; }
    else app_waitable_join(t->pending, t->set);
  }
  return drive(t);
}
app_callback_code_t cx_task_start(cx_request request, uint32_t cancel_handle) {
  cx_task *t = calloc(1, sizeof *t);
  if (!t) {
    cx_task local = {.request=request, .cancel=cancel_handle};
    cx_task_fail(&local, "APP_MEMORY_LIMIT", "task allocation failed");
    return finish(&local);
  }
  t->request = request; t->cancel = cancel_handle; t->set = app_waitable_set_new(); app_context_set_0(t);
  if (cx_application.state_size) t->state = cx_task_alloc(t, cx_application.state_size, NULL);
  if (request.has_input && !t->failed) {
    cx_json_status status = cx_json_parse((const char *)request.input.ptr, request.input.len, &t->json);
    if (status != CX_JSON_OK) cx_task_fail(t, status == CX_JSON_MEMORY ? "APP_MEMORY_LIMIT" : "INVALID_JSON", "input must be bounded interoperable JSON");
  }
  app_waitable_status_t s = exports_chariox_app_handlers_future_string_read(t->cancel, &t->cancel_reason);
  if (s == APP_WAITABLE_STATUS_BLOCKED) { t->cancel_reading = true; app_waitable_join(t->cancel, t->set); }
  else if (APP_WAITABLE_STATE(s) == APP_WAITABLE_COMPLETED) return cancel(t);
  return drive(t);
}
app_callback_code_t cx_task_callback(app_event_t *event) {
  cx_task *t = app_context_get_0();
  if (event->event == APP_EVENT_NONE) return drive(t);
  if (event->event == APP_EVENT_CANCEL) return cancel(t);
  if (event->event == APP_EVENT_FUTURE_READ && event->waitable == t->cancel) {
    app_waitable_join(t->cancel, 0); t->cancel_reading = false;
    if (APP_WAITABLE_STATE(event->code) == APP_WAITABLE_COMPLETED) return cancel(t);
    return APP_CALLBACK_CODE_WAIT(t->set);
  }
  if (event->event == APP_EVENT_SUBTASK && event->waitable == t->pending) {
    if (!cx_subtask_terminal(event->code)) return APP_CALLBACK_CODE_WAIT(t->set);
    app_waitable_join(t->pending, 0); app_subtask_drop(t->pending); t->pending = 0;
    if (APP_SUBTASK_STATE(event->code) == APP_SUBTASK_RETURNED_CANCELLED) t->cancelled = true;
    return drive(t);
  }
  if (event->event == APP_EVENT_STREAM_READ) for (cx_reader *r = t->readers; r; r = r->next) {
    if (r->handle == event->waitable && r->blocked) {
      app_waitable_join(r->handle, 0); r->blocked = false; r->ready = true; r->status = event->code; return drive(t);
    }
  }
  return APP_CALLBACK_CODE_WAIT(t->set);
}
