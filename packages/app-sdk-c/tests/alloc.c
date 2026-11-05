#include <stddef.h>
#include <stdlib.h>
long cx_test_fail_after = -1;
static int refused(void) {
  if (cx_test_fail_after < 0) return 0;
  if (cx_test_fail_after == 0) return 1;
  cx_test_fail_after--; return 0;
}
void *cx_test_malloc(size_t n) { return refused() ? NULL : malloc(n); }
void *cx_test_calloc(size_t n, size_t s) { return refused() ? NULL : calloc(n,s); }
void *cx_test_realloc(void *p, size_t s) { return refused() ? NULL : realloc(p,s); }
