// Spike C App on the async Chariox world (wit/app.wit), built with wasi-sdk 33
// and wit-bindgen 0.63 `c` (callback ABI). Same tools as apps/rust-async so
// the harness cells run unchanged: echo, counter, fetch, compute, spin, hog,
// shared, stream, wait. One instance serves many calls at once: every call is
// a task whose state lives in a heap struct stored in the task-local context.
#include "app.h"
#include <stdarg.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

enum tool { T_ECHO, T_COUNTER, T_FETCH, T_COMPUTE, T_SPIN, T_HOG, T_SHARED, T_STREAM, T_WAIT, T_TRANSFORM, T_UNKNOWN };
static const char *TOOLS[] = { "echo", "counter", "fetch", "compute", "spin", "hog", "shared", "stream", "wait", "transform" };

typedef struct task {
  enum tool tool;
  int step;
  app_waitable_set_t set;
  exports_chariox_app_handlers_future_string_t cancel;  // reader handle
  int cancel_pending;
  app_string_t cancel_reason;
  app_subtask_t pending;                                 // import in flight
  char *input;                                           // JSON text, NUL-terminated
  char url[512];
  uint64_t num;
  // counter
  chariox_app_state_result_option_state_record_app_error_t got;
  chariox_app_state_result_committed_app_error_t committed;
  app_string_t key, value;
  chariox_app_state_check_t check;
  chariox_app_state_write_t write;
  uint64_t next;
  // http
  chariox_app_http_request_t request;
  app_tuple2_string_string_t header;
  chariox_app_http_result_response_app_error_t response;
  chariox_app_http_result_tuple2_response_head_stream_u8_app_error_t fetched;
  uint8_t *buf;
  uint64_t bytes, chunks, work;
  uint32_t hash;
  app_waitable_status_t read_status;
  // shared
  uint64_t n, sequence;
  char connection[32];
} task_t;

// State every concurrent caller shares (as in the Rust App).
static uint64_t shared_counter;
static char shared_connection[32];
static uint64_t shared_requests;
static uint64_t cache_keys[4096];
static size_t cache_len;
static void *hog_kept[4 << 20];
static size_t hog_len;

// --- tiny JSON readers (inputs are small flat objects) ---------------------
static const char *json_find(const char *json, const char *key) {
  char pattern[64];
  snprintf(pattern, sizeof pattern, "\"%s\"", key);
  const char *at = strstr(json, pattern);
  if (!at) return NULL;
  at = strchr(at + strlen(pattern), ':');
  if (!at) return NULL;
  at++;
  while (*at == ' ') at++;
  return at;
}
static uint64_t json_num(const char *json, const char *key, uint64_t fallback) {
  const char *at = json_find(json, key);
  return at && *at >= '0' && *at <= '9' ? strtoull(at, NULL, 10) : fallback;
}
static void json_str(const char *json, const char *key, char *out, size_t cap) {
  out[0] = 0;
  const char *at = json_find(json, key);
  if (!at || *at != '"') return;
  at++;
  size_t n = 0;
  while (*at && *at != '"' && n + 1 < cap) out[n++] = *at++;
  out[n] = 0;
}

// --- completion ---------------------------------------------------------------
static void finish_cleanup(task_t *t) {
  if (t->cancel_pending) {
    // Sync builtins (cancel-read, drop) trap on a waitable still in a set.
    app_waitable_join(t->cancel, 0);
    exports_chariox_app_handlers_future_string_cancel_read(t->cancel);
  }
  exports_chariox_app_handlers_future_string_drop_readable(t->cancel);
  app_waitable_set_drop(t->set);
  // The state import transfers its result to this task; transact only borrows
  // the write. Keep both buffers alive until the task finishes or is cancelled.
  if (t->tool == T_COUNTER) {
    if (!t->got.is_err && t->got.val.ok.is_some) app_string_free(&t->got.val.ok.val.value);
    app_string_free(&t->write.val.put.f1);
  }
  free(t->input);
  free(t->buf);
  free(t);
}

