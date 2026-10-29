#ifndef CHARIOX_JSON_H
#define CHARIOX_JSON_H
#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>
#ifdef __cplusplus
extern "C" {
#endif
/* Conservative R0 package limits also fit the wire's 64-level JSON rule. */
#define CX_JSON_MAX_DEPTH 32u
#define CX_JSON_MAX_NODES 32768u
#define CX_JSON_MAX_BYTES (1024u * 1024u)
#define CX_JSON_MAX_SAFE_INTEGER UINT64_C(9007199254740991)
typedef enum { CX_JSON_OK, CX_JSON_INVALID, CX_JSON_LIMIT, CX_JSON_MEMORY } cx_json_status;
typedef enum { CX_JSON_NULL, CX_JSON_BOOL, CX_JSON_NUMBER, CX_JSON_STRING, CX_JSON_ARRAY, CX_JSON_OBJECT } cx_json_type;
typedef struct { const char *ptr; size_t len; } cx_text;
typedef struct cx_json_doc cx_json_doc;
typedef struct {
  cx_json_type type;
  cx_text text, key;
  double number;
  bool boolean;
  size_t first, next, count;
} cx_json_value;
/* Owns its input copy. Returned value pointers live until cx_json_free. */
cx_json_status cx_json_parse(const char *text, size_t len, cx_json_doc **out);
void cx_json_free(cx_json_doc *doc);
const cx_json_value *cx_json_root(const cx_json_doc *doc);
const cx_json_value *cx_json_get(const cx_json_doc *doc, const cx_json_value *object, const char *key);
const cx_json_value *cx_json_at(const cx_json_doc *doc, const cx_json_value *array, size_t index);
/* O(1) sibling iteration; cx_json_at is O(index). Object children expose key. */
const cx_json_value *cx_json_first(const cx_json_doc *doc, const cx_json_value *container);
const cx_json_value *cx_json_next(const cx_json_doc *doc, const cx_json_value *value);
bool cx_json_u64(const cx_json_value *value, uint64_t *out);
/* Writes an interoperable integer with a NUL terminator; 0 means refusal. */
size_t cx_json_write_u64(char *buffer, size_t capacity, uint64_t value);
/* Produces validated JSON string text, including quotes. Caller frees *out. */
cx_json_status cx_json_quote(const char *text, size_t len, char **out);
#ifdef __cplusplus
}
#endif
#endif
