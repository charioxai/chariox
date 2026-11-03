#include "chariox/json.h"
#include <math.h>
#include <stdlib.h>
#include <string.h>

#define NONE SIZE_MAX
struct cx_json_doc {
  char *input, *strings;
  cx_json_value *values;
  size_t len, at, used, count, capacity;
  cx_json_status error;
};

static bool fail(cx_json_doc *d, cx_json_status status) { d->error = status; return false; }
static void space(cx_json_doc *d) {
  while (d->at < d->len && strchr(" \t\r\n", d->input[d->at]) && d->input[d->at]) d->at++;
}
/* Reject overlong encodings, UTF-16 surrogates and code points above U+10FFFF. */
static size_t utf8(const unsigned char *s, size_t len) {
  if (!len) return 0;
  if (s[0] < 0x80) return 1;
  size_t n = s[0] >= 0xc2 && s[0] <= 0xdf ? 2 : s[0] >= 0xe0 && s[0] <= 0xef ? 3 : s[0] >= 0xf0 && s[0] <= 0xf4 ? 4 : 0;
  if (!n || n > len) return 0;
  for (size_t i = 1; i < n; i++) if ((s[i] & 0xc0) != 0x80) return 0;
  if ((s[0] == 0xe0 && s[1] < 0xa0) || (s[0] == 0xed && s[1] >= 0xa0) ||
      (s[0] == 0xf0 && s[1] < 0x90) || (s[0] == 0xf4 && s[1] >= 0x90)) return 0;
  return n;
}
static bool hex4(cx_json_doc *d, unsigned *out) {
  if (d->len - d->at < 4) return false;
  unsigned value = 0;
  for (unsigned i = 0; i < 4; i++) {
    unsigned char c = (unsigned char)d->input[d->at++];
    unsigned x = c >= '0' && c <= '9' ? c-'0' : c >= 'a' && c <= 'f' ? c-'a'+10 : c >= 'A' && c <= 'F' ? c-'A'+10 : 16;
    if (x == 16) return false;
    value = value * 16 + x;
  }
  *out = value; return true;
}
static bool string(cx_json_doc *d, cx_text *out) {
  if (d->at == d->len || d->input[d->at++] != '"') return fail(d, CX_JSON_INVALID);
  size_t start = d->used;
  while (d->at < d->len) {
    unsigned char c = (unsigned char)d->input[d->at++];
    if (c == '"') { *out = (cx_text){d->strings + start, d->used - start}; return true; }
    if (c < 0x20) return fail(d, CX_JSON_INVALID);
    if (c != '\\') {
      size_t n = utf8((const unsigned char *)d->input + d->at - 1, d->len - d->at + 1);
      if (!n) return fail(d, CX_JSON_INVALID);
      memcpy(d->strings + d->used, d->input + d->at - 1, n); d->used += n; d->at += n - 1; continue;
    }
    if (d->at == d->len) return fail(d, CX_JSON_INVALID);
    c = (unsigned char)d->input[d->at++];
    switch (c) {
    case '"': case '\\': case '/': d->strings[d->used++] = (char)c; break;
    case 'b': d->strings[d->used++] = '\b'; break;
    case 'f': d->strings[d->used++] = '\f'; break;
    case 'n': d->strings[d->used++] = '\n'; break;
    case 'r': d->strings[d->used++] = '\r'; break;
    case 't': d->strings[d->used++] = '\t'; break;
    case 'u': {
      unsigned code, low;
      if (!hex4(d, &code)) return fail(d, CX_JSON_INVALID);
      if (code >= 0xd800 && code <= 0xdbff) {
        if (d->len-d->at < 6 || d->input[d->at] != '\\' || d->input[d->at+1] != 'u') return fail(d, CX_JSON_INVALID);
        d->at += 2;
        if (!hex4(d, &low) || low < 0xdc00 || low > 0xdfff) return fail(d, CX_JSON_INVALID);
        code = 0x10000 + ((code-0xd800)<<10) + low-0xdc00;
      } else if (code >= 0xdc00 && code <= 0xdfff) return fail(d, CX_JSON_INVALID);
      if (code < 0x80) d->strings[d->used++] = (char)code;
      else {
        if (code >= 0x10000) d->strings[d->used++] = (char)(0xf0 | (code >> 18));
        if (code >= 0x800) d->strings[d->used++] = (char)((code >= 0x10000 ? 0x80 : 0xe0) | ((code >> 12) & 0x3f));
        d->strings[d->used++] = (char)((code >= 0x800 ? 0x80 : 0xc0) | ((code >> 6) & 0x3f));
        d->strings[d->used++] = (char)(0x80 | (code & 0x3f));
      }
      break;
    }
    default: return fail(d, CX_JSON_INVALID);
    }
  }
  return fail(d, CX_JSON_INVALID);
}
static int key_order(const void *a, const void *b) {
  const cx_json_value *x = *(const cx_json_value *const *)a, *y = *(const cx_json_value *const *)b;
  size_t n = x->key.len < y->key.len ? x->key.len : y->key.len;
  int cmp = memcmp(x->key.ptr, y->key.ptr, n);
  return cmp ? cmp : (x->key.len > y->key.len) - (x->key.len < y->key.len);
}
static bool unique(cx_json_doc *d, size_t object) {
  size_t count = d->values[object].count;
  if (count < 2) return true;
  const cx_json_value **sorted = malloc(count * sizeof *sorted);
  if (!sorted) return fail(d, CX_JSON_MEMORY);
  size_t i = 0;
  for (size_t v = d->values[object].first; v != NONE; v = d->values[v].next) sorted[i++] = &d->values[v];
  qsort(sorted, count, sizeof *sorted, key_order);
  bool ok = true;
  for (i = 1; i < count; i++) if (key_order(&sorted[i-1], &sorted[i]) == 0) { ok = false; break; }
  free(sorted);
  return ok || fail(d, CX_JSON_INVALID);
}
static bool digit(char c) { return c >= '0' && c <= '9'; }
static bool number(cx_json_doc *d, size_t id) {
  size_t start = d->at;
  if (d->input[d->at] == '-') d->at++;
  if (d->at == d->len) return fail(d, CX_JSON_INVALID);
  if (d->input[d->at] == '0') d->at++;
  else {
    if (d->input[d->at] < '1' || d->input[d->at] > '9') return fail(d, CX_JSON_INVALID);
    while (d->at < d->len && digit(d->input[d->at])) d->at++;
  }
  if (d->at < d->len && d->input[d->at] == '.') {
    d->at++;
    if (d->at == d->len || !digit(d->input[d->at])) return fail(d, CX_JSON_INVALID);
    while (d->at < d->len && digit(d->input[d->at])) d->at++;
  }
  if (d->at < d->len && (d->input[d->at] == 'e' || d->input[d->at] == 'E')) {
    d->at++;
    if (d->at < d->len && (d->input[d->at] == '+' || d->input[d->at] == '-')) d->at++;
    if (d->at == d->len || !digit(d->input[d->at])) return fail(d, CX_JSON_INVALID);
    while (d->at < d->len && digit(d->input[d->at])) d->at++;
  }
  char saved = d->input[d->at]; d->input[d->at] = 0;
  char *end;
  double n = strtod(d->input + start, &end);
  bool ok = end == d->input + d->at && isfinite(n) && !(trunc(n) == n && fabs(n) > (double)CX_JSON_MAX_SAFE_INTEGER);
  d->input[d->at] = saved;
  if (!ok) return fail(d, CX_JSON_INVALID);
  d->values[id].type = CX_JSON_NUMBER; d->values[id].number = n; return true;
}
static bool value(cx_json_doc *d, size_t depth, size_t *out) {
  if (depth > CX_JSON_MAX_DEPTH || d->count == CX_JSON_MAX_NODES) return fail(d, CX_JSON_LIMIT);
  space(d);
  if (d->at == d->len) return fail(d, CX_JSON_INVALID);
  if (d->count == d->capacity) {
    size_t cap = d->capacity ? d->capacity * 2 : 32;
    cx_json_value *next = realloc(d->values, cap * sizeof *next);
    if (!next) return fail(d, CX_JSON_MEMORY);
    d->values = next; d->capacity = cap;
  }
  size_t id = d->count++;
  d->values[id] = (cx_json_value){.first=NONE, .next=NONE}; *out = id;
  char c = d->input[d->at];
  if (c == '"') { d->values[id].type = CX_JSON_STRING; return string(d, &d->values[id].text); }
  if (c == '[' || c == '{') {
    bool object = c == '{'; char close = object ? '}' : ']';
    d->values[id].type = object ? CX_JSON_OBJECT : CX_JSON_ARRAY;
    d->at++; space(d);
    if (d->at < d->len && d->input[d->at] == close) { d->at++; return true; }
    size_t previous = NONE;
    for (;;) {
      cx_text key = {0};
      if (object) {
        if (!string(d, &key)) return false;
        space(d);
        if (d->at == d->len || d->input[d->at++] != ':') return fail(d, CX_JSON_INVALID);
      }
      size_t child;
      if (!value(d, depth + 1, &child)) return false;
      d->values[child].key = key;
      if (previous == NONE) d->values[id].first = child; else d->values[previous].next = child;
      previous = child; d->values[id].count++;
      space(d);
      if (d->at == d->len) return fail(d, CX_JSON_INVALID);
      c = d->input[d->at++];
      if (c == close) break;
      if (c != ',') return fail(d, CX_JSON_INVALID);
      space(d);
    }
    return !object || unique(d, id);
  }
  const char *literal = c == 'n' ? "null" : c == 't' ? "true" : c == 'f' ? "false" : NULL;
  if (!literal) return number(d, id);
  size_t len = strlen(literal);
  if (d->len-d->at < len || memcmp(d->input + d->at, literal, len)) return fail(d, CX_JSON_INVALID);
  d->at += len; d->values[id].type = c == 'n' ? CX_JSON_NULL : CX_JSON_BOOL; d->values[id].boolean = c == 't'; return true;
}
void cx_json_free(cx_json_doc *d) { if (d) { free(d->input); free(d->strings); free(d->values); free(d); } }
cx_json_status cx_json_parse(const char *text, size_t len, cx_json_doc **out) {
  *out = NULL;
  if (len > CX_JSON_MAX_BYTES) return CX_JSON_LIMIT;
  if (!text || !len) return CX_JSON_INVALID;
  cx_json_doc *d = calloc(1, sizeof *d);
  if (!d) return CX_JSON_MEMORY;
  d->input = malloc(len + 1); d->strings = malloc(len); d->len = len;
  if (!d->input || !d->strings) { cx_json_free(d); return CX_JSON_MEMORY; }
  memcpy(d->input, text, len); d->input[len] = 0;
  size_t root;
  if (!value(d, 0, &root)) { cx_json_status error = d->error; cx_json_free(d); return error; }
  space(d);
  if (d->at != len) { cx_json_free(d); return CX_JSON_INVALID; }
  *out = d; return CX_JSON_OK;
}
const cx_json_value *cx_json_root(const cx_json_doc *d) { return d ? d->values : NULL; }
const cx_json_value *cx_json_get(const cx_json_doc *d, const cx_json_value *object, const char *key) {
  if (!object || object->type != CX_JSON_OBJECT) return NULL;
  size_t len = strlen(key);
  for (size_t i = object->first; i != NONE; i = d->values[i].next)
    if (d->values[i].key.len == len && !memcmp(d->values[i].key.ptr, key, len)) return &d->values[i];
  return NULL;
}
const cx_json_value *cx_json_at(const cx_json_doc *d, const cx_json_value *array, size_t index) {
  if (!array || array->type != CX_JSON_ARRAY || index >= array->count) return NULL;
  size_t i = array->first;
  while (index--) i = d->values[i].next;
  return &d->values[i];
}
bool cx_json_u64(const cx_json_value *v, uint64_t *out) {
  if (!v || v->type != CX_JSON_NUMBER || v->number < 0 || v->number > (double)CX_JSON_MAX_SAFE_INTEGER || trunc(v->number) != v->number) return false;
  *out = (uint64_t)v->number; return true;
}
cx_json_status cx_json_quote(const char *text, size_t len, char **out) {
  *out = NULL;
  if (len > CX_JSON_MAX_BYTES) return CX_JSON_LIMIT;
  if (!text && len) return CX_JSON_INVALID;
  char *buffer = malloc(len * 6 + 3);
  if (!buffer) return CX_JSON_MEMORY;
  size_t used = 0; buffer[used++] = '"';
  const char hex[] = "0123456789abcdef";
  for (size_t i = 0; i < len;) {
    unsigned char c = (unsigned char)text[i];
    size_t n = utf8((const unsigned char *)text + i, len-i);
    if (!n) { free(buffer); return CX_JSON_INVALID; }
    if (c < 0x20) { memcpy(buffer+used, "\\u00", 4); used += 4; buffer[used++] = hex[c>>4]; buffer[used++] = hex[c&15]; }
    else {
      if (c == '"' || c == '\\') buffer[used++] = '\\';
      memcpy(buffer+used, text+i, n); used += n;
    }
    i += n;
  }
  buffer[used++] = '"'; buffer[used] = 0;
  if (used > CX_JSON_MAX_BYTES) { free(buffer); return CX_JSON_LIMIT; }
  *out = buffer; return CX_JSON_OK;
}

size_t cx_json_write_u64(char *buffer, size_t capacity, uint64_t value) {
  if (capacity) buffer[0] = 0;
  if (value > CX_JSON_MAX_SAFE_INTEGER) return 0;
  char reversed[16]; size_t len = 0;
  do { reversed[len++] = (char)('0' + value % 10); value /= 10; } while (value);
  if (len >= capacity) return 0;
  for (size_t i = 0; i < len; i++) buffer[i] = reversed[len-i-1];
  buffer[len] = 0; return len;
}

const cx_json_value *cx_json_first(const cx_json_doc *d, const cx_json_value *container) {
  return container && (container->type == CX_JSON_ARRAY || container->type == CX_JSON_OBJECT) && container->first != NONE ? &d->values[container->first] : NULL;
}
const cx_json_value *cx_json_next(const cx_json_doc *d, const cx_json_value *v) {
  return v && v->next != NONE ? &d->values[v->next] : NULL;
}