static app_callback_code_t ok_json(task_t *t, char *json) {
  exports_chariox_app_handlers_result_json_app_error_t ret = { .is_err = false };
  app_string_set(&ret.val.ok, json);
  exports_chariox_app_handlers_call_tool_return(ret);
  free(json);
  finish_cleanup(t);
  return APP_CALLBACK_CODE_EXIT;
}

static app_callback_code_t fail(task_t *t, const char *code, const char *message) {
  exports_chariox_app_handlers_result_json_app_error_t ret = { .is_err = true };
  app_string_set(&ret.val.err.code, code);
  app_string_set(&ret.val.err.message, message);
  ret.val.err.retryable.is_some = false;
  exports_chariox_app_handlers_call_tool_return(ret);
  finish_cleanup(t);
  return APP_CALLBACK_CODE_EXIT;
}

static app_callback_code_t fail_app(task_t *t, chariox_app_types_app_error_t *error) {
  char code[128] = {0}, message[256] = {0};
  memcpy(code, error->code.ptr, error->code.len < 127 ? error->code.len : 127);
  memcpy(message, error->message.ptr, error->message.len < 255 ? error->message.len : 255);
  chariox_app_types_app_error_free(error);
  return fail(t, code, message);
}

static char *fmt(const char *format, ...) {
  char *out = malloc(512);
  va_list args;
  va_start(args, format);
  vsnprintf(out, 512, format, args);
  va_end(args);
  return out;
}

// An import returned `status`: either done now (RETURNED) or in flight. In
// flight: join the subtask into the set and wait for its event.
#define AWAIT(t, status, next_step) do { \
  app_subtask_status_t s_ = (status); \
  (t)->step = (next_step); \
  if (APP_SUBTASK_STATE(s_) != APP_SUBTASK_RETURNED) { \
    (t)->pending = APP_SUBTASK_HANDLE(s_); \
    app_waitable_join((t)->pending, (t)->set); \
    return APP_CALLBACK_CODE_WAIT((t)->set); \
  } \
} while (0)

static void make_request(task_t *t, const char *connection) {
  app_string_dup(&t->request.url, t->url);
  app_string_set(&t->request.method, "GET");
  app_string_set(&t->header.f0, "accept");
  app_string_set(&t->header.f1, "text/plain");
  t->request.headers.ptr = &t->header;
  t->request.headers.len = 1;
  t->request.body.is_some = false;
  t->request.connection_id.is_some = connection != NULL;
  if (connection) app_string_dup(&t->request.connection_id.val, connection);
  t->request.operation_id.is_some = false;
}

