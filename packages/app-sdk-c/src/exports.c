#include "task-internal.h"

bool exports_chariox_app_handlers_register(exports_chariox_app_handlers_app_error_t *error) {
  return cx_application.register_app(error);
}
app_callback_code_t exports_chariox_app_handlers_call_tool(app_string_t *name, app_string_t *input, exports_chariox_app_handlers_invocation_context_t *context, uint32_t cancel) {
  return cx_task_start((cx_request){.kind=CX_TOOL, .name=*name, .input=*input, .has_input=true, .context=*context}, cancel);
}
app_callback_code_t exports_chariox_app_handlers_deliver_event(app_string_t *name, app_string_t *id, app_string_t *payload, exports_chariox_app_handlers_invocation_context_t *context, uint32_t cancel) {
  return cx_task_start((cx_request){.kind=CX_EVENT, .name=*name, .input=*payload, .has_input=true, .occurrence_id=*id, .context=*context}, cancel);
}
app_callback_code_t exports_chariox_app_handlers_lifecycle(app_string_t *name, app_string_t *data, exports_chariox_app_handlers_invocation_context_t *context, uint32_t cancel) {
  return cx_task_start((cx_request){.kind=CX_LIFECYCLE, .name=*name, .input=data ? *data : (app_string_t){0}, .has_input=data != NULL, .context=*context}, cancel);
}
app_callback_code_t exports_chariox_app_handlers_deliver_wake(exports_chariox_app_handlers_wake_t *wake, bool overdue, exports_chariox_app_handlers_invocation_context_t *context, uint32_t cancel) {
  return cx_task_start((cx_request){.kind=CX_WAKE, .wake=*wake, .overdue=overdue, .context=*context}, cancel);
}
app_callback_code_t exports_chariox_app_handlers_call_tool_callback(app_event_t *event) { return cx_task_callback(event); }
app_callback_code_t exports_chariox_app_handlers_deliver_event_callback(app_event_t *event) { return cx_task_callback(event); }
app_callback_code_t exports_chariox_app_handlers_lifecycle_callback(app_event_t *event) { return cx_task_callback(event); }
app_callback_code_t exports_chariox_app_handlers_deliver_wake_callback(app_event_t *event) { return cx_task_callback(event); }
