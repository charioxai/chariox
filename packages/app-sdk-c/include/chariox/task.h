#ifndef CHARIOX_TASK_H
#define CHARIOX_TASK_H
#include "app.h"
#include "chariox/json.h"
#ifdef __cplusplus
extern "C" {
#endif

typedef struct cx_task cx_task;
typedef struct cx_reader cx_reader;
/* READY is only an await result. resume returns WAIT, YIELD or DONE.
 * YIELD is valid only without a pending import. */
typedef enum { CX_READY, CX_WAIT, CX_YIELD, CX_DONE } cx_poll;
typedef enum { CX_TOOL, CX_EVENT, CX_LIFECYCLE, CX_WAKE } cx_call_kind;
typedef struct {
  cx_call_kind kind;
  app_string_t name, input, occurrence_id;
  bool has_input, overdue;
  chariox_app_app_types_invocation_context_t context;
  chariox_app_app_types_wake_t wake;
} cx_request;
typedef struct {
  size_t state_size;
  bool (*register_app)(chariox_app_app_types_app_error_t *error);
  cx_poll (*resume)(cx_task *task);
  void (*destroy)(void *state);
} cx_app;
/* The application defines this table; the SDK implements every R1 export. */
extern const cx_app cx_application;

app_string_t cx_string(const char *text); /* borrowed, never free it */
bool cx_string_is(app_string_t text, const char *expected);
const cx_request *cx_task_request(const cx_task *task);
void *cx_task_state(cx_task *task);
const cx_json_doc *cx_task_json(const cx_task *task);
unsigned cx_task_step(const cx_task *task);
void cx_task_set_step(cx_task *task, unsigned step);
/* Zeroed allocations and destructors live until all pending imports terminate.
 * The application state destructor runs first. Allocation destructors then run
 * in reverse allocation order before freeing each allocation. */
void *cx_task_alloc(cx_task *task, size_t bytes, void (*destroy)(void *));
cx_poll cx_task_await(cx_task *task, app_subtask_status_t status);
#define CX_AWAIT(task, expression, next_step) do { \
  cx_task_set_step((task), (next_step)); \
  cx_poll cx_await_result_ = cx_task_await((task), (expression)); \
  if (cx_await_result_ != CX_READY) return cx_await_result_; \
} while (0)
/* Borrow the result through return; keep it in task state or task allocations.
 * NULL is a successful absent lifecycle result / void wake result. */
cx_poll cx_task_ok(cx_task *task, const char *json, size_t len);
cx_poll cx_task_fail(cx_task *task, const char *code, const char *message);
cx_poll cx_task_error(cx_task *task, const chariox_app_app_types_app_error_t *error);

/* Byte streams from the R1 HTTP/files broker share this generated stream type.
 * Takes ownership even on allocation failure; the task closes it on all exits. */
cx_reader *cx_task_reader(cx_task *task, chariox_app_files_stream_u8_t handle, size_t capacity);
typedef enum { CX_READ_DATA, CX_READ_END, CX_READ_WAIT, CX_READ_ERROR } cx_read_status;
cx_read_status cx_reader_next(cx_reader *reader, const uint8_t **bytes, size_t *count);
void cx_reader_close(cx_reader *reader);
#ifdef __cplusplus
}
#endif
#endif
