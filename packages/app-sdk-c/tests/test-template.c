#include "abi.h"
#include <stdio.h>
extern long cx_test_fail_after;
static uint64_t count = 41, version = 7;
static const char *stored = "41";
static unsigned registrations, transactions, gets, next_handle = 16;
static bool pending_get, pending_tx, get_error, tx_error;
static chariox_app_state_result_option_state_record_app_error_t *pending_results[TEST_HANDLES];
static chariox_app_state_result_committed_app_error_t *pending_commit;
static chariox_app_state_transaction_t *pending_transaction;
static app_subtask_status_t timer_status = APP_SUBTASK_RETURNED;

void chariox_app_registry_tool(app_string_t *name) { assert(cx_string_is(*name,"echo") || cx_string_is(*name,"counter") || cx_string_is(*name,"wait")); registrations++; }
static void error(chariox_app_app_types_app_error_t *err, const char *code) { *err = (chariox_app_app_types_app_error_t){.code=test_string(code), .message=test_string("broker refusal"), .retryable={true,true}}; }
static void free_error(chariox_app_app_types_app_error_t *err) { app_string_free(&err->code); app_string_free(&err->message); }
void chariox_app_state_result_option_state_record_app_error_free(chariox_app_state_result_option_state_record_app_error_t *r) {
  if (r->is_err) free_error(&r->val.err); else if (r->val.ok.is_some) app_string_free(&r->val.ok.val.value);
}
void chariox_app_state_result_committed_app_error_free(chariox_app_state_result_committed_app_error_t *r) {
  if (r->is_err) free_error(&r->val.err);
  else {
    for (size_t i=0; i<r->val.ok.receipts.len; i++) { app_string_free(&r->val.ok.receipts.ptr[i].receipt_id); app_string_free(&r->val.ok.receipts.ptr[i].state); }
    free(r->val.ok.receipts.ptr);
  }
}
static void fill_get(chariox_app_state_result_option_state_record_app_error_t *out) {
  out->is_err = get_error;
  if (get_error) error(&out->val.err,"GET_REFUSED");
  else if (stored) { out->val.ok.is_some = true; out->val.ok.val.value = test_string(stored); out->val.ok.val.version = version; }
}
app_subtask_status_t chariox_app_state_get(app_string_t key, chariox_app_state_result_option_state_record_app_error_t *out) {
  assert(cx_string_is(key,"count")); gets++;
  if (pending_get) { unsigned handle = next_handle++; assert(handle<TEST_HANDLES); pending_results[handle] = out; return (handle<<4) | APP_SUBTASK_STARTED; }
  fill_get(out); return APP_SUBTASK_RETURNED;
}
static void transact(chariox_app_state_transaction_t *tx, chariox_app_state_result_committed_app_error_t *out) {
  assert(tx->schema_version == 1 && tx->checks.len == 1 && tx->writes.len == 1);
  assert(cx_string_is(tx->checks.ptr[0].key,"count"));
  if (tx_error || (tx->checks.ptr[0].version.is_some && tx->checks.ptr[0].version.val != version)) {
    out->is_err = true; error(&out->val.err,"STATE_CONFLICT"); return;
  }
  assert(tx->writes.ptr[0].tag == CHARIOX_APP_STATE_WRITE_PUT);
  app_string_t value = tx->writes.ptr[0].val.put.f1;
  cx_json_doc *doc = NULL;
  assert(cx_json_parse((const char *)value.ptr,value.len,&doc) == CX_JSON_OK);
  assert(cx_json_u64(cx_json_root(doc),&count)); cx_json_free(doc);
  out->val.ok.revision = ++version;
  out->val.ok.receipts.ptr = calloc(1,sizeof *out->val.ok.receipts.ptr); out->val.ok.receipts.len = 1;
  out->val.ok.receipts.ptr[0].receipt_id = test_string("receipt"); out->val.ok.receipts.ptr[0].state = test_string("accepted");
}
app_subtask_status_t chariox_app_state_transact(chariox_app_state_transaction_t *tx, chariox_app_state_result_committed_app_error_t *out) {
  transactions++;
  if (pending_tx) { pending_commit = out; pending_transaction = tx; return (7<<4)|APP_SUBTASK_STARTED; }
  transact(tx,out); return APP_SUBTASK_RETURNED;
}
app_subtask_status_t wasi_clocks_0_3_0_monotonic_clock_wait_for(uint64_t ns) { assert(ns <= UINT64_C(30000000000)); return timer_status; }
static void expect(const char *tool, const char *input, const char *expected) {
  test_reset(); assert(test_call(tool,input,2)==APP_CALLBACK_CODE_EXIT); assert(!test_return_error); assert(!strcmp(test_result,expected)); assert(test_returns==1);
}
static void expect_error(const char *tool, const char *input, const char *code) {
  test_reset(); assert(test_call(tool,input,2)==APP_CALLBACK_CODE_EXIT); assert(test_return_error); assert(!strcmp(test_error,code)); assert(test_returns==1);
}
int main(void) {
  chariox_app_app_types_app_error_t err;
  assert(exports_chariox_app_handlers_register(&err)); assert(registrations==3);
  expect("echo","{\"text\":\"quotes\\\" and 😀\"}","{\"text\":\"quotes\\\" and 😀\"}");
  expect("counter","{}","{\"count\":42,\"revision\":8}");
  expect("wait","{\"ms\":0}","{\"waited\":true}");
  expect_error("wait","{\"ms\":30001}","INVALID_INPUT");
  expect_error("wait","{\"ms\":0.5}","INVALID_INPUT");
  expect_error("missing","{}","TOOL_NOT_FOUND");
  expect_error("echo","{\"a\":1,\"a\":2}","INVALID_JSON");
  stored = "9007199254740991"; expect_error("counter","{}","INVALID_STATE");
  stored = "{\"count\":3}"; expect_error("counter","{}","INVALID_STATE");
  stored = "invalid"; expect_error("counter","{}","INVALID_STATE");
  stored = NULL; expect("counter","{}","{\"count\":1,\"revision\":9}"); stored = "41";
  get_error = true; expect_error("counter","{}","GET_REFUSED"); get_error = false;
  tx_error = true; expect_error("counter","{}","STATE_CONFLICT"); tx_error = false;
  test_reset(); pending_get = true;
  unsigned handle = next_handle;
  assert(test_call("counter","{}",2)!=APP_CALLBACK_CODE_EXIT);
  test_cancel_status = APP_SUBTASK_STARTED;
  assert(test_event(APP_EVENT_FUTURE_READ,2,APP_WAITABLE_COMPLETED)!=APP_CALLBACK_CODE_EXIT);
  unsigned before = transactions; fill_get(pending_results[handle]);
  assert(test_event(APP_EVENT_SUBTASK,handle,APP_SUBTASK_RETURNED)==APP_CALLBACK_CODE_EXIT);
  assert(test_return_error && !strcmp(test_error,"CANCELLED") && transactions==before);
  test_reset(); pending_get = false; pending_tx = true;
  assert(test_call("counter","{}",2)!=APP_CALLBACK_CODE_EXIT);
  test_cancel_status = APP_SUBTASK_STARTED;
  assert(test_event(APP_EVENT_FUTURE_READ,2,APP_WAITABLE_COMPLETED)!=APP_CALLBACK_CODE_EXIT);
  /* Arguments and result buffers still exist; cancellation never retries effects. */
  assert(cx_string_is(pending_transaction->writes.ptr[0].val.put.f1,"42"));
  transact(pending_transaction,pending_commit);
  before = transactions;
  assert(test_event(APP_EVENT_SUBTASK,7,APP_SUBTASK_RETURNED)==APP_CALLBACK_CODE_EXIT);
  assert(test_return_error && !strcmp(test_error,"CANCELLED") && transactions==before);
  pending_tx = false;
  test_reset(); timer_status = (7<<4)|APP_SUBTASK_STARTED;
  assert(test_call("wait","{\"ms\":1000}",2)!=APP_CALLBACK_CODE_EXIT);
  assert(test_event(APP_EVENT_FUTURE_READ,2,APP_WAITABLE_COMPLETED)==APP_CALLBACK_CODE_EXIT);
  assert(test_return_error && test_import_drops==1); timer_status = APP_SUBTASK_RETURNED;
  test_reset(); pending_get = true;
  void *tasks[64]; unsigned handles[64];
  for (unsigned i=0;i<64;i++) { handles[i]=next_handle; assert(test_call("counter","{}",128+i)!=APP_CALLBACK_CODE_EXIT); tasks[i]=test_context; fill_get(pending_results[handles[i]]); }
  unsigned success=0, conflicts=0;
  for (unsigned i=0;i<64;i++) {
    app_context_set_0(tasks[i]); assert(test_event(APP_EVENT_SUBTASK,handles[i],APP_SUBTASK_RETURNED)==APP_CALLBACK_CODE_EXIT);
    if (test_return_error) { assert(!strcmp(test_error,"STATE_CONFLICT")); conflicts++; } else success++;
  }
  assert(success==1 && conflicts==63); pending_get = false;
  for (long i=0;i<24;i++) {
    test_reset(); cx_test_fail_after=i;
    assert(test_call("echo","{\"text\":\"ok\"}",2)==APP_CALLBACK_CODE_EXIT);
    assert(test_returns==1 && !test_context);
    if (test_return_error) assert(!strcmp(test_error,"APP_MEMORY_LIMIT"));
    cx_test_fail_after=-1;
  }
  test_reset();
  app_string_t lifecycle = test_string("start"); chariox_app_app_types_invocation_context_t ctx={0};
  assert(exports_chariox_app_handlers_lifecycle(&lifecycle,NULL,&ctx,2)==APP_CALLBACK_CODE_EXIT); assert(!test_return_error);
  test_reset();
  app_string_t event=test_string("missing"), id=test_string("occurrence"), payload=test_string("{}");
  assert(exports_chariox_app_handlers_deliver_event(&event,&id,&payload,&ctx,2)==APP_CALLBACK_CODE_EXIT); assert(test_return_error);
  test_reset();
  chariox_app_app_types_wake_t wake={.id=test_string("wake"),.revision={true,test_string("rev")}};
  assert(exports_chariox_app_handlers_deliver_wake(&wake,true,&ctx,2)==APP_CALLBACK_CODE_EXIT); assert(test_return_error);
  test_reset();
  printf("template: registration, echo/counter/wait, conflicts, 64 calls, cancel races, 24 OOM positions, lifecycle/event/wake passed (%u reads)\n",gets);
}
