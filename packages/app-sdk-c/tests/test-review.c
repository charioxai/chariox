#include "abi.h"
#include <stdio.h>
extern long cx_test_fail_after;
typedef struct { cx_reader *reader; } state;
static unsigned resumes;
static bool register_app(chariox_app_app_types_app_error_t *err) { (void)err; return true; }
static cx_poll resume(cx_task *t) {
  resumes++;
  state *s=cx_task_state(t);
  app_string_t name=cx_task_request(t)->name;
  if(cx_string_is(name,"ready")) return CX_READY;
  if(cx_string_is(name,"first-error")) {
    cx_task_fail(t,"FIRST","first error"); return cx_task_fail(t,"SECOND","second error");
  }
  if(cx_string_is(name,"complete") || cx_string_is(name,"yield")) {
    assert(cx_task_await(t,(7<<4)|APP_SUBTASK_STARTED)==CX_WAIT);
    return cx_string_is(name,"yield") ? CX_YIELD : cx_task_ok(t,"true",4);
  }
  if(cx_task_step(t)==0) {
    s->reader=cx_task_reader(t,8,4096);
    if(!s->reader) return CX_DONE;
    const uint8_t *bytes;size_t count;
    assert(cx_reader_next(s->reader,&bytes,&count)==CX_READ_WAIT);
    cx_task_set_step(t,1);
    if(cx_string_is(name,"mixed")) return cx_task_await(t,(7<<4)|APP_SUBTASK_STARTED);
    return CX_WAIT;
  }
  const uint8_t *bytes;size_t count;
  cx_read_status status=cx_reader_next(s->reader,&bytes,&count);
  if(status==CX_READ_ERROR) {
    assert(cx_reader_next(s->reader,&bytes,&count)==CX_READ_ERROR);
    return CX_DONE;
  }
  assert(status==CX_READ_DATA && count==3 && !memcmp(bytes,"abc",3));
  return cx_task_ok(t,"true",4);
}
const cx_app cx_application={sizeof(state),register_app,resume,NULL};
static void check(const char *name) {
  test_reset();resumes=0;
  if(!strcmp(name,"mixed")) {
    assert(test_call("mixed","{}",2)!=APP_CALLBACK_CODE_EXIT);
    memcpy(test_stream_buffer,"abc",3);
    assert(test_event(APP_EVENT_STREAM_READ,8,(3<<4)|APP_WAITABLE_COMPLETED)!=APP_CALLBACK_CODE_EXIT);
    assert(resumes==1 && test_returns==0 && test_cancel_calls==0);
    assert(test_event(APP_EVENT_SUBTASK,7,APP_SUBTASK_RETURNED)==APP_CALLBACK_CODE_EXIT);
    assert(resumes==2 && !test_return_error);
  } else if(!strcmp(name,"complete") || !strcmp(name,"yield")) {
    assert(test_call(name,"{}",2)==APP_CALLBACK_CODE_EXIT);
    assert(test_return_error && !strcmp(test_error,"SDK_STATE"));
    test_reset();resumes=0;test_cancel_status=APP_SUBTASK_STARTED_CANCELLED;
    assert(test_call(name,"{}",2)!=APP_CALLBACK_CODE_EXIT);
    assert(test_event(APP_EVENT_SUBTASK,7,APP_SUBTASK_RETURNED_CANCELLED)==APP_CALLBACK_CODE_EXIT);
    assert(test_return_error && !strcmp(test_error,"SDK_STATE"));
  } else if(!strcmp(name,"ready")) {
    app_string_t event=test_string("ready");chariox_app_app_types_invocation_context_t ctx={0};
    assert(exports_chariox_app_handlers_lifecycle(&event,NULL,&ctx,2)==APP_CALLBACK_CODE_EXIT);
    assert(test_return_error && !strcmp(test_error,"SDK_STATE"));
  } else if(!strcmp(name,"first-error")) {
    assert(test_call(name,"{}",2)==APP_CALLBACK_CODE_EXIT);
    assert(test_return_error && !strcmp(test_error,"FIRST"));
    test_reset();cx_test_fail_after=1;
    assert(test_call("mixed","bad JSON",2)==APP_CALLBACK_CODE_EXIT);
    assert(test_return_error && !strcmp(test_error,"APP_MEMORY_LIMIT"));cx_test_fail_after=-1;
  } else if(!strcmp(name,"reader-error")) {
    assert(test_call("reader-error","{}",2)!=APP_CALLBACK_CODE_EXIT);
    assert(test_event(APP_EVENT_STREAM_READ,8,APP_WAITABLE_CANCELLED)==APP_CALLBACK_CODE_EXIT);
    assert(test_return_error && !strcmp(test_error,"CANCELLED"));
  } else if(!strcmp(name,"reader-memory")) {
    for(long i=0;i<16;i++) {
      test_reset();cx_test_fail_after=i;
      app_callback_code_t code=test_call("reader-memory","{}",2);cx_test_fail_after=-1;
      if(code==APP_CALLBACK_CODE_EXIT) assert(test_return_error && !strcmp(test_error,"APP_MEMORY_LIMIT"));
      else assert(test_event(APP_EVENT_FUTURE_READ,2,APP_WAITABLE_COMPLETED)==APP_CALLBACK_CODE_EXIT);
      assert(test_returns==1 && !test_context);
    }
  }
  test_reset();
}
int main(int argc,char **argv) {
  const char *cases[]={"mixed","complete","yield","ready","first-error","reader-error","reader-memory"};
  if(argc>1)check(argv[1]);else for(size_t i=0;i<sizeof cases/sizeof cases[0];i++)check(cases[i]);
  puts("review: mixed readiness, invalid polls, stable errors and reader failures passed");
}
