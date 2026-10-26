#include "abi.h"
#include <stdio.h>
extern long cx_test_fail_after;
static void check(const char *input, const char *expected) {
  test_reset();
  assert(test_call("transform",input,2) == APP_CALLBACK_CODE_EXIT);
  assert(!test_return_error && strcmp(test_result,expected) == 0 && test_returns == 1);
}
int main(void) {
  for (unsigned round=0;round<3;round++) {
    check("{\"n\":0}","{\"sum\":0,\"inBytes\":2,\"outBytes\":2}");
    check("{\"n\":1}","{\"sum\":0,\"inBytes\":57,\"outBytes\":2}");
    check("{\"n\":10,\"rounds\":3}","{\"sum\":351,\"inBytes\":570,\"outBytes\":126}");
    check("{\"n\":1000,\"rounds\":3}","{\"sum\":36750,\"inBytes\":60681,\"outBytes\":14103}");
  }
  for (long i=0;i<30;i++) {
    test_reset(); cx_test_fail_after=i;
    assert(test_call("transform","{\"n\":10}",2)==APP_CALLBACK_CODE_EXIT);
    assert(test_returns==1 && !test_context);
    if (test_return_error) assert(!strcmp(test_error,"APP_MEMORY_LIMIT"));
    cx_test_fail_after=-1;
  }
  test_reset();
  assert(test_call("transform","{\"n\":18446744073709551615}",2)==APP_CALLBACK_CODE_EXIT);
  assert(test_return_error && !strcmp(test_error,"INVALID_JSON"));
  test_reset();
  puts("R0 transform: 12 original entry calls, bounded inputs and 30 allocation failures passed on the SDK");
}
