#ifndef CHARIOX_C_TEST_ABI_H
#define CHARIOX_C_TEST_ABI_H
#include "chariox/task.h"
#include <assert.h>
#include <stdlib.h>
#include <string.h>
#define TEST_HANDLES 512u
extern void *test_context;
extern unsigned test_returns, test_destroys, test_cancel_calls, test_import_drops, test_stream_drops;
extern bool test_return_error, test_cancel_immediate, test_read_immediate, test_cancel_reason_race;
extern app_subtask_status_t test_cancel_status;
extern app_waitable_status_t test_read_status;
extern app_waitable_set_t test_joined[TEST_HANDLES];
extern bool test_future_active[TEST_HANDLES], test_stream_active[TEST_HANDLES];
extern app_string_t *test_future_buffers[TEST_HANDLES];
extern uint8_t *test_stream_buffer;
extern char test_result[1024], test_error[128], test_message[256];
app_string_t test_string(const char *text);
app_callback_code_t test_call(const char *name, const char *input, uint32_t cancel);
app_callback_code_t test_event(app_event_code_t event, uint32_t handle, uint32_t status);
void test_reset(void);
#endif