// --- the tools, as resumable state machines -------------------------------------
static app_callback_code_t step(task_t *t) {
  switch (t->tool) {
  case T_ECHO:
    return ok_json(t, strdup(t->input));

  case T_COUNTER:
    if (t->step == 0) {
      app_string_set(&t->key, "count");
      AWAIT(t, chariox_app_state_get(t->key, &t->got), 1);
    }
    if (t->step == 1) {
      if (t->got.is_err) return fail_app(t, &t->got.val.err);
      uint64_t count = 0;
      t->check.version.is_some = t->got.val.ok.is_some;
      if (t->got.val.ok.is_some) {
        app_string_t value = t->got.val.ok.val.value;
        char *text = malloc(value.len + 1);
        if (!text) return fail(t, "OUT_OF_MEMORY", "could not read counter state");
        memcpy(text, value.ptr, value.len);
        text[value.len] = '\0';
        count = strtoull(text, NULL, 10);
        free(text);
        t->check.version.val = t->got.val.ok.val.version;
      }
      t->next = count + 1;
      app_string_set(&t->check.key, "count");
      char text[32];
      snprintf(text, sizeof text, "%llu", (unsigned long long) t->next);
      t->write.tag = CHARIOX_APP_STATE_WRITE_PUT;
      app_string_set(&t->write.val.put.f0, "count");
      app_string_dup(&t->write.val.put.f1, text);
      chariox_app_state_transaction_t tx = { .schema_version = 0, .checks = { &t->check, 1 }, .writes = { &t->write, 1 } };
      AWAIT(t, chariox_app_state_transact(&tx, &t->committed), 2);
    }
    if (t->committed.is_err) return fail_app(t, &t->committed.val.err);
    return ok_json(t, fmt("{\"count\":%llu,\"revision\":%llu}", (unsigned long long) t->next, (unsigned long long) t->committed.val.ok.revision));

  case T_FETCH:
  case T_WAIT:
    if (t->step == 0) {
      make_request(t, NULL);
      AWAIT(t, chariox_app_http_send(&t->request, &t->response), 1);
    }
    if (t->response.is_err) return fail_app(t, &t->response.val.err);
    return ok_json(t, fmt("{\"status\":%u,\"bytes\":%zu,\"head\":\"%.*s\"}", t->response.val.ok.head.status, t->response.val.ok.body.len,
                          (int) (t->response.val.ok.body.len < 16 ? t->response.val.ok.body.len : 16), (const char *) t->response.val.ok.body.ptr));

  case T_COMPUTE: {
    uint64_t n = json_num(t->input, "n", 1000000);
    static uint64_t counts[1000];
    memset(counts, 0, sizeof counts);
    int32_t h = 0;
    for (uint64_t i = 0; i < n; i++) {
      counts[i % 1000]++;
      h = (int32_t) ((uint32_t) h * 31u + (uint32_t) i);
    }
    return ok_json(t, fmt("{\"h\":%d,\"keys\":1000}", h));
  }

  case T_TRANSFORM: {
    // Same shape as the Rust tool: build n JSON records, then `rounds` times
    // pick score > 50 and upper-case the name. C has no JSON library here, so
    // the "parse" is a scan of the text it just built (not an equal workload).
    uint64_t n = json_num(t->input, "n", 1000), rounds = json_num(t->input, "rounds", 1);
    size_t cap = n * 80 + 16;
    char *text = malloc(cap), *out = malloc(cap);
    if (!text || !out) return fail(t, "APP_MEMORY_LIMIT", "malloc failed (guest-handled)");
    size_t len = 0;
    text[len++] = '[';
    for (uint64_t i = 0; i < n; i++)
      len += snprintf(text + len, cap - len, "%s{\"id\":%llu,\"name\":\"item-%llu\",\"score\":%llu,\"tags\":[\"a\",\"b\",\"c\"]}",
                      i ? "," : "", (unsigned long long) i, (unsigned long long) i, (unsigned long long) ((i * 7919) % 100));
    text[len++] = ']';
    text[len] = '\0';
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
    free(text); free(out);
    return ok_json(t, fmt("{\"sum\":%llu,\"inBytes\":%zu,\"outBytes\":%zu}", (unsigned long long) sum, len, out_len));
  }

  case T_SPIN: {
    volatile uint64_t x = 0;
    for (;;) x++;
  }

  case T_HOG: {
    uint64_t mb = json_num(t->input, "mb", 64);
    char kind[16];
    json_str(t->input, "kind", kind, sizeof kind);
    if (strcmp(kind, "objects") == 0) {
      uint64_t objects = mb * 10000;
      for (uint64_t i = 0; i < objects; i++) {
        char *object = malloc(80);
        // The guest sees the refused memory.grow as a NULL malloc: a C App can
        // handle its own out-of-memory, like QuickJS and Python do.
        if (!object) return fail(t, "APP_MEMORY_LIMIT", "malloc failed (guest-handled)");
        memset(object, 'x', 80);
        if (hog_len < sizeof hog_kept / sizeof hog_kept[0]) hog_kept[hog_len++] = object;  // keep it live
      }
      return ok_json(t, fmt("{\"allocatedMb\":%llu,\"objects\":%llu}", (unsigned long long) mb, (unsigned long long) objects));
    }
    for (uint64_t i = 0; i < mb; i++) {
      uint8_t *chunk = malloc(1 << 20);
      if (!chunk) return fail(t, "APP_MEMORY_LIMIT", "malloc failed (guest-handled)");
      memset(chunk, (int) (i % 251), 1 << 20);
      if (hog_len < sizeof hog_kept / sizeof hog_kept[0]) hog_kept[hog_len++] = chunk;  // keep it live
    }
    return ok_json(t, fmt("{\"allocatedMb\":%llu}", (unsigned long long) mb));
  }

  case T_SHARED:
    if (t->step == 0) {
      // Between awaits the instance runs one task: these updates are atomic.
      shared_counter++;
      t->n = shared_counter;
      if (!shared_connection[0]) snprintf(shared_connection, sizeof shared_connection, "conn-%llu", (unsigned long long) t->n);
      shared_requests++;
      t->sequence = shared_requests;
      strcpy(t->connection, shared_connection);
      uint64_t wait_ms = json_num(t->input, "waitMs", 0);
      if (wait_ms > 0) {
        t->bytes = 0;
        AWAIT(t, wasi_clocks_monotonic_clock_wait_for(wait_ms * 1000000ull), 2);
      } else {
        make_request(t, t->connection);
        AWAIT(t, chariox_app_http_send(&t->request, &t->response), 1);
      }
    }
    if (t->step == 1) {
      if (t->response.is_err) return fail_app(t, &t->response.val.err);
      t->bytes = t->response.val.ok.body.len;
    }
    if (cache_len < sizeof cache_keys / sizeof cache_keys[0]) cache_keys[cache_len++] = t->n;
    return ok_json(t, fmt("{\"n\":%llu,\"sequence\":%llu,\"connection\":\"%s\",\"cached\":%zu}", (unsigned long long) t->n,
                          (unsigned long long) t->sequence, t->connection, cache_len));

  case T_STREAM:
    if (t->step == 0) {
      t->work = json_num(t->input, "work", 0);
      make_request(t, NULL);
      AWAIT(t, chariox_app_http_fetch(&t->request, &t->fetched), 1);
    }
    if (t->step == 1) {
      if (t->fetched.is_err) return fail_app(t, &t->fetched.val.err);
      t->buf = malloc(64 * 1024);
      t->step = 2;
    }
    for (;;) {
      if (t->step == 2) {
        // Issue a read; BLOCKED means the result arrives as a STREAM_READ event.
        t->read_status = chariox_app_http_stream_u8_read(t->fetched.val.ok.f1, t->buf, 64 * 1024);
        if (t->read_status == APP_WAITABLE_STATUS_BLOCKED) {
          app_waitable_join(t->fetched.val.ok.f1, t->set);
          t->step = 3;
          return APP_CALLBACK_CODE_WAIT(t->set);
        }
      }
      t->step = 2;
      app_waitable_state_t state = APP_WAITABLE_STATE(t->read_status);
      if (state == APP_WAITABLE_COMPLETED) {
        uint32_t count = APP_WAITABLE_COUNT(t->read_status);
        t->bytes += count;
        t->chunks++;
        for (uint64_t round = 0; round < t->work; round++)
          for (uint32_t i = 0; i < count; i++) t->hash = t->hash * 31u + t->buf[i];
        continue;
      }
      app_waitable_join(t->fetched.val.ok.f1, 0);
      chariox_app_http_stream_u8_drop_readable(t->fetched.val.ok.f1);
      return ok_json(t, fmt("{\"status\":%u,\"bytes\":%llu,\"chunks\":%llu,\"hash\":%u}", t->fetched.val.ok.f0.status,
                            (unsigned long long) t->bytes, (unsigned long long) t->chunks, t->hash));
    }

  default:
    return fail(t, "TOOL_NOT_FOUND", "unknown tool");
  }
}

