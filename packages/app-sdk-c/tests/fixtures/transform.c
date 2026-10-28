/* R0's transform allocation regression, now driven through the one SDK runtime.
 * The scan reads only the deterministic text built here, never arbitrary JSON. */
#include "chariox/task.h"
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
typedef struct { char result[128]; } state;
static bool register_app(chariox_app_app_types_app_error_t *error) { (void)error; return true; }
static cx_poll resume(cx_task *task) {
  if (!cx_string_is(cx_task_request(task)->name,"transform")) return cx_task_fail(task,"TOOL_NOT_FOUND","unknown tool");
  const cx_json_doc *doc = cx_task_json(task);
  const cx_json_value *root = cx_json_root(doc);
  uint64_t n = 1000, rounds = 1;
  const cx_json_value *value = cx_json_get(doc,root,"n");
  if (value && !cx_json_u64(value,&n)) return cx_task_fail(task,"INVALID_INPUT","invalid n");
  value = cx_json_get(doc,root,"rounds");
  if (value && !cx_json_u64(value,&rounds)) return cx_task_fail(task,"INVALID_INPUT","invalid rounds");
  if (n > 10000 || rounds > 10 || rounds == 0) return cx_task_fail(task,"INVALID_INPUT","fixture bounds exceeded");
  size_t cap = n * 80 + 16;
  char *text = cx_task_alloc(task, cap, NULL), *out = cx_task_alloc(task, cap, NULL);
  if (!text || !out) return CX_DONE;
  /* Preserve R0's nonzero-fill red case even with task-owned zeroed memory. */
  memset(text, 0xbe, cap);
  size_t input_len = 0;
  text[input_len++] = '[';
  for (uint64_t i = 0; i < n; i++)
    input_len += snprintf(text + input_len, cap - input_len, "%s{\"id\":%llu,\"name\":\"item-%llu\",\"score\":%llu,\"tags\":[\"a\",\"b\",\"c\"]}",
                    i ? "," : "", (unsigned long long) i, (unsigned long long) i, (unsigned long long) ((i * 7919) % 100));
  text[input_len++] = ']';
  text[input_len] = '\0';
  uint64_t sum = 0; size_t out_len = 0;
  for (uint64_t round = 0; round < rounds; round++) {
    sum = 0; out_len = 0; out[out_len++] = '[';
    const char *at = text;
    while ((at = strstr(at, "\"id\":")) != NULL) {
      unsigned long long id = strtoull(at + 5, NULL, 10);
      const char *name = strstr(at, "\"name\":\"") + 8;
      const char *score_at = strstr(at, "\"score\":") + 8;
      unsigned long long score = strtoull(score_at, NULL, 10);
      if (score > 50) {
        sum += score;
        out_len += snprintf(out + out_len, cap - out_len, "%s{\"id\":%llu,\"name\":\"", out_len > 1 ? "," : "", id);
        for (const char *c = name; *c != '"'; c++) out[out_len++] = (char) ((*c >= 'a' && *c <= 'z') ? *c - 32 : *c);
        out_len += snprintf(out + out_len, cap - out_len, "\"}");
      }
      at = score_at;
    }
    out[out_len++] = ']';
  }
  state *s = cx_task_state(task);
  int result_len = snprintf(s->result,sizeof s->result,"{\"sum\":%llu,\"inBytes\":%zu,\"outBytes\":%zu}",(unsigned long long)sum,input_len,out_len);
  return cx_task_ok(task,s->result,(size_t)result_len);
}
const cx_app cx_application = {sizeof(state),register_app,resume,NULL};
