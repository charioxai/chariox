#ifndef CHARIOX_TEST_ALLOC_H
#define CHARIOX_TEST_ALLOC_H
#include <stddef.h>
extern long cx_test_fail_after;
void *cx_test_malloc(size_t);
void *cx_test_calloc(size_t, size_t);
void *cx_test_realloc(void *, size_t);
#define malloc cx_test_malloc
#define calloc cx_test_calloc
#define realloc cx_test_realloc
#endif