// The kernel cancelled the call (or its deadline passed): stop the import or
// stream read in flight, release it, and answer.
static app_callback_code_t cancelled(task_t *t) {
  t->cancel_pending = 0;
  app_waitable_join(t->cancel, 0);
  if (t->pending) {
    app_waitable_join(t->pending, 0);
    app_subtask_status_t status = app_subtask_cancel(t->pending);
    if (APP_SUBTASK_STATE(status) >= APP_SUBTASK_RETURNED) app_subtask_drop(t->pending);
    t->pending = 0;
  }
  if (t->tool == T_STREAM && t->step == 3) {
    app_waitable_join(t->fetched.val.ok.f1, 0);
    chariox_app_http_stream_u8_cancel_read(t->fetched.val.ok.f1);
    chariox_app_http_stream_u8_drop_readable(t->fetched.val.ok.f1);
  }
  char reason[128] = {0};
  memcpy(reason, t->cancel_reason.ptr, t->cancel_reason.len < 127 ? t->cancel_reason.len : 127);
  app_string_free(&t->cancel_reason);
  return fail(t, "CANCELLED", reason);
}

bool exports_chariox_app_handlers_register(exports_chariox_app_handlers_app_error_t *err) {
  (void) err;
  for (size_t i = 0; i < sizeof TOOLS / sizeof TOOLS[0]; i++) {
    app_string_t name;
    app_string_set(&name, TOOLS[i]);
    chariox_app_registry_tool(&name);
  }
  app_string_t message, fields;
  app_string_set(&message, "registered");
  app_string_set(&fields, "{\"tools\":10,\"language\":\"c\"}");
  chariox_app_log_write(CHARIOX_APP_LOG_LEVEL_INFO, &message, &fields);
  return true;
}

