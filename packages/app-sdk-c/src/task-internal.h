#ifndef CHARIOX_TASK_INTERNAL_H
#define CHARIOX_TASK_INTERNAL_H
#include "chariox/task.h"
#include <stdalign.h>

typedef struct cx_allocation {
  struct cx_allocation *next;
  void (*destroy)(void *);
  max_align_t data[];
} cx_allocation;
struct cx_reader {
  cx_reader *next;
  cx_task *task;
  uint32_t handle;
  size_t capacity;
  bool blocked, ready, failed;
  app_waitable_status_t status;
  uint8_t bytes[];
};
struct cx_task {
  cx_request request;
  cx_json_doc *json;
  app_waitable_set_t set;
  uint32_t cancel, pending;
  bool cancel_reading, cancelled, failed, has_result;
  app_string_t cancel_reason, result;
  chariox_app_app_types_app_error_t error;
  unsigned step;
  void *state;
  cx_allocation *allocations;
  cx_reader *readers;
};
bool cx_subtask_terminal(app_subtask_status_t status);
app_callback_code_t cx_task_start(cx_request request, uint32_t cancel);
app_callback_code_t cx_task_callback(app_event_t *event);
#endif
