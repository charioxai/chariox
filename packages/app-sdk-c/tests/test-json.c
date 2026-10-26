#include "chariox/json.h"
#include <assert.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

static void valid(const char *text) {
  cx_json_doc *doc = NULL;
  assert(cx_json_parse(text, strlen(text), &doc) == CX_JSON_OK);
  cx_json_free(doc);
}
static void invalid(const char *text) {
  cx_json_doc *doc = NULL;
  assert(cx_json_parse(text, strlen(text), &doc) != CX_JSON_OK);
  assert(doc == NULL);
}
extern long cx_test_fail_after;
int main(void) {
  valid("{\"a\":[null,true,false,-0,0.125,9007199254740991,-9007199254740991]}");
  valid("{\"a\":1,\"nested\":{\"a\":2},\"unicode\":\"\\uD83D\\uDE00\"}");
  const char *bad[] = {"", "01", "+1", "1.", "1e", "NaN", "1e999", "9007199254740992", "-9007199254740992", "9.007199254740992e15", "[1,]", "{\"a\":1,}", "{}x", "{\"a\":1,\"a\":2}", "{\"a\":1,\"\\u0061\":2}", "{\"\\uD83D\\uDE00\":1,\"😀\":2}", "\"\\uD800\"", "\"\\uDC00\"", "\"\xc0\x80\"", "\"\xed\xa0\x80\"", "\"\xf4\x90\x80\x80\"", "\"\n\"", "\"\\x12\"", "[", "{", "true false"};
  for (size_t i = 0; i < sizeof bad / sizeof bad[0]; i++) invalid(bad[i]);
  char depth[80];
  for (size_t n = 0; n <= 34; n++) {
    memset(depth, '[', n); depth[n] = '0'; memset(depth + n + 1, ']', n); depth[2*n+1] = 0;
    if (n <= 32) valid(depth); else invalid(depth);
  }
  size_t bytes = CX_JSON_MAX_NODES * 2 + 4;
  char *wide = malloc(bytes);
  wide[0] = '[';
  for (size_t i = 0; i < CX_JSON_MAX_NODES; i++) { wide[1+2*i] = '0'; wide[2+2*i] = ','; }
  wide[2*CX_JSON_MAX_NODES-2] = ']'; wide[2*CX_JSON_MAX_NODES-1] = 0; valid(wide);
  cx_json_doc *many=NULL;
  assert(cx_json_parse(wide,strlen(wide),&many)==CX_JSON_OK);
  size_t seen=0;
  for(const cx_json_value *v=cx_json_first(many,cx_json_root(many));v;v=cx_json_next(many,v)) { assert(v->number==0); seen++; }
  assert(seen==CX_JSON_MAX_NODES-1); cx_json_free(many);
  wide[2*CX_JSON_MAX_NODES-2] = ','; wide[2*CX_JSON_MAX_NODES] = ']'; wide[2*CX_JSON_MAX_NODES+1] = 0; invalid(wide); free(wide);
  cx_json_doc *doc = NULL;
  const char input[] = "{\"n\":42,\"s\":\"a\\u0000b\",\"a\":[1,2]}";
  assert(cx_json_parse(input, sizeof input - 1, &doc) == CX_JSON_OK);
  const cx_json_value *root = cx_json_root(doc), *n = cx_json_get(doc, root, "n"), *s = cx_json_get(doc, root, "s");
  uint64_t number = 0;
  assert(cx_json_u64(n, &number) && number == 42);
  assert(s->text.len == 3 && memcmp(s->text.ptr, "a\0b", 3) == 0);
  assert(cx_json_at(doc, cx_json_get(doc, root, "a"), 1)->number == 2);
  assert(cx_json_get(doc, root, "missing") == NULL);
  cx_json_free(doc);
  char *quoted = NULL;
  assert(cx_json_quote("a\0\"\\\n😀", 10, &quoted) == CX_JSON_OK);
  valid(quoted); free(quoted);
  assert(cx_json_quote("\xc0\x80", 2, &quoted) != CX_JSON_OK && quoted == NULL);
  cx_test_fail_after=0;
  assert(cx_json_quote("text",4,&quoted)==CX_JSON_MEMORY && quoted==NULL);
  cx_test_fail_after=-1;
  const char embedded[] = "null\0true";
  assert(cx_json_parse(embedded, sizeof embedded - 1, &doc) != CX_JSON_OK && doc == NULL);
  char *large = malloc(CX_JSON_MAX_BYTES + 1); memset(large, ' ', CX_JSON_MAX_BYTES + 1);
  assert(cx_json_parse(large, CX_JSON_MAX_BYTES + 1, &doc) == CX_JSON_LIMIT); free(large);
  char integer[17];
  assert(cx_json_write_u64(integer,sizeof integer,0)==1 && !strcmp(integer,"0"));
  assert(cx_json_write_u64(integer,sizeof integer,CX_JSON_MAX_SAFE_INTEGER)==16 && !strcmp(integer,"9007199254740991"));
  assert(cx_json_write_u64(integer,sizeof integer,CX_JSON_MAX_SAFE_INTEGER+1)==0 && integer[0]==0);
  assert(cx_json_write_u64(integer,1,0)==0 && integer[0]==0);
  assert(cx_json_write_u64(NULL,0,0)==0);
  const char allocation_json[]="{\"a\":[1,2],\"b\":3}";
  for (long i=0;i<16;i++) {
    cx_test_fail_after=i;
    cx_json_status status=cx_json_parse(allocation_json,sizeof allocation_json-1,&doc);
    assert(status==CX_JSON_OK || status==CX_JSON_MEMORY);
    cx_json_free(doc); cx_test_fail_after=-1;
  }
  puts("JSON: syntax, UTF-8, escaped duplicates, safe numbers, depth/nodes/bytes and helpers passed");
}