app_callback_code_t exports_chariox_app_handlers_call_tool(app_string_t *name, exports_chariox_app_handlers_json_t *input,
                                                           exports_chariox_app_handlers_invocation_context_t *context,
                                                           exports_chariox_app_handlers_future_string_t cancel) {
  task_t *t = calloc(1, sizeof *t);
  t->tool = T_UNKNOWN;
  for (size_t i = 0; i < sizeof TOOLS / sizeof TOOLS[0]; i++)
    if (name->len == strlen(TOOLS[i]) && memcmp(name->ptr, TOOLS[i], name->len) == 0) t->tool = (enum tool) i;
  t->input = malloc(input->len + 1);
  memcpy(t->input, input->ptr, input->len);
  t->input[input->len] = 0;
  json_str(t->input, "url", t->url, sizeof t->url);
  app_string_free(name);
  app_string_free(input);
  chariox_app_types_invocation_context_free(context);
  t->set = app_waitable_set_new();
  t->cancel = cancel;
  app_context_set_0(t);
  // Arm the cancel future: it resolves only if the kernel cancels this call.
  app_waitable_status_t status = exports_chariox_app_handlers_future_string_read(cancel, &t->cancel_reason);
  if (status == APP_WAITABLE_STATUS_BLOCKED) {
    t->cancel_pending = 1;
    app_waitable_join(cancel, t->set);
  } else if (APP_WAITABLE_STATE(status) == APP_WAITABLE_COMPLETED) {
    return cancelled(t);
  }
  return step(t);
}

app_callback_code_t exports_chariox_app_handlers_call_tool_callback(app_event_t *event) {
  task_t *t = app_context_get_0();
  if (event->event == APP_EVENT_FUTURE_READ && event->waitable == t->cancel) {
    if (APP_WAITABLE_STATE(event->code) == APP_WAITABLE_COMPLETED) return cancelled(t);
    t->cancel_pending = 0;  // the kernel dropped the writer: the call ends normally
    app_waitable_join(t->cancel, 0);
    return APP_CALLBACK_CODE_WAIT(t->set);
  }
  if (event->event == APP_EVENT_SUBTASK && event->waitable == t->pending) {
    if (APP_SUBTASK_STATE(event->code) < APP_SUBTASK_RETURNED) return APP_CALLBACK_CODE_WAIT(t->set);
    app_waitable_join(t->pending, 0);
    app_subtask_drop(t->pending);
    t->pending = 0;
    return step(t);
  }
  if (event->event == APP_EVENT_STREAM_READ && t->tool == T_STREAM) {
    t->read_status = event->code;
    return step(t);
  }
  return APP_CALLBACK_CODE_WAIT(t->set);
}
